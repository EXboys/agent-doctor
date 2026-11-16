//! WorkBuddy (`codebuddy`) custom model & API key wiring.
//!
//! `codebuddy` ignores `OPENAI_BASE_URL` / `OPENAI_API_KEY`. It bypasses Tencent
//! browser login without any prompt when two things line up:
//! 1. `CODEBUDDY_API_KEY` + `CODEBUDDY_MODEL` are present (via process env or
//!    `~/.codebuddy/settings.json` `env`).
//! 2. `~/.codebuddy/models.json` registers that model id with an OpenAI
//!    `/chat/completions` URL and `apiKey`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result as AnyhowResult};
use serde_json::{json, Value};

use crate::adapters::util::home_join;
use crate::setup::{backup_file, ensure_parent, RuntimeSetupResult};

const DEFAULT_WORKBUDDY_MODEL: &str = "deepseek-v4.1-flash";

/// Normalize an OpenAI-compatible gateway URL into the `/chat/completions` URL
/// that CodeBuddy's custom model registry expects.
pub fn workbuddy_chat_completions_url(gateway_url: &str) -> String {
    let base = super::hermes_chat_base_url(gateway_url);
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

/// Write `~/.codebuddy/models.json` and `~/.codebuddy/settings.json` so both
/// in-app Ask (`codebuddy --acp`) and native terminal (`codebuddy`) run directly
/// against the user's configured provider or TeamUps official service.
pub fn apply_workbuddy_with_model(
    gateway_url: &str,
    api_key: &str,
    model: Option<&str>,
) -> AnyhowResult<RuntimeSetupResult> {
    let codebuddy_dir = home_join(".codebuddy");
    apply_workbuddy_in_dir(&codebuddy_dir, gateway_url, api_key, model)
}

pub fn apply_workbuddy_in_dir(
    codebuddy_dir: &Path,
    gateway_url: &str,
    api_key: &str,
    model: Option<&str>,
) -> AnyhowResult<RuntimeSetupResult> {
    let key = api_key.trim();
    let chat_url = workbuddy_chat_completions_url(gateway_url);
    if key.is_empty() || chat_url.is_empty() {
        return Ok(RuntimeSetupResult {
            runtime_id: "workbuddy".into(),
            display_name: "WorkBuddy".into(),
            applied: false,
            message: "skipped — gateway URL or API key is empty".into(),
            ..Default::default()
        });
    }
    let model_id = model
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(DEFAULT_WORKBUDDY_MODEL);

    let models_path = codebuddy_dir.join("models.json");
    let settings_path = codebuddy_dir.join("settings.json");
    ensure_parent(&models_path)?;
    ensure_parent(&settings_path)?;
    let _ = backup_file(&models_path)?;
    let backup_path = backup_file(&settings_path)?;

    upsert_workbuddy_models_file(&models_path, model_id, &chat_url, key)?;
    upsert_workbuddy_settings_file(&settings_path, model_id, gateway_url.trim(), key)?;

    Ok(RuntimeSetupResult {
        runtime_id: "workbuddy".into(),
        display_name: "WorkBuddy".into(),
        config_path: Some(settings_path.display().to_string()),
        applied: true,
        backup_path: backup_path.map(|p| p.display().to_string()),
        message: format!("wired WorkBuddy model {model_id} to {chat_url}"),
        ..Default::default()
    })
}

/// Remove Agent Doctor's API-key override from `~/.codebuddy/settings.json` so
/// WorkBuddy falls back to its own Tencent / WeChat account login.
pub fn clear_workbuddy_provider_override_in_dir(codebuddy_dir: &Path) -> AnyhowResult<bool> {
    let settings_path = codebuddy_dir.join("settings.json");
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
        if let Some(env) = obj.get_mut("env").and_then(Value::as_object_mut) {
            for k in ["CODEBUDDY_API_KEY", "CODEBUDDY_BASE_URL", "CODEBUDDY_MODEL"] {
                if env.remove(k).is_some() {
                    changed = true;
                }
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

fn upsert_workbuddy_models_file(
    models_path: &Path,
    model_id: &str,
    chat_url: &str,
    api_key: &str,
) -> AnyhowResult<()> {
    let mut root = if models_path.exists() {
        fs::read_to_string(models_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}))
    } else {
        json!({})
    };

    let obj = root
        .as_object_mut()
        .context("models.json root must be an object")?;
    let entry = json!({
        "id": model_id,
        "name": model_id,
        "vendor": "AgentDoctor",
        "url": chat_url,
        "apiKey": api_key,
        "maxInputTokens": 128000,
        "supportsToolCall": true,
        "supportsImages": true,
    });

    let models = obj
        .entry("models")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !models.is_array() {
        *models = Value::Array(Vec::new());
    }
    let arr = models.as_array_mut().expect("array");
    if let Some(existing) = arr.iter_mut().find(|item| {
        item.get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == model_id)
    }) {
        *existing = entry;
    } else {
        arr.insert(0, entry);
    }

    if let Some(avail) = obj.get_mut("availableModels").and_then(Value::as_array_mut) {
        if !avail.iter().any(|v| v.as_str() == Some(model_id)) {
            avail.insert(0, Value::String(model_id.to_string()));
        }
    }

    fs::write(models_path, serde_json::to_string_pretty(&root)?)
        .with_context(|| format!("write {}", models_path.display()))
}

fn upsert_workbuddy_settings_file(
    settings_path: &Path,
    model_id: &str,
    base_url: &str,
    api_key: &str,
) -> AnyhowResult<()> {
    let mut root = if settings_path.exists() {
        fs::read_to_string(settings_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}))
    } else {
        json!({})
    };

    let obj = root
        .as_object_mut()
        .context("settings.json root must be an object")?;
    obj.insert("model".into(), Value::String(model_id.to_string()));
    let env = obj.entry("env").or_insert_with(|| json!({}));
    if !env.is_object() {
        *env = json!({});
    }
    let env_obj = env.as_object_mut().expect("object");
    env_obj.insert(
        "CODEBUDDY_API_KEY".into(),
        Value::String(api_key.to_string()),
    );
    env_obj.insert(
        "CODEBUDDY_BASE_URL".into(),
        Value::String(base_url.to_string()),
    );
    env_obj.insert(
        "CODEBUDDY_MODEL".into(),
        Value::String(model_id.to_string()),
    );

    fs::write(settings_path, serde_json::to_string_pretty(&root)?)
        .with_context(|| format!("write {}", settings_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_gateway_to_chat_completions_url() {
        assert_eq!(
            workbuddy_chat_completions_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            workbuddy_chat_completions_url("https://teamups.vip/api/v1/official/"),
            "https://teamups.vip/api/v1/official/chat/completions"
        );
        assert_eq!(
            workbuddy_chat_completions_url("https://open.bigmodel.cn/api/paas/v4/chat/completions"),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
    }

    #[test]
    fn writes_models_and_settings_and_clears_override() {
        let dir = tempfile::tempdir().unwrap();
        let res = apply_workbuddy_in_dir(
            dir.path(),
            "https://api.deepseek.com/v1",
            "sk-demo-key",
            Some("deepseek-chat"),
        )
        .unwrap();
        assert!(res.applied);

        let models_raw = fs::read_to_string(dir.path().join("models.json")).unwrap();
        assert!(models_raw.contains("https://api.deepseek.com/v1/chat/completions"));
        assert!(models_raw.contains("deepseek-chat"));
        assert!(models_raw.contains("sk-demo-key"));

        let settings_raw = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(settings_raw.contains("\"CODEBUDDY_API_KEY\": \"sk-demo-key\""));
        assert!(settings_raw.contains("\"CODEBUDDY_MODEL\": \"deepseek-chat\""));

        assert!(clear_workbuddy_provider_override_in_dir(dir.path()).unwrap());
        let cleared = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(!cleared.contains("CODEBUDDY_API_KEY"));
    }
}
