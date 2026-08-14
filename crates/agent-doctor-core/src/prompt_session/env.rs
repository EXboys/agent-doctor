use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

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
