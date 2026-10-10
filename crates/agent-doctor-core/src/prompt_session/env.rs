use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::profile::{
    agent_profile_path, read_env_map, COMPANY_API_KEY_ENV, GATEWAY_URL_ENV, PROVIDER_KIND_ENV,
    PROVIDER_KIND_PERSONAL,
};
use crate::setup::{
    anthropic_gateway_for_provider_url, anthropic_gateway_url_from_evotown_base, apply_codex_slot,
    clear_codex_chatgpt_auth_for_gateway, clear_codex_placeholder_auth, evotown_agent_env_path,
    normalize_protocol, EVOTOWN_API_KEY_ENV, EVOTOWN_URL_ENV, MODEL_ENV, PROTOCOL_ANTHROPIC,
    PROVIDER_PROTOCOL_ENV,
};
use crate::workspace::active_env_path;

/// Overlay env for an Ask turn, optionally bound to a registered workspace (not global `active` only).
pub(crate) fn collect_overlay_env_for_options(
    options: &super::PromptSessionOptions,
) -> HashMap<String, String> {
    collect_overlay_env_for_workspace(options.workspace_name.as_deref())
}

pub(crate) fn collect_overlay_env_for_workspace(
    workspace_name: Option<&str>,
) -> HashMap<String, String> {
    let mut env = collect_overlay_env();
    if let Some(name) = workspace_name.map(str::trim).filter(|n| !n.is_empty()) {
        if let Ok(session) = crate::workspace::session_env_for_workspace(name) {
            env.extend(session);
        }
    }
    env
}

pub(crate) fn collect_overlay_env() -> HashMap<String, String> {
    let mut env = HashMap::new();
    let personal_edition =
        crate::edition::product_edition() == crate::edition::ProductEdition::Personal;
    let mut merge = |path: Option<PathBuf>| {
        let Some(path) = path.filter(|p| p.exists()) else {
            return;
        };
        if let Ok(map) = read_env_map(&path) {
            env.extend(map);
        }
    };
    merge(active_env_path().ok());
    merge(agent_profile_path());
    // Personal builds must not inherit a leftover team/Evotown agent.env — that
    // skilllite gateway was answering Ask while the saved personal provider sat unused.
    if !personal_edition {
        merge(evotown_agent_env_path());
    }
    for key in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "DEEPSEEK_API_KEY",
        "DEEPSEEK_BASE_URL",
        "CODEX_HOME",
        COMPANY_API_KEY_ENV,
        EVOTOWN_API_KEY_ENV,
        GATEWAY_URL_ENV,
        EVOTOWN_URL_ENV,
        MODEL_ENV,
        PROVIDER_PROTOCOL_ENV,
        "AGENT_DOCTOR_CLAUDE_BIN",
        "AGENT_DOCTOR_CODEX_BIN",
        "AGENT_DOCTOR_HERMES_BIN",
        "AGENT_DOCTOR_OPENCLAW_BIN",
        "AGENT_DOCTOR_DSH_BIN",
    ] {
        if let Ok(v) = std::env::var(key as &str) {
            if !v.trim().is_empty() {
                env.insert((*key).to_string(), crate::profile::unquote_env_value(&v));
            }
        }
    }
    // Apply saved personal/team settings last so they win over shell leftovers.
    merge_settings_store_overlay(&mut env);
    env
}

fn strip_team_gateway_leftovers(env: &mut HashMap<String, String>) {
    env.remove(EVOTOWN_URL_ENV);
    env.remove("AGENT_DOCTOR_EVOTOWN_URL");
    if let Some(url) = env
        .get("ANTHROPIC_BASE_URL")
        .map(|v| v.to_ascii_lowercase())
    {
        if url.contains("skilllite.ai")
            || url.contains("evotown")
            || url.contains("/api/gateway/anthropic")
        {
            env.remove("ANTHROPIC_BASE_URL");
        }
    }
}

fn merge_settings_store_overlay(env: &mut HashMap<String, String>) {
    let Ok(store) = crate::store::open_settings_store() else {
        return;
    };
    let personal_edition =
        crate::edition::product_edition() == crate::edition::ProductEdition::Personal;
    // Personal builds must not inherit a team gateway. That URL was answering
    // Ask while the saved DeepSeek provider sat unused.
    if !personal_edition {
        if let Ok(team) = store.get_team_settings() {
            if let Some(url) = team
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                env.insert(
                    EVOTOWN_URL_ENV.to_string(),
                    url.trim_end_matches('/').to_string(),
                );
                let gateway = crate::setup::gateway_url_from_evotown_base(url);
                env.entry(GATEWAY_URL_ENV.to_string())
                    .or_insert_with(|| gateway.clone());
                env.entry("OPENAI_BASE_URL".into()).or_insert(gateway);
                let anthropic = anthropic_gateway_url_from_evotown_base(url);
                env.entry("ANTHROPIC_BASE_URL".into()).or_insert(anthropic);
            }
        }
        if let Ok(Some(key)) = store.get_overlay_api_key() {
            if !key.trim().is_empty() {
                let key = key.trim().to_string();
                env.insert(COMPANY_API_KEY_ENV.to_string(), key.clone());
                env.insert(EVOTOWN_API_KEY_ENV.to_string(), key.clone());
                env.entry("OPENAI_API_KEY".into())
                    .or_insert_with(|| key.clone());
                env.entry("ANTHROPIC_API_KEY".into()).or_insert(key);
            }
        } else if let Ok(Some(key)) = store.get_team_api_key() {
            if !key.trim().is_empty() {
                let key = key.trim().to_string();
                env.insert(COMPANY_API_KEY_ENV.to_string(), key.clone());
                env.insert(EVOTOWN_API_KEY_ENV.to_string(), key.clone());
                env.entry("OPENAI_API_KEY".into())
                    .or_insert_with(|| key.clone());
                env.entry("ANTHROPIC_API_KEY".into()).or_insert(key);
            }
        }
    }
    let stored_personal = store
        .get_active_mode()
        .ok()
        .flatten()
        .is_some_and(|mode| mode == crate::setup::MODE_PERSONAL);
    if personal_edition || stored_personal {
        if let Ok(providers) = store.list_personal_providers() {
            // DeepSeek Harness keeps using a saved DeepSeek provider while another one is active.
            let deepseek = providers
                .iter()
                .filter(|p| p.url.to_ascii_lowercase().contains("api.deepseek.com"))
                .max_by_key(|p| p.active);
            if let Some(saved) = deepseek {
                if let Ok(Some(key)) = store.get_personal_api_key(&saved.id) {
                    if !key.trim().is_empty() {
                        env.insert("DEEPSEEK_API_KEY".into(), key.trim().to_string());
                        env.insert("DEEPSEEK_BASE_URL".into(), saved.url.clone());
                    }
                }
            }
            if let Some(active) = providers.into_iter().find(|p| p.active) {
                strip_team_gateway_leftovers(env);
                env.insert(
                    PROVIDER_KIND_ENV.to_string(),
                    PROVIDER_KIND_PERSONAL.to_string(),
                );
                if let Ok(Some(key)) = store.get_personal_api_key(&active.id) {
                    if !key.trim().is_empty() {
                        let key = key.trim().to_string();
                        env.insert("OPENAI_API_KEY".into(), key.clone());
                        env.insert(COMPANY_API_KEY_ENV.to_string(), key.clone());
                        env.insert("ANTHROPIC_API_KEY".into(), key);
                    }
                }
                env.insert(GATEWAY_URL_ENV.to_string(), active.url.clone());
                env.insert("OPENAI_BASE_URL".into(), active.url.clone());
                let protocol = normalize_protocol(&active.protocol);
                env.insert(PROVIDER_PROTOCOL_ENV.to_string(), protocol.clone());
                if protocol == PROTOCOL_ANTHROPIC {
                    env.insert("ANTHROPIC_BASE_URL".into(), active.url.clone());
                } else {
                    // OpenAI-compatible personal providers must not keep a leftover
                    // Anthropic/Evotown base URL — Claude Ask would call skilllite.
                    env.remove("ANTHROPIC_BASE_URL");
                }
                if !active.model.trim().is_empty() {
                    let model = crate::setup::coerce_gateway_model(&active.url, &active.model)
                        .unwrap_or_else(|| active.model.clone());
                    if model != active.model {
                        let _ = crate::setup::persist_personal_provider_model(&active.id, &model);
                    }
                    env.insert(MODEL_ENV.to_string(), model);
                }
            }
        }
    }
}

pub(crate) fn apply_overlay_env(cmd: &mut Command, overlay: &HashMap<String, String>) {
    for (key, value) in overlay {
        cmd.env(key, value);
    }
}

pub(crate) fn apply_claude_env(cmd: &mut Command, overlay: &HashMap<String, String>) {
    // Claude Code only offers the checklist tools on some of its own models.
    // DeepSeek and other providers need this or the plan card never appears.
    if !overlay.contains_key("CLAUDE_CODE_ENABLE_TODO_TOOLS") {
        cmd.env("CLAUDE_CODE_ENABLE_TODO_TOOLS", "1");
    }
    if let Some((url, key)) = resolve_claude_overlay(overlay) {
        // A previous provider can leave these in the app's own environment.
        for stale in CLAUDE_PROVIDER_ENV {
            cmd.env_remove(stale);
        }
        for (name, value) in claude_provider_env(overlay).unwrap_or_default() {
            cmd.env(name, value);
        }
        cmd.env(COMPANY_API_KEY_ENV, &key);
        cmd.env(EVOTOWN_API_KEY_ENV, &key);
        let model = claude_model(overlay);
        if let Some(model_id) = model {
            cmd.env(MODEL_ENV, model_id);
        }
        // Keep ~/.claude/settings.json aligned — Claude CLI still reads it when env is sparse.
        let _ = crate::setup::apply_claude_code_with_model(&url, &key, model);
    }
}

/// Every variable that decides where Claude Code sends a message and with which model.
const CLAUDE_PROVIDER_ENV: &[&str] = &[
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
];

fn claude_model(overlay: &HashMap<String, String>) -> Option<&str> {
    overlay
        .get(MODEL_ENV)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

/// The active provider for Claude Code, every slot filled. Claude prefers
/// ANTHROPIC_AUTH_TOKEN over ANTHROPIC_API_KEY, so both carry the same key.
pub(crate) fn claude_provider_env(
    overlay: &HashMap<String, String>,
) -> Option<Vec<(&'static str, String)>> {
    let (url, key) = resolve_claude_overlay(overlay)?;
    let model = claude_model(overlay).unwrap_or_default().to_string();
    Some(
        CLAUDE_PROVIDER_ENV
            .iter()
            .map(|name| {
                let value = match *name {
                    "ANTHROPIC_BASE_URL" => url.clone(),
                    "ANTHROPIC_API_KEY" | "ANTHROPIC_AUTH_TOKEN" => key.clone(),
                    _ => model.clone(),
                };
                (*name, value)
            })
            .filter(|(_, value)| !value.is_empty())
            .collect(),
    )
}

pub(crate) fn apply_codex_env(cmd: &mut Command, overlay: &HashMap<String, String>) {
    if let Some((url, key, _, _)) = resolve_codex_overlay(overlay) {
        cmd.env("OPENAI_BASE_URL", url);
        cmd.env("OPENAI_API_KEY", &key);
        cmd.env(COMPANY_API_KEY_ENV, &key);
        cmd.env(EVOTOWN_API_KEY_ENV, &key);
    }
}

pub(crate) fn apply_hermes_env(cmd: &mut Command, overlay: &HashMap<String, String>) {
    let home = hermes_home_from_overlay(overlay);
    cmd.env("HERMES_HOME", &home);
    if let Some((url, key, model)) = resolve_hermes_overlay(overlay) {
        cmd.env("OPENAI_BASE_URL", &url);
        cmd.env("OPENAI_API_KEY", &key);
        cmd.env(COMPANY_API_KEY_ENV, &key);
        cmd.env(EVOTOWN_API_KEY_ENV, &key);
        if let Some(model_id) = model {
            cmd.env(MODEL_ENV, &model_id);
            cmd.env("OPENAI_MODEL", &model_id);
        }
    }
}

pub(crate) fn apply_deepseek_harness_env(cmd: &mut Command, overlay: &HashMap<String, String>) {
    let (url, key) = resolve_deepseek_harness_overlay(overlay);
    if let Some(key) = key {
        cmd.env("DEEPSEEK_API_KEY", key);
    }
    if let Some(url) = url {
        cmd.env("DEEPSEEK_BASE_URL", url);
    }
}

pub(crate) fn resolve_deepseek_harness_overlay(
    overlay: &HashMap<String, String>,
) -> (Option<String>, Option<String>) {
    let direct_key = overlay
        .get("DEEPSEEK_API_KEY")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let direct_url = overlay
        .get("DEEPSEEK_BASE_URL")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    // DeepSeek Harness only talks to DeepSeek. A DeepSeek key must never be
    // paired with another provider's address, or the other way round.
    if direct_key.is_some() {
        return (direct_url, direct_key);
    }
    match resolve_hermes_overlay(overlay) {
        Some((url, key, _)) if url.to_ascii_lowercase().contains("api.deepseek.com") => {
            (Some(url), Some(key))
        }
        _ => (None, None),
    }
}

pub(crate) fn prepare_codex_home(overlay: &HashMap<String, String>) {
    let _ = clear_codex_placeholder_auth();
    if let Some((url, key, model, slot)) = resolve_codex_overlay(overlay) {
        let _ = clear_codex_chatgpt_auth_for_gateway();
        let _ = apply_codex_slot(&url, &key, model.as_deref(), Some(&slot));
    }
    let global = crate::adapters::util::home_join(".codex");
    let _ = crate::setup::merge::drop_unreachable_codex_mcp_servers(&global.join("config.toml"));
    if let Some(home) = overlay.get("CODEX_HOME").map(PathBuf::from) {
        let workspace = overlay.get("AGENT_DOCTOR_WORKSPACE").map(|s| s.as_str());
        let explicit_root = overlay.get("AGENT_DOCTOR_PROJECT_ROOT").map(Path::new);
        let project = crate::session_launch::resolve_session_cwd(explicit_root, workspace);
        let _ = crate::workspace::backends::bind_codex_for_project(&home, Some(&project));
        // Existing installs may still have provider keys in ~/.codex from older
        // wiring. Strip them when Ask uses an isolated CODEX_HOME so newer Codex
        // does not warn on every app-server turn.
        if !crate::workspace::path::paths_equal(&home, &global) {
            let _ =
                crate::setup::strip_codex_project_denied_provider_keys(&global.join("config.toml"));
            let _ =
                crate::setup::merge::drop_unreachable_codex_mcp_servers(&home.join("config.toml"));
        }
    }
}

/// Ensure the active Hermes profile (`HERMES_HOME`) has a usable model pointer + API key.
///
/// Workspace isolation points Hermes at `~/.hermes/profiles/<workspace>` which often only
/// has `terminal.cwd`. Without `model.provider`/`base_url`, Hermes falls back to OpenRouter
/// and fails with HTTP 401 when `OPENROUTER_API_KEY` is missing.
pub(crate) fn prepare_hermes_home(overlay: &HashMap<String, String>) {
    let Some((url, key, model)) = resolve_hermes_overlay(overlay) else {
        return;
    };
    let home = hermes_home_from_overlay(overlay);
    let _ = ensure_hermes_model_config(&home, &url, model.as_deref());
    let _ = upsert_dotenv_key(&home.join(".env"), "OPENAI_API_KEY", &key);
}

pub(crate) fn hermes_home_from_overlay(overlay: &HashMap<String, String>) -> PathBuf {
    overlay
        .get("HERMES_HOME")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::adapters::util::home_join(".hermes"))
}

/// (base_url, api_key, model)
pub(crate) fn resolve_hermes_overlay(
    env: &HashMap<String, String>,
) -> Option<(String, String, Option<String>)> {
    let url = env
        .get("OPENAI_BASE_URL")
        .or_else(|| env.get(GATEWAY_URL_ENV))
        .or_else(|| env.get(EVOTOWN_URL_ENV))
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())?
        .to_string();
    let key = [
        "OPENAI_API_KEY",
        COMPANY_API_KEY_ENV,
        EVOTOWN_API_KEY_ENV,
        "ANTHROPIC_API_KEY",
    ]
    .into_iter()
    .find_map(|k| {
        env.get(k)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    })?;
    let model = env
        .get(MODEL_ENV)
        .or_else(|| env.get("OPENAI_MODEL"))
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    Some((url, key, model))
}

fn ensure_hermes_model_config(home: &Path, gateway_url: &str, model: Option<&str>) -> Result<()> {
    use serde_yaml::{Mapping, Value as YamlValue};
    let path = home.join("config.yaml");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut root: YamlValue = if path.exists() {
        let raw =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        serde_yaml::from_str(&raw).unwrap_or_else(|_| YamlValue::Mapping(Mapping::new()))
    } else {
        YamlValue::Mapping(Mapping::new())
    };
    let root_map = root
        .as_mapping_mut()
        .context("Hermes config root must be a mapping")?;
    let model_section = root_map
        .entry(YamlValue::from("model"))
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
    let model_map = model_section
        .as_mapping_mut()
        .context("Hermes model section must be a mapping")?;
    let requested = model.map(str::trim).filter(|m| !m.is_empty());
    let existing_model = model_map
        .get(YamlValue::from("default"))
        .and_then(YamlValue::as_str)
        .unwrap_or("")
        .to_string();
    let chosen = requested.unwrap_or(existing_model.as_str());
    let pin_chat = chosen.is_empty() || !crate::setup::merge::model_is_claude(chosen);
    let gateway_url = if pin_chat {
        crate::setup::merge::hermes_chat_base_url(gateway_url)
    } else {
        gateway_url.trim().trim_end_matches('/').to_string()
    };
    model_map.insert(YamlValue::from("provider"), YamlValue::from("custom"));
    model_map.insert(
        YamlValue::from("base_url"),
        YamlValue::from(gateway_url.as_str()),
    );
    // A key written here beats OPENAI_API_KEY in .env, so an old one would stay in use.
    model_map.remove(YamlValue::from("api_key"));
    let model_id = (!chosen.is_empty()).then(|| {
        if pin_chat {
            crate::setup::merge::restore_hyphenated_vendor_model(chosen)
        } else {
            chosen.to_string()
        }
    });
    if let Some(model_id) = &model_id {
        model_map.insert(
            YamlValue::from("default"),
            YamlValue::from(model_id.as_str()),
        );
    }
    if pin_chat {
        model_map.insert(
            YamlValue::from("api_mode"),
            YamlValue::from("chat_completions"),
        );
    } else {
        model_map.remove(YamlValue::from("api_mode"));
    }
    // Keep auxiliary helpers on the same gateway so Hermes does not fall back to OpenRouter.
    let aux = root_map
        .entry(YamlValue::from("auxiliary"))
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
    if let Some(aux_map) = aux.as_mapping_mut() {
        for section in ["title_generation", "compression"] {
            let entry = aux_map
                .entry(YamlValue::from(section))
                .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
            if let Some(map) = entry.as_mapping_mut() {
                map.insert(YamlValue::from("provider"), YamlValue::from("custom"));
                map.insert(
                    YamlValue::from("base_url"),
                    YamlValue::from(gateway_url.as_str()),
                );
                map.remove(YamlValue::from("api_key"));
                // The previous provider's model id is unknown to this gateway.
                if let Some(model_id) = &model_id {
                    map.insert(YamlValue::from("model"), YamlValue::from(model_id.as_str()));
                }
            }
        }
    }
    std::fs::write(&path, serde_yaml::to_string(&root)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn upsert_dotenv_key(path: &Path, key: &str, value: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut lines: Vec<String> = if path.exists() {
        std::fs::read_to_string(path)?
            .lines()
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    };
    let mut replaced = false;
    for line in &mut lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((name, _)) = trimmed.split_once('=') {
            if name.trim() == key {
                *line = format!("{key}={value}");
                replaced = true;
                break;
            }
        }
    }
    if !replaced {
        lines.push(format!("{key}={value}"));
    }
    std::fs::write(path, format!("{}\n", lines.join("\n")))
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub(crate) fn resolve_claude_overlay(env: &HashMap<String, String>) -> Option<(String, String)> {
    let api_key = [
        "ANTHROPIC_API_KEY",
        COMPANY_API_KEY_ENV,
        EVOTOWN_API_KEY_ENV,
        "OPENAI_API_KEY",
    ]
    .into_iter()
    .find_map(|key| {
        env.get(key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })?;

    let protocol = env
        .get(PROVIDER_PROTOCOL_ENV)
        .map(|value| normalize_protocol(value));
    let personal = env
        .get(PROVIDER_KIND_ENV)
        .map(|value| value.trim())
        .is_some_and(|value| value.eq_ignore_ascii_case(PROVIDER_KIND_PERSONAL));

    let gateway = env
        .get(GATEWAY_URL_ENV)
        .or_else(|| env.get("OPENAI_BASE_URL"))
        .or_else(|| env.get("ANTHROPIC_BASE_URL"))
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    // Personal providers: never fall back to Evotown/skilllite leftovers.
    if personal {
        if protocol.as_deref() == Some(PROTOCOL_ANTHROPIC) {
            if let Some(url) = gateway {
                return Some((url, api_key));
            }
            return None;
        }
        if let Some(url) = gateway
            .as_deref()
            .and_then(anthropic_gateway_for_provider_url)
        {
            return Some((url, api_key));
        }
        return None;
    }

    if let Some(url) = env
        .get("ANTHROPIC_BASE_URL")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        return Some((url.to_string(), api_key));
    }

    if protocol.as_deref() == Some(PROTOCOL_ANTHROPIC) {
        if let Some(url) = gateway {
            return Some((url, api_key));
        }
    }

    if let Some(url) = gateway
        .as_deref()
        .and_then(anthropic_gateway_for_provider_url)
    {
        return Some((url, api_key));
    }

    let evotown = env
        .get("AGENT_DOCTOR_EVOTOWN_URL")
        .or_else(|| env.get(EVOTOWN_URL_ENV))
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())?;
    Some((anthropic_gateway_url_from_evotown_base(evotown), api_key))
}

/// (base_url, api_key, model, slot)
pub(crate) fn resolve_codex_overlay(
    env: &HashMap<String, String>,
) -> Option<(String, String, Option<String>, String)> {
    let url = env
        .get("OPENAI_BASE_URL")
        .or_else(|| env.get(GATEWAY_URL_ENV))
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())?
        .to_string();
    let key = [
        "OPENAI_API_KEY",
        COMPANY_API_KEY_ENV,
        EVOTOWN_API_KEY_ENV,
        "ANTHROPIC_API_KEY",
    ]
    .into_iter()
    .find_map(|k| {
        env.get(k)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    })?;
    let model = env
        .get(MODEL_ENV)
        .or_else(|| env.get("OPENAI_MODEL"))
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    let slot = env
        .get("AGENT_DOCTOR_PROVIDER_KIND")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| {
            if v.eq_ignore_ascii_case("personal") {
                "personal".to_string()
            } else {
                "company".to_string()
            }
        })
        .unwrap_or_else(|| "company".to_string());
    let url = crate::setup::merge::codex_responses_gateway_url(&url);
    Some((url, key, model, slot))
}

pub(crate) fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `-c` overrides shared by Codex exec / app-server for company/personal gateway.
pub(crate) fn codex_provider_config_args(
    launch: Option<&(String, String, Option<String>, String)>,
) -> Vec<String> {
    let mut argv = Vec::new();
    let Some((url, _, model, slot)) = launch else {
        return argv;
    };
    let mut set = |key: &str, value: String| {
        argv.push("-c".to_string());
        argv.push(format!("{key}={value}"));
    };
    let prefix = format!("model_providers.{slot}");
    set(
        &format!("{prefix}.name"),
        toml_string(if slot == "company" {
            "Company"
        } else {
            "Personal"
        }),
    );
    set(&format!("{prefix}.base_url"), toml_string(url));
    set(&format!("{prefix}.env_key"), toml_string("OPENAI_API_KEY"));
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
    if let Some(model_id) = model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        set("model", toml_string(model_id));
    }
    argv
}

pub(crate) fn format_command_display(cmd: &Command) -> String {
    let program = cmd.get_program().to_string_lossy();
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| {
            let s = a.to_string_lossy();
            if s.contains(' ') || s.contains('"') {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.into_owned()
            }
        })
        .collect();
    if args.is_empty() {
        program.into_owned()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn claude_ask_enables_plan_tools_by_default() {
        let mut cmd = Command::new("claude");
        apply_claude_env(&mut cmd, &HashMap::new());
        let enabled = cmd.get_envs().any(|(key, value)| {
            key == "CLAUDE_CODE_ENABLE_TODO_TOOLS" && value.and_then(|v| v.to_str()) == Some("1")
        });
        assert!(enabled);

        let mut overlay = HashMap::new();
        overlay.insert("CLAUDE_CODE_ENABLE_TODO_TOOLS".into(), "0".into());
        let mut cmd = Command::new("claude");
        apply_claude_env(&mut cmd, &overlay);
        let overridden = cmd
            .get_envs()
            .any(|(key, _)| key == "CLAUDE_CODE_ENABLE_TODO_TOOLS");
        assert!(!overridden);
    }

    #[test]
    fn resolve_claude_personal_official_maps_to_anthropic() {
        let mut env = HashMap::new();
        env.insert(PROVIDER_KIND_ENV.into(), PROVIDER_KIND_PERSONAL.into());
        env.insert(PROVIDER_PROTOCOL_ENV.into(), "openai".into());
        env.insert(
            GATEWAY_URL_ENV.into(),
            "https://teamups.vip/api/v1/official".into(),
        );
        env.insert(
            "ANTHROPIC_BASE_URL".into(),
            "https://api.deepseek.com/anthropic".into(),
        );
        env.insert("OPENAI_API_KEY".into(), "sk-test".into());

        let (url, key) = resolve_claude_overlay(&env).expect("overlay");
        assert_eq!(url, "https://teamups.vip/api/v1/official/anthropic");
        assert_eq!(key, "sk-test");
    }

    #[test]
    fn resolve_claude_personal_deepseek_openai_maps_to_anthropic() {
        let mut env = HashMap::new();
        env.insert(PROVIDER_KIND_ENV.into(), PROVIDER_KIND_PERSONAL.into());
        env.insert(PROVIDER_PROTOCOL_ENV.into(), "openai".into());
        env.insert(GATEWAY_URL_ENV.into(), "https://api.deepseek.com/v1".into());
        env.insert(
            "ANTHROPIC_BASE_URL".into(),
            "https://www.skilllite.ai/api/gateway/anthropic".into(),
        );
        env.insert("OPENAI_API_KEY".into(), "sk-test".into());

        let (url, key) = resolve_claude_overlay(&env).expect("overlay");
        assert_eq!(url, "https://api.deepseek.com/anthropic");
        assert_eq!(key, "sk-test");
    }

    #[test]
    fn resolve_claude_prefers_personal_gateway_over_skilllite_leftover() {
        let mut env = HashMap::new();
        env.insert(PROVIDER_KIND_ENV.into(), PROVIDER_KIND_PERSONAL.into());
        env.insert(PROVIDER_PROTOCOL_ENV.into(), PROTOCOL_ANTHROPIC.into());
        env.insert(
            GATEWAY_URL_ENV.into(),
            "https://api.deepseek.com/anthropic".into(),
        );
        env.insert(
            "ANTHROPIC_BASE_URL".into(),
            "https://www.skilllite.ai/api/gateway/anthropic".into(),
        );
        env.insert("ANTHROPIC_API_KEY".into(), "sk-test".into());

        let (url, key) = resolve_claude_overlay(&env).expect("overlay");
        assert_eq!(url, "https://api.deepseek.com/anthropic");
        assert_eq!(key, "sk-test");
    }

    #[test]
    fn resolve_claude_personal_does_not_fall_back_to_evotown() {
        let mut env = HashMap::new();
        env.insert(PROVIDER_KIND_ENV.into(), PROVIDER_KIND_PERSONAL.into());
        env.insert(PROVIDER_PROTOCOL_ENV.into(), "openai".into());
        env.insert(EVOTOWN_URL_ENV.into(), "https://www.skilllite.ai".into());
        env.insert("OPENAI_API_KEY".into(), "sk-test".into());

        assert!(resolve_claude_overlay(&env).is_none());
    }

    #[test]
    fn deepseek_harness_never_mixes_providers() {
        let mut env = HashMap::new();
        env.insert(
            GATEWAY_URL_ENV.to_string(),
            "https://open.bigmodel.cn/api/coding/paas/v4".to_string(),
        );
        env.insert("OPENAI_API_KEY".to_string(), "glm-key".to_string());
        assert_eq!(resolve_deepseek_harness_overlay(&env), (None, None));

        env.insert("DEEPSEEK_API_KEY".to_string(), "ds-key".to_string());
        assert_eq!(
            resolve_deepseek_harness_overlay(&env),
            (None, Some("ds-key".to_string()))
        );

        env.remove("DEEPSEEK_API_KEY");
        env.insert(
            GATEWAY_URL_ENV.to_string(),
            "https://api.deepseek.com/v1".to_string(),
        );
        assert_eq!(
            resolve_deepseek_harness_overlay(&env),
            (
                Some("https://api.deepseek.com/v1".to_string()),
                Some("glm-key".to_string())
            )
        );
    }

    #[test]
    fn strip_team_gateway_removes_skilllite_anthropic_base() {
        let mut env = HashMap::new();
        env.insert(
            "ANTHROPIC_BASE_URL".into(),
            "https://www.skilllite.ai/api/gateway/anthropic".into(),
        );
        env.insert(EVOTOWN_URL_ENV.into(), "https://www.skilllite.ai".into());
        strip_team_gateway_leftovers(&mut env);
        assert!(!env.contains_key("ANTHROPIC_BASE_URL"));
        assert!(!env.contains_key(EVOTOWN_URL_ENV));
    }
}
