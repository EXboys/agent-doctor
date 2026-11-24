//! Codex and Claude Code can use their own subscription login.
//!
//! The choice is stored per tool. A provider switch skips a tool that is on
//! its subscription. The provider page only offers the login when that
//! login site answers, or when the tool is already on the subscription.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

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
const QODER_ID: &str = "qoder";
const CURSOR_ID: &str = "cursor";
const WORKBUDDY_ID: &str = "workbuddy";
const ANTIGRAVITY_ID: &str = "antigravity";
const ZCODE_ID: &str = "zcode";
const DEEPSEEK_HARNESS_ID: &str = "deepseek-harness";
const CODEX_LOGIN_URL: &str = "https://auth.openai.com";
const CLAUDE_LOGIN_URL: &str = "https://claude.ai";
const LOGIN_WAIT: Duration = Duration::from_secs(180);

/// Prefix for per-session provider pins that target a runtime's own signed-in account
/// (e.g. `native:claude-code`, `native:codex`, `native:qoder`).
pub const NATIVE_PROVIDER_PREFIX: &str = "native:";

pub fn native_provider_id(runtime_id: &str) -> String {
    format!("{NATIVE_PROVIDER_PREFIX}{}", runtime_id.trim())
}

pub fn parse_native_provider_id(provider_id: Option<&str>) -> Option<&str> {
    let raw = provider_id?.trim();
    raw.strip_prefix(NATIVE_PROVIDER_PREFIX).map(str::trim)
}

pub fn is_native_provider_id(provider_id: Option<&str>) -> bool {
    parse_native_provider_id(provider_id).is_some()
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeAccountRow {
    pub runtime_id: String,
    pub using_subscription: bool,
    pub logged_in: bool,
    /// True when the companion desktop IDE (e.g. Qoder / Cursor) is signed in on this machine.
    #[serde(default)]
    pub desktop_logged_in: bool,
    /// True when the companion desktop IDE has an active login that the CLI has not yet synced.
    #[serde(default)]
    pub desktop_needs_sync: bool,
    pub reachable: bool,
    pub visible: bool,
    pub provider_name: String,
    /// Models this signed-in account listed. Empty until that list arrives.
    #[serde(default)]
    pub models: Vec<String>,
    /// Menu text for `models`, same order. Empty entry keeps the id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_labels: Vec<String>,
    /// `auto`, `cursor`, or `other`, same order as `models`. Empty keeps one list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_sections: Vec<String>,
    /// Cursor Models bar on the plan page, 0–100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_cursor_percent: Option<u8>,
    /// Other Models bar on the plan page, 0–100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_other_percent: Option<u8>,
    /// Short ChatGPT window length, in hours. The used percent is shared by every model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_hours: Option<u32>,
    /// Used percent of that short window, 0–100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_hours_percent: Option<u8>,
    /// Used percent of the weekly window, 0–100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_week_percent: Option<u8>,
    /// Credits still left on a Qoder or WorkBuddy account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_credits_left: Option<f64>,
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
    if !matches!(
        runtime_id,
        CODEX_ID | CLAUDE_ID | QODER_ID | CURSOR_ID | WORKBUDDY_ID | ZCODE_ID | ANTIGRAVITY_ID
    ) || read_mode(runtime_id) != MODE_SUBSCRIPTION
    {
        return false;
    }
    // Never pretend a subscription is active when the local account is not signed in.
    logged_in(runtime_id)
}

pub fn is_runtime_logged_in(runtime_id: &str) -> bool {
    logged_in(runtime_id)
}

/// Whether this Ask turn should run on `runtime_id`'s own signed-in account
/// instead of a configured API-key provider.
///
/// - `provider_id = Some("native:<rt>")`: explicit account selection — true iff
///   that runtime is currently signed in.
/// - `provider_id = Some("<saved-id>")`: explicit saved provider — false.
/// - `provider_id = None` / empty: follows global subscription mode unless the
///   session model names a foreign vendor gateway model not offered by the account.
pub fn turn_uses_native_account(
    runtime_id: &str,
    model: Option<&str>,
    provider_id: Option<&str>,
) -> bool {
    if let Some(target) = parse_native_provider_id(provider_id) {
        return (target.is_empty() || target == runtime_id) && logged_in(runtime_id);
    }
    if provider_id.map(str::trim).is_some_and(|id| !id.is_empty()) {
        return false;
    }
    if !is_native_subscription(runtime_id) {
        return false;
    }
    if matches!(runtime_id, CURSOR_ID | ZCODE_ID | ANTIGRAVITY_ID) {
        return true;
    }
    if let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) {
        if crate::setup::load_account_models(runtime_id).is_some_and(|list| list.allows(m)) {
            return true;
        }
    }
    !crate::session_env::chat_uses_saved_service(model, None)
}

pub fn native_account_status(_probe: bool) -> NativeAccountStatus {
    if ensure_edition_allows_mode(MODE_PERSONAL).is_err() {
        return NativeAccountStatus { rows: Vec::new() };
    }
    let provider_name = active_provider_name();
    let mut rows = vec![
        row(CODEX_ID, true, &provider_name),
        row(CLAUDE_ID, true, &provider_name),
    ];
    // Every installed subscription-capable CLI (or signed-in companion desktop IDE)
    // appears consistently; reachability is checked only when the user taps Sign in.
    for runtime_id in [QODER_ID, CURSOR_ID, WORKBUDDY_ID, ZCODE_ID, ANTIGRAVITY_ID] {
        if runtime_has_cli_or_headless(runtime_id) || desktop_login_state(runtime_id).0 {
            rows.push(row(runtime_id, true, &provider_name));
        }
    }
    // DeepSeek Harness signs in inside its own tool, so its row stays hidden from
    // the main-page login fold (`visible: false`) while Ask and the phone inventory
    // still learn whether its account is currently signed in.
    for runtime_id in [DEEPSEEK_HARNESS_ID] {
        if extra_account_runtime_installed(runtime_id) {
            let mut extra = row(runtime_id, false, &provider_name);
            extra.visible = false;
            rows.push(extra);
        }
    }
    NativeAccountStatus { rows }
}

pub fn use_native_subscription(runtime_id: &str) -> Result<NativeAccountSwitch> {
    ensure_edition_allows_mode(MODE_PERSONAL)?;
    let runtime_id = known_runtime(runtime_id)?;
    let provider_name = active_provider_name();
    if runtime_id == ANTIGRAVITY_ID {
        return use_agy_subscription(provider_name);
    }
    if matches!(runtime_id, QODER_ID | CURSOR_ID | WORKBUDDY_ID | ZCODE_ID) {
        return use_cli_subscription(runtime_id, provider_name);
    }
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

/// The saved sign-in can no longer refresh, or the user wants to switch/resync accounts.
pub fn resign_native_subscription(runtime_id: &str) -> Result<NativeAccountSwitch> {
    ensure_edition_allows_mode(MODE_PERSONAL)?;
    let runtime_id = known_runtime(runtime_id)?;
    forget_saved_login(runtime_id);
    let _ = write_mode(runtime_id, "service");
    use_native_subscription(runtime_id)
}

/// Sign out of this runtime's saved CLI account and revert it to the active personal provider.
pub fn sign_out_native_subscription(runtime_id: &str) -> Result<NativeAccountSwitch> {
    ensure_edition_allows_mode(MODE_PERSONAL)?;
    let runtime_id = known_runtime(runtime_id)?;
    forget_saved_login(runtime_id);
    let _ = write_mode(runtime_id, "service");
    if let Ok(switched) = use_current_service(runtime_id) {
        if switched.outcome == "reverted" {
            return Ok(switch(
                runtime_id,
                false,
                "signed_out",
                switched.provider_name,
            ));
        }
    }
    Ok(switch(
        runtime_id,
        false,
        "signed_out",
        active_provider_name(),
    ))
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
        WORKBUDDY_ID => {
            if provider.protocol == PROTOCOL_ANTHROPIC
                && crate::setup::merge::hermes_chat_base_url(&provider.url)
                    == provider.url.trim().trim_end_matches('/')
            {
                return Ok(switch(runtime_id, true, "unsupported", name));
            }
            crate::setup::merge::apply_workbuddy_with_model(
                &provider.url,
                &provider.api_key,
                Some(provider.model.as_str()),
            )?;
        }
        QODER_ID => {
            crate::setup::merge::apply_qoder_with_model(
                &provider.url,
                &provider.api_key,
                &provider.protocol,
                Some(provider.model.as_str()),
            )?;
        }
        CURSOR_ID | ZCODE_ID => {}
        ANTIGRAVITY_ID => {
            if agy_url_is_gemini_api(&provider.url) {
                crate::adapters::enable_gemini_key_provider();
            } else if agy_url_is_vertex(&provider.url) {
                crate::adapters::disable_gemini_key_provider();
            } else {
                return Ok(switch(runtime_id, true, "unsupported", name));
            }
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

fn runtime_has_cli_or_headless(runtime_id: &str) -> bool {
    match runtime_id {
        ZCODE_ID => crate::adapters::locate_zcode_headless().is_some(),
        _ => cli_binary(runtime_id).is_some(),
    }
}

fn row(runtime_id: &str, reachable: bool, provider_name: &str) -> NativeAccountRow {
    let using = is_native_subscription(runtime_id);
    let is_logged_in = logged_in(runtime_id);
    let (desktop_logged_in, desktop_needs_sync) = desktop_login_state(runtime_id);
    let installed = if matches!(
        runtime_id,
        QODER_ID | CURSOR_ID | WORKBUDDY_ID | ZCODE_ID | ANTIGRAVITY_ID
    ) {
        runtime_has_cli_or_headless(runtime_id) || desktop_logged_in
    } else if runtime_id == DEEPSEEK_HARNESS_ID {
        extra_account_runtime_installed(runtime_id)
    } else {
        discover_binary(binary_name(runtime_id))
            .binary_path
            .is_some()
    };
    let menu = account_menu(runtime_id, is_logged_in);
    let usage = if runtime_id == CURSOR_ID && is_logged_in {
        crate::setup::cursor_period_usage()
    } else {
        crate::setup::CursorPeriodUsage::default()
    };
    let allowance = if runtime_id == CODEX_ID && is_logged_in {
        crate::setup::codex_allowance()
    } else if runtime_id == ZCODE_ID && is_logged_in {
        let windows = crate::setup::zcode_plan_windows();
        crate::setup::CodexAllowance {
            hours: windows.hours,
            hours_used_percent: windows.hours_used_percent,
            week_used_percent: windows.week_used_percent,
        }
    } else {
        crate::setup::CodexAllowance::default()
    };
    let credits = if runtime_id == QODER_ID && is_logged_in {
        crate::setup::qoder_credits()
    } else if runtime_id == WORKBUDDY_ID && is_logged_in {
        crate::setup::workbuddy_credits()
    } else {
        crate::setup::CreditAllowance::default()
    };
    NativeAccountRow {
        runtime_id: runtime_id.to_string(),
        using_subscription: using,
        logged_in: is_logged_in,
        desktop_logged_in,
        desktop_needs_sync,
        reachable,
        visible: account_row_visible(using, reachable, installed),
        provider_name: provider_name.to_string(),
        models: menu.0,
        model_labels: menu.1,
        model_sections: menu.2,
        quota_cursor_percent: usage.cursor_models_percent,
        quota_other_percent: usage.other_models_percent,
        quota_hours: allowance.hours,
        quota_hours_percent: allowance.hours_used_percent,
        quota_week_percent: allowance.week_used_percent,
        quota_credits_left: credits.left,
    }
}

fn account_menu(runtime_id: &str, logged_in: bool) -> (Vec<String>, Vec<String>, Vec<String>) {
    if !logged_in {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let list = if runtime_id == CURSOR_ID {
        cursor_picker_list()
    } else if runtime_id == CODEX_ID {
        codex_catalog_list()
    } else if runtime_id == WORKBUDDY_ID {
        workbuddy_catalog_list()
    } else {
        crate::setup::load_account_models(runtime_id)
    };
    let Some(list) = list else {
        return (Vec::new(), Vec::new(), Vec::new());
    };
    let aligned = list.labels.len() == list.models.len();
    let labels = if aligned { list.labels } else { Vec::new() };
    let sections = if aligned && list.sections.len() == list.models.len() {
        list.sections
    } else {
        Vec::new()
    };
    (list.models, labels, sections)
}

static CURSOR_PICKER_REFRESH_FAILED: AtomicBool = AtomicBool::new(false);
static CODEX_CATALOG_REFRESH_FAILED: AtomicBool = AtomicBool::new(false);

/// The ChatGPT catalog has a display name on each model. An older app-server
/// list does not. Replace that list once with the account's own catalog.
fn codex_catalog_named(list: &crate::setup::AccountModelList) -> bool {
    !list.models.is_empty()
        && list.labels.len() == list.models.len()
        && list.labels.iter().any(|label| !label.trim().is_empty())
}

/// Prefer the model list this WorkBuddy sign-in saved. Refresh once when the
/// saved list is still the command-line help text.
fn workbuddy_catalog_list() -> Option<crate::setup::AccountModelList> {
    let live = crate::setup::workbuddy_account_catalog();
    let saved = crate::setup::load_account_models(WORKBUDDY_ID);
    let Some(live) = live else {
        return saved;
    };
    let same = saved.as_ref().is_some_and(|list| {
        list.models == live.models && list.labels == live.labels && list.sections == live.sections
    });
    if same {
        return saved;
    }
    match crate::setup::refresh_account_models(WORKBUDDY_ID) {
        Ok(list) if !list.models.is_empty() => Some(list),
        _ => Some(live),
    }
}

fn codex_catalog_list() -> Option<crate::setup::AccountModelList> {
    let saved = crate::setup::load_account_models(CODEX_ID);
    if saved.as_ref().is_some_and(codex_catalog_named) {
        return saved;
    }
    if CODEX_CATALOG_REFRESH_FAILED.load(Ordering::Relaxed) {
        return saved;
    }
    match crate::setup::refresh_account_models(CODEX_ID) {
        Ok(list) => {
            if !codex_catalog_named(&list) {
                CODEX_CATALOG_REFRESH_FAILED.store(true, Ordering::Relaxed);
            }
            Some(list)
        }
        Err(_) => {
            CODEX_CATALOG_REFRESH_FAILED.store(true, Ordering::Relaxed);
            saved
        }
    }
}

/// The saved Cursor list used to be every model. Replace it once with the
/// short list the Cursor client shows.
fn cursor_menu_grouped(list: &crate::setup::AccountModelList) -> bool {
    list.picker
        && list.sections.len() == list.models.len()
        && list
            .sections
            .iter()
            .any(|section| section == "cursor" || section == "other")
}

fn cursor_picker_list() -> Option<crate::setup::AccountModelList> {
    let saved = crate::setup::load_account_models(CURSOR_ID);
    if saved.as_ref().is_some_and(cursor_menu_grouped) {
        return saved;
    }
    if CURSOR_PICKER_REFRESH_FAILED.load(Ordering::Relaxed) {
        return saved;
    }
    match crate::setup::refresh_account_models(CURSOR_ID) {
        Ok(list) => {
            if !cursor_menu_grouped(&list) {
                CURSOR_PICKER_REFRESH_FAILED.store(true, Ordering::Relaxed);
            }
            Some(list)
        }
        Err(_) => {
            CURSOR_PICKER_REFRESH_FAILED.store(true, Ordering::Relaxed);
            saved
        }
    }
}

fn extra_account_runtime_installed(runtime_id: &str) -> bool {
    match runtime_id {
        ZCODE_ID => crate::adapters::locate_zcode_headless().is_some(),
        DEEPSEEK_HARNESS_ID => discover_binary(crate::adapters::DEEPSEEK_HARNESS_CLI)
            .binary_path
            .is_some(),
        _ => false,
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
        QODER_ID => Ok(QODER_ID),
        CURSOR_ID => Ok(CURSOR_ID),
        WORKBUDDY_ID => Ok(WORKBUDDY_ID),
        ZCODE_ID => Ok(ZCODE_ID),
        ANTIGRAVITY_ID => Ok(ANTIGRAVITY_ID),
        other => bail!("subscription login is only for an official command line, not {other}"),
    }
}

fn cli_binary(runtime_id: &str) -> Option<PathBuf> {
    match runtime_id {
        QODER_ID => crate::adapters::qoder_cli_path(),
        CURSOR_ID => crate::adapters::cursor_cli_path(),
        WORKBUDDY_ID => crate::adapters::workbuddy_cli_path(),
        ANTIGRAVITY_ID => crate::adapters::agy_cli_path(),
        _ => None,
    }
}

fn use_cli_subscription(runtime_id: &str, provider_name: String) -> Result<NativeAccountSwitch> {
    match runtime_id {
        QODER_ID => return use_qoder_subscription(provider_name),
        WORKBUDDY_ID => return use_workbuddy_subscription(provider_name),
        ZCODE_ID => return use_zcode_subscription(provider_name),
        _ => {}
    }
    let Some(binary) = cli_binary(runtime_id) else {
        return Ok(switch(runtime_id, false, "missing_tool", provider_name));
    };
    if cli_logged_in(runtime_id) {
        let _ = crate::setup::refresh_account_models(runtime_id);
        write_mode(runtime_id, MODE_SUBSCRIPTION)?;
        return Ok(switch(runtime_id, true, "using", provider_name));
    }
    let _ = spawn_cli_login(&binary);
    if cli_logged_in(runtime_id) {
        let _ = crate::setup::refresh_account_models(runtime_id);
        write_mode(runtime_id, MODE_SUBSCRIPTION)?;
        return Ok(switch(runtime_id, true, "using", provider_name));
    }
    Ok(switch(runtime_id, false, "login_incomplete", provider_name))
}

fn use_workbuddy_subscription(provider_name: String) -> Result<NativeAccountSwitch> {
    let Some(binary) = crate::adapters::workbuddy_cli_path() else {
        return Ok(switch(WORKBUDDY_ID, false, "missing_tool", provider_name));
    };
    let codebuddy_dir = home_join(".codebuddy");
    let _ = crate::setup::merge::clear_workbuddy_provider_override_in_dir(&codebuddy_dir);
    if workbuddy_logged_in() {
        let _ = crate::setup::refresh_account_models(WORKBUDDY_ID);
        write_mode(WORKBUDDY_ID, MODE_SUBSCRIPTION)?;
        return Ok(switch(WORKBUDDY_ID, true, "using", provider_name));
    }
    let ok = spawn_workbuddy_acp_login(&binary, &codebuddy_dir).unwrap_or(false);
    if ok || workbuddy_logged_in() {
        record_workbuddy_login_marker(&codebuddy_dir);
        let _ = crate::setup::refresh_account_models(WORKBUDDY_ID);
        write_mode(WORKBUDDY_ID, MODE_SUBSCRIPTION)?;
        return Ok(switch(WORKBUDDY_ID, true, "using", provider_name));
    }
    Ok(switch(
        WORKBUDDY_ID,
        false,
        "login_incomplete",
        provider_name,
    ))
}

fn use_zcode_subscription(provider_name: String) -> Result<NativeAccountSwitch> {
    let Some(launch) = crate::adapters::locate_zcode_headless() else {
        return Ok(switch(ZCODE_ID, false, "missing_tool", provider_name));
    };
    let family = zcode_preferred_family();
    if zcode_logged_in() && !zcode_needs_family_sync(family) {
        let _ = crate::setup::refresh_account_models(ZCODE_ID);
        write_mode(ZCODE_ID, MODE_SUBSCRIPTION)?;
        return Ok(switch(ZCODE_ID, true, "using", provider_name));
    }
    let _ = spawn_zcode_login(&launch, family);
    if zcode_logged_in() {
        let _ = crate::setup::refresh_account_models(ZCODE_ID);
        write_mode(ZCODE_ID, MODE_SUBSCRIPTION)?;
        return Ok(switch(ZCODE_ID, true, "using", provider_name));
    }
    Ok(switch(ZCODE_ID, false, "login_incomplete", provider_name))
}

fn agy_url_is_gemini_api(url: &str) -> bool {
    url.to_ascii_lowercase()
        .contains("generativelanguage.googleapis.com")
}

fn agy_url_is_vertex(url: &str) -> bool {
    url.to_ascii_lowercase()
        .contains("aiplatform.googleapis.com")
}

/// One Google login on this computer. The command is `agy acp --login`.
fn use_agy_subscription(provider_name: String) -> Result<NativeAccountSwitch> {
    let Some(binary) = crate::adapters::agy_cli_path() else {
        return Ok(switch(ANTIGRAVITY_ID, false, "missing_tool", provider_name));
    };
    crate::adapters::disable_gemini_key_provider();
    if agy_logged_in() {
        write_mode(ANTIGRAVITY_ID, MODE_SUBSCRIPTION)?;
        return Ok(switch(ANTIGRAVITY_ID, true, "using", provider_name));
    }
    let _ = spawn_agy_login(&binary);
    if agy_logged_in() {
        write_mode(ANTIGRAVITY_ID, MODE_SUBSCRIPTION)?;
        return Ok(switch(ANTIGRAVITY_ID, true, "using", provider_name));
    }
    Ok(switch(
        ANTIGRAVITY_ID,
        false,
        "login_incomplete",
        provider_name,
    ))
}

fn spawn_agy_login(binary: &Path) -> Result<()> {
    let mut cmd = crate::exec::command_for_path(binary);
    cmd.args(["acp", "--login"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let open_shim = install_macos_login_open_shim(&mut cmd);
    let mut child = cmd.spawn()?;
    let opened = Arc::new(AtomicBool::new(false));
    if let Some(pipe) = child.stdout.take() {
        watch_login_output_mode(pipe, Arc::clone(&opened), true);
    }
    if let Some(pipe) = child.stderr.take() {
        watch_login_output_mode(pipe, Arc::clone(&opened), true);
    }
    let started = Instant::now();
    loop {
        drain_macos_login_open_shim(open_shim.as_ref(), &opened);
        if agy_logged_in() {
            wait_for_login_exit(&mut child);
            return Ok(());
        }
        if started.elapsed() > LOGIN_WAIT {
            let _ = child.kill();
            let _ = child.wait();
            bail!("login timed out");
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                drain_macos_login_open_shim(open_shim.as_ref(), &opened);
                return Ok(());
            }
            Ok(None) => thread::sleep(Duration::from_millis(200)),
            Err(_) => return Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QoderDesktopKind {
    Cn,
    Global,
}

fn use_qoder_subscription(provider_name: String) -> Result<NativeAccountSwitch> {
    let desktop_kind = qoder_desktop_login_kind();
    // When the user's Qoder CN desktop IDE is signed in, align the CLI to `qoderclicn`
    // (installing it automatically if only global `qodercli` was present) so browser
    // authorization reuses their signed-in `qoder.com.cn` session in one click. Never
    // fall back to global `qodercli` here — signing into `qoder.com` would leave the
    // CN account unsynced.
    let (binary, config_dir) = if desktop_kind == Some(QoderDesktopKind::Cn) {
        if crate::adapters::qoder_cn_cli_path().is_none() {
            let _ = install_qoder_cn_cli();
        }
        let Some(bin) = crate::adapters::qoder_cn_cli_path() else {
            return Ok(switch(QODER_ID, false, "missing_tool", provider_name));
        };
        (bin, home_join(".qoder-cn"))
    } else {
        let Some(bin) = crate::adapters::qoder_cli_path() else {
            return Ok(switch(QODER_ID, false, "missing_tool", provider_name));
        };
        (bin, crate::adapters::qoder_config_dir())
    };
    let _ = crate::setup::merge::clear_qoder_provider_override_in_dir(&config_dir);
    if qoder_dir_logged_in(&config_dir) {
        let _ = crate::setup::refresh_account_models(QODER_ID);
        write_mode(QODER_ID, MODE_SUBSCRIPTION)?;
        return Ok(switch(QODER_ID, true, "using", provider_name));
    }
    let _ = spawn_cli_login(&binary);
    if qoder_dir_logged_in(&config_dir) {
        let _ = crate::setup::refresh_account_models(QODER_ID);
        write_mode(QODER_ID, MODE_SUBSCRIPTION)?;
        return Ok(switch(QODER_ID, true, "using", provider_name));
    }
    Ok(switch(QODER_ID, false, "login_incomplete", provider_name))
}

fn install_qoder_cn_cli() -> Result<()> {
    // Prefer the lightweight `@qodercn-ai/qoderclicn` npm bundle (~35 MB JS bundle vs
    // ~70 MB Bun archive on `static.qoder.com.cn`, which often crawls or times out when
    // TUN proxies are active). Try npmjs first, then npmmirror, then the official script.
    let npm_name = if cfg!(windows) { "npm.cmd" } else { "npm" };
    if let Some(npm) = crate::adapters::util::find_binary(npm_name) {
        for registry in [
            "https://registry.npmjs.org",
            "https://registry.npmmirror.com",
        ] {
            let _ = crate::exec::command_for_path(&npm)
                .args([
                    "install",
                    "-g",
                    "@qodercn-ai/qoderclicn@latest",
                    "--omit=optional",
                    "--no-audit",
                    "--no-fund",
                    &format!("--registry={registry}"),
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            crate::adapters::refresh_managed_runtime_path();
            if crate::adapters::qoder_cn_cli_path().is_some() {
                return Ok(());
            }
        }
    }

    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(10))
        .build()?;
    #[cfg(windows)]
    {
        let script = client
            .get("https://static.qoder.com.cn/qoder-cli-cn/install.ps1")
            .send()?
            .error_for_status()?
            .text()?;
        let dir = tempfile::tempdir()?;
        let script_path = dir.path().join("install-qoderclicn.ps1");
        fs::write(&script_path, script)?;
        let mut cmd = crate::exec::command_for_path(Path::new("powershell"));
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            cmd.env_remove(key);
        }
        let _ = cmd
            .env("NO_PROXY", "*")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &script_path.to_string_lossy(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let script = client
            .get("https://static.qoder.com.cn/qoder-cli-cn/install.sh")
            .send()?
            .error_for_status()?
            .text()?;
        let dir = tempfile::tempdir()?;
        let script_path = dir.path().join("install-qoderclicn.sh");
        fs::write(&script_path, script)?;
        let mut cmd = crate::exec::command_for_path(Path::new("bash"));
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            cmd.env_remove(key);
        }
        let _ = cmd
            .env("NO_PROXY", "*")
            .arg(&script_path)
            .arg("--skip-path")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    crate::adapters::refresh_managed_runtime_path();
    Ok(())
}

fn qoder_dir_logged_in(dir: &Path) -> bool {
    qoder_auth_dir_has_user(&dir.join(".auth"))
        || read_json(&dir.join("auth.json")).is_some_and(|value| tokenish(&value))
}

/// `(desktop_logged_in, desktop_needs_sync)` for runtimes that have a companion desktop IDE.
fn desktop_login_state(runtime_id: &str) -> (bool, bool) {
    match runtime_id {
        QODER_ID => match qoder_desktop_login_kind() {
            Some(QoderDesktopKind::Cn) => {
                let cn_cli_ready = crate::adapters::qoder_cn_cli_path().is_some()
                    && qoder_dir_logged_in(&home_join(".qoder-cn"));
                (true, !cn_cli_ready)
            }
            Some(QoderDesktopKind::Global) => {
                let global_cli_ready = qoder_dir_logged_in(&home_join(".qoder"));
                (true, !global_cli_ready)
            }
            None => (false, false),
        },
        CURSOR_ID => {
            let desktop = cursor_desktop_logged_in();
            (desktop, desktop && !cursor_logged_in())
        }
        WORKBUDDY_ID => {
            let desktop = workbuddy_desktop_logged_in();
            (desktop, desktop && !workbuddy_logged_in())
        }
        ZCODE_ID => {
            let desktop = zcode_desktop_logged_in();
            let cli_ready = zcode_logged_in() && !zcode_needs_family_sync(zcode_preferred_family());
            (desktop, desktop && !cli_ready)
        }
        _ => (false, false),
    }
}

pub(crate) fn qoder_desktop_login_kind() -> Option<QoderDesktopKind> {
    let mut bases = Vec::new();
    if using_test_home() {
        bases.push(home_join("Library/Application Support"));
        bases.push(home_join(".config"));
    } else {
        if let Some(cfg) = dirs::config_dir() {
            bases.push(cfg);
        }
        if let Some(local) = dirs::data_local_dir() {
            if !bases.contains(&local) {
                bases.push(local);
            }
        }
    }
    for base in &bases {
        for folder in ["QoderCN", "Qoder CN"] {
            let status_path = base
                .join(folder)
                .join("SharedClientCache")
                .join("cache")
                .join("status.json");
            if qoder_desktop_status_is_signed_in(&status_path) {
                return Some(QoderDesktopKind::Cn);
            }
        }
    }
    for base in &bases {
        for folder in ["Qoder", "Qoder IDE"] {
            let status_path = base
                .join(folder)
                .join("SharedClientCache")
                .join("cache")
                .join("status.json");
            if qoder_desktop_status_is_signed_in(&status_path) {
                return Some(QoderDesktopKind::Global);
            }
        }
    }
    None
}

pub(crate) fn qoder_desktop_status_is_signed_in(path: &Path) -> bool {
    read_json(path)
        .and_then(|value| value.get("logged_in").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn cursor_desktop_logged_in() -> bool {
    let mut dbs = Vec::new();
    if using_test_home() {
        dbs.push(home_join(
            "Library/Application Support/Cursor/User/globalStorage/state.vscdb",
        ));
    } else if let Some(cfg) = dirs::config_dir() {
        dbs.push(
            cfg.join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb"),
        );
    }
    dbs.into_iter().any(|path| cursor_state_db_has_token(&path))
}

pub(crate) fn cursor_state_db_has_token(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let Ok(conn) = rusqlite::Connection::open_with_flags(path, flags) else {
        return false;
    };
    let _ = conn.busy_timeout(Duration::from_millis(200));
    conn.query_row(
        "SELECT 1 FROM ItemTable WHERE key = 'cursorAuth/accessToken' AND length(trim(value)) > 0 LIMIT 1",
        [],
        |_| Ok(true),
    )
    .unwrap_or(false)
}

fn cli_logged_in(runtime_id: &str) -> bool {
    match runtime_id {
        QODER_ID => {
            let dir = if using_test_home() {
                home_join(".qoder")
            } else {
                crate::adapters::qoder_config_dir()
            };
            qoder_auth_dir_has_user(&dir.join(".auth"))
                || read_json(&dir.join("auth.json")).is_some_and(|value| tokenish(&value))
        }
        CURSOR_ID => cursor_logged_in(),
        WORKBUDDY_ID => workbuddy_logged_in(),
        ZCODE_ID => zcode_logged_in(),
        ANTIGRAVITY_ID => agy_logged_in(),
        DEEPSEEK_HARNESS_ID => dsh_logged_in(),
        _ => false,
    }
}

fn cursor_logged_in() -> bool {
    if read_json(&home_join(".cursor/cli-config.json"))
        .is_some_and(|value| cursor_cli_config_is_signed_in(&value))
    {
        return true;
    }
    for path in cursor_auth_file_candidates() {
        if read_json(&path).is_some_and(|value| cursor_auth_file_is_signed_in(&value)) {
            return true;
        }
    }
    for env_key in ["CURSOR_API_KEY", "CURSOR_AUTH_TOKEN"] {
        if std::env::var(env_key).is_ok_and(|value| !value.trim().is_empty()) {
            return true;
        }
    }
    macos_keychain_has_cursor_auth()
}

fn cursor_auth_file_candidates() -> Vec<PathBuf> {
    let mut paths = vec![
        home_join(".cursor/auth.json"),
        home_join(".config/cursor/auth.json"),
    ];
    if !using_test_home() {
        if let Some(cfg) = dirs::config_dir() {
            paths.push(cfg.join("Cursor").join("auth.json"));
            paths.push(cfg.join("cursor").join("auth.json"));
        }
    }
    paths
}

pub(crate) fn cursor_auth_file_is_signed_in(value: &Value) -> bool {
    tokenish(value) || nonempty_str(value.get("apiKey"))
}

const WORKBUDDY_AUTH_MARKER: &str = ".agent-doctor-auth.json";
const WORKBUDDY_SIGNOUT_MARKER: &str = ".agent-doctor-signed-out";

fn workbuddy_desktop_logged_in() -> bool {
    let mut bases = Vec::new();
    if using_test_home() {
        bases.push(home_join("Library/Application Support"));
        bases.push(home_join(".config"));
    } else {
        if let Some(cfg) = dirs::config_dir() {
            bases.push(cfg);
        }
        if let Some(local) = dirs::data_local_dir() {
            if !bases.contains(&local) {
                bases.push(local);
            }
        }
    }
    for base in &bases {
        for folder in ["WorkBuddy", "WorkBuddy CN", "CodeBuddy"] {
            let status_path = base
                .join(folder)
                .join("SharedClientCache")
                .join("cache")
                .join("status.json");
            if qoder_desktop_status_is_signed_in(&status_path) {
                return true;
            }
            let vscdb = base
                .join(folder)
                .join("User")
                .join("globalStorage")
                .join("state.vscdb");
            if workbuddy_state_db_has_token(&vscdb) {
                return true;
            }
        }
    }
    false
}

pub(crate) fn workbuddy_state_db_has_token(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let Ok(conn) = rusqlite::Connection::open_with_flags(path, flags) else {
        return false;
    };
    let _ = conn.busy_timeout(Duration::from_millis(200));
    conn.query_row(
        "SELECT 1 FROM ItemTable WHERE (lower(key) LIKE '%codebuddy%auth%' OR lower(key) LIKE '%workbuddy%auth%' OR lower(key) LIKE '%tencent%auth%' OR key = 'cursorAuth/accessToken') AND length(trim(value)) > 2 AND trim(value) != '{}' AND trim(value) != '[]' LIMIT 1",
        [],
        |_| Ok(true),
    )
    .unwrap_or(false)
}

fn record_workbuddy_login_marker(codebuddy_dir: &Path) {
    let _ = fs::create_dir_all(codebuddy_dir);
    let _ = fs::remove_file(codebuddy_dir.join(WORKBUDDY_SIGNOUT_MARKER));
    let _ = fs::write(
        codebuddy_dir.join(WORKBUDDY_AUTH_MARKER),
        r#"{"logged_in":true}"#,
    );
}

fn workbuddy_logged_in() -> bool {
    let dir = home_join(".codebuddy");
    if read_json(&dir.join("auth.json")).is_some_and(|value| tokenish(&value))
        || read_json(&dir.join("credentials.json")).is_some_and(|value| tokenish(&value))
        || read_json(&dir.join(WORKBUDDY_AUTH_MARKER)).is_some_and(|value| {
            value
                .get("logged_in")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
    {
        return true;
    }
    let signed_out_at = fs::metadata(dir.join(WORKBUDDY_SIGNOUT_MARKER))
        .and_then(|m| m.modified())
        .ok();
    workbuddy_logs_show_signed_in_since(&dir.join("logs"), signed_out_at)
}

/// CodeBuddy encrypts its `local_storage` entries and also creates an
/// `anonymous_*` session whenever `CODEBUDDY_API_KEY` runs. Scan recent startup
/// logs newest-first: skip pure `anonymous_*` API-key runs, stop false on an
/// explicit user sign-out, and return true when a real Tencent / WeChat user
/// (`uid=<id>`, not `<none>` and not `anonymous_*`) signed in after `since`.
fn workbuddy_logs_show_signed_in_since(logs_dir: &Path, since: Option<SystemTime>) -> bool {
    let mut files = sorted_workbuddy_logs_newest_first(logs_dir);
    for (modified, path) in files.drain(..).take(24) {
        if since.is_some_and(|cutoff| modified <= cutoff) {
            break;
        }
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        match classify_workbuddy_log_text(&raw) {
            Some(true) => return true,
            Some(false) => return false,
            None => continue,
        }
    }
    false
}

fn sorted_workbuddy_logs_newest_first(logs_dir: &Path) -> Vec<(SystemTime, PathBuf)> {
    let Ok(entries) = fs::read_dir(logs_dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let Ok(files) = fs::read_dir(&path) else {
                continue;
            };
            for file_entry in files.flatten() {
                let fpath = file_entry.path();
                if fpath.extension().and_then(|ext| ext.to_str()) == Some("log") {
                    if let Ok(modified) = file_entry.metadata().and_then(|m| m.modified()) {
                        out.push((modified, fpath));
                    }
                }
            }
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("log") {
            if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                out.push((modified, path));
            }
        }
    }
    out.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    out
}

/// `Some(true)` = real user signed in; `Some(false)` = explicit sign-out;
/// `None` = neutral/anonymous API-key session only.
fn classify_workbuddy_log_text(raw: &str) -> Option<bool> {
    let mut state: Option<bool> = None;
    for line in raw.lines() {
        if !line.contains("[AuthenticationManager]")
            && !line.contains("[AuthenticationSessionListener]")
        {
            continue;
        }
        if let Some(idx) = line.find("[auth_success:") {
            let rest = line[idx + "[auth_success:".len()..].trim_start();
            let uid = rest.split(']').next().unwrap_or("").trim();
            if is_real_workbuddy_uid(uid) {
                state = Some(true);
            }
            continue;
        }
        if let Some(idx) = line.find("uid=") {
            let rest = &line[idx + "uid=".len()..];
            let uid = rest.split_whitespace().next().unwrap_or("").trim();
            if line.contains("hasAccessToken=true") && is_real_workbuddy_uid(uid) {
                state = Some(true);
            }
            continue;
        }
        if line.contains("userId changed:")
            && line.ends_with("-> <empty>")
            && !line.contains("<init> -> <empty>")
        {
            state = Some(false);
        }
    }
    state
}

#[cfg(test)]
pub(crate) fn workbuddy_log_text_is_signed_in(raw: &str) -> bool {
    classify_workbuddy_log_text(raw) == Some(true)
}

fn is_real_workbuddy_uid(uid: &str) -> bool {
    let trimmed = uid.trim();
    !trimmed.is_empty()
        && trimmed != "<none>"
        && trimmed != "<empty>"
        && !trimmed.starts_with("anonymous_")
}

fn zcode_preferred_family() -> &'static str {
    if let Some(setting) = read_json(&home_join(".zcode/v2/setting.json")) {
        if let Some(domain) = setting.get("providerFamilyDomain").and_then(Value::as_str) {
            if domain.trim().eq_ignore_ascii_case("zai") {
                return "zai";
            }
            if domain.trim().eq_ignore_ascii_case("bigmodel") {
                return "bigmodel";
            }
        }
    }
    if let Some(creds) = read_json(&home_join(".zcode/v2/credentials.json")) {
        if nonempty_str(creds.get("oauth:zai:access_token"))
            && !nonempty_str(creds.get("oauth:bigmodel:access_token"))
        {
            return "zai";
        }
    }
    "bigmodel"
}

fn zcode_needs_family_sync(preferred_family: &str) -> bool {
    let Some(creds) = read_json(&home_join(".zcode/v2/credentials.json")) else {
        return true;
    };
    let Some(map) = creds.as_object() else {
        return true;
    };
    let family_prefix = format!("account-provider:account:{preferred_family}-");
    let has_family_identity = map.iter().any(|(k, v)| {
        k.starts_with(&family_prefix)
            && k.ends_with(":identity")
            && !k.ends_with("-start-plan:identity")
            && nonempty_str(Some(v))
    });
    !has_family_identity
}

fn zcode_logged_in() -> bool {
    read_json(&home_join(".zcode/v2/credentials.json"))
        .is_some_and(|value| zcode_cli_credentials_are_signed_in(&value))
}

fn zcode_desktop_logged_in() -> bool {
    read_json(&home_join(".zcode/v2/credentials.json"))
        .is_some_and(|value| zcode_credentials_are_signed_in(&value))
}

pub(crate) fn zcode_cli_credentials_are_signed_in(value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    map.iter().any(|(key, val)| {
        key.starts_with("account-provider:")
            && key.ends_with(":identity")
            && !key.ends_with("-start-plan:identity")
            && nonempty_str(Some(val))
    })
}

pub(crate) fn zcode_credentials_are_signed_in(value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    for key in [
        "oauth:zai:access_token",
        "oauth:bigmodel:access_token",
        "zcodejwttoken",
    ] {
        if nonempty_str(map.get(key)) {
            return true;
        }
    }
    zcode_cli_credentials_are_signed_in(value)
}

fn dsh_logged_in() -> bool {
    let path = if using_test_home() {
        home_join(".dsh/.credentials.yaml")
    } else {
        crate::adapters::DeepSeekHarnessAdapter::credentials_path()
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    dsh_credentials_are_signed_in(&raw)
}

pub(crate) fn dsh_credentials_are_signed_in(raw: &str) -> bool {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(raw) else {
        return false;
    };
    yaml_has_auth_token(&value)
}

fn yaml_has_auth_token(value: &serde_yaml::Value) -> bool {
    match value {
        serde_yaml::Value::Mapping(map) => map.iter().any(|(k, v)| {
            let key = k.as_str().unwrap_or("");
            if matches!(
                key,
                "access_token"
                    | "accessToken"
                    | "token"
                    | "refresh_token"
                    | "refreshToken"
                    | "apiKey"
                    | "api_key"
                    | "DEEPSEEK_API_KEY"
            ) {
                v.as_str().is_some_and(|s| !s.trim().is_empty())
            } else {
                yaml_has_auth_token(v)
            }
        }),
        serde_yaml::Value::Sequence(items) => items.iter().any(yaml_has_auth_token),
        _ => false,
    }
}

fn using_test_home() -> bool {
    dirs::home_dir().as_deref() != Some(crate::adapters::util::home_dir().as_path())
}

pub(crate) fn qoder_auth_dir_has_user(auth_dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(auth_dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_user_record = name == "user" || name.starts_with("user.");
        is_user_record
            && entry
                .metadata()
                .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
    })
}

fn tokenish(value: &Value) -> bool {
    nonempty_str(value.get("access_token"))
        || nonempty_str(value.get("accessToken"))
        || nonempty_str(value.get("token"))
        || nonempty_str(value.get("refresh_token"))
        || value.get("tokens").is_some()
}

fn nonempty_str(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|text| !text.trim().is_empty())
}

pub(crate) fn cursor_cli_config_is_signed_in(value: &Value) -> bool {
    nonempty_str(value.get("accessToken"))
        || nonempty_str(value.get("authToken"))
        || value.get("auth").is_some_and(|auth| {
            nonempty_str(auth.get("accessToken")) || nonempty_str(auth.get("token"))
        })
}

fn spawn_workbuddy_acp_login(binary: &Path, codebuddy_dir: &Path) -> Result<bool> {
    let started_sys = SystemTime::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(SystemTime::now);
    let mut cmd = crate::exec::command_for_path(binary);
    cmd.arg("--acp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "CODEBUDDY_API_KEY",
        "CODEBUDDY_BASE_URL",
        "CODEBUDDY_MODEL",
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
    ] {
        cmd.env_remove(key);
    }
    let open_shim = install_macos_login_open_shim(&mut cmd);
    let mut child = cmd.spawn()?;
    let mut stdin = child.stdin.take();
    let opened = Arc::new(AtomicBool::new(false));
    if let Some(pipe) = child.stderr.take() {
        watch_login_output_mode(pipe, Arc::clone(&opened), false);
    }
    let (tx, rx) = mpsc::channel::<String>();
    if let Some(stdout) = child.stdout.take() {
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
    }
    if let Some(ref mut pipe) = stdin {
        for payload in [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientInfo":{"name":"agent_doctor","version":"0"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"authenticate","params":{"methodId":"internal"}}"#,
        ] {
            let _ = writeln!(pipe, "{payload}");
        }
        let _ = pipe.flush();
    }

    let logs_dir = codebuddy_dir.join("logs");
    let started = Instant::now();
    let mut authed = false;
    loop {
        drain_macos_login_open_shim(open_shim.as_ref(), &opened);
        if !opened.load(Ordering::SeqCst) {
            if let Some(url) = find_workbuddy_log_auth_url(&logs_dir, started_sys) {
                if !opened.swap(true, Ordering::SeqCst) {
                    let _ = crate::session_launch::open_url(&url);
                }
            }
        }
        while let Ok(line) = rx.try_recv() {
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if msg.get("id").and_then(Value::as_u64) == Some(2) {
                if msg.get("error").is_none() {
                    authed = true;
                } else {
                    drop(stdin);
                    wait_for_login_exit(&mut child);
                    return Ok(false);
                }
            }
        }
        if authed || workbuddy_logs_show_signed_in_since(&logs_dir, Some(started_sys)) {
            drop(stdin);
            wait_for_login_exit(&mut child);
            return Ok(true);
        }
        if started.elapsed() > LOGIN_WAIT {
            drop(stdin);
            let _ = child.kill();
            let _ = child.wait();
            bail!("login timed out");
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                drain_macos_login_open_shim(open_shim.as_ref(), &opened);
                return Ok(
                    authed || workbuddy_logs_show_signed_in_since(&logs_dir, Some(started_sys))
                );
            }
            Ok(None) => thread::sleep(Duration::from_millis(200)),
            Err(_) => return Ok(false),
        }
    }
}

fn find_workbuddy_log_auth_url(logs_dir: &Path, since: SystemTime) -> Option<String> {
    for (modified, path) in sorted_workbuddy_logs_newest_first(logs_dir)
        .into_iter()
        .take(4)
    {
        if modified < since {
            break;
        }
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        for line in raw.lines().rev() {
            if let Some(idx) = line.find("Opening auth URL: ") {
                let tail = line[idx + "Opening auth URL: ".len()..].trim();
                if let Some(url) = login_url_at_end(tail) {
                    return Some(url);
                }
            }
        }
    }
    None
}

fn spawn_zcode_login(launch: &crate::adapters::ZcodeHeadless, family: &str) -> Result<()> {
    let mut cmd = crate::adapters::zcode_headless_command(launch);
    crate::session_env::scrub_subscription_gateway_env(&mut cmd);
    cmd.current_dir(std::env::temp_dir())
        .arg("login")
        .arg(family)
        .arg("--no-browser")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let open_shim = install_macos_login_open_shim(&mut cmd);
    let mut child = cmd.spawn()?;
    let opened = Arc::new(AtomicBool::new(false));
    if let Some(pipe) = child.stdout.take() {
        watch_login_output_mode(pipe, Arc::clone(&opened), true);
    }
    if let Some(pipe) = child.stderr.take() {
        watch_login_output_mode(pipe, Arc::clone(&opened), true);
    }
    let started = Instant::now();
    loop {
        drain_macos_login_open_shim(open_shim.as_ref(), &opened);
        if zcode_logged_in() && !zcode_needs_family_sync(family) {
            wait_for_login_exit(&mut child);
            return Ok(());
        }
        if started.elapsed() > LOGIN_WAIT {
            let _ = child.kill();
            let _ = child.wait();
            bail!("login timed out");
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                drain_macos_login_open_shim(open_shim.as_ref(), &opened);
                return Ok(());
            }
            Ok(None) => thread::sleep(Duration::from_millis(200)),
            Err(_) => return Ok(()),
        }
    }
}

fn spawn_cli_login(binary: &Path) -> Result<()> {
    let bin_lower = binary
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_qoder = bin_lower.contains("qoder");
    let is_cursor = bin_lower == "agent" || bin_lower.contains("cursor");
    let qoder_dir = if bin_lower.contains("qoderclicn") {
        Some(home_join(".qoder-cn"))
    } else if is_qoder {
        Some(home_join(".qoder"))
    } else {
        None
    };

    let mut cmd = crate::exec::command_for_path(binary);
    cmd.arg("login")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if is_qoder {
        // Tell Qoder CLI to skip its silent `.catch(() => {})` opener so we open
        // the printed authorization URL deterministically once via `open_url`.
        cmd.env("BROWSER", "www-browser");
    }
    if is_cursor {
        // Cursor `agent login` honors `NO_OPEN_BROWSER` so `open_url` can route
        // directly to the user's everyday browser session instead of `chrome-cdp`.
        cmd.env("NO_OPEN_BROWSER", "1");
    }
    let open_shim = install_macos_login_open_shim(&mut cmd);
    let mut child = cmd.spawn()?;
    let opened = Arc::new(AtomicBool::new(false));
    let intercept_url = is_qoder || is_cursor;
    if let Some(pipe) = child.stdout.take() {
        watch_login_output_mode(pipe, Arc::clone(&opened), intercept_url);
    }
    if let Some(pipe) = child.stderr.take() {
        watch_login_output_mode(pipe, Arc::clone(&opened), intercept_url);
    }
    let started = Instant::now();
    loop {
        drain_macos_login_open_shim(open_shim.as_ref(), &opened);
        if let Some(ref dir) = qoder_dir {
            if qoder_dir_logged_in(dir) {
                wait_for_login_exit(&mut child);
                return Ok(());
            }
        }
        if is_cursor && cursor_logged_in() {
            wait_for_login_exit(&mut child);
            return Ok(());
        }
        if started.elapsed() > LOGIN_WAIT {
            let _ = child.kill();
            let _ = child.wait();
            bail!("login timed out");
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                drain_macos_login_open_shim(open_shim.as_ref(), &opened);
                return Ok(());
            }
            Ok(None) => thread::sleep(Duration::from_millis(200)),
            Err(_) => return Ok(()),
        }
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
    match runtime_id {
        QODER_ID | CURSOR_ID | WORKBUDDY_ID | ZCODE_ID | ANTIGRAVITY_ID | DEEPSEEK_HARNESS_ID => {
            cli_logged_in(runtime_id)
        }
        CODEX_ID => {
            read_json(&home_join(".codex/auth.json"))
                .is_some_and(|value| codex_auth_is_chatgpt(&value))
                || stash_path()
                    .and_then(|path| read_json(&path))
                    .is_some_and(|value| codex_auth_is_chatgpt(&value))
        }
        CLAUDE_ID => {
            read_json(&home_join(".claude/.credentials.json"))
                .is_some_and(|value| claude_credentials_are_oauth(&value))
                || macos_keychain_has_claude_oauth()
        }
        _ => false,
    }
}

#[cfg(all(target_os = "macos", not(test)))]
fn keychain_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, (Instant, bool)>>
{
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, bool)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(all(target_os = "macos", not(test)))]
fn clear_macos_keychain_cache() {
    if let Ok(mut guard) = keychain_cache().lock() {
        guard.clear();
    }
}

#[cfg(not(all(target_os = "macos", not(test))))]
fn clear_macos_keychain_cache() {}

#[cfg(all(target_os = "macos", not(test)))]
fn macos_keychain_item_exists(service: &str, account: Option<&str>) -> bool {
    let key = format!("{service}\0{}", account.unwrap_or(""));
    let cache = keychain_cache();
    if let Ok(guard) = cache.lock() {
        if let Some((at, present)) = guard.get(&key) {
            if at.elapsed() < Duration::from_secs(5) {
                return *present;
            }
        }
    }
    let mut cmd = std::process::Command::new("security"); // raw-spawn-ok
    cmd.arg("find-generic-password").arg("-s").arg(service);
    if let Some(acc) = account {
        cmd.arg("-a").arg(acc);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let present = cmd.status().is_ok_and(|s| s.success());
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, (Instant::now(), present));
    }
    present
}

#[cfg(not(all(target_os = "macos", not(test))))]
fn macos_keychain_item_exists(_service: &str, _account: Option<&str>) -> bool {
    false
}

fn macos_keychain_has_claude_oauth() -> bool {
    !using_test_home() && macos_keychain_item_exists("Claude Code-credentials", None)
}

fn macos_keychain_has_cursor_auth() -> bool {
    !using_test_home()
        && (macos_keychain_item_exists("cursor-access-token", Some("cursor-user"))
            || macos_keychain_item_exists("cursor-api-key", Some("cursor-user")))
}

/// The CLI stores the Google login in the OS keyring under this service and account.
/// Presence is enough. The secret itself is never read.
const AGY_KEYRING_SERVICE: &str = "gemini";
const AGY_KEYRING_ACCOUNT: &str = "antigravity";

fn agy_test_login_marker() -> PathBuf {
    home_join(".gemini/antigravity-cli/auth-present")
}

fn agy_logged_in() -> bool {
    if using_test_home() {
        return agy_test_login_marker().is_file();
    }
    agy_keyring_present()
}

#[cfg(all(target_os = "macos", not(test)))]
fn agy_keyring_present() -> bool {
    macos_keychain_item_exists(AGY_KEYRING_SERVICE, Some(AGY_KEYRING_ACCOUNT))
}

#[cfg(any(not(target_os = "macos"), test))]
fn agy_keyring_present() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("secret-tool") // raw-spawn-ok
            .args([
                "lookup",
                "service",
                AGY_KEYRING_SERVICE,
                "username",
                AGY_KEYRING_ACCOUNT,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
    #[cfg(target_os = "windows")]
    {
        let Ok(output) = std::process::Command::new("cmdkey") // raw-spawn-ok
            .arg("/list")
            .output()
        else {
            return false;
        };
        let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
        text.contains(AGY_KEYRING_SERVICE) && text.contains(AGY_KEYRING_ACCOUNT)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    false
}

fn forget_agy_keyring() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("security") // raw-spawn-ok
            .args([
                "delete-generic-password",
                "-s",
                AGY_KEYRING_SERVICE,
                "-a",
                AGY_KEYRING_ACCOUNT,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("secret-tool") // raw-spawn-ok
            .args([
                "clear",
                "service",
                AGY_KEYRING_SERVICE,
                "username",
                AGY_KEYRING_ACCOUNT,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmdkey") // raw-spawn-ok
            .arg("/delete:gemini:antigravity")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
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
    _reachable: bool,
    installed: bool,
) -> bool {
    using_subscription || installed
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

pub(crate) fn codex_chatgpt_stash_path() -> Option<PathBuf> {
    stash_path()
}

fn stash_path() -> Option<PathBuf> {
    if using_test_home() {
        return Some(home_join(".agent-doctor/codex-chatgpt-auth.json"));
    }
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
    if let Err(error) = fs::write(&dest, raw) {
        eprintln!(
            "! could not stash the Codex login into {}: {error}",
            dest.display()
        );
    }
}

fn forget_saved_login(runtime_id: &str) {
    if !using_test_home() {
        if let Ok(store) = open_settings_store() {
            let _ = store.set_meta(&format!("native_models:{runtime_id}"), "{}");
        }
    }
    match runtime_id {
        CODEX_ID => {
            let _ = fs::remove_file(home_join(".codex/auth.json"));
            if let Some(path) = stash_path() {
                let _ = fs::remove_file(path);
            }
            if !using_test_home() {
                let _ = clear_codex_chatgpt_auth_for_gateway();
            }
        }
        CLAUDE_ID => {
            let _ = fs::remove_file(home_join(".claude/.credentials.json"));
            if !using_test_home() {
                if let Some(bin) = discover_binary("claude").binary_path {
                    let _ = crate::exec::command_for_path(&bin)
                        .args(["auth", "logout"])
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
                clear_macos_keychain_cache();
            }
        }
        QODER_ID => {
            for folder in [".qoder", ".qoder-cn"] {
                let dir = home_join(folder);
                let _ = fs::remove_file(dir.join("auth.json"));
                clear_qoder_auth_dir(&dir.join(".auth"));
            }
        }
        CURSOR_ID => {
            for path in cursor_auth_file_candidates() {
                let _ = fs::remove_file(path);
            }
            if !using_test_home() {
                if let Some(bin) = crate::adapters::cursor_cli_path() {
                    let _ = crate::exec::command_for_path(&bin)
                        .arg("logout")
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
                clear_macos_keychain_cache();
            }
        }
        WORKBUDDY_ID => {
            let dir = home_join(".codebuddy");
            let _ = fs::remove_file(dir.join("auth.json"));
            let _ = fs::remove_file(dir.join("credentials.json"));
            let _ = fs::remove_file(dir.join(WORKBUDDY_AUTH_MARKER));
            clear_workbuddy_local_storage(&dir.join("local_storage"));
            let _ = fs::create_dir_all(&dir);
            let _ = fs::write(dir.join(WORKBUDDY_SIGNOUT_MARKER), "signed-out");
        }
        ANTIGRAVITY_ID => {
            let _ = fs::remove_file(agy_test_login_marker());
            if !using_test_home() {
                forget_agy_keyring();
                clear_macos_keychain_cache();
            }
        }
        ZCODE_ID => {
            if !using_test_home() {
                if let Some(launch) = crate::adapters::locate_zcode_headless() {
                    let mut cmd = crate::adapters::zcode_headless_command(&launch);
                    let _ = cmd
                        .arg("logout")
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
            scrub_zcode_credentials_file(&home_join(".zcode/v2/credentials.json"));
        }
        _ => {}
    }
}

fn clear_workbuddy_local_storage(storage_dir: &Path) {
    let Ok(entries) = fs::read_dir(storage_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let _ = fs::remove_file(path);
        }
    }
}

pub(crate) fn scrub_zcode_credentials_file(path: &Path) {
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&raw) else {
        return;
    };
    let Some(map) = value.as_object_mut() else {
        return;
    };
    map.retain(|key, _| {
        key != "zcodejwttoken"
            && !key.starts_with("oauth:")
            && !key.starts_with("account-provider:")
    });
    if let Ok(serialized) = serde_json::to_string_pretty(&value) {
        let _ = fs::write(path, serialized);
    }
}

pub(crate) fn clear_qoder_auth_dir(auth_dir: &Path) {
    let Ok(entries) = fs::read_dir(auth_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "user" || name.starts_with("user.") {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Replace a project Codex home's account file with the signed-in ChatGPT auth
/// from `~/.codex/auth.json` (or the stashed copy when a personal gateway is active).
pub(crate) fn publish_codex_subscription_auth(codex_home: Option<&Path>) {
    let global_auth = home_join(".codex/auth.json");
    let global_has_chatgpt =
        read_json(&global_auth).is_some_and(|value| codex_auth_is_chatgpt(&value));
    let src = if global_has_chatgpt {
        global_auth.clone()
    } else if let Some(stash) =
        stash_path().filter(|p| read_json(p).is_some_and(|value| codex_auth_is_chatgpt(&value)))
    {
        stash
    } else {
        global_auth.clone()
    };
    if let Some(home) = codex_home {
        copy_chatgpt_auth_file(&src, home);
    } else if !global_has_chatgpt {
        restore_codex_auth();
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
    if let Err(error) = fs::copy(&src, &dest) {
        eprintln!(
            "! could not restore the Codex login into {}: {error}",
            dest.display()
        );
    }
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
    cmd.args(args)
        // Sign-in must land in ~/.codex. A project CODEX_HOME would keep the new
        // account away from the file the next chat copies in.
        .env_remove("CODEX_HOME")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let open_shim = install_macos_login_open_shim(&mut cmd);
    let mut child = cmd.spawn()?;
    let opened = Arc::new(AtomicBool::new(false));
    // Codex and Claude each open the authorize page themselves. Opening that
    // same address again shows the account that is already signed in, and that
    // page never comes back to this computer.
    // On Windows the login process has no window, so the browser Codex starts
    // stays invisible. The printed line is the full address, not a failure.
    // Open that address once from this app (the shell keeps `redirect_uri`),
    // and close the browser Codex already started.
    let open_printed_url = official_login_opens_printed_url(runtime_id);
    if let Some(pipe) = child.stdout.take() {
        watch_login_output_mode(pipe, Arc::clone(&opened), open_printed_url);
    }
    if let Some(pipe) = child.stderr.take() {
        watch_login_output_mode(pipe, Arc::clone(&opened), open_printed_url);
    }
    // Codex opens its browser before it prints the address. Close that window
    // as it appears, so only the page this app opens is left on screen.
    let _stop_browsers = open_printed_url.then(|| hide_codex_spawned_browsers(child.id()));
    let started = Instant::now();
    loop {
        drain_macos_login_open_shim(open_shim.as_ref(), &opened);
        if logged_in(runtime_id) {
            // The tool writes the account, then tells the browser where to go.
            // Killing it in that gap leaves the signed-in page with nowhere to jump.
            wait_for_login_exit(&mut child);
            return Ok(());
        }
        if started.elapsed() > LOGIN_WAIT {
            let _ = child.kill();
            let _ = child.wait();
            bail!("login timed out");
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                drain_macos_login_open_shim(open_shim.as_ref(), &opened);
                return Ok(());
            }
            Ok(None) => thread::sleep(Duration::from_millis(200)),
            Err(_) => return Ok(()),
        }
    }
}

/// Intercepts `open <https://...>` from login CLIs on macOS so we route the
/// authorization link to the user's everyday browser session instead of Agent
/// Doctor's isolated `chrome-cdp` process.
#[cfg(target_os = "macos")]
fn install_macos_login_open_shim(cmd: &mut std::process::Command) -> Option<tempfile::TempDir> {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().ok()?;
    let shim_path = dir.path().join("open");
    let url_file = dir.path().join("opened-url.txt");
    let script = format!(
        "#!/bin/sh\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    http://*|https://*)\n      printf '%s\\n' \"$arg\" > '{}'\n      exit 0\n      ;;\n  esac\ndone\nexec /usr/bin/open \"$@\"\n",
        url_file.display().to_string().replace('\'', "'\\''")
    );
    fs::write(&shim_path, script).ok()?;
    fs::set_permissions(&shim_path, fs::Permissions::from_mode(0o755)).ok()?;

    let existing_path = cmd
        .get_envs()
        .find_map(|(k, v)| {
            (k == "PATH")
                .then_some(v)
                .flatten()
                .map(|v| v.to_os_string())
        })
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    let joined = std::env::join_paths(
        std::iter::once(dir.path().to_path_buf()).chain(std::env::split_paths(&existing_path)),
    )
    .ok()?;
    cmd.env("PATH", joined);
    Some(dir)
}

#[cfg(not(target_os = "macos"))]
fn install_macos_login_open_shim(_cmd: &mut std::process::Command) -> Option<tempfile::TempDir> {
    None
}

fn drain_macos_login_open_shim(shim: Option<&tempfile::TempDir>, opened: &AtomicBool) {
    let Some(dir) = shim else {
        return;
    };
    let url_file = dir.path().join("opened-url.txt");
    let Ok(raw) = fs::read_to_string(&url_file) else {
        return;
    };
    let _ = fs::remove_file(&url_file);
    let url = raw.lines().next().unwrap_or("").trim();
    if (url.starts_with("http://") || url.starts_with("https://"))
        && !opened.swap(true, Ordering::SeqCst)
    {
        let _ = crate::session_launch::open_url(url);
    }
}

fn watch_login_output_mode<R>(pipe: R, opened: Arc<AtomicBool>, always_open_url: bool)
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
            maybe_open_login_url_mode(&pending, &opened, false, always_open_url);
        }
        maybe_open_login_url_mode(&pending, &opened, true, always_open_url);
    });
}

fn maybe_open_login_url_mode(text: &str, opened: &AtomicBool, at_end: bool, always_open_url: bool) {
    if !always_open_url && !login_url_should_open(text) {
        return;
    }
    let url = if at_end {
        login_url_at_end(text)
    } else {
        first_login_url(text)
    };
    let Some(url) = url else {
        return;
    };
    if !opened.swap(true, Ordering::SeqCst) {
        let _ = crate::session_launch::open_url(&url);
    }
}

/// Windows Codex login is started with a hidden console. The browser it
/// launches inherits that and never appears, while the "if your browser did
/// not open" line is only a hint. Open the printed page from this app there.
/// Everywhere else the tool still owns the page.
fn official_login_opens_printed_url(runtime_id: &str) -> bool {
    printed_login_url_is_ours(runtime_id, cfg!(windows))
}

fn printed_login_url_is_ours(runtime_id: &str, windows: bool) -> bool {
    windows && runtime_id == CODEX_ID
}

/// A browser process Codex itself started. The page this app opens is not a
/// child of that login process, so it stays. WebView is the app window.
#[cfg(any(windows, test))]
fn login_spawned_browser(exe: &str) -> bool {
    let name = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    matches!(
        name.to_ascii_lowercase().as_str(),
        "chrome.exe"
            | "msedge.exe"
            | "firefox.exe"
            | "brave.exe"
            | "opera.exe"
            | "vivaldi.exe"
            | "chromium.exe"
            | "iexplore.exe"
    )
}

#[cfg(any(windows, test))]
struct ProcessRow {
    pid: u32,
    parent: u32,
    exe: String,
}

#[cfg(any(windows, test))]
fn spawned_browser_pids(root: u32, processes: &[ProcessRow]) -> Vec<u32> {
    let mut family = vec![root];
    let mut index = 0;
    while index < family.len() {
        let parent = family[index];
        for process in processes {
            if process.parent == parent && !family.contains(&process.pid) {
                family.push(process.pid);
            }
        }
        index += 1;
    }
    processes
        .iter()
        .filter(|process| {
            process.pid != root
                && family.contains(&process.pid)
                && login_spawned_browser(&process.exe)
        })
        .map(|process| process.pid)
        .collect()
}

struct StopBrowsers(Arc<AtomicBool>);

impl Drop for StopBrowsers {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Close browsers the Codex login process starts, until this value is dropped.
/// On Windows that browser inherits the hidden console and never appears.
/// The page this app opens is not a child of the login process, so it stays.
pub(crate) fn hide_codex_spawned_browsers(root: u32) -> HiddenLoginBrowsers {
    HiddenLoginBrowsers {
        _stop: cfg!(windows).then(|| watch_spawned_browsers(root)),
    }
}

pub(crate) struct HiddenLoginBrowsers {
    _stop: Option<StopBrowsers>,
}

fn watch_spawned_browsers(root: u32) -> StopBrowsers {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            close_browsers_spawned_by(root);
            thread::sleep(Duration::from_millis(40));
        }
        close_browsers_spawned_by(root);
    });
    StopBrowsers(stop)
}

fn close_browsers_spawned_by(root: u32) {
    #[cfg(windows)]
    {
        for pid in spawned_browser_pids(root, &windows_process_snapshot()) {
            terminate_pid(pid);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = root;
    }
}

#[cfg(windows)]
fn windows_process_snapshot() -> Vec<ProcessRow> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return Vec::new();
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..PROCESSENTRY32W::default()
    };
    let mut rows = Vec::new();
    // The snapshot handle is owned here and closed before return.
    let started = unsafe { Process32FirstW(snapshot, &mut entry) };
    if started.is_ok() {
        loop {
            let len = entry
                .szExeFile
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(entry.szExeFile.len());
            rows.push(ProcessRow {
                pid: entry.th32ProcessID,
                parent: entry.th32ParentProcessID,
                exe: String::from_utf16_lossy(&entry.szExeFile[..len]),
            });
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }
    let _ = unsafe { CloseHandle(snapshot) };
    rows
}

#[cfg(windows)]
fn terminate_pid(pid: u32) {
    use windows::Win32::Foundation::{CloseHandle, BOOL};
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    let Ok(handle) = (unsafe { OpenProcess(PROCESS_TERMINATE, BOOL(0), pid) }) else {
        return;
    };
    let _ = unsafe { TerminateProcess(handle, 1) };
    let _ = unsafe { CloseHandle(handle) };
}

/// Both tools open the page, then print a hint in case that open failed.
/// The hint is not a failure. Open the address only after the tool says so.
fn login_url_should_open(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("failed to open browser") || lower.contains("couldn't open your browser")
}

fn wait_for_login_exit(child: &mut std::process::Child) {
    let grace = Instant::now();
    while grace.elapsed() < Duration::from_secs(5) {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => thread::sleep(Duration::from_millis(100)),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// A login link is only ready once a space or quote follows it. The address is
/// long, and the first read often stops in the middle.
fn first_login_url(text: &str) -> Option<String> {
    let start = text.find("https://")?;
    let rest = &text[start..];
    let end = rest.find(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\'')?;
    trimmed_login_url(&rest[..end])
}

fn login_url_at_end(text: &str) -> Option<String> {
    let start = text.find("https://")?;
    let rest = &text[start..];
    let end = rest
        .find(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\'')
        .unwrap_or(rest.len());
    trimmed_login_url(&rest[..end])
}

fn trimmed_login_url(raw: &str) -> Option<String> {
    let url = raw.trim_end_matches(['.', ',', ')']);
    if url.len() > "https://".len() {
        Some(url.to_string())
    } else {
        None
    }
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
    fn cursor_cli_config_counts_only_its_own_token() {
        assert!(cursor_cli_config_is_signed_in(
            &json!({"accessToken": "cli-token"})
        ));
        assert!(!cursor_cli_config_is_signed_in(&json!({"version": 1})));
    }

    #[test]
    fn antigravity_login_marker_is_only_for_tests() {
        let dir = tempfile::tempdir().unwrap();
        crate::adapters::util::with_test_home(dir.path(), || {
            assert!(!logged_in(ANTIGRAVITY_ID));
            let marker = agy_test_login_marker();
            fs::create_dir_all(marker.parent().unwrap()).unwrap();
            fs::write(&marker, "1").unwrap();
            assert!(logged_in(ANTIGRAVITY_ID));
            assert!(turn_uses_native_account(
                ANTIGRAVITY_ID,
                Some("gemini-3-flash"),
                Some("native:antigravity"),
            ));
            assert!(!turn_uses_native_account(
                ANTIGRAVITY_ID,
                Some("gemini-3-flash"),
                Some("saved-key"),
            ));
            forget_saved_login(ANTIGRAVITY_ID);
            assert!(!logged_in(ANTIGRAVITY_ID));
        });
    }

    #[test]
    fn gemini_and_vertex_addresses_can_back_antigravity() {
        assert!(agy_url_is_gemini_api(
            "https://generativelanguage.googleapis.com/v1beta/openai/"
        ));
        assert!(agy_url_is_vertex("https://aiplatform.googleapis.com/v1"));
        assert!(!agy_url_is_gemini_api("https://api.deepseek.com/v1"));
        assert!(!agy_url_is_vertex("https://api.deepseek.com/v1"));
    }

    #[test]
    fn cursor_subscription_requires_a_saved_login() {
        // Mode alone is not enough — same soft degrade as Qoder.
        let _ = write_mode(CURSOR_ID, MODE_SUBSCRIPTION);
        if !cli_logged_in(CURSOR_ID) {
            assert!(!is_native_subscription(CURSOR_ID));
        }
        let _ = write_mode(CURSOR_ID, "service");
    }

    #[test]
    fn qoder_auth_dir_ignores_machine_id_and_detects_saved_user() {
        let dir = tempfile::tempdir().unwrap();
        let auth = dir.path().join(".auth");
        fs::create_dir_all(&auth).unwrap();
        fs::write(auth.join("machine_id"), "36-byte-id").unwrap();
        fs::write(auth.join(".credential-transaction"), "").unwrap();
        assert!(!qoder_auth_dir_has_user(&auth));

        fs::write(auth.join("user"), "encrypted-qoder-credential").unwrap();
        assert!(qoder_auth_dir_has_user(&auth));
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
    fn visible_whenever_installed_or_already_using_subscription() {
        assert!(account_row_visible(false, false, true));
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
            first_login_url("open https://auth.openai.com/codex/device).\n"),
            Some("https://auth.openai.com/codex/device".to_string())
        );
        assert_eq!(first_login_url("no link"), None);
    }

    #[test]
    fn windows_codex_login_opens_the_printed_page_itself() {
        assert!(printed_login_url_is_ours(CODEX_ID, true));
        assert!(!printed_login_url_is_ours(CLAUDE_ID, true));
        assert!(!printed_login_url_is_ours(CODEX_ID, false));
        assert!(!printed_login_url_is_ours("qoder", true));
    }

    #[test]
    fn codex_login_banner_points_at_the_authorize_page() {
        let banner = "Starting local login server on http://localhost:1455.\nIf your browser did not open, navigate to this URL to authenticate:\n\nhttps://auth.openai.com/oauth/authorize?x=1\n";
        assert_eq!(
            first_login_url(banner).as_deref(),
            Some("https://auth.openai.com/oauth/authorize?x=1")
        );
    }

    #[test]
    fn authorize_url_keeps_redirect_uri_past_every_ampersand() {
        let url = "https://auth.openai.com/oauth/authorize?response_type=code&client_id=app&redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fauth%2Fcallback&state=abc";
        let banner = format!(
            "Starting local login server on http://localhost:1455.\nIf your browser did not open, navigate to this URL to authenticate:\n\n{url}\n"
        );
        assert_eq!(first_login_url(&banner).as_deref(), Some(url));
    }

    #[test]
    fn only_the_browser_codex_started_is_closed() {
        let processes = vec![
            ProcessRow {
                pid: 1,
                parent: 0,
                exe: "node.exe".to_string(),
            },
            ProcessRow {
                pid: 2,
                parent: 1,
                exe: "codex.exe".to_string(),
            },
            ProcessRow {
                pid: 3,
                parent: 2,
                exe: r"C:\Program Files\Google\Chrome\Application\chrome.exe".to_string(),
            },
            ProcessRow {
                pid: 4,
                parent: 3,
                exe: "chrome.exe".to_string(),
            },
            ProcessRow {
                pid: 5,
                parent: 2,
                exe: "msedgewebview2.exe".to_string(),
            },
            ProcessRow {
                pid: 9,
                parent: 8,
                exe: "chrome.exe".to_string(),
            },
        ];
        assert_eq!(spawned_browser_pids(1, &processes), vec![3, 4]);
    }

    #[test]
    fn login_page_stays_with_the_tool_unless_its_open_failed() {
        let codex = "Starting local login server on http://localhost:1455.\nIf your browser did not open, navigate to this URL to authenticate:\n\nhttps://auth.openai.com/oauth/authorize?x=1\n";
        let claude = "Opening browser to sign in…\nhttps://claude.com/cai/oauth/authorize?x=1\nPaste code here if prompted > ";
        assert!(!login_url_should_open(codex));
        assert!(!login_url_should_open(claude));
        assert!(login_url_should_open(
            "failed to open browser for login URL: no display\nhttps://auth.openai.com/oauth/authorize?x=1\n"
        ));
        assert!(login_url_should_open(
            "Couldn't open your browser. Visit https://claude.com/cai/oauth/authorize?x=1\n"
        ));
    }

    #[test]
    fn login_output_url_waits_until_the_address_is_finished() {
        let partial = "https://auth.openai.com/oauth/authorize?redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fauth";
        assert_eq!(first_login_url(partial), None);
        assert_eq!(
            first_login_url(&format!("{partial}&state=abc\n")),
            Some(format!("{partial}&state=abc"))
        );
    }

    #[test]
    fn workbuddy_log_distinguishes_real_login_from_anonymous_api_key_session() {
        let anon = "[Info] [AuthenticationManager] [AuthDoInitProbe] stage=fastPathBeforeEmit uid=anonymous_vv_thbz8 hasAccessToken=true\n";
        assert!(!workbuddy_log_text_is_signed_in(anon));

        let empty = "[Info] [AuthenticationManager] [AuthDoInitProbe] stage=fastPathBeforeEmit uid=<none> hasAccessToken=false\n";
        assert!(!workbuddy_log_text_is_signed_in(empty));

        let real =
            "[Info] [AuthenticationSessionListener] [auth_success: wx_user_12345] 认证成功\n";
        assert!(workbuddy_log_text_is_signed_in(real));

        let signed_out = format!(
            "{real}[Info] [AuthenticationManager] userId changed: wx_user_12345 -> <empty>\n"
        );
        assert!(!workbuddy_log_text_is_signed_in(&signed_out));
    }

    #[test]
    fn zcode_and_dsh_credentials_detect_only_real_sign_ins() {
        assert!(!zcode_credentials_are_signed_in(
            &json!({"web-remote-control:external-relay:pass_hash": "hash"})
        ));
        assert!(zcode_credentials_are_signed_in(
            &json!({"oauth:zai:access_token": "enc-tok"})
        ));
        assert!(!zcode_cli_credentials_are_signed_in(
            &json!({"oauth:zai:access_token": "enc-tok", "zcodejwttoken": "jwt"})
        ));
        assert!(!zcode_cli_credentials_are_signed_in(
            &json!({"account-provider:account:bigmodel-start-plan:identity": "u_desktop"})
        ));
        assert!(zcode_cli_credentials_are_signed_in(
            &json!({"account-provider:account:zai-individual-coding-plan:identity": "u_123"})
        ));

        assert!(!dsh_credentials_are_signed_in("{}\n"));
        assert!(dsh_credentials_are_signed_in(
            "deepseek:\n  access_token: tok-123\n"
        ));
    }

    #[test]
    fn explicit_native_provider_pin_follows_actual_login_state() {
        use crate::adapters::util::with_test_home;

        let dir = tempfile::tempdir().unwrap();
        with_test_home(dir.path(), || {
            // Not logged in: `native:qoder` is refused and does not pretend to be subscribed.
            assert!(!is_runtime_logged_in("qoder"));
            assert!(!turn_uses_native_account(
                "qoder",
                Some("auto"),
                Some("native:qoder")
            ));

            // Sign Qoder in: `native:qoder` activates the account even when mode is "service",
            // while an explicit saved provider id (`pp_1`) keeps using that saved provider.
            let auth = dir.path().join(".qoder/.auth");
            fs::create_dir_all(&auth).unwrap();
            fs::write(auth.join("user"), "cred").unwrap();
            assert!(is_runtime_logged_in("qoder"));
            assert!(turn_uses_native_account(
                "qoder",
                Some("auto"),
                Some("native:qoder")
            ));
            assert!(!turn_uses_native_account(
                "qoder",
                Some("deepseek-flash"),
                Some("pp_1")
            ));

            // Stashed Codex ChatGPT auth still counts as logged in while a gateway is active.
            let stash = dir.path().join(".agent-doctor/codex-chatgpt-auth.json");
            fs::create_dir_all(stash.parent().unwrap()).unwrap();
            fs::write(
                &stash,
                r#"{"auth_mode":"chatgpt","tokens":{"access_token":"tok"}}"#,
            )
            .unwrap();
            assert!(is_runtime_logged_in("codex"));
            assert!(turn_uses_native_account(
                "codex",
                Some("gpt-5.6-sol"),
                Some("native:codex")
            ));
        });
    }

    #[test]
    fn detects_qoder_and_cursor_desktop_ide_sign_ins() {
        use crate::adapters::util::with_test_home;

        let dir = tempfile::tempdir().unwrap();
        with_test_home(dir.path(), || {
            assert_eq!(qoder_desktop_login_kind(), None);

            let qoder_cn_status = dir
                .path()
                .join("Library/Application Support/QoderCN/SharedClientCache/cache/status.json");
            fs::create_dir_all(qoder_cn_status.parent().unwrap()).unwrap();
            fs::write(&qoder_cn_status, r#"{"logged_in":true,"username":"u1"}"#).unwrap();
            assert_eq!(qoder_desktop_login_kind(), Some(QoderDesktopKind::Cn));

            let cursor_db = dir
                .path()
                .join("Library/Application Support/Cursor/User/globalStorage/state.vscdb");
            fs::create_dir_all(cursor_db.parent().unwrap()).unwrap();
            let conn = rusqlite::Connection::open(&cursor_db).unwrap();
            conn.execute(
                "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)",
                [],
            )
            .unwrap();
            assert!(!cursor_state_db_has_token(&cursor_db));
            conn.execute(
                "INSERT INTO ItemTable (key, value) VALUES ('cursorAuth/accessToken', 'jwt-tok')",
                [],
            )
            .unwrap();
            drop(conn);
            assert!(cursor_state_db_has_token(&cursor_db));
            assert_eq!(desktop_login_state("cursor"), (true, true));
        });
    }

    #[test]
    fn workbuddy_sign_in_survives_anonymous_api_key_log_and_clears_on_sign_out() {
        use crate::adapters::util::with_test_home;

        let dir = tempfile::tempdir().unwrap();
        with_test_home(dir.path(), || {
            let day = dir.path().join(".codebuddy/logs/2026-11-18");
            fs::create_dir_all(&day).unwrap();
            let real_log = day.join("01_login.log");
            fs::write(
                &real_log,
                "[Info] [AuthenticationSessionListener] [auth_success: wx_user_88] 认证成功\n",
            )
            .unwrap();
            thread::sleep(Duration::from_millis(20));
            let anon_log = day.join("02_apikey.log");
            fs::write(
                &anon_log,
                "[Info] [AuthenticationSessionListener] [auth_success: anonymous_abc] 认证成功\n",
            )
            .unwrap();
            assert!(is_runtime_logged_in("workbuddy"));

            forget_saved_login("workbuddy");
            assert!(!is_runtime_logged_in("workbuddy"));

            record_workbuddy_login_marker(&dir.path().join(".codebuddy"));
            assert!(is_runtime_logged_in("workbuddy"));
        });
    }

    #[test]
    fn zcode_sign_in_and_sign_out_roundtrip_preserves_non_auth_keys() {
        use crate::adapters::util::with_test_home;

        let dir = tempfile::tempdir().unwrap();
        with_test_home(dir.path(), || {
            let zcode_v2 = dir.path().join(".zcode/v2");
            fs::create_dir_all(&zcode_v2).unwrap();
            let creds_path = zcode_v2.join("credentials.json");

            // Desktop-only sign-in shows desktop_logged_in + needs_sync, not CLI-ready.
            fs::write(
                &creds_path,
                r#"{"web-remote-control:external-relay:pass_hash":"keep","oauth:bigmodel:access_token":"tok","zcodejwttoken":"jwt"}"#,
            )
            .unwrap();
            assert!(!is_runtime_logged_in("zcode"));
            assert_eq!(desktop_login_state("zcode"), (true, true));

            // CLI Coding Plan identity marks ZCode CLI-ready and clears needs_sync.
            fs::write(
                &creds_path,
                r#"{"web-remote-control:external-relay:pass_hash":"keep","oauth:bigmodel:access_token":"tok","zcodejwttoken":"jwt","account-provider:account:bigmodel-individual-coding-plan:identity":"cli-id"}"#,
            )
            .unwrap();
            assert!(is_runtime_logged_in("zcode"));
            assert_eq!(desktop_login_state("zcode"), (true, false));
            assert!(turn_uses_native_account(
                "zcode",
                Some("GLM-5.3"),
                Some("native:zcode")
            ));

            forget_saved_login("zcode");
            assert!(!is_runtime_logged_in("zcode"));
            assert_eq!(desktop_login_state("zcode"), (false, false));
            let after: Value =
                serde_json::from_str(&fs::read_to_string(&creds_path).unwrap()).unwrap();
            assert_eq!(after["web-remote-control:external-relay:pass_hash"], "keep");
            assert!(after.get("zcodejwttoken").is_none());
            assert!(after.get("oauth:bigmodel:access_token").is_none());
            assert!(after
                .get("account-provider:account:bigmodel-individual-coding-plan:identity")
                .is_none());
        });
    }
}
