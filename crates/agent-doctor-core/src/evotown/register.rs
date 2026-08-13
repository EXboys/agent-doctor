//! Register this machine as an Evotown engine and persist the per-engine `evi_` token.
//!
//! Replaces `evotown-agent-setup.py register --save-token`.

use std::collections::HashMap;
use std::env;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::evotown::client::EvotownClient;
use crate::evotown::jobs::normalize_runtime;
use crate::evotown::preferred_runtime::resolve_preferred_runtime;
use crate::setup::{
    evotown_agent_env_path, remove_evotown_agent_env_key, upsert_evotown_agent_env_key,
    EVOTOWN_RUNTIME_ENV, EVOTOWN_URL_ENV,
};

pub const ENGINE_ID_ENV: &str = "EVOTOWN_ENGINE_ID";
pub const ENGINE_INGEST_TOKEN_ENV: &str = "EVOTOWN_ENGINE_INGEST_TOKEN";
/// IT bootstrap token (register only). Not the per-engine `evi_` token.
pub const BOOTSTRAP_INGEST_TOKEN_ENV: &str = "EVOTOWN_INGEST_TOKEN";

#[derive(Debug, Clone, Default)]
pub struct RegisterOptions {
    pub bootstrap_token: Option<String>,
    pub engine_id: Option<String>,
    pub engine_type: Option<String>,
    pub runtime: Option<String>,
    pub display_name: Option<String>,
    pub owner_team: Option<String>,
    pub deployment_kind: Option<String>,
    pub engine_version: Option<String>,
    /// Force issuance of a new `evi_` even if the engine already exists.
    pub rotate: bool,
    /// Write `EVOTOWN_ENGINE_ID` + `EVOTOWN_ENGINE_INGEST_TOKEN` into evotown.agent.env.
    pub save_token: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterReport {
    pub base_url: String,
    pub engine_id: String,
    pub engine_type: String,
    pub ingest_token_issued: bool,
    /// Present only when Evotown returned a fresh token (shown once).
    pub ingest_token: Option<String>,
    pub saved_to: Option<String>,
    pub rotated: bool,
    pub detail: String,
}

/// Register (or update) this laptop engine with Evotown.
pub fn execute_register(options: &RegisterOptions) -> Result<RegisterReport> {
    let env_path = evotown_agent_env_path()
        .context("could not resolve ~/.config/evotown/evotown.agent.env")?;
    let file_env = load_env_map_optional(&env_path)?;

    let base_url = resolve_base_url(&file_env)?;
    let bootstrap = resolve_bootstrap_token(options, &file_env)?;
    let runtime_hint = options
        .runtime
        .as_deref()
        .map(normalize_runtime)
        .or_else(|| {
            file_env
                .get(EVOTOWN_RUNTIME_ENV)
                .map(|v| normalize_runtime(v))
        })
        .or_else(resolve_preferred_runtime)
        .unwrap_or_else(|| "openclaw".to_string());

    let engine_type = resolve_engine_type(options.engine_type.as_deref(), &runtime_hint);
    let engine_id = resolve_engine_id(options.engine_id.as_deref(), &file_env, &runtime_hint);
    let display_name = options
        .display_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| engine_id.clone());
    let owner_team = options
        .owner_team
        .clone()
        .or_else(|| file_env.get("EVOTOWN_TEAM_ID").cloned())
        .unwrap_or_default();
    let deployment_kind = options
        .deployment_kind
        .clone()
        .or_else(|| file_env.get("EVOTOWN_DEPLOYMENT_KIND").cloned())
        .unwrap_or_else(|| "laptop".to_string());
    let engine_version = options
        .engine_version
        .clone()
        .or_else(|| file_env.get("EVOTOWN_ENGINE_VERSION").cloned())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    let body = json!({
        "engine_id": engine_id,
        "engine_type": engine_type,
        "engine_version": engine_version,
        "owner_team": owner_team,
        "deployment_kind": deployment_kind,
        "display_name": display_name,
        "capabilities": {
            "dispatch_lease": true,
            "events": true,
            "handoff": true,
        },
        "rotate_ingest_token": options.rotate,
    });

    // Auth with IT bootstrap (or admin), not the employee `evk_` key.
    let client = EvotownClient::new(&base_url, &bootstrap)?;
    let payload = client
        .post_json("/api/v1/engines/register", body)
        .context("Evotown engine register failed")?;

    let registered_id = payload
        .pointer("/engine/engine_id")
        .and_then(Value::as_str)
        .unwrap_or(&engine_id)
        .to_string();

    let issued = payload
        .get("ingest_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string);

    if let Some(token) = issued.as_ref() {
        if !token.starts_with("evi_") {
            bail!(
                "Evotown returned an ingest token that does not start with evi_ (prefix {:?})",
                token.chars().take(4).collect::<String>()
            );
        }
    }

    let mut saved_to = None;
    if options.save_token {
        if let Some(token) = issued.as_ref() {
            save_engine_credentials(&env_path, &registered_id, token, options.runtime.as_deref())?;
            saved_to = Some(env_path.display().to_string());
        } else if options.rotate {
            bail!(
                "register succeeded but Evotown did not return a new ingest_token despite --rotate"
            );
        } else {
            // Re-register without rotate: still persist engine_id so connect can find it.
            upsert_evotown_agent_env_key(&env_path, ENGINE_ID_ENV, &registered_id)?;
            if let Some(runtime) = options.runtime.as_deref() {
                upsert_evotown_agent_env_key(
                    &env_path,
                    EVOTOWN_RUNTIME_ENV,
                    &normalize_runtime(runtime),
                )?;
            }
            saved_to = Some(env_path.display().to_string());
        }
    }

    let detail = if issued.is_some() {
        if saved_to.is_some() {
            format!(
                "Engine `{registered_id}` registered; per-engine evi_ token saved for `agent-doctor connect`."
            )
        } else {
            format!(
                "Engine `{registered_id}` registered; ingest token issued (not saved — re-run without --no-save-token)."
            )
        }
    } else {
        format!(
            "Engine `{registered_id}` updated; no new token issued. Use --rotate to mint a fresh evi_, \
             or keep the existing EVOTOWN_ENGINE_INGEST_TOKEN on disk."
        )
    };

    Ok(RegisterReport {
        base_url,
        engine_id: registered_id,
        engine_type,
        ingest_token_issued: issued.is_some(),
        ingest_token: issued,
        saved_to,
        rotated: options.rotate,
        detail,
    })
}

fn save_engine_credentials(
    path: &Path,
    engine_id: &str,
    ingest_token: &str,
    runtime: Option<&str>,
) -> Result<()> {
    upsert_evotown_agent_env_key(path, ENGINE_ID_ENV, engine_id)?;
    upsert_evotown_agent_env_key(path, ENGINE_INGEST_TOKEN_ENV, ingest_token)?;
    // Retire legacy IT bootstrap key name if it was sitting in the employee env file.
    remove_evotown_agent_env_key(path, BOOTSTRAP_INGEST_TOKEN_ENV)?;
    if let Some(runtime) = runtime {
        upsert_evotown_agent_env_key(path, EVOTOWN_RUNTIME_ENV, &normalize_runtime(runtime))?;
    }
    Ok(())
}

fn resolve_base_url(file_env: &HashMap<String, String>) -> Result<String> {
    file_env
        .get(EVOTOWN_URL_ENV)
        .cloned()
        .or_else(|| env::var(EVOTOWN_URL_ENV).ok())
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .context(
            "EVOTOWN_URL is required — run `agent-doctor setup --url <evotown> --key evk_...` first",
        )
}

fn resolve_bootstrap_token(
    options: &RegisterOptions,
    file_env: &HashMap<String, String>,
) -> Result<String> {
    let token = options
        .bootstrap_token
        .clone()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| env::var(BOOTSTRAP_INGEST_TOKEN_ENV).ok())
        .or_else(|| file_env.get(BOOTSTRAP_INGEST_TOKEN_ENV).cloned())
        .or_else(|| env::var(ENGINE_INGEST_TOKEN_ENV).ok())
        .or_else(|| file_env.get(ENGINE_INGEST_TOKEN_ENV).cloned())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());

    let Some(token) = token else {
        bail!(
            "IT bootstrap token required for register — pass --bootstrap-token, or set \
             {BOOTSTRAP_INGEST_TOKEN_ENV} (from Evotown IT). Do not use the employee evk_ key."
        );
    };
    Ok(token)
}

fn resolve_engine_id(
    explicit: Option<&str>,
    file_env: &HashMap<String, String>,
    runtime: &str,
) -> String {
    if let Some(id) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return id.to_string();
    }
    if let Some(id) = file_env
        .get(ENGINE_ID_ENV)
        .map(|v| v.trim())
        .filter(|s| !s.is_empty())
    {
        return id.to_string();
    }
    if let Ok(id) = env::var(ENGINE_ID_ENV) {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let user = env::var("USER")
        .or_else(|_| env::var("USERNAME"))
        .unwrap_or_else(|_| "local".to_string());
    let safe_user: String = user
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("{runtime}-{safe_user}")
}

/// Map Doctor/runtime ids onto Evotown `EngineType` literals.
pub fn resolve_engine_type(explicit: Option<&str>, runtime: &str) -> String {
    if let Some(t) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return t.to_string();
    }
    match normalize_runtime(runtime).as_str() {
        "openclaw" => "openclaw".into(),
        "hermes" => "hermes".into(),
        "skilllite" => "skilllite".into(),
        "agent-doctor" => "agent-doctor".into(),
        // Claude / Codex are dispatched by Agent Doctor, not native Evotown engine types.
        "claude-code" | "codex" => "agent-doctor".into(),
        _ => "custom".into(),
    }
}

fn load_env_map_optional(path: &Path) -> Result<HashMap<String, String>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = std::fs::read_to_string(path)?;
    let mut values = HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let assignment = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = assignment.split_once('=') else {
            continue;
        };
        values.insert(
            key.trim().to_string(),
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string(),
        );
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn maps_claude_and_codex_to_agent_doctor_engine_type() {
        assert_eq!(resolve_engine_type(None, "claude-code"), "agent-doctor");
        assert_eq!(resolve_engine_type(None, "codex"), "agent-doctor");
        assert_eq!(resolve_engine_type(None, "openclaw"), "openclaw");
        assert_eq!(resolve_engine_type(Some("custom"), "openclaw"), "custom");
    }

    #[test]
    fn saves_engine_credentials_and_drops_legacy_bootstrap_key() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("evotown.agent.env");
        std::fs::write(
            &path,
            "EVOTOWN_URL=https://evotown.example\nEVOTOWN_INGEST_TOKEN=bootstrap\nEVOTOWN_API_KEY=evk_x\n",
        )
        .unwrap();
        save_engine_credentials(&path, "claude-code-airlu", "evi_secret", Some("claude")).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("EVOTOWN_ENGINE_ID=claude-code-airlu"));
        assert!(raw.contains("EVOTOWN_ENGINE_INGEST_TOKEN=evi_secret"));
        assert!(raw.contains("EVOTOWN_RUNTIME=claude-code"));
        assert!(!raw.contains("EVOTOWN_INGEST_TOKEN="));
        assert!(raw.contains("EVOTOWN_URL=https://evotown.example"));
    }

    #[test]
    fn default_engine_id_uses_runtime_and_user() {
        let id = resolve_engine_id(None, &HashMap::new(), "openclaw");
        assert!(id.starts_with("openclaw-"));
        assert_eq!(
            resolve_engine_id(Some("my-engine"), &HashMap::new(), "openclaw"),
            "my-engine"
        );
    }
}
