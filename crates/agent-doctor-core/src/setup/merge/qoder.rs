//! Qoder (`qodercli`) custom provider & API key wiring.
//!
//! `qodercli` ignores bare `OPENAI_BASE_URL` / `OPENAI_API_KEY`. It speaks to a
//! user-configured OpenAI or Anthropic endpoint without signing in to a Qoder
//! account when `~/.qoder/settings.json` defines `providers.<slot>` and points
//! `model.name` (or `QODER_MODEL`) at `<slot>/<model_id>`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result as AnyhowResult};
use serde_json::{json, Value};

use crate::adapters::util::home_join;
use crate::setup::{backup_file, ensure_parent, RuntimeSetupResult, PROTOCOL_ANTHROPIC};

pub const QODER_PROVIDER_SLOT: &str = "agent-doctor";
const DEFAULT_QODER_MODEL: &str = "deepseek-v4.1-flash";

pub fn qoder_model_ref(model_id: &str) -> String {
    let trimmed = model_id.trim();
    let id = if trimmed.is_empty() {
        DEFAULT_QODER_MODEL
    } else {
        trimmed
    };
    format!("{QODER_PROVIDER_SLOT}/{id}")
}

pub fn apply_qoder_with_model(
    gateway_url: &str,
    api_key: &str,
    protocol: &str,
    model: Option<&str>,
) -> AnyhowResult<RuntimeSetupResult> {
    let qoder_dir = home_join(".qoder");
    apply_qoder_in_dir(&qoder_dir, gateway_url, api_key, protocol, model)
}

pub fn apply_qoder_in_dir(
    qoder_dir: &Path,
    gateway_url: &str,
    api_key: &str,
    protocol: &str,
    model: Option<&str>,
) -> AnyhowResult<RuntimeSetupResult> {
    let url = gateway_url.trim().trim_end_matches('/');
    let key = api_key.trim();
    if url.is_empty() || key.is_empty() {
        return Ok(RuntimeSetupResult {
            runtime_id: "qoder".into(),
            display_name: "Qoder".into(),
            applied: false,
            message: "skipped — gateway URL or API key is empty".into(),
            ..Default::default()
        });
    }
    let model_id = model
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(DEFAULT_QODER_MODEL);
    let qoder_protocol = if protocol.eq_ignore_ascii_case(PROTOCOL_ANTHROPIC) {
        "anthropic"
    } else {
        "openai"
    };

    let settings_path = qoder_dir.join("settings.json");
    ensure_parent(&settings_path)?;
    let backup_path = backup_file(&settings_path)?;

    let mut root = if settings_path.exists() {
        fs::read_to_string(&settings_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}))
    } else {
        json!({})
    };

    let obj = root
        .as_object_mut()
        .context("Qoder settings.json root must be an object")?;
    let providers = obj.entry("providers").or_insert_with(|| json!({}));
    if !providers.is_object() {
        *providers = json!({});
    }
    providers.as_object_mut().expect("object").insert(
        QODER_PROVIDER_SLOT.to_string(),
        json!({
            "type": "openai-compatible",
            "protocol": qoder_protocol,
            "baseUrl": url,
            "apiKey": key,
            "displayName": "Agent Doctor",
            "model": model_id,
            "models": [
                {
                    "model": model_id,
                    "displayName": model_id,
                    "capabilities": {
                        "tools": true,
                        "vision": true
                    }
                }
            ]
        }),
    );

    let model_ref = qoder_model_ref(model_id);
    let model_obj = obj.entry("model").or_insert_with(|| json!({}));
    if !model_obj.is_object() {
        *model_obj = json!({});
    }
    model_obj
        .as_object_mut()
        .expect("object")
        .insert("name".into(), Value::String(model_ref.clone()));

    fs::write(&settings_path, serde_json::to_string_pretty(&root)?)
        .with_context(|| format!("write {}", settings_path.display()))?;

    Ok(RuntimeSetupResult {
        runtime_id: "qoder".into(),
        display_name: "Qoder".into(),
        config_path: Some(settings_path.display().to_string()),
        applied: true,
        backup_path: backup_path.map(|p| p.display().to_string()),
        message: format!("wired Qoder {model_ref} to {url} ({qoder_protocol})"),
        ..Default::default()
    })
}

/// Remove Agent Doctor's custom provider override so Qoder uses its own login.
pub fn clear_qoder_provider_override_in_dir(qoder_dir: &Path) -> AnyhowResult<bool> {
    let settings_path = qoder_dir.join("settings.json");
    if !settings_path.exists() {
        return Ok(false);
    }
    let raw = fs::read_to_string(&settings_path)
        .with_context(|| format!("read {}", settings_path.display()))?;
    let Ok(mut root) = serde_json::from_str::<Value>(&raw) else {
        return Ok(false);
    };
    let mut changed = false;
    if let Some(obj) = root.as_object_mut() {
        if let Some(providers) = obj.get_mut("providers").and_then(Value::as_object_mut) {
            if providers.remove(QODER_PROVIDER_SLOT).is_some() {
                changed = true;
            }
        }
        if let Some(model_obj) = obj.get_mut("model").and_then(Value::as_object_mut) {
            let points_to_us = model_obj
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.starts_with(&format!("{QODER_PROVIDER_SLOT}/")));
            if points_to_us {
                model_obj.remove("name");
                changed = true;
            }
        }
    }
    if changed {
        let _ = backup_file(&settings_path)?;
        fs::write(&settings_path, serde_json::to_string_pretty(&root)?)
            .with_context(|| format!("write {}", settings_path.display()))?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_clears_qoder_custom_provider() {
        let dir = tempfile::tempdir().unwrap();
        let res = apply_qoder_in_dir(
            dir.path(),
            "https://api.deepseek.com/v1/",
            "sk-qoder-test",
            "openai",
            Some("deepseek-chat"),
        )
        .unwrap();
        assert!(res.applied);
        let raw = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(raw.contains("\"baseUrl\": \"https://api.deepseek.com/v1\""));
        assert!(raw.contains("\"apiKey\": \"sk-qoder-test\""));
        assert!(raw.contains("\"name\": \"agent-doctor/deepseek-chat\""));

        assert!(clear_qoder_provider_override_in_dir(dir.path()).unwrap());
        let cleared = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(!cleared.contains("sk-qoder-test"));
        assert!(!cleared.contains("agent-doctor/deepseek-chat"));
    }
}
