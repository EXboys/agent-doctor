//! Models a subscription account actually offers.
//!
//! Codex answers `model/list`. Claude Code answers `GET /v1/models`. The saved
//! list is what the chat may show and send. A name that is not on it is
//! replaced with the account default.

use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::util::{discover_binary, home_join};

const CODEX_ID: &str = "codex";
const CLAUDE_ID: &str = "claude-code";
const QODER_ID: &str = "qoder";
const CURSOR_ID: &str = "cursor";
const LIST_WAIT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountModelList {
    pub models: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

impl AccountModelList {
    /// The id this account will accept. A name it did not list becomes the default.
    pub fn choose(&self, model: Option<&str>) -> Option<String> {
        if self.models.is_empty() {
            return None;
        }
        if let Some(model) = model
            .map(str::trim)
            .filter(|id| !id.is_empty() && !id.eq_ignore_ascii_case("auto"))
        {
            if let Some(id) = self.canonical(model) {
                return Some(id);
            }
        }
        self.default_model
            .clone()
            .filter(|id| self.canonical(id).is_some())
            .or_else(|| self.models.first().cloned())
    }

    pub fn allows(&self, model: &str) -> bool {
        self.canonical(model).is_some()
    }

    fn canonical(&self, model: &str) -> Option<String> {
        let tail = model.rsplit(['/', ':']).next().unwrap_or(model).trim();
        self.models
            .iter()
            .find(|id| id.eq_ignore_ascii_case(model) || id.eq_ignore_ascii_case(tail))
            .cloned()
    }
}

pub fn load_account_models(runtime_id: &str) -> Option<AccountModelList> {
    let raw = crate::store::open_settings_store()
        .ok()?
        .get_meta(&models_key(runtime_id))
        .ok()??;
    let list = serde_json::from_str::<AccountModelList>(&raw).ok()?;
    if list.models.is_empty() {
        None
    } else {
        Some(list)
    }
}

fn store_account_models(runtime_id: &str, list: &AccountModelList) -> Result<()> {
    let store = crate::store::open_settings_store()?;
    store.set_meta(&models_key(runtime_id), &serde_json::to_string(list)?)
}

fn models_key(runtime_id: &str) -> String {
    format!("native_models:{runtime_id}")
}

/// Ask the signed-in account which models it offers, and remember the answer.
pub fn refresh_account_models(runtime_id: &str) -> Result<AccountModelList> {
    let list = match runtime_id {
        CODEX_ID => fetch_codex_models()?,
        CLAUDE_ID => fetch_claude_models()?,
        QODER_ID => fetch_qoder_models()?,
        CURSOR_ID => fetch_cursor_models()?,
        other => bail!("subscription models are not supported for {other}"),
    };
    if list.models.is_empty() && matches!(runtime_id, CODEX_ID | CLAUDE_ID) {
        bail!("这个账号的模型名单没取到，这一轮没有发出去。再发一次试试。");
    }
    store_account_models(runtime_id, &list)?;
    Ok(list)
}

/// The saved list, or a fresh answer from the account when nothing is saved.
pub fn ensure_account_models(runtime_id: &str) -> Result<AccountModelList> {
    if let Some(list) = load_account_models(runtime_id) {
        return Ok(list);
    }
    refresh_account_models(runtime_id)
}

fn fetch_codex_models() -> Result<AccountModelList> {
    let bin = discover_binary("codex")
        .binary_path
        .context("codex is not installed")?;
    // When a personal gateway is active, `~/.codex/auth.json` may have been
    // stashed to `codex-chatgpt-auth.json`. Give `model/list` a temporary
    // `CODEX_HOME` seeded with that stashed login without disturbing `~/.codex`.
    let temp_codex_home = if !home_join(".codex/auth.json").exists() {
        crate::setup::codex_chatgpt_stash_path()
            .filter(|stash| stash.is_file())
            .and_then(|stash| {
                let dir = tempfile::tempdir().ok()?;
                std::fs::copy(&stash, dir.path().join("auth.json")).ok()?;
                Some(dir)
            })
    } else {
        None
    };
    let mut cmd = crate::exec::command_for_path(&bin);
    cmd.arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for key in [
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
    ] {
        cmd.env_remove(key);
    }
    if let Some(ref dir) = temp_codex_home {
        cmd.env("CODEX_HOME", dir.path());
    } else {
        cmd.env_remove("CODEX_HOME");
    }
    let mut child = cmd.spawn().context("failed to start Codex")?;
    let mut stdin = child.stdin.take().context("Codex is missing stdin")?;
    let stdout = child.stdout.take().context("Codex is missing stdout")?;
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(line.trim().to_string()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    for payload in [
        r#"{"method":"initialize","id":1,"params":{"clientInfo":{"name":"agent_doctor","title":"Agent Doctor","version":"0"}}}"#,
        r#"{"method":"initialized","params":{}}"#,
        r#"{"method":"model/list","id":2,"params":{}}"#,
    ] {
        writeln!(stdin, "{payload}")?;
    }
    let _ = stdin.flush();
    let deadline = std::time::Instant::now() + LIST_WAIT;
    let mut parsed = None;
    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(remaining.min(Duration::from_millis(200))) {
            Ok(line) => {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if value.get("id").and_then(Value::as_u64) == Some(2) {
                    if let Some(error) = value.get("error") {
                        let _ = child.kill();
                        let _ = child.wait();
                        bail!(
                            "Codex model list failed: {}",
                            error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("model/list")
                        );
                    }
                    parsed = Some(parse_codex_model_list(
                        value.get("result").unwrap_or(&Value::Null),
                    ));
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    parsed.context("这个账号的模型名单没取到，这一轮没有发出去。再发一次试试。")
}

fn fetch_claude_models() -> Result<AccountModelList> {
    let token = claude_access_token().context("Claude is not signed in")?;
    let client = reqwest::blocking::Client::builder()
        .timeout(LIST_WAIT)
        .build()?;
    let response = client
        .get("https://api.anthropic.com/v1/models?beta=true&limit=100")
        .header("Authorization", format!("Bearer {token}"))
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .context("Claude model list failed")?;
    let status = response.status();
    let body: Value = response.json().unwrap_or(Value::Null);
    if !status.is_success() {
        bail!("Claude model list failed: {status}");
    }
    Ok(parse_claude_model_list(&body))
}

fn claude_access_token() -> Option<String> {
    if let Ok(raw) = std::fs::read_to_string(home_join(".claude/.credentials.json")) {
        if let Some(token) = parse_claude_oauth_token(&raw) {
            return Some(token);
        }
    }
    #[cfg(all(target_os = "macos", not(test)))]
    {
        let output = std::process::Command::new("security") // raw-spawn-ok
            .args([
                "find-generic-password",
                "-s",
                "Claude Code-credentials",
                "-w",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if output.status.success() {
            let raw = String::from_utf8(output.stdout).ok()?;
            return parse_claude_oauth_token(raw.trim());
        }
    }
    None
}

fn parse_claude_oauth_token(raw: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(raw).ok()?;
    let oauth = value.get("claudeAiOauth").unwrap_or(&value);
    ["accessToken", "access_token"]
        .iter()
        .find_map(|key| oauth.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

pub fn parse_codex_model_list(result: &Value) -> AccountModelList {
    let mut models = Vec::new();
    let mut default_model = None;
    let items = result
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in items {
        if item.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(id) = item
            .get("model")
            .and_then(Value::as_str)
            .or_else(|| item.get("id").and_then(Value::as_str))
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        if models.iter().any(|seen: &String| seen == id) {
            continue;
        }
        if item.get("isDefault").and_then(Value::as_bool) == Some(true) && default_model.is_none() {
            default_model = Some(id.to_string());
        }
        models.push(id.to_string());
    }
    AccountModelList {
        models,
        default_model,
    }
}

pub fn parse_claude_model_list(result: &Value) -> AccountModelList {
    let mut models = Vec::new();
    let items = result
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in items {
        let Some(id) = item
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        if models.iter().any(|seen: &String| seen == id) {
            continue;
        }
        models.push(id.to_string());
    }
    let default_model = models.first().cloned();
    AccountModelList {
        models,
        default_model,
    }
}

fn fetch_qoder_models() -> Result<AccountModelList> {
    let bin = crate::adapters::qoder_cli_path().context("Qoder CLI is not installed")?;
    let mut cmd = crate::exec::command_for_path(&bin);
    // Force `useExternal: false` + empty `providers` so `--list-models` always
    // queries the signed-in Qoder account instead of an external custom provider.
    cmd.arg("--list-models")
        .arg("--settings")
        .arg(r#"{"providers":{},"security":{"auth":{"useExternal":false}}}"#)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "QODER_MODEL",
    ] {
        cmd.env_remove(key);
    }
    let out = run_cli_with_timeout(cmd, LIST_WAIT)?;
    Ok(parse_qoder_model_list(&out))
}

fn fetch_cursor_models() -> Result<AccountModelList> {
    let bin = crate::adapters::cursor_cli_path().context("Cursor CLI is not installed")?;
    let mut cmd = crate::exec::command_for_path(&bin);
    cmd.arg("models")
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
    ] {
        cmd.env_remove(key);
    }
    let out = run_cli_with_timeout(cmd, LIST_WAIT)?;
    Ok(parse_cursor_model_list(&out))
}

fn run_cli_with_timeout(mut cmd: std::process::Command, timeout: Duration) -> Result<String> {
    let mut child = cmd.spawn().context("failed to spawn model list command")?;
    let stdout = child.stdout.take().context("missing stdout")?;
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut reader, &mut text);
        let _ = tx.send(text);
    });
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            let out = rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default();
            return Ok(out);
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    bail!("读取账号模型列表超时")
}

fn strip_ansi(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

/// Parse `qodercli --list-models` stdout (`MODEL\nAuto\nUltimate\n...`).
/// Skips custom provider entries (`(agent-doctor/...)`) and warning banners.
pub fn parse_qoder_model_list(stdout: &str) -> AccountModelList {
    let clean = strip_ansi(stdout);
    let mut models = Vec::new();
    let mut default_model = None;
    for raw_line in clean.lines() {
        let line = raw_line.trim();
        if line.is_empty()
            || line.eq_ignore_ascii_case("MODEL")
            || line.starts_with("Warning:")
            || line.starts_with("Not logged in")
            || line.starts_with("Failed to")
            || line.contains("(agent-doctor/")
        {
            continue;
        }
        // Custom provider format is `Display (provider/key)`; take the key inside parens if present,
        // otherwise keep Qoder's native model name (`Auto`, `Ultimate`, `Performance`, `Efficient`, `Lite`, ...).
        let id = if let Some((left, right)) = line.rsplit_once(" (") {
            if right.ends_with(')') {
                let inner = right.trim_end_matches(')').trim();
                if !inner.is_empty() && !inner.contains(' ') && !inner.ends_with('…') {
                    inner
                } else {
                    left.trim()
                }
            } else {
                line
            }
        } else {
            line
        };
        if id.is_empty() || models.iter().any(|seen: &String| seen == id) {
            continue;
        }
        if id.eq_ignore_ascii_case("auto") && default_model.is_none() {
            default_model = Some(id.to_string());
        }
        models.push(id.to_string());
    }
    if default_model.is_none() {
        default_model = models.first().cloned();
    }
    AccountModelList {
        models,
        default_model,
    }
}

/// Parse `agent models` (Cursor CLI) stdout:
/// `Available models\n\nclaude-4.5-sonnet - Claude 4.5 Sonnet (current, default)\n...`
pub fn parse_cursor_model_list(stdout: &str) -> AccountModelList {
    let clean = strip_ansi(stdout);
    let mut models = Vec::new();
    let mut default_model = None;
    for raw_line in clean.lines() {
        let line = raw_line.trim();
        if line.is_empty()
            || line.eq_ignore_ascii_case("Available models")
            || line.starts_with("No models available")
            || line.starts_with("Tip:")
            || line.starts_with("Authentication ")
            || line.starts_with("Not logged in")
        {
            continue;
        }
        let is_default =
            line.contains("(default)") || line.contains(", default)") || line.contains("(default,");
        let head = line
            .split(" - ")
            .next()
            .unwrap_or(line)
            .split(" (")
            .next()
            .unwrap_or(line)
            .trim();
        if head.is_empty() || head.contains(' ') {
            continue;
        }
        if models.iter().any(|seen: &String| seen == head) {
            continue;
        }
        if is_default && default_model.is_none() {
            default_model = Some(head.to_string());
        }
        models.push(head.to_string());
    }
    if default_model.is_none() {
        default_model = models.first().cloned();
    }
    AccountModelList {
        models,
        default_model,
    }
}

pub fn account_model_allowed(allowed: &[String], model: &str) -> bool {
    AccountModelList {
        models: allowed.to_vec(),
        default_model: None,
    }
    .allows(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn account_list_keeps_only_models_the_account_named() {
        let list = parse_codex_model_list(&json!({
            "data": [
                {"id": "gpt-5.6-sol", "model": "gpt-5.6-sol", "hidden": false, "isDefault": true},
                {"id": "gpt-5.5", "model": "gpt-5.5", "hidden": false},
                {"id": "hidden-one", "model": "hidden-one", "hidden": true}
            ]
        }));
        assert_eq!(list.models, vec!["gpt-5.6-sol", "gpt-5.5"]);
        assert_eq!(list.choose(Some("gpt-5.5")).as_deref(), Some("gpt-5.5"));
        assert_eq!(
            list.choose(Some("openai/gpt-5.5")).as_deref(),
            Some("gpt-5.5")
        );
        assert_eq!(list.choose(Some("glm-5.3")).as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(list.choose(Some("llama-3")).as_deref(), Some("gpt-5.6-sol"));
        assert!(!list.allows("gpt-5.4"));

        let claude = parse_claude_model_list(&json!({
            "data": [
                {"id": "claude-sonnet-5", "type": "model"},
                {"id": "claude-opus-5", "type": "model"}
            ]
        }));
        assert_eq!(
            claude.choose(Some("gpt-5.6-sol")).as_deref(),
            Some("claude-sonnet-5")
        );
        assert_eq!(
            claude.choose(Some("claude-opus-5")).as_deref(),
            Some("claude-opus-5")
        );

        let qoder = parse_qoder_model_list(
            "MODEL\nAuto\nUltimate\nPerformance\nEfficient\nLite\nagent-doctor (agent-doctor/deepseek-flash)\n",
        );
        assert_eq!(
            qoder.models,
            vec!["Auto", "Ultimate", "Performance", "Efficient", "Lite"]
        );
        assert_eq!(qoder.default_model.as_deref(), Some("Auto"));
        assert_eq!(qoder.choose(Some("auto")).as_deref(), Some("Auto"));
        assert_eq!(qoder.choose(Some("Ultimate")).as_deref(), Some("Ultimate"));

        let empty_qoder =
            parse_qoder_model_list("Warning: no models available for your account.\n");
        assert!(empty_qoder.models.is_empty());

        let cursor = parse_cursor_model_list(
            "\x1b[2mAvailable models\x1b[22m\n\n\x1b[36mgpt-5\x1b[39m - GPT-5\n\x1b[32mclaude-4.5-sonnet\x1b[39m - Claude 4.5 Sonnet \x1b[2m(current, default)\x1b[22m\n\nTip: use --model <id> to switch.\n",
        );
        assert_eq!(cursor.models, vec!["gpt-5", "claude-4.5-sonnet"]);
        assert_eq!(cursor.default_model.as_deref(), Some("claude-4.5-sonnet"));
    }
}
