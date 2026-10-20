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
        if let Some(model) = model.map(str::trim).filter(|id| !id.is_empty()) {
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
        CODEX_ID => fetch_codex_models(),
        CLAUDE_ID => fetch_claude_models(),
        other => bail!("subscription models are only for Codex and Claude Code, not {other}"),
    }?;
    if list.models.is_empty() {
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
    let raw = std::fs::read_to_string(home_join(".claude/.credentials.json")).ok()?;
    let value = serde_json::from_str::<Value>(&raw).ok()?;
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
    }
}
