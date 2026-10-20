//! Codex and Claude Code can use their own subscription login.
//!
//! The choice is stored per tool. A provider switch skips a tool that is on
//! its subscription. The provider page only offers the login when that
//! login site answers, or when the tool is already on the subscription.

use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::Value;

use crate::adapters::util::{discover_binary, home_join};
use crate::edition::ensure_edition_allows_mode;
use crate::setup::merge::{
    align_codex_subscription_configs, apply_codex_slot, clear_codex_chatgpt_auth_for_gateway,
    known_claude_subscription_dirs, known_codex_subscription_config_paths,
    repair_claude_subscription_configs, strip_codex_project_denied_provider_keys,
    CODEX_PERSONAL_SLOT,
};
use crate::setup::{
    anthropic_gateway_for_provider_url, apply_claude_code_with_model, backup_file,
    load_active_personal_provider, MODE_PERSONAL, PROTOCOL_ANTHROPIC,
};
use crate::store::open_settings_store;

const MODE_SUBSCRIPTION: &str = "subscription";
const CODEX_ID: &str = "codex";
const CLAUDE_ID: &str = "claude-code";
const CODEX_LOGIN_URL: &str = "https://auth.openai.com";
const CLAUDE_LOGIN_URL: &str = "https://claude.ai";
const LOGIN_WAIT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Serialize)]
pub struct NativeAccountRow {
    pub runtime_id: String,
    pub using_subscription: bool,
    pub logged_in: bool,
    pub reachable: bool,
    pub visible: bool,
    pub provider_name: String,
    /// Models this signed-in account listed. Empty until that list arrives.
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeAccountStatus {
    pub rows: Vec<NativeAccountRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeAccountSwitch {
    pub runtime_id: String,
    pub using_subscription: bool,
    /// `using` | `reverted` | `login_incomplete` | `unreachable` | `missing_tool` | `no_provider` | `unsupported`
    pub outcome: String,
    pub provider_name: String,
}

pub fn is_native_subscription(runtime_id: &str) -> bool {
    matches!(runtime_id, CODEX_ID | CLAUDE_ID) && read_mode(runtime_id) == MODE_SUBSCRIPTION
}

pub fn native_account_status(probe: bool) -> NativeAccountStatus {
    if ensure_edition_allows_mode(MODE_PERSONAL).is_err() {
        return NativeAccountStatus { rows: Vec::new() };
    }
    let provider_name = active_provider_name();
    let (codex_up, claude_up) = if probe {
        probe_login_sites()
    } else {
        (false, false)
    };
    NativeAccountStatus {
        rows: vec![
            row(CODEX_ID, codex_up, &provider_name),
            row(CLAUDE_ID, claude_up, &provider_name),
        ],
    }
}

pub fn use_native_subscription(runtime_id: &str) -> Result<NativeAccountSwitch> {
    ensure_edition_allows_mode(MODE_PERSONAL)?;
    let runtime_id = known_runtime(runtime_id)?;
    let provider_name = active_provider_name();
    if runtime_id == CODEX_ID {
        restore_codex_auth();
    }
    if logged_in(runtime_id) {
        finish_subscription(runtime_id)?;
        write_mode(runtime_id, MODE_SUBSCRIPTION)?;
        if runtime_id == CODEX_ID {
            publish_codex_subscription_auth(None);
        }
        return Ok(switch(runtime_id, true, "using", provider_name));
    }
    if discover_binary(binary_name(runtime_id))
        .binary_path
        .is_none()
    {
        return Ok(switch(runtime_id, false, "missing_tool", provider_name));
    }
    if !host_reachable(login_url(runtime_id)) {
        return Ok(switch(runtime_id, false, "unreachable", provider_name));
    }
    let _ = spawn_official_login(runtime_id);
    if !logged_in(runtime_id) {
        return Ok(switch(runtime_id, false, "login_incomplete", provider_name));
    }
    finish_subscription(runtime_id)?;
    write_mode(runtime_id, MODE_SUBSCRIPTION)?;
    if runtime_id == CODEX_ID {
        publish_codex_subscription_auth(None);
    }
    Ok(switch(runtime_id, true, "using", provider_name))
}

/// The saved sign-in can no longer refresh. Drop it and run the login again.
pub fn resign_native_subscription(runtime_id: &str) -> Result<NativeAccountSwitch> {
    ensure_edition_allows_mode(MODE_PERSONAL)?;
    let runtime_id = known_runtime(runtime_id)?;
    forget_saved_login(runtime_id);
    use_native_subscription(runtime_id)
}

pub fn use_current_service(runtime_id: &str) -> Result<NativeAccountSwitch> {
    ensure_edition_allows_mode(MODE_PERSONAL)?;
    let runtime_id = known_runtime(runtime_id)?;
    let Some(provider) = load_active_personal_provider()? else {
        return Ok(switch(
            runtime_id,
            is_native_subscription(runtime_id),
            "no_provider",
            String::new(),
        ));
    };
    let name = provider.name.clone();
    match runtime_id {
        CODEX_ID => {
            if provider.protocol == PROTOCOL_ANTHROPIC {
                return Ok(switch(runtime_id, true, "unsupported", name));
            }
            stash_codex_auth();
            if let Err(error) = apply_codex_slot(
                &provider.url,
                &provider.api_key,
                Some(provider.model.as_str()),
                Some(CODEX_PERSONAL_SLOT),
            ) {
                restore_codex_auth();
                return Err(error);
            }
            // ChatGPT tokens override the address just written.
            let _ = clear_codex_chatgpt_auth_for_gateway();
        }
        CLAUDE_ID => {
            let Some(url) = claude_target_url(&provider.url, &provider.protocol) else {
                return Ok(switch(runtime_id, true, "unsupported", name));
            };
            apply_claude_code_with_model(&url, &provider.api_key, Some(provider.model.as_str()))?;
        }
        _ => bail!("unknown runtime"),
    }
    write_mode(runtime_id, "service")?;
    Ok(switch(runtime_id, false, "reverted", name))
}

fn switch(
    runtime_id: &str,
    using_subscription: bool,
    outcome: &str,
    provider_name: String,
) -> NativeAccountSwitch {
    NativeAccountSwitch {
        runtime_id: runtime_id.to_string(),
        using_subscription,
        outcome: outcome.to_string(),
        provider_name,
    }
}

fn row(runtime_id: &str, reachable: bool, provider_name: &str) -> NativeAccountRow {
    let using = is_native_subscription(runtime_id);
    let installed = discover_binary(binary_name(runtime_id))
        .binary_path
        .is_some();
    NativeAccountRow {
        runtime_id: runtime_id.to_string(),
        using_subscription: using,
        logged_in: logged_in(runtime_id),
        reachable,
        visible: account_row_visible(using, reachable, installed),
        provider_name: provider_name.to_string(),
        models: if using {
            crate::setup::load_account_models(runtime_id)
                .map(|list| list.models)
                .unwrap_or_default()
        } else {
            Vec::new()
        },
    }
}

fn finish_subscription(runtime_id: &str) -> Result<()> {
    let models = crate::setup::refresh_account_models(runtime_id).ok();
    release_provider_override(runtime_id, models.as_ref())
}

fn known_runtime(runtime_id: &str) -> Result<&'static str> {
    match runtime_id {
        CODEX_ID => Ok(CODEX_ID),
        CLAUDE_ID => Ok(CLAUDE_ID),
        other => bail!("subscription login is only for Codex and Claude Code, not {other}"),
    }
}

fn binary_name(runtime_id: &str) -> &'static str {
    if runtime_id == CODEX_ID {
        "codex"
    } else {
        "claude"
    }
}

fn login_url(runtime_id: &str) -> &'static str {
    if runtime_id == CODEX_ID {
        CODEX_LOGIN_URL
    } else {
        CLAUDE_LOGIN_URL
    }
}

fn mode_key(runtime_id: &str) -> String {
    format!("native_login:{runtime_id}")
}

fn read_mode(runtime_id: &str) -> String {
    open_settings_store()
        .and_then(|store| store.get_meta(&mode_key(runtime_id)))
        .ok()
        .flatten()
        .unwrap_or_else(|| "service".to_string())
}

fn write_mode(runtime_id: &str, mode: &str) -> Result<()> {
    let store = open_settings_store()?;
    store.set_meta(&mode_key(runtime_id), mode)
}

fn active_provider_name() -> String {
    load_active_personal_provider()
        .ok()
        .flatten()
        .map(|entry| entry.name)
        .unwrap_or_default()
}

fn claude_target_url(url: &str, protocol: &str) -> Option<String> {
    if protocol == PROTOCOL_ANTHROPIC {
        let trimmed = url.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    } else {
        anthropic_gateway_for_provider_url(url)
    }
}

fn logged_in(runtime_id: &str) -> bool {
    if runtime_id == CODEX_ID {
        read_json(&home_join(".codex/auth.json"))
            .map(|value| codex_auth_is_chatgpt(&value))
            .unwrap_or(false)
    } else {
        read_json(&home_join(".claude/.credentials.json"))
            .map(|value| claude_credentials_are_oauth(&value))
            .unwrap_or(false)
    }
}

fn read_json(path: &Path) -> Option<Value> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub(crate) fn codex_auth_is_chatgpt(value: &Value) -> bool {
    let mode = value.get("auth_mode").and_then(Value::as_str).unwrap_or("");
    if mode.eq_ignore_ascii_case("chatgpt") {
        return true;
    }
    value.get("tokens").is_some()
        || value.get("refresh_token").is_some()
        || value.get("access_token").is_some()
}

pub(crate) fn claude_credentials_are_oauth(value: &Value) -> bool {
    value.get("claudeAiOauth").is_some()
        || value.get("accessToken").is_some()
        || value.get("access_token").is_some()
}

pub(crate) fn account_row_visible(
    using_subscription: bool,
    reachable: bool,
    installed: bool,
) -> bool {
    using_subscription || (reachable && installed)
}

fn release_provider_override(
    runtime_id: &str,
    models: Option<&crate::setup::AccountModelList>,
) -> Result<()> {
    let allowed = models.map(|list| list.models.as_slice()).unwrap_or(&[]);
    if runtime_id == CODEX_ID {
        let paths = known_codex_subscription_config_paths();
        for path in &paths {
            if path.exists() {
                let _ = backup_file(path)?;
                let _ = strip_codex_project_denied_provider_keys(path);
            }
        }
        // A model the account listed stays. Anything else is removed.
        align_codex_subscription_configs(&paths, None, allowed)?;
        return Ok(());
    }
    for dir in known_claude_subscription_dirs() {
        for path in crate::setup::merge::claude_subscription_settings_paths(&dir) {
            if path.exists() {
                let _ = backup_file(&path)?;
            }
        }
        // A settings file that cannot be read is left as it is. Sending later
        // stops instead of replacing it with an empty file.
        repair_claude_subscription_configs(&dir, None, allowed)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn strip_claude_provider_keys(root: &mut Value) {
    let Some(object) = root.as_object_mut() else {
        return;
    };
    object.remove("anthropicBaseUrl");
    let Some(env) = object.get_mut("env").and_then(Value::as_object_mut) else {
        return;
    };
    for key in [
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
    ] {
        env.remove(key);
    }
}

fn stash_path() -> Option<PathBuf> {
    crate::store::agent_doctor_config_dir().map(|dir| dir.join("codex-chatgpt-auth.json"))
}

fn stash_codex_auth() {
    let Some(dest) = stash_path() else {
        return;
    };
    let src = home_join(".codex/auth.json");
    let Ok(raw) = fs::read_to_string(&src) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return;
    };
    if !codex_auth_is_chatgpt(&value) {
        return;
    }
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(dest, raw);
}

fn forget_saved_login(runtime_id: &str) {
    if runtime_id == CODEX_ID {
        let _ = fs::remove_file(home_join(".codex/auth.json"));
        if let Some(path) = stash_path() {
            let _ = fs::remove_file(path);
        }
        return;
    }
    let _ = fs::remove_file(home_join(".claude/.credentials.json"));
}

/// Replace a project Codex home's account file with the one just written to `~/.codex`.
pub(crate) fn publish_codex_subscription_auth(codex_home: Option<&Path>) {
    let src = home_join(".codex/auth.json");
    if let Some(home) = codex_home {
        copy_chatgpt_auth_file(&src, home);
    }
    let Ok(doc) = crate::workspace::load_workspaces() else {
        return;
    };
    for entry in doc.workspaces.values() {
        copy_chatgpt_auth_file(&src, &entry.codex_home);
    }
}

pub(crate) fn copy_chatgpt_auth_file(src: &Path, dest_home: &Path) -> bool {
    let Some(src_home) = src.parent() else {
        return false;
    };
    if crate::workspace::path::paths_equal(src_home, dest_home) {
        return false;
    }
    let Ok(raw) = fs::read_to_string(src) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    if !codex_auth_is_chatgpt(&value) {
        return false;
    }
    if fs::create_dir_all(dest_home).is_err() {
        return false;
    }
    fs::write(dest_home.join("auth.json"), raw).is_ok()
}

fn restore_codex_auth() {
    let Some(src) = stash_path() else {
        return;
    };
    if !src.exists() {
        return;
    }
    let dest = home_join(".codex/auth.json");
    if read_json(&dest)
        .map(|value| codex_auth_is_chatgpt(&value))
        .unwrap_or(false)
    {
        return;
    }
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::copy(src, dest);
}

fn spawn_official_login(runtime_id: &str) -> Result<()> {
    let Some(binary) = discover_binary(binary_name(runtime_id)).binary_path else {
        bail!("tool is not installed");
    };
    let args: &[&str] = if runtime_id == CODEX_ID {
        &["login"]
    } else {
        &["auth", "login"]
    };
    let mut cmd = crate::exec::command_for_path(&binary);
    let mut child = cmd
        .args(args)
        // Sign-in must land in ~/.codex. A project CODEX_HOME would keep the new
        // account away from the file the next chat copies in.
        .env_remove("CODEX_HOME")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let opened = Arc::new(AtomicBool::new(false));
    if let Some(pipe) = child.stdout.take() {
        watch_login_output(pipe, Arc::clone(&opened));
    }
    if let Some(pipe) = child.stderr.take() {
        watch_login_output(pipe, opened);
    }
    let started = Instant::now();
    loop {
        if logged_in(runtime_id) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        if started.elapsed() > LOGIN_WAIT {
            let _ = child.kill();
            let _ = child.wait();
            bail!("login timed out");
        }
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => thread::sleep(Duration::from_millis(400)),
            Err(_) => return Ok(()),
        }
    }
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
            if let Some(url) = first_login_url(&pending) {
                if !opened.swap(true, Ordering::SeqCst) {
                    let _ = crate::session_launch::open_url(&url);
                }
            }
        }
    });
}

fn first_login_url(text: &str) -> Option<String> {
    let start = text.find("https://")?;
    let rest = &text[start..];
    let end = rest
        .find(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\'')
        .unwrap_or(rest.len());
    let url = rest[..end].trim_end_matches(['.', ',', ')']);
    if url.len() > "https://".len() {
        Some(url.to_string())
    } else {
        None
    }
}

fn probe_login_sites() -> (bool, bool) {
    thread::scope(|scope| {
        let codex = scope.spawn(|| host_reachable(CODEX_LOGIN_URL));
        let claude = scope.spawn(|| host_reachable(CLAUDE_LOGIN_URL));
        (
            codex.join().unwrap_or(false),
            claude.join().unwrap_or(false),
        )
    })
}

fn host_reachable(url: &str) -> bool {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(2))
        .build()
    else {
        return false;
    };
    client.head(url).send().is_ok() || client.get(url).send().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chatgpt_login_is_not_an_empty_api_key() {
        let empty = json!({"auth_mode": "apikey"});
        assert!(!codex_auth_is_chatgpt(&empty));
        let login = json!({"auth_mode": "chatgpt", "tokens": {"access_token": "x"}});
        assert!(codex_auth_is_chatgpt(&login));
    }

    #[test]
    fn claude_oauth_file_counts_as_logged_in() {
        assert!(claude_credentials_are_oauth(
            &json!({"claudeAiOauth": {"accessToken": "x"}})
        ));
        assert!(!claude_credentials_are_oauth(&json!({"note": "empty"})));
    }

    #[test]
    fn chatgpt_auth_replaces_the_isolated_home_and_skips_an_api_key() {
        let root = std::env::temp_dir().join(format!(
            "agent-doctor-auth-copy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        let global = root.join("global");
        let isolated = root.join("isolated");
        fs::create_dir_all(&global).unwrap();
        fs::create_dir_all(&isolated).unwrap();
        let src = global.join("auth.json");
        fs::write(
            &src,
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"new"}}"#,
        )
        .unwrap();
        fs::write(
            isolated.join("auth.json"),
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"old"}}"#,
        )
        .unwrap();
        assert!(copy_chatgpt_auth_file(&src, &isolated));
        let copied = fs::read_to_string(isolated.join("auth.json")).unwrap();
        assert!(copied.contains("new"), "{copied}");
        assert!(!copied.contains("old"));
        fs::write(&src, r#"{"auth_mode":"apikey"}"#).unwrap();
        assert!(!copy_chatgpt_auth_file(&src, &isolated));
        assert!(fs::read_to_string(isolated.join("auth.json"))
            .unwrap()
            .contains("new"));
        assert!(!copy_chatgpt_auth_file(&src, &global));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn hidden_until_the_login_site_answers_unless_already_using_it() {
        assert!(!account_row_visible(false, false, true));
        assert!(account_row_visible(false, true, true));
        assert!(account_row_visible(true, false, false));
        assert!(!account_row_visible(false, true, false));
    }

    #[test]
    fn strip_removes_the_provider_address_and_keeps_other_settings() {
        let mut root = json!({
            "anthropicBaseUrl": "https://api.example/anthropic",
            "theme": "dark",
            "env": {
                "ANTHROPIC_API_KEY": "sk",
                "ANTHROPIC_BASE_URL": "https://api.example/anthropic",
                "ANTHROPIC_MODEL": "claude-sonnet-5",
                "OTHER": "keep"
            }
        });
        strip_claude_provider_keys(&mut root);
        assert!(root.get("anthropicBaseUrl").is_none());
        assert_eq!(root["theme"], "dark");
        assert!(root["env"].get("ANTHROPIC_API_KEY").is_none());
        assert_eq!(root["env"]["ANTHROPIC_MODEL"], "claude-sonnet-5");
        assert_eq!(root["env"]["OTHER"], "keep");
    }

    #[test]
    fn login_output_url_is_taken_without_the_trailing_punctuation() {
        assert_eq!(
            first_login_url("open https://auth.openai.com/codex/device)."),
            Some("https://auth.openai.com/codex/device".to_string())
        );
        assert_eq!(first_login_url("no link"), None);
    }
}
