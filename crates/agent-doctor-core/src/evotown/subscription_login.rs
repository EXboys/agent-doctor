//! Start a Claude Code or Codex login into one subscription directory.
//!
//! The credential file stays in that directory. This path does not write
//! `~/.codex/auth.json` or `~/.claude/.credentials.json`.

use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use super::subscription_pool::{
    remember_reported_subscription_in, ReportedSubscription, SubscriptionPoolRow,
};
use crate::adapters::util::{discover_binary, home_join};
use crate::edition::{product_edition, ProductEdition};
use crate::store::open_settings_store;

const LOGIN_WAIT: Duration = Duration::from_secs(180);

pub fn start_subscription_login(runtime: &str, name: &str) -> Result<SubscriptionPoolRow> {
    if product_edition() != ProductEdition::Team {
        bail!("订阅号登录只在团队版里");
    }
    let runtime = match runtime.trim() {
        "claude-code" => "claude-code",
        "codex" => "codex",
        _ => bail!("先选择 Claude Code 或 Codex。"),
    };
    let name = name.trim();
    if name.is_empty() {
        bail!("先写一个名字，用来认出这个号。");
    }
    let store = open_settings_store()?;
    if let Some(existing) = super::subscription_pool::load_reported_subscriptions_from(&store)?
        .into_iter()
        .find(|item| item.name == name)
    {
        if !existing.runtime.is_empty() && existing.runtime != runtime {
            let other = if existing.runtime == "codex" {
                "Codex"
            } else {
                "Claude Code"
            };
            bail!("这个名字已经是 {other}。");
        }
    }
    let tool = if runtime == "codex" {
        "Codex"
    } else {
        "Claude Code"
    };
    let binary_name = if runtime == "codex" {
        "codex"
    } else {
        "claude"
    };
    let Some(binary) = discover_binary(binary_name).binary_path else {
        bail!("这台电脑还没装 {tool}。");
    };
    let dir = account_dir(runtime, name)?;
    fs::create_dir_all(&dir)?;
    let args: &[&str] = if runtime == "codex" {
        &["login"]
    } else {
        &["auth", "login"]
    };
    let mut cmd = crate::exec::command_for_path(&binary);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if runtime == "codex" {
        cmd.env("CODEX_HOME", &dir);
    } else {
        cmd.env("CLAUDE_CONFIG_DIR", &dir);
    }
    let mut child = cmd.spawn()?;
    let opened = Arc::new(AtomicBool::new(false));
    if let Some(pipe) = child.stdout.take() {
        watch_login_output(pipe, Arc::clone(&opened));
    }
    if let Some(pipe) = child.stderr.take() {
        watch_login_output(pipe, opened);
    }
    let started = Instant::now();
    loop {
        if credential_ready(runtime, &dir) {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        if started.elapsed() > LOGIN_WAIT {
            let _ = child.kill();
            let _ = child.wait();
            bail!("登录还没完成。在打开的窗口里登完，再回到这里点一次。");
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                if credential_ready(runtime, &dir) {
                    break;
                }
                bail!("登录还没完成。在打开的窗口里登完，再回到这里点一次。");
            }
            Ok(None) => thread::sleep(Duration::from_millis(400)),
            Err(_) => break,
        }
    }
    if !credential_ready(runtime, &dir) {
        bail!("登录还没完成。在打开的窗口里登完，再回到这里点一次。");
    }
    let mut account = ReportedSubscription {
        name: name.to_string(),
        remaining: 0,
        expires_at: "未知".into(),
        disabled: false,
        cap: None,
        holder: None,
        proxy: None,
        runtime: runtime.to_string(),
        quota_known: false,
    };
    if let Some(existing) = super::subscription_pool::load_reported_subscriptions_from(&store)?
        .into_iter()
        .find(|item| item.name == name)
    {
        if !existing.runtime.is_empty() && existing.runtime != runtime {
            let other = if existing.runtime == "codex" {
                "Codex"
            } else {
                "Claude Code"
            };
            bail!("这个名字已经是 {other}。");
        }
        account.remaining = existing.remaining;
        account.expires_at = existing.expires_at;
        account.disabled = existing.disabled;
        account.cap = existing.cap;
        account.holder = existing.holder;
        account.proxy = existing.proxy;
        account.quota_known = existing.quota_known;
    }
    remember_reported_subscription_in(&store, account)?;
    super::subscription_pool::list_subscription_pool_from(&store)
        .into_iter()
        .find(|row| row.name == name)
        .ok_or_else(|| anyhow::anyhow!("这次没记下这个号。"))
}

fn account_dir(runtime: &str, name: &str) -> Result<PathBuf> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        bail!("这个名字不能用来建目录。");
    }
    Ok(home_join(".agent-doctor/subscriptions")
        .join(runtime)
        .join(name))
}

fn credential_ready(runtime: &str, dir: &Path) -> bool {
    if runtime == "codex" {
        read_json(&dir.join("auth.json"))
            .map(|value| {
                let mode = value
                    .get("auth_mode")
                    .and_then(|item| item.as_str())
                    .unwrap_or("");
                mode.eq_ignore_ascii_case("chatgpt")
                    || value.get("tokens").is_some()
                    || value.get("refresh_token").is_some()
                    || value.get("access_token").is_some()
            })
            .unwrap_or(false)
    } else {
        read_json(&dir.join(".credentials.json"))
            .map(|value| {
                value.get("claudeAiOauth").is_some()
                    || value.get("accessToken").is_some()
                    || value.get("access_token").is_some()
            })
            .unwrap_or(false)
    }
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn watch_login_output<R>(pipe: R, opened: Arc<AtomicBool>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut pending = String::new();
        let mut chunk = [0_u8; 512];
        while let Ok(n) = reader.read(&mut chunk) {
            if n == 0 {
                break;
            }
            pending.push_str(&String::from_utf8_lossy(&chunk[..n]));
            if let Some(url) = first_https(&pending) {
                if !opened.swap(true, Ordering::SeqCst) {
                    let _ = crate::session_launch::open_url(&url);
                }
            }
        }
        if !opened.load(Ordering::SeqCst) {
            if let Some(url) = https_at_end(&pending) {
                let _ = crate::session_launch::open_url(&url);
            }
        }
    });
}

/// Wait until whitespace follows the link. A half-printed authorize URL is
/// rejected by the login site.
fn first_https(text: &str) -> Option<String> {
    let start = text.find("https://")?;
    let rest = &text[start..];
    let end = rest.find(char::is_whitespace)?;
    trimmed_https(&rest[..end])
}

fn https_at_end(text: &str) -> Option<String> {
    let start = text.find("https://")?;
    let rest = &text[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    trimmed_https(&rest[..end])
}

fn trimmed_https(raw: &str) -> Option<String> {
    let url = raw.trim_end_matches([')', ']', '>', '"', '\'', '.', ',']);
    if url.len() > "https://".len() {
        Some(url.to_string())
    } else {
        None
    }
}
