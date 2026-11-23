//! Models a subscription account actually offers.
//!
//! Codex answers the ChatGPT account's model list, then `model/list`.
//! Claude Code answers `GET /v1/models`. The saved list is what the chat
//! may show and send. A name that is not on it is replaced with the
//! account default.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::util::{discover_binary, home_join};

const CODEX_ID: &str = "codex";
const CLAUDE_ID: &str = "claude-code";
const QODER_ID: &str = "qoder";
const CURSOR_ID: &str = "cursor";
const WORKBUDDY_ID: &str = "workbuddy";
const ZCODE_ID: &str = "zcode";
const LIST_WAIT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AccountModelList {
    pub models: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// Names shown in the menu, one per model. An empty entry keeps the id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// `auto`, `cursor`, or `other`, one per model. Empty keeps one flat list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<String>,
    /// The short list the Cursor client shows, not every model the account can run.
    #[serde(default, skip_serializing_if = "is_false")]
    pub picker: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl AccountModelList {
    /// The id this account will accept. A name it did not list becomes the default.
    pub fn choose(&self, model: Option<&str>) -> Option<String> {
        self.choose_inner(model, false)
    }

    /// Cursor lists one id per model the client shows. An older chat may still
    /// name an effort or speed slug (`gpt-5.3-codex-xhigh`); that is the same model.
    pub fn choose_cursor(&self, model: Option<&str>) -> Option<String> {
        self.choose_inner(model, true)
    }

    fn choose_inner(&self, model: Option<&str>, map_cursor_variant: bool) -> Option<String> {
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
            if map_cursor_variant {
                if let Some(id) = self.cursor_family_match(model) {
                    return Some(id);
                }
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

    fn cursor_family_match(&self, model: &str) -> Option<String> {
        let tail = model.rsplit(['/', ':']).next().unwrap_or(model).trim();
        let family = cursor_model_family(tail);
        if family.eq_ignore_ascii_case(tail) {
            return None;
        }
        self.models
            .iter()
            .find(|id| cursor_model_family(id).eq_ignore_ascii_case(&family))
            .cloned()
    }
}

pub fn load_account_models(runtime_id: &str) -> Option<AccountModelList> {
    let raw = crate::store::open_settings_store()
        .ok()?
        .get_meta(&models_key(runtime_id))
        .ok()??;
    let list = serde_json::from_str::<AccountModelList>(&raw).ok()?;
    let list = if runtime_id == CURSOR_ID && !list.picker {
        collapse_saved_cursor_models(list)
    } else {
        list
    };
    let list = annotate_product_sections(runtime_id, list);
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
        WORKBUDDY_ID => fetch_workbuddy_models()?,
        ZCODE_ID => fetch_zcode_models()?,
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
    if let Some(list) = fetch_codex_chatgpt_models() {
        if !list.models.is_empty() {
            return Ok(list);
        }
    }
    fetch_codex_app_server_models()
}

/// The model list this ChatGPT account is allowed to use.
/// Empty when the sign-in or the list cannot be read.
fn fetch_codex_chatgpt_models() -> Option<AccountModelList> {
    let (token, account) = codex_chatgpt_token()?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?;
    let version = codex_client_version();
    let url = format!("https://chatgpt.com/backend-api/codex/models?client_version={version}");
    let response = codex_account_request(&client, &url, &token, &account)
        .send()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = response.json().ok()?;
    let list = parse_codex_chatgpt_models(&body);
    if list.models.is_empty() {
        None
    } else {
        Some(list)
    }
}

fn fetch_codex_app_server_models() -> Result<AccountModelList> {
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

/// ChatGPT's `/codex/models` list: one row per model this plan can use.
/// Hidden rows stay out. Display names are the menu text.
pub fn parse_codex_chatgpt_models(body: &Value) -> AccountModelList {
    let mut rows = Vec::new();
    let items = body
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in items {
        if item.get("visibility").and_then(Value::as_str) == Some("hide") {
            continue;
        }
        let Some(id) = item
            .get("slug")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        if rows.iter().any(|(_, seen, _)| seen == id) {
            continue;
        }
        let label = item
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let priority = item.get("priority").and_then(Value::as_i64).unwrap_or(0);
        rows.push((priority, id.to_string(), label));
    }
    rows.sort_by_key(|row| row.0);
    let models: Vec<String> = rows.iter().map(|row| row.1.clone()).collect();
    let labels: Vec<String> = rows.iter().map(|row| row.2.clone()).collect();
    let sections = models
        .iter()
        .map(|id| codex_section(id).to_string())
        .collect();
    AccountModelList {
        default_model: models.first().cloned(),
        models,
        labels,
        sections,
        ..Default::default()
    }
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
    let sections = models
        .iter()
        .map(|id| codex_section(id).to_string())
        .collect();
    AccountModelList {
        models,
        default_model,
        sections,
        ..Default::default()
    }
}

/// Codex is a coding agent. Models with `codex` in the name are its coding line.
/// The rest are general models.
fn codex_section(id: &str) -> &'static str {
    if id.to_ascii_lowercase().contains("codex") {
        "codex-coding"
    } else {
        "codex-general"
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
        ..Default::default()
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
    if let Ok(list) = fetch_cursor_picker_from_api() {
        if list.picker && list.models.len() > 1 {
            return Ok(list);
        }
    }
    fetch_cursor_models_from_cli()
}

fn fetch_cursor_picker_from_api() -> Result<AccountModelList> {
    let token = cursor_desktop_access_token().context("Cursor is not signed in")?;
    let client = reqwest::blocking::Client::builder()
        .timeout(LIST_WAIT)
        .build()?;
    let response = client
        .post("https://api2.cursor.sh/aiserver.v1.AiService/AvailableModels")
        .header("Authorization", format!("Bearer {token}"))
        .header("Connect-Protocol-Version", "1")
        .json(&serde_json::json!({
            "useModelParameters": true,
            "doNotUseMarkdown": true,
            "useReactModelPicker": true,
        }))
        .send()
        .context("读取 Cursor 模型名单失败")?;
    if !response.status().is_success() {
        bail!("读取 Cursor 模型名单失败");
    }
    let body: Value = response.json().context("读取 Cursor 模型名单失败")?;
    Ok(parse_cursor_available_models(&body))
}

/// The two bars on Cursor's Plan & Usage page. Spend divided by the plan
/// limit is a different number and is not shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CursorPeriodUsage {
    pub cursor_models_percent: Option<u8>,
    pub other_models_percent: Option<u8>,
}

impl CursorPeriodUsage {
    fn known(self) -> bool {
        self.cursor_models_percent.is_some() || self.other_models_percent.is_some()
    }
}

struct CursorQuotaSnapshot {
    at: Instant,
    usage: CursorPeriodUsage,
}

static CURSOR_QUOTA: Mutex<Option<CursorQuotaSnapshot>> = Mutex::new(None);
const QUOTA_OK_TTL: Duration = Duration::from_secs(90);
const QUOTA_MISS_TTL: Duration = Duration::from_secs(15);
const QUOTA_WAIT: Duration = Duration::from_secs(4);

/// Cursor Models and Other Models used this period, matching the plan page.
/// Cached so opening the menu does not ask Cursor again on every status check.
pub fn cursor_period_usage() -> CursorPeriodUsage {
    if let Some(hit) = cached_cursor_quota() {
        return hit;
    }
    let usage = fetch_cursor_period_usage();
    if let Ok(mut slot) = CURSOR_QUOTA.lock() {
        *slot = Some(CursorQuotaSnapshot {
            at: Instant::now(),
            usage,
        });
    }
    usage
}

fn cached_cursor_quota() -> Option<CursorPeriodUsage> {
    let slot = CURSOR_QUOTA.lock().ok()?;
    let snap = slot.as_ref()?;
    let ttl = if snap.usage.known() {
        QUOTA_OK_TTL
    } else {
        QUOTA_MISS_TTL
    };
    if snap.at.elapsed() < ttl {
        Some(snap.usage)
    } else {
        None
    }
}

fn fetch_cursor_period_usage() -> CursorPeriodUsage {
    let Some(token) = cursor_desktop_access_token() else {
        return CursorPeriodUsage::default();
    };
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(QUOTA_WAIT)
        .build()
    else {
        return CursorPeriodUsage::default();
    };
    let Ok(response) = client
        .post("https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage")
        .header("Authorization", format!("Bearer {token}"))
        .header("Connect-Protocol-Version", "1")
        .header("Content-Type", "application/json")
        .body("{}")
        .send()
    else {
        return CursorPeriodUsage::default();
    };
    if !response.status().is_success() {
        return CursorPeriodUsage::default();
    }
    let Ok(body) = response.json() else {
        return CursorPeriodUsage::default();
    };
    parse_cursor_period_usage(&body)
}

/// Same rounding as Cursor's plan page: under 1% shows as 1, otherwise
/// the nearest whole percent.
pub fn parse_cursor_period_usage(body: &Value) -> CursorPeriodUsage {
    if body.get("enabled").and_then(|value| value.as_bool()) == Some(false) {
        return CursorPeriodUsage::default();
    }
    let Some(plan) = body.get("planUsage").or_else(|| body.get("plan_usage")) else {
        return CursorPeriodUsage::default();
    };
    CursorPeriodUsage {
        cursor_models_percent: json_f64(plan, &["autoPercentUsed", "auto_percent_used"])
            .map(cursor_bar_percent),
        other_models_percent: json_f64(plan, &["apiPercentUsed", "api_percent_used"])
            .map(cursor_bar_percent),
    }
}

fn cursor_bar_percent(value: f64) -> u8 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    if value < 1.0 {
        return 1;
    }
    value.clamp(0.0, 100.0).round() as u8
}

/// How much of this ChatGPT account's allowance is already used.
/// The short window and the weekly window are one pool for every model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CodexAllowance {
    /// Length of the short window, in whole hours. Empty when that window is missing.
    pub hours: Option<u32>,
    pub hours_used_percent: Option<u8>,
    pub week_used_percent: Option<u8>,
}

impl CodexAllowance {
    fn known(self) -> bool {
        self.hours_used_percent.is_some() || self.week_used_percent.is_some()
    }
}

struct CodexQuotaSnapshot {
    at: Instant,
    allowance: CodexAllowance,
}

static CODEX_QUOTA: Mutex<Option<CodexQuotaSnapshot>> = Mutex::new(None);

/// Used percent of the 5-hour window and the weekly window.
/// Cached so opening the menu does not ask ChatGPT on every status check.
pub fn codex_allowance() -> CodexAllowance {
    if let Some(hit) = cached_codex_allowance() {
        return hit;
    }
    let allowance = fetch_codex_allowance();
    if let Ok(mut slot) = CODEX_QUOTA.lock() {
        *slot = Some(CodexQuotaSnapshot {
            at: Instant::now(),
            allowance,
        });
    }
    allowance
}

fn cached_codex_allowance() -> Option<CodexAllowance> {
    let slot = CODEX_QUOTA.lock().ok()?;
    let snap = slot.as_ref()?;
    let ttl = if snap.allowance.known() {
        QUOTA_OK_TTL
    } else {
        QUOTA_MISS_TTL
    };
    if snap.at.elapsed() < ttl {
        Some(snap.allowance)
    } else {
        None
    }
}

fn fetch_codex_allowance() -> CodexAllowance {
    let Some((token, account)) = codex_chatgpt_token() else {
        return CodexAllowance::default();
    };
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(QUOTA_WAIT)
        .build()
    else {
        return CodexAllowance::default();
    };
    let Ok(response) = codex_account_request(
        &client,
        "https://chatgpt.com/backend-api/wham/usage",
        &token,
        &account,
    )
    .send() else {
        return CodexAllowance::default();
    };
    if !response.status().is_success() {
        return CodexAllowance::default();
    }
    let Ok(body) = response.json() else {
        return CodexAllowance::default();
    };
    parse_codex_usage(&body)
}

/// `used_percent` on each window. A window of at least a day is the week.
/// A shorter whole-hour window is the short allowance.
pub fn parse_codex_usage(body: &Value) -> CodexAllowance {
    let rate = body.get("rate_limit").unwrap_or(&Value::Null);
    let mut allowance = CodexAllowance::default();
    for key in ["primary_window", "secondary_window"] {
        let Some(window) = rate.get(key) else {
            continue;
        };
        let Some(used) = json_f64(window, &["used_percent"]) else {
            continue;
        };
        let secs = json_f64(window, &["limit_window_seconds"]).unwrap_or(0.0) as i64;
        let percent = cursor_bar_percent(used);
        if secs >= 86_400 {
            if allowance.week_used_percent.is_none() {
                allowance.week_used_percent = Some(percent);
            }
        } else if secs >= 3_600 && secs % 3_600 == 0 && allowance.hours_used_percent.is_none() {
            allowance.hours = Some((secs / 3_600) as u32);
            allowance.hours_used_percent = Some(percent);
        }
    }
    allowance
}

fn codex_chatgpt_token() -> Option<(String, String)> {
    let mut paths = vec![home_join(".codex/auth.json")];
    if let Some(stash) = crate::setup::codex_chatgpt_stash_path() {
        paths.push(stash);
    }
    for path in paths {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(pair) = token_from_auth_json(&raw) {
            return Some(pair);
        }
    }
    #[cfg(all(target_os = "macos", not(test)))]
    if let Some(raw) = codex_keychain_auth_json() {
        if let Some(pair) = token_from_auth_json(&raw) {
            return Some(pair);
        }
    }
    None
}

fn token_from_auth_json(raw: &str) -> Option<(String, String)> {
    let value = serde_json::from_str::<Value>(raw).ok()?;
    if value.get("auth_mode").and_then(Value::as_str) == Some("apikey") {
        return None;
    }
    let tokens = value.get("tokens");
    let access = tokens
        .and_then(|item| item.get("access_token"))
        .and_then(Value::as_str)
        .or_else(|| value.get("access_token").and_then(Value::as_str))
        .map(str::trim)
        .filter(|token| !token.is_empty())?;
    let account = tokens
        .and_then(|item| item.get("account_id"))
        .and_then(Value::as_str)
        .or_else(|| value.get("account_id").and_then(Value::as_str))
        .map(str::trim)
        .unwrap_or("");
    Some((access.to_string(), account.to_string()))
}

/// Codex desktop keeps the sign-in in the macOS keychain when `auth.json` is absent.
/// Service `Codex Auth`, account `cli|` plus the first 16 hex of the home path's SHA-256.
#[cfg(all(target_os = "macos", not(test)))]
fn codex_keychain_auth_json() -> Option<String> {
    use sha2::{Digest, Sha256};
    let home = home_join(".codex");
    let canonical = fs::canonicalize(&home).unwrap_or(home);
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let account = format!("cli|{}", &hex[..16]);
    let output = std::process::Command::new("security") // raw-spawn-ok
        .args([
            "find-generic-password",
            "-s",
            "Codex Auth",
            "-a",
            &account,
            "-w",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

fn codex_account_request(
    client: &reqwest::blocking::Client,
    url: &str,
    token: &str,
    account_id: &str,
) -> reqwest::blocking::RequestBuilder {
    let version = codex_client_version();
    let mut request = client
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("originator", "codex_cli_rs")
        .header("User-Agent", codex_user_agent(&version))
        .header("Accept", "application/json");
    if !account_id.is_empty() {
        request = request.header("chatgpt-account-id", account_id);
    }
    request
}

fn codex_user_agent(version: &str) -> String {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    };
    let os = if cfg!(target_os = "macos") {
        "Mac OS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Linux"
    };
    format!("codex_cli_rs/{version} ({os}; {arch}) Apple_Terminal/455")
}

fn codex_client_version() -> String {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION
        .get_or_init(|| {
            discover_binary("codex")
                .binary_path
                .and_then(|bin| {
                    let output = crate::exec::command_for_path(&bin)
                        .arg("--version")
                        .stdin(Stdio::null())
                        .stderr(Stdio::null())
                        .output()
                        .ok()?;
                    semver_in(&String::from_utf8_lossy(&output.stdout))
                })
                .unwrap_or_else(|| "0.145.0".to_string())
        })
        .clone()
}

fn semver_in(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_digit() {
            let start = index;
            let mut dots = 0;
            while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.') {
                if bytes[index] == b'.' {
                    dots += 1;
                }
                index += 1;
            }
            let raw = &text[start..index];
            if dots >= 1 && !raw.ends_with('.') && !raw.starts_with('.') {
                return Some(raw.to_string());
            }
        } else {
            index += 1;
        }
    }
    None
}

fn json_f64(value: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        let Some(item) = value.get(*key) else {
            continue;
        };
        let number = item
            .as_f64()
            .or_else(|| item.as_i64().map(|n| n as f64))
            .or_else(|| item.as_u64().map(|n| n as f64));
        if let Some(number) = number.filter(|n| n.is_finite()) {
            return Some(number);
        }
    }
    None
}

fn cursor_desktop_access_token() -> Option<String> {
    let path = dirs::config_dir()?
        .join("Cursor")
        .join("User")
        .join("globalStorage")
        .join("state.vscdb");
    if !path.is_file() {
        return None;
    }
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = rusqlite::Connection::open_with_flags(&path, flags).ok()?;
    let token: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'cursorAuth/accessToken' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok()?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn fetch_cursor_models_from_cli() -> Result<AccountModelList> {
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

fn fetch_workbuddy_models() -> Result<AccountModelList> {
    if let Some(list) = workbuddy_account_catalog() {
        if !list.models.is_empty() {
            return Ok(list);
        }
    }
    let bin = crate::adapters::workbuddy_cli_path().context("WorkBuddy CLI is not installed")?;
    let mut cmd = crate::exec::command_for_path(&bin);
    cmd.arg("--help")
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .env("COLUMNS", "1000")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CODEBUDDY_API_KEY",
        "CODEBUDDY_BASE_URL",
        "CODEBUDDY_MODEL",
    ] {
        cmd.env_remove(key);
    }
    let out = run_cli_with_timeout(cmd, LIST_WAIT)?;
    Ok(parse_workbuddy_model_list(&out))
}

fn fetch_zcode_models() -> Result<AccountModelList> {
    let zcode_v2 = home_join(".zcode/v2");
    let builtin = load_zcode_builtin_json(&zcode_v2);
    Ok(zcode_account_models_from_dir(&zcode_v2, builtin.as_ref()))
}

/// Resolve `(providerId, modelId)` in ZCode's account registry for a user-chosen model id.
pub fn resolve_zcode_account_selection(model: Option<&str>) -> Option<(String, String)> {
    let zcode_v2 = home_join(".zcode/v2");
    let builtin = load_zcode_builtin_json(&zcode_v2);
    resolve_zcode_account_selection_in_dir(&zcode_v2, builtin.as_ref(), model)
}

/// Resolve `(providerId, modelId, reasoningLevel)` for launching ZCode headless CLI.
pub fn resolve_zcode_account_launch(model: Option<&str>) -> Option<(String, String, String)> {
    let zcode_v2 = home_join(".zcode/v2");
    let builtin = load_zcode_builtin_json(&zcode_v2);
    resolve_zcode_account_launch_in_dir(&zcode_v2, builtin.as_ref(), model)
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
    let sections = models
        .iter()
        .map(|id| qoder_section(id).to_string())
        .collect();
    AccountModelList {
        models,
        default_model,
        sections,
        ..Default::default()
    }
}

/// Qoder sells capability tiers, not vendors. Auto and Fast stay on top.
/// Max and Pro are the strong tier, Flash is the cheaper tier, Lite is the
/// light tier, and a bare preview is a new model.
fn qoder_section(id: &str) -> &'static str {
    let lower = id.to_ascii_lowercase();
    if lower == "auto" || lower == "fast" {
        return "auto";
    }
    if lower == "lite" || lower.ends_with("-lite") {
        return "qoder-lite";
    }
    if lower == "efficient" || lower.contains("flash") {
        return "qoder-value";
    }
    if lower == "ultimate"
        || lower == "performance"
        || lower.contains("max")
        || lower.contains("-pro")
        || lower.ends_with("pro")
    {
        return "qoder-power";
    }
    if lower.contains("preview") {
        return "qoder-new";
    }
    "qoder-power"
}

/// Models the Cursor client puts in its picker: `defaultOn` rows that have a
/// section, plus Auto. Effort and speed stay inside the model.
pub fn parse_cursor_available_models(body: &Value) -> AccountModelList {
    let mut models = vec!["auto".to_string()];
    let mut labels = vec![String::new()];
    let mut sections = vec!["auto".to_string()];
    let Some(items) = body.get("models").and_then(Value::as_array) else {
        return AccountModelList {
            models,
            labels,
            sections,
            default_model: Some("auto".to_string()),
            picker: true,
        };
    };
    for item in items {
        if !cursor_picker_entry(item) {
            continue;
        }
        let Some(name) = item
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if name.eq_ignore_ascii_case("default") || name.eq_ignore_ascii_case("auto") {
            continue;
        }
        if models.iter().any(|id| id.eq_ignore_ascii_case(name)) {
            continue;
        }
        let label = item
            .get("clientDisplayName")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        models.push(name.to_string());
        labels.push(label);
        sections.push(cursor_model_section(item).to_string());
    }
    AccountModelList {
        models,
        labels,
        sections,
        default_model: Some("auto".to_string()),
        picker: true,
    }
}

/// Cursor's picker puts its own models (Grok, Composer) under "Cursor Models"
/// and every other vendor under "Other Models".
fn cursor_model_section(item: &Value) -> &'static str {
    let vendor = item
        .get("vendor")
        .and_then(|value| {
            value
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| value.as_str())
        })
        .unwrap_or("");
    if vendor.eq_ignore_ascii_case("MODEL_VENDOR_ID_CURSOR")
        || vendor.eq_ignore_ascii_case("CURSOR")
    {
        "cursor"
    } else {
        "other"
    }
}

fn cursor_picker_entry(item: &Value) -> bool {
    if item.get("isHidden").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    if item.get("supportsAgent").and_then(Value::as_bool) == Some(false) {
        return false;
    }
    if item
        .get("namedModelSectionIndex")
        .and_then(Value::as_i64)
        .is_none()
    {
        return false;
    }
    item.get("defaultOn")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Parse `agent models` (Cursor CLI) stdout.
///
/// The CLI prints every effort and speed slug (`gpt-5.3-codex-low-fast`, …).
/// The Cursor client shows one row per model and keeps effort and speed as
/// options on that row. This keeps the row the client shows.
pub fn parse_cursor_model_list(stdout: &str) -> AccountModelList {
    let clean = strip_ansi(stdout);
    let mut rows = Vec::new();
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
        let display = line
            .split_once(" - ")
            .map(|(_, name)| name.split(" (").next().unwrap_or(name).trim())
            .unwrap_or("")
            .to_string();
        if rows.iter().any(|seen: &CursorModelRow| seen.id == head) {
            continue;
        }
        rows.push(CursorModelRow {
            id: head.to_string(),
            display,
            is_default,
        });
    }
    collapse_cursor_rows(&rows)
}

struct CursorModelRow {
    id: String,
    display: String,
    is_default: bool,
}

/// Saved lists from before this filter still contain every slug. Drop them on read.
fn collapse_saved_cursor_models(list: AccountModelList) -> AccountModelList {
    let default_id = list.default_model.as_deref().unwrap_or("");
    let rows: Vec<CursorModelRow> = list
        .models
        .into_iter()
        .map(|id| {
            let is_default = !default_id.is_empty() && id.eq_ignore_ascii_case(default_id);
            CursorModelRow {
                id,
                display: String::new(),
                is_default,
            }
        })
        .collect();
    collapse_cursor_rows(&rows)
}

fn collapse_cursor_rows(rows: &[CursorModelRow]) -> AccountModelList {
    let mut families: Vec<Vec<usize>> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let family = cursor_model_family(&row.id);
        if let Some(group) = families
            .iter_mut()
            .find(|group| cursor_model_family(&rows[group[0]].id).eq_ignore_ascii_case(&family))
        {
            group.push(index);
        } else {
            families.push(vec![index]);
        }
    }
    let mut models = Vec::new();
    let mut default_model = None;
    for group in families {
        let picked = group
            .iter()
            .copied()
            .min_by_key(|&index| cursor_row_rank(&rows[index]))
            .map(|index| &rows[index]);
        let Some(picked) = picked else {
            continue;
        };
        if rows.iter().any(|row| {
            row.is_default
                && cursor_model_family(&row.id)
                    .eq_ignore_ascii_case(&cursor_model_family(&picked.id))
        }) && default_model.is_none()
        {
            default_model = Some(picked.id.clone());
        }
        models.push(picked.id.clone());
    }
    if default_model.is_none() {
        default_model = models.first().cloned();
    }
    AccountModelList {
        models,
        default_model,
        ..Default::default()
    }
}

/// Effort and speed suffixes the CLI appends. Longer ones come first so
/// `-xhigh` is not read as `-high`.
const CURSOR_VARIANT_SUFFIXES: &[&str] = &[
    "-extra-high",
    "-xhigh",
    "-minimal",
    "-medium",
    "-thinking",
    "-none",
    "-fast",
    "-high",
    "-low",
    "-max",
];

fn cursor_model_family(id: &str) -> String {
    let mut rest = id.trim().to_string();
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(suffix) = CURSOR_VARIANT_SUFFIXES
            .iter()
            .copied()
            .find(|suffix| lower.ends_with(suffix) && lower.len() > suffix.len())
        else {
            break;
        };
        rest.truncate(rest.len() - suffix.len());
    }
    rest
}

fn cursor_row_rank(row: &CursorModelRow) -> (u8, u8, u8, u8, usize) {
    let lower = row.id.to_ascii_lowercase();
    let fast = lower.ends_with("-fast");
    let family = cursor_model_family(&row.id);
    let bare = row.id.eq_ignore_ascii_case(&family);
    let client_name = cursor_display_is_client_name(&row.display);
    (
        if client_name { 0 } else { 1 },
        if bare { 0 } else { 1 },
        if fast { 1 } else { 0 },
        cursor_effort_rank(&lower),
        row.id.len(),
    )
}

fn cursor_effort_rank(id: &str) -> u8 {
    let base = id.strip_suffix("-fast").unwrap_or(id);
    if base.ends_with("-medium") {
        0
    } else if base.ends_with("-extra-high") || base.ends_with("-xhigh") {
        2
    } else if base.ends_with("-high") {
        1
    } else if base.ends_with("-low") {
        3
    } else if base.ends_with("-max") {
        4
    } else if base.ends_with("-none") {
        5
    } else if base.ends_with("-minimal") {
        6
    } else if base.ends_with("-thinking") {
        7
    } else {
        8
    }
}

fn cursor_display_is_client_name(display: &str) -> bool {
    let normalized = normalize_cursor_display(display);
    if normalized.is_empty() {
        return false;
    }
    cursor_display_base(&normalized).eq_ignore_ascii_case(&normalized)
}

fn normalize_cursor_display(display: &str) -> String {
    display
        .chars()
        .map(|ch| {
            if ch.is_whitespace() || matches!(ch, '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}')
            {
                ' '
            } else {
                ch
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn cursor_display_base(display: &str) -> String {
    let mut words: Vec<String> = display.split_whitespace().map(str::to_string).collect();
    loop {
        let len = words.len();
        if len >= 2
            && words[len - 2].eq_ignore_ascii_case("no")
            && words[len - 1].eq_ignore_ascii_case("thinking")
        {
            words.truncate(len - 2);
            continue;
        }
        if len >= 2
            && words[len - 2].eq_ignore_ascii_case("extra")
            && words[len - 1].eq_ignore_ascii_case("high")
        {
            words.truncate(len - 2);
            continue;
        }
        let stripped = words
            .last()
            .map(|word| {
                word.eq_ignore_ascii_case("thinking")
                    || word.eq_ignore_ascii_case("minimal")
                    || word.eq_ignore_ascii_case("medium")
                    || word.eq_ignore_ascii_case("none")
                    || word.eq_ignore_ascii_case("fast")
                    || word.eq_ignore_ascii_case("high")
                    || word.eq_ignore_ascii_case("low")
                    || word.eq_ignore_ascii_case("max")
            })
            .unwrap_or(false);
        if stripped {
            words.pop();
            continue;
        }
        break;
    }
    words.join(" ")
}

/// The model list saved when this WorkBuddy account signed in.
/// The chat agent names which ids it can use. Display names come from the
/// same saved catalog. Empty when that sign-in has not been written yet.
pub fn workbuddy_account_catalog() -> Option<AccountModelList> {
    let dir = home_join(".codebuddy/local_storage");
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("info") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() || meta.len() > 2_000_000 {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if let Some(list) = parse_workbuddy_account_catalog(&value) {
            if !list.models.is_empty() {
                return Some(list);
            }
        }
    }
    None
}

/// `[{ userId, data: { agents: [{ name: "cli", models: [...] }], models: [{ id, name }] } }]`
pub fn parse_workbuddy_account_catalog(value: &Value) -> Option<AccountModelList> {
    let users = value.as_array()?;
    for user in users {
        let data = user.get("data")?;
        let Some(list) = workbuddy_catalog_from_data(data) else {
            continue;
        };
        if !list.models.is_empty() {
            return Some(list);
        }
    }
    None
}

fn workbuddy_catalog_from_data(data: &Value) -> Option<AccountModelList> {
    let catalog = data.get("models")?.as_array()?;
    let agents = data.get("agents")?.as_array()?;
    let agent = agents
        .iter()
        .find(|item| item.get("name").and_then(Value::as_str) == Some("cli"))
        .or_else(|| agents.first())?;
    let ids = agent.get("models")?.as_array()?;
    let mut names = std::collections::HashMap::<String, (String, bool)>::new();
    for item in catalog {
        let Some(id) = item
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let default_marked = item.get("isDefault").and_then(Value::as_bool) == Some(true);
        names.insert(id.to_string(), (name, default_marked));
    }
    let mut models = Vec::new();
    let mut labels = Vec::new();
    let mut default_model = None;
    for id in ids {
        let Some(id) = id.as_str().map(str::trim).filter(|id| !id.is_empty()) else {
            continue;
        };
        if models.iter().any(|seen: &String| seen == id) {
            continue;
        }
        let (name, default_marked) = names.get(id).cloned().unwrap_or_default();
        if default_marked && default_model.is_none() {
            default_model = Some(id.to_string());
        }
        labels.push(name);
        models.push(id.to_string());
    }
    if models.is_empty() {
        return None;
    }
    let sections = models
        .iter()
        .map(|id| workbuddy_section(id).to_string())
        .collect();
    Some(AccountModelList {
        models,
        default_model,
        labels,
        sections,
        ..Default::default()
    })
}

/// Parse `codebuddy --help` `--model` supported list:
/// `--model <model> ... Currently supported: (hy4-preview, hy3, ..., custom-local:deepseek-flash)`
pub fn parse_workbuddy_model_list(stdout: &str) -> AccountModelList {
    let clean = strip_ansi(stdout);
    let lower = clean.to_ascii_lowercase();
    let mut models = Vec::new();
    if let Some(marker_pos) = lower.find("currently supported:") {
        let after = &clean[marker_pos + "currently supported:".len()..];
        if let Some(open) = after.find('(') {
            let inside_start = &after[open + 1..];
            let raw_list = match inside_start.find(')') {
                Some(close) => &inside_start[..close],
                None => inside_start,
            };
            for raw_item in raw_list.split(',') {
                // `commander` may wrap long option descriptions across indented lines;
                // collapse any line break inside a token or around commas cleanly.
                let compact: String = raw_item.split_whitespace().collect::<Vec<_>>().join("");
                let id = compact.trim();
                if id.is_empty()
                    || id.starts_with("custom-local:")
                    || id.starts_with("agent-doctor/")
                    || id.contains('(')
                    || id.contains(')')
                {
                    continue;
                }
                if models
                    .iter()
                    .any(|seen: &String| seen.eq_ignore_ascii_case(id))
                {
                    continue;
                }
                models.push(id.to_string());
            }
        }
    }
    let default_model = models.first().cloned();
    let sections = models
        .iter()
        .map(|id| workbuddy_section(id).to_string())
        .collect();
    let labels = models.iter().map(|id| workbuddy_label(id)).collect();
    AccountModelList {
        models,
        default_model,
        labels,
        sections,
        ..Default::default()
    }
}

/// WorkBuddy's own models are Hunyuan (`hy` / `hunyuan`). Everything else is
/// another company's model.
fn workbuddy_section(id: &str) -> &'static str {
    let lower = id.to_ascii_lowercase();
    if lower.starts_with("hy") || lower.starts_with("hunyuan") {
        "workbuddy-own"
    } else {
        "other"
    }
}

fn workbuddy_label(id: &str) -> String {
    let lower = id.to_ascii_lowercase();
    let rest = if let Some(rest) = lower.strip_prefix("hunyuan") {
        rest
    } else if let Some(rest) = lower.strip_prefix("hy") {
        rest
    } else {
        return String::new();
    };
    let rest = rest
        .trim_start_matches(['-', '_'])
        .replace('-', " ")
        .replace("preview", "预览");
    if rest.is_empty() {
        "混元".to_string()
    } else {
        format!("混元 {rest}")
    }
}

fn annotate_product_sections(runtime_id: &str, mut list: AccountModelList) -> AccountModelList {
    if list.sections.len() == list.models.len()
        && list
            .sections
            .iter()
            .any(|section| !section.is_empty() && section != "auto")
    {
        return list;
    }
    let sections: Vec<String> = match runtime_id {
        QODER_ID => list
            .models
            .iter()
            .map(|id| qoder_section(id).to_string())
            .collect(),
        WORKBUDDY_ID => list
            .models
            .iter()
            .map(|id| workbuddy_section(id).to_string())
            .collect(),
        CODEX_ID => list
            .models
            .iter()
            .map(|id| codex_section(id).to_string())
            .collect(),
        _ => return list,
    };
    if runtime_id == WORKBUDDY_ID && list.labels.len() != list.models.len() {
        list.labels = list.models.iter().map(|id| workbuddy_label(id)).collect();
    }
    list.sections = sections;
    list
}

fn load_zcode_builtin_json(zcode_v2: &Path) -> Option<Value> {
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    collect_newest_named_file(
        &zcode_v2.join("runtime").join("provider"),
        "zcode-builtin.json",
        0,
        &mut newest,
    );
    if let Some((_, path)) = newest {
        if let Some(val) = read_json_file(&path) {
            return Some(val);
        }
    }
    let bundled = crate::adapters::locate_zcode_headless()
        .and_then(|launch| launch.script)
        .and_then(|script| crate::adapters::zcode_bundled_provider_config(&script))?;
    read_json_file(&bundled)
}

fn collect_newest_named_file(
    dir: &Path,
    target_name: &str,
    depth: usize,
    newest: &mut Option<(SystemTime, PathBuf)>,
) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_newest_named_file(&path, target_name, depth + 1, newest);
        } else if entry.file_name().to_str() == Some(target_name) {
            if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                if newest.as_ref().map_or(true, |(best, _)| modified > *best) {
                    *newest = Some((modified, path));
                }
            }
        }
    }
}

fn read_json_file(path: &Path) -> Option<Value> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn nonempty_json_str(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
}

fn active_zcode_account_providers(zcode_v2: &Path) -> Vec<String> {
    let Some(creds) = read_json_file(&zcode_v2.join("credentials.json")) else {
        return Vec::new();
    };
    let Some(map) = creds.as_object() else {
        return Vec::new();
    };
    let setting_family = read_json_file(&zcode_v2.join("setting.json"))
        .and_then(|val| {
            val.get("providerFamilyDomain")
                .and_then(Value::as_str)
                .map(str::trim)
                .map(str::to_ascii_lowercase)
        })
        .filter(|f| f == "bigmodel" || f == "zai");

    // ZCode standalone CLI (`zcode.cjs`) only registers `individual-coding-plan`
    // account providers and only marks them entitled when `account-provider:*:identity`
    // exists in `credentials.json`. Desktop-only `zcodejwttoken` (`*-start-plan`) or
    // unsynced `oauth:*:access_token` entries cannot run headless CLI turns.
    let mut identities = Vec::new();
    for (key, val) in map {
        if let Some(rest) = key.strip_prefix("account-provider:") {
            if let Some(provider_id) = rest.strip_suffix(":identity") {
                if !provider_id.ends_with("-start-plan") && nonempty_json_str(Some(val)) {
                    identities.push(provider_id.to_string());
                }
            }
        }
    }
    identities.sort();
    identities.dedup();

    let mut providers = Vec::new();
    if let Some(pref) = setting_family {
        let preferred_prefix = format!("account:{pref}-");
        for id in &identities {
            if id.starts_with(&preferred_prefix) {
                providers.push(id.clone());
            }
        }
    }
    for id in identities {
        if !providers.contains(&id) {
            providers.push(id);
        }
    }

    providers
}

fn zcode_provider_models(provider_id: &str, builtin: Option<&Value>) -> Vec<String> {
    if let Some(rules) = builtin
        .and_then(|v| v.pointer("/config/providerConfigRules/providerRules"))
        .and_then(Value::as_array)
    {
        for rule in rules {
            if rule.get("providerId").and_then(Value::as_str) != Some(provider_id) {
                continue;
            }
            if rule.pointer("/config/visibility").and_then(Value::as_str) == Some("hidden") {
                return Vec::new();
            }
            if let Some(arr) = rule
                .pointer("/config/builtinModelIds")
                .and_then(Value::as_array)
            {
                let parsed: Vec<String> = arr
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
                if !parsed.is_empty() {
                    return parsed;
                }
            }
        }
    }

    if provider_id.ends_with("-start-plan") {
        vec![
            "GLM-5.3-Flash".to_string(),
            "GLM-5.2".to_string(),
            "GLM-5-Turbo".to_string(),
        ]
    } else if provider_id.starts_with("account:") && provider_id.contains("coding-plan") {
        vec!["GLM-5.3".to_string(), "GLM-5.3-Flash".to_string()]
    } else {
        Vec::new()
    }
}

pub fn zcode_account_models_from_dir(zcode_v2: &Path, builtin: Option<&Value>) -> AccountModelList {
    let mut models = Vec::new();
    for provider_id in active_zcode_account_providers(zcode_v2) {
        for model_id in zcode_provider_models(&provider_id, builtin) {
            if !models
                .iter()
                .any(|seen: &String| seen.eq_ignore_ascii_case(&model_id))
            {
                models.push(model_id);
            }
        }
    }
    let default_model = models.first().cloned();
    AccountModelList {
        models,
        default_model,
        ..Default::default()
    }
}

pub fn resolve_zcode_account_selection_in_dir(
    zcode_v2: &Path,
    builtin: Option<&Value>,
    requested_model: Option<&str>,
) -> Option<(String, String)> {
    let providers = active_zcode_account_providers(zcode_v2);
    if providers.is_empty() {
        return None;
    }
    let requested = requested_model
        .map(str::trim)
        .filter(|m| !m.is_empty() && !m.eq_ignore_ascii_case("auto"));
    if let Some(target) = requested {
        let tail = target.rsplit(['/', ':']).next().unwrap_or(target).trim();
        for provider_id in &providers {
            for model_id in zcode_provider_models(provider_id, builtin) {
                if model_id.eq_ignore_ascii_case(target) || model_id.eq_ignore_ascii_case(tail) {
                    return Some((provider_id.clone(), model_id));
                }
            }
        }
    }
    for provider_id in &providers {
        if let Some(first) = zcode_provider_models(provider_id, builtin)
            .into_iter()
            .next()
        {
            return Some((provider_id.clone(), first));
        }
    }
    None
}

pub fn zcode_default_reasoning_level(
    provider_id: &str,
    model_id: &str,
    builtin: Option<&Value>,
) -> String {
    if let Some(rules) = builtin
        .and_then(|v| v.pointer("/config/modelConfigRules/builtinProviderModelRules"))
        .and_then(Value::as_array)
    {
        for rule in rules.iter().rev() {
            let matches_provider =
                rule.get("providerId").and_then(Value::as_str) == Some(provider_id);
            let matches_model = rule
                .get("modelId")
                .and_then(Value::as_str)
                .is_some_and(|m| m.eq_ignore_ascii_case(model_id));
            if matches_provider && matches_model {
                if let Some(level) = pick_reasoning_level_from_rule(rule) {
                    return level;
                }
            }
        }
    }

    let tail = model_id
        .rsplit('/')
        .next()
        .unwrap_or(model_id)
        .trim()
        .to_ascii_lowercase();
    if tail == "glm-5"
        || tail.starts_with("glm-5-turbo")
        || tail.starts_with("glm-5.1")
        || tail.starts_with("glm-5v")
        || tail.starts_with("glm-4")
    {
        "enabled".to_string()
    } else if tail.starts_with("glm") {
        "high".to_string()
    } else {
        "enabled".to_string()
    }
}

fn pick_reasoning_level_from_rule(rule: &Value) -> Option<String> {
    let values = rule
        .pointer("/config/optionSpecs/reasoningLevel/values")
        .and_then(Value::as_array)?;
    let strs: Vec<&str> = values
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if strs.contains(&"high") {
        Some("high".to_string())
    } else {
        strs.last().map(|s| (*s).to_string())
    }
}

pub fn resolve_zcode_account_launch_in_dir(
    zcode_v2: &Path,
    builtin: Option<&Value>,
    requested_model: Option<&str>,
) -> Option<(String, String, String)> {
    let (provider_id, model_id) =
        resolve_zcode_account_selection_in_dir(zcode_v2, builtin, requested_model)?;
    let reasoning_level = zcode_default_reasoning_level(&provider_id, &model_id, builtin);
    Some((provider_id, model_id, reasoning_level))
}

pub fn account_model_allowed(allowed: &[String], model: &str) -> bool {
    AccountModelList {
        models: allowed.to_vec(),
        default_model: None,
        ..Default::default()
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
        assert_eq!(
            qoder.sections,
            vec![
                "auto",
                "qoder-power",
                "qoder-power",
                "qoder-value",
                "qoder-lite"
            ]
        );
        let qoder_named = parse_qoder_model_list(
            "MODEL\nAuto\nFast\nQwen4-Max\nQwen3.8-Flash\nGLM-5.3\nKimi-K2.8-Preview\n",
        );
        assert_eq!(
            qoder_named.sections,
            vec![
                "auto",
                "auto",
                "qoder-power",
                "qoder-value",
                "qoder-power",
                "qoder-new"
            ]
        );

        let empty_qoder =
            parse_qoder_model_list("Warning: no models available for your account.\n");
        assert!(empty_qoder.models.is_empty());

        let cursor = parse_cursor_model_list(
            "\x1b[2mAvailable models\x1b[22m\n\n\x1b[36mgpt-5\x1b[39m - GPT-5\n\x1b[32mclaude-4.5-sonnet\x1b[39m - Claude 4.5 Sonnet \x1b[2m(current, default)\x1b[22m\n\nTip: use --model <id> to switch.\n",
        );
        assert_eq!(cursor.models, vec!["gpt-5", "claude-4.5-sonnet"]);
        assert_eq!(cursor.default_model.as_deref(), Some("claude-4.5-sonnet"));

        let cursor_client = parse_cursor_model_list(
            "Available models\n\n\
             auto - Auto (current, default)\n\
             gpt-5.3-codex-low - Codex 5.3 Low\n\
             gpt-5.3-codex-low-fast - Codex 5.3 Low Fast\n\
             gpt-5.3-codex - Codex 5.3\n\
             gpt-5.3-codex-fast - Codex 5.3 Fast\n\
             gpt-5.3-codex-high - Codex 5.3 High\n\
             gpt-5.3-codex-xhigh - Codex 5.3 Extra High\n\
             gpt-5.2-low - GPT-5.2 Low\n\
             gpt-5.2 - GPT-5.2\n\
             composer-2.5 - Composer 2.5\n\
             composer-2.5-fast - Composer 2.5 Fast\n\
             claude-opus-5-low - Claude Opus 5 1M Low\n\
             claude-opus-5-high - Claude Opus 5 1M\n\
             claude-opus-5-thinking-high - Claude Opus 5 1M Thinking\n\
             gpt-5.5-medium - GPT-5.5 1M\n\
             gpt-5.5-medium-fast - GPT-5.5 Fast\n\
             claude-4.5-sonnet - Claude Sonnet 4.5\n\
             claude-4.5-sonnet-thinking - Claude Sonnet 4.5 Thinking\n\
             \nTip: use --model <id> to switch.\n",
        );
        assert_eq!(
            cursor_client.models,
            vec![
                "auto",
                "gpt-5.3-codex",
                "gpt-5.2",
                "composer-2.5",
                "claude-opus-5-high",
                "gpt-5.5-medium",
                "claude-4.5-sonnet",
            ]
        );
        assert_eq!(cursor_client.default_model.as_deref(), Some("auto"));
        assert_eq!(
            cursor_client
                .choose_cursor(Some("gpt-5.3-codex-xhigh"))
                .as_deref(),
            Some("gpt-5.3-codex")
        );
        let saved = collapse_saved_cursor_models(AccountModelList {
            models: vec![
                "auto".to_string(),
                "gpt-5.3-codex-low".to_string(),
                "gpt-5.3-codex-xhigh".to_string(),
                "gpt-5.3-codex".to_string(),
                "composer-2.5".to_string(),
                "composer-2.5-fast".to_string(),
            ],
            default_model: Some("auto".to_string()),
            ..Default::default()
        });
        assert_eq!(
            saved.models,
            vec![
                "auto".to_string(),
                "gpt-5.3-codex".to_string(),
                "composer-2.5".to_string()
            ]
        );
        assert_eq!(saved.default_model.as_deref(), Some("auto"));

        let picker = parse_cursor_available_models(&json!({
            "models": [
                {"name": "default", "defaultOn": true, "supportsAgent": true, "clientDisplayName": "Auto"},
                {"name": "gpt-5.6-sol", "defaultOn": true, "supportsAgent": true, "namedModelSectionIndex": 1, "clientDisplayName": "GPT-5.6 Sol"},
                {"name": "composer-2.5", "defaultOn": false, "supportsAgent": true, "namedModelSectionIndex": 1, "clientDisplayName": "Composer 2.5"},
                {"name": "gpt-5.3-codex", "defaultOn": true, "supportsAgent": false, "namedModelSectionIndex": 1, "clientDisplayName": "Codex 5.3"},
                {"name": "grok-4.7", "defaultOn": true, "supportsAgent": true, "namedModelSectionIndex": 1, "clientDisplayName": "Grok 4.7", "vendor": {"id": "MODEL_VENDOR_ID_CURSOR"}}
            ]
        }));
        assert!(picker.picker);
        assert_eq!(picker.models, vec!["auto", "gpt-5.6-sol", "grok-4.7"]);
        assert_eq!(picker.labels, vec!["", "GPT-5.6 Sol", "Grok 4.7"]);
        assert_eq!(picker.sections, vec!["auto", "other", "cursor"]);

        let usage = parse_cursor_period_usage(&json!({
            "enabled": true,
            "displayMessage": "You've used 94% of your included usage",
            "planUsage": {
                "includedSpend": 37622,
                "limit": 40000,
                "autoPercentUsed": 8.2,
                "apiPercentUsed": 52.6,
                "totalPercentUsed": 11.6
            }
        }));
        assert_eq!(usage.cursor_models_percent, Some(8));
        assert_eq!(usage.other_models_percent, Some(53));
        let tiny = parse_cursor_period_usage(&json!({
            "planUsage": { "autoPercentUsed": 0.4, "includedSpend": 99, "limit": 100 }
        }));
        assert_eq!(tiny.cursor_models_percent, Some(1));
        assert_eq!(tiny.other_models_percent, None);
        assert_eq!(
            parse_cursor_period_usage(&json!({
                "enabled": false,
                "planUsage": { "autoPercentUsed": 8, "apiPercentUsed": 53 }
            })),
            CursorPeriodUsage::default()
        );
        assert_eq!(
            parse_cursor_period_usage(&json!({})),
            CursorPeriodUsage::default()
        );

        let workbuddy = parse_workbuddy_model_list(
            "  --model <model>  Model for the current session. Currently supported:\n                   (hy4-preview, hy3, deepseek-v4.1-flash,\n                   glm-5.3, custom-local:deepseek-flash)\n",
        );
        assert_eq!(
            workbuddy.models,
            vec!["hy4-preview", "hy3", "deepseek-v4.1-flash", "glm-5.3"]
        );
        assert_eq!(workbuddy.default_model.as_deref(), Some("hy4-preview"));
        assert_eq!(
            workbuddy.sections,
            vec!["workbuddy-own", "workbuddy-own", "other", "other"]
        );
        assert_eq!(workbuddy.labels[0], "混元 4 预览");
        assert_eq!(workbuddy.labels[1], "混元 3");
        let signed_in = parse_workbuddy_account_catalog(&json!([
            {
                "userId": "account",
                "data": {
                    "agents": [{
                        "name": "cli",
                        "models": ["hy4flash-free", "hy4std", "glm-5.3"]
                    }],
                    "models": [
                        {"id": "hy4flash-free", "name": "Hy4 flash", "isDefault": true},
                        {"id": "hy4flash", "name": "Hy4 flash", "isDefault": true},
                        {"id": "hy4std", "name": "Hy4"},
                        {"id": "glm-5.3", "name": "GLM-5.3"},
                        {"id": "hunyuan-image-alpha", "name": "Hunyuan Image Alpha"}
                    ]
                }
            }
        ]))
        .expect("signed-in catalog");
        assert_eq!(signed_in.models, vec!["hy4flash-free", "hy4std", "glm-5.3"]);
        assert_eq!(signed_in.labels, vec!["Hy4 flash", "Hy4", "GLM-5.3"]);
        assert_eq!(signed_in.default_model.as_deref(), Some("hy4flash-free"));
        assert_eq!(
            signed_in.sections,
            vec!["workbuddy-own", "workbuddy-own", "other"]
        );
        let codex_groups = parse_codex_model_list(&json!({
            "data": [
                {"id": "gpt-5.3-codex", "model": "gpt-5.3-codex", "isDefault": true},
                {"id": "gpt-5.2", "model": "gpt-5.2"}
            ]
        }));
        assert_eq!(codex_groups.sections, vec!["codex-coding", "codex-general"]);
        let chatgpt = parse_codex_chatgpt_models(&json!({
            "models": [
                {"slug": "gpt-5.2", "display_name": "GPT-5.2", "priority": 20, "visibility": "list"},
                {"slug": "hidden-codex", "display_name": "Hidden", "priority": 1, "visibility": "hide"},
                {"slug": "gpt-5.3-codex", "display_name": "GPT-5.3 Codex", "priority": 10, "visibility": "list"}
            ]
        }));
        assert_eq!(chatgpt.models, vec!["gpt-5.3-codex", "gpt-5.2"]);
        assert_eq!(chatgpt.labels, vec!["GPT-5.3 Codex", "GPT-5.2"]);
        assert_eq!(chatgpt.sections, vec!["codex-coding", "codex-general"]);
        let allowance = parse_codex_usage(&json!({
            "rate_limit": {
                "primary_window": {"used_percent": 12.4, "limit_window_seconds": 18000},
                "secondary_window": {"used_percent": 40.2, "limit_window_seconds": 604800}
            }
        }));
        assert_eq!(allowance.hours, Some(5));
        assert_eq!(allowance.hours_used_percent, Some(12));
        assert_eq!(allowance.week_used_percent, Some(40));
        assert_eq!(parse_codex_usage(&json!({})), CodexAllowance::default());

        let zcode_dir = tempfile::tempdir().unwrap();
        fs::write(
            zcode_dir.path().join("setting.json"),
            r#"{"providerFamilyDomain":"bigmodel"}"#,
        )
        .unwrap();
        // Desktop-only tokens (`zcodejwttoken` / unsynced `oauth:*`) do not entitle ZCode CLI.
        fs::write(
            zcode_dir.path().join("credentials.json"),
            r#"{"oauth:bigmodel:access_token":"tok","zcodejwttoken":"jwt"}"#,
        )
        .unwrap();
        assert!(zcode_account_models_from_dir(zcode_dir.path(), None)
            .models
            .is_empty());
        assert_eq!(
            resolve_zcode_account_launch_in_dir(zcode_dir.path(), None, None),
            None
        );

        // Once CLI login records `account-provider:*:identity`, Coding Plan models resolve
        // with a valid `reasoningLevel` option while Desktop-only `start-plan` stays excluded.
        fs::write(
            zcode_dir.path().join("credentials.json"),
            r#"{
                "oauth:bigmodel:access_token":"tok",
                "zcodejwttoken":"jwt",
                "account-provider:account:bigmodel-start-plan:identity":"desktop-only",
                "account-provider:account:bigmodel-individual-coding-plan:identity":"cli-id"
            }"#,
        )
        .unwrap();
        let zcode = zcode_account_models_from_dir(zcode_dir.path(), None);
        assert_eq!(zcode.models, vec!["GLM-5.3", "GLM-5.3-Flash"]);
        assert_eq!(zcode.default_model.as_deref(), Some("GLM-5.3"));
        assert_eq!(
            resolve_zcode_account_launch_in_dir(zcode_dir.path(), None, Some("GLM-5.3-Flash")),
            Some((
                "account:bigmodel-individual-coding-plan".to_string(),
                "GLM-5.3-Flash".to_string(),
                "high".to_string(),
            ))
        );
        assert_eq!(
            zcode_default_reasoning_level(
                "account:bigmodel-individual-coding-plan",
                "GLM-5-Turbo",
                None
            ),
            "enabled"
        );
    }
}
