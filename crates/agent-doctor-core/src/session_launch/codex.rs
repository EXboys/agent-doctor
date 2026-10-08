use std::collections::HashMap;
use std::path::Path;
#[cfg(target_os = "windows")]
use std::process::Stdio;

use anyhow::Result;

#[cfg(windows)]
use crate::profile::read_company_profile;
use crate::profile::{
    GATEWAY_URL_ENV, PROVIDER_KIND_COMPANY, PROVIDER_KIND_ENV, PROVIDER_KIND_PERSONAL,
};
#[cfg(windows)]
use crate::setup::anthropic_gateway_url_from_evotown_base;
use crate::setup::merge::{
    apply_codex_slot, codex_slot_display_name, codex_slot_env_key, CODEX_PERSONAL_SLOT,
    CODEX_TEAM_SLOT,
};
use crate::setup::{
    clear_codex_chatgpt_auth_for_gateway, clear_codex_placeholder_auth, COMPANY_API_KEY_ENV,
    EVOTOWN_API_KEY_ENV, MODEL_ENV,
};

use super::*;

pub(crate) fn open_codex(cwd: &Path, prompt: Option<&str>) -> Result<OpenSessionReport> {
    let _ = clear_codex_placeholder_auth();
    // Mirror Claude: rewrite Codex config before launch. Env OPENAI_BASE_URL alone is
    // not enough — Codex 0.14x still routes via model_provider / openai_base_url in config.toml,
    // and without that it silently hits api.openai.com (401 with company keys).
    // Prefer isolated workspace CODEX_HOME when active (avoid project-local deny on ~/.codex).
    let launch = resolve_codex_launch_env();
    // ChatGPT login tokens in auth.json make Codex ignore gateway keys and hit api.openai.com.
    if launch.is_some() {
        let _ = clear_codex_chatgpt_auth_for_gateway();
    }
    let refreshed = launch.as_ref().and_then(|(url, key, model, slot)| {
        apply_codex_slot(url, key, model.as_deref(), Some(slot))
            .ok()
            .filter(|r| r.applied)
            .map(|r| (url.clone(), r.config_path))
    });

    // Also pass -c overrides so this process cannot fall back to built-in openai
    // even if ~/.codex was stale, CODEX_HOME pointed elsewhere, or the user never
    // re-ran mode switch. The launched command line itself proves the gateway.
    let argv = codex_launch_argv(launch.as_ref());
    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let mut report = open_in_terminal("codex", &argv_refs, cwd, prompt)?;
    if let Some((url, config_path)) = refreshed {
        let where_written = config_path.unwrap_or_else(|| "~/.codex/config.toml".into());
        report.detail = format!(
            "Opened Codex after writing model_provider + openai_base_url={url} to {where_written}. {}",
            report.detail
        );
    }
    Ok(report)
}

/// Build a self-contained `codex -c …` invocation that defines the whole slot
/// provider table inline, so launch does not depend on `~/.codex/config.toml`
/// being present or current.
///
/// Do NOT route through Codex's built-in `openai` provider: it carries
/// `supports_websockets` and `requires_openai_auth`, which make third-party
/// gateways reject the handshake with 401 on `wss://<host>/v1/responses`.
/// The inline table mirrors exactly what `write_codex_provider_config` writes.
pub(crate) fn codex_launch_argv(
    launch: Option<&(String, String, Option<String>, String)>,
) -> Vec<String> {
    let Some((url, _, model, slot)) = launch else {
        return vec!["codex".to_string()];
    };
    let mut argv = vec!["codex".to_string()];
    let mut set = |key: &str, value: String| {
        argv.push("-c".to_string());
        argv.push(format!("{key}={value}"));
    };
    let prefix = format!("model_providers.{slot}");
    set(
        &format!("{prefix}.name"),
        toml_string(codex_slot_display_name(slot)),
    );
    set(&format!("{prefix}.base_url"), toml_string(url));
    set(
        &format!("{prefix}.env_key"),
        toml_string(codex_slot_env_key(slot)),
    );
    set(&format!("{prefix}.wire_api"), toml_string("responses"));
    set(
        &format!("{prefix}.requires_openai_auth"),
        "false".to_string(),
    );
    set(
        &format!("{prefix}.supports_websockets"),
        "false".to_string(),
    );
    set("model_provider", toml_string(slot));
    // Only pin the model when the active profile names one; otherwise leave the
    // user's own `model` in config.toml alone.
    if let Some(model_id) = model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        set("model", toml_string(model_id));
    }
    argv
}

/// Quote a value as a TOML basic string so `codex -c key=value` parses it as a string.
pub(crate) fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Resolve OpenAI-compatible gateway + key (+ optional model/slot) from active overlays.
pub(crate) fn resolve_codex_launch_env() -> Option<(String, String, Option<String>, String)> {
    codex_launch_from_env(&collect_launch_env_map())
}

pub(crate) fn collect_launch_env_map() -> HashMap<String, String> {
    // Same overlay rules as Ask — personal edition must not inherit Evotown leftovers.
    crate::prompt_session::env::collect_overlay_env()
}

pub(crate) fn codex_launch_from_env(
    env: &HashMap<String, String>,
) -> Option<(String, String, Option<String>, String)> {
    let gateway_url = env
        .get(GATEWAY_URL_ENV)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)?;

    let api_key = [COMPANY_API_KEY_ENV, EVOTOWN_API_KEY_ENV, "OPENAI_API_KEY"]
        .into_iter()
        .find_map(|key| {
            env.get(key)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })?;

    let model = env
        .get(MODEL_ENV)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let slot = match env.get(PROVIDER_KIND_ENV).map(|v| v.trim()) {
        Some(PROVIDER_KIND_PERSONAL) => CODEX_PERSONAL_SLOT.to_string(),
        Some(PROVIDER_KIND_COMPANY) => CODEX_TEAM_SLOT.to_string(),
        _ => {
            // Legacy profiles omit PROVIDER_KIND — infer from gateway host.
            if gateway_url
                .to_ascii_lowercase()
                .contains("api.deepseek.com")
            {
                CODEX_PERSONAL_SLOT.to_string()
            } else {
                CODEX_TEAM_SLOT.to_string()
            }
        }
    };

    let gateway_url = crate::setup::merge::codex_responses_gateway_url(&gateway_url);
    Some((gateway_url, api_key, model, slot))
}

/// Resolve Anthropic base URL + key from the active overlay / Evotown env.
pub(crate) fn resolve_claude_launch_env() -> Option<(String, String)> {
    anthropic_launch_from_env(&collect_launch_env_map())
}

pub(crate) fn anthropic_launch_from_env(env: &HashMap<String, String>) -> Option<(String, String)> {
    // Reuse Ask overlay resolution so opening Claude Code cannot rewrite
    // ~/.claude/settings.json back to a leftover Evotown/skilllite gateway.
    crate::prompt_session::env::resolve_claude_overlay(env)
}
