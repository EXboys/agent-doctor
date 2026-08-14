use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

use crate::profile::{agent_profile_path, read_env_map, COMPANY_API_KEY_ENV, GATEWAY_URL_ENV};
use crate::setup::{
    anthropic_gateway_url_from_evotown_base, apply_codex_slot, clear_codex_chatgpt_auth_for_gateway,
    clear_codex_placeholder_auth, evotown_agent_env_path, normalize_protocol, EVOTOWN_API_KEY_ENV,
    EVOTOWN_URL_ENV, MODEL_ENV, PROTOCOL_ANTHROPIC, PROVIDER_PROTOCOL_ENV,
};
use crate::workspace::active_env_path;

pub(crate) fn collect_overlay_env() -> HashMap<String, String> {
    let mut env = HashMap::new();
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
    merge(evotown_agent_env_path());
    for key in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
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
    ] {
        if let Ok(v) = std::env::var(key as &str) {
            if !v.trim().is_empty() {
                env.insert((*key).to_string(), crate::profile::unquote_env_value(&v));
            }
        }
    }
    env
}

pub(crate) fn apply_overlay_env(cmd: &mut Command, overlay: &HashMap<String, String>) {
    for (key, value) in overlay {
        cmd.env(key, value);
    }
}

pub(crate) fn apply_claude_env(cmd: &mut Command, overlay: &HashMap<String, String>) {
    if let Some((url, key)) = resolve_claude_overlay(overlay) {
        cmd.env("ANTHROPIC_BASE_URL", url);
        cmd.env("ANTHROPIC_API_KEY", &key);
        cmd.env(COMPANY_API_KEY_ENV, &key);
        cmd.env(EVOTOWN_API_KEY_ENV, &key);
    }
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

pub(crate) fn prepare_codex_home(overlay: &HashMap<String, String>) {
    let _ = clear_codex_placeholder_auth();
    if let Some((url, key, model, slot)) = resolve_codex_overlay(overlay) {
        let _ = clear_codex_chatgpt_auth_for_gateway();
        let _ = apply_codex_slot(&url, &key, model.as_deref(), Some(&slot));
    }
    if let Some(home) = overlay.get("CODEX_HOME").map(PathBuf::from) {
        let _ = crate::workspace::backends::bind_codex(&home);
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

fn ensure_hermes_model_config(home: &PathBuf, gateway_url: &str, model: Option<&str>) -> Result<()> {
    use serde_yaml::{Mapping, Value as YamlValue};
    let path = home.join("config.yaml");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let mut root: YamlValue = if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
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
    model_map.insert(YamlValue::from("provider"), YamlValue::from("custom"));
    model_map.insert(YamlValue::from("base_url"), YamlValue::from(gateway_url));
    if let Some(model_id) = model.map(str::trim).filter(|m| !m.is_empty()) {
        model_map.insert(YamlValue::from("default"), YamlValue::from(model_id));
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
                map.insert(YamlValue::from("base_url"), YamlValue::from(gateway_url));
            }
        }
    }
    std::fs::write(&path, serde_yaml::to_string(&root)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn upsert_dotenv_key(path: &PathBuf, key: &str, value: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
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

    if let Some(url) = env
        .get("ANTHROPIC_BASE_URL")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        return Some((url.to_string(), api_key));
    }

    let protocol = env
        .get(PROVIDER_PROTOCOL_ENV)
        .map(|value| normalize_protocol(value));
    if protocol.as_deref() == Some(PROTOCOL_ANTHROPIC) {
        if let Some(url) = env
            .get(GATEWAY_URL_ENV)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return Some((url.to_string(), api_key));
        }
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
    set(
        &format!("{prefix}.env_key"),
        toml_string("OPENAI_API_KEY"),
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
