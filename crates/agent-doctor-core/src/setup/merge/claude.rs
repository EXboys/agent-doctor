use std::fs;

use anyhow::{Context, Result as AnyhowResult};
use serde_json::json;

use crate::adapters::util::home_join;
use crate::setup::{backup_file, ensure_parent, RuntimeSetupResult};

const CLAUDE_MODEL_KEYS: &[&str] = &[
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
];

/// Settings files Claude reads for this chat: the user file and the project file.
pub fn claude_subscription_settings_paths(cwd: &std::path::Path) -> Vec<std::path::PathBuf> {
    vec![
        home_join(".claude/settings.json"),
        cwd.join(".claude").join("settings.json"),
        cwd.join(".claude").join("settings.local.json"),
    ]
}

/// The user folder and every project this app already has.
pub fn known_claude_subscription_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = vec![home_join("")];
    if let Ok(doc) = crate::workspace::load_workspaces() {
        dirs.extend(doc.workspaces.values().map(|entry| entry.path.clone()));
    }
    dirs
}

/// Make a Claude settings file safe for a Claude login.
///
/// A model Claude would refuse is replaced with `chosen` when that id is a
/// Claude model, otherwise removed. The main model is set to `chosen` when
/// the chat named one. A saved gateway address and key are removed. A file
/// that cannot be read is left unchanged.
pub fn repair_claude_subscription_file(
    path: &std::path::Path,
    chosen: Option<&str>,
    allowed: &[String],
) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut root = serde_json::from_str::<serde_json::Value>(&raw).with_context(|| {
        format!(
            "Claude settings could not be read, so this turn was not sent: {}",
            path.display()
        )
    })?;
    let chosen = chosen
        .map(str::trim)
        .filter(|id| crate::setup::account_model_allowed(allowed, id));
    let enforce_models = !allowed.is_empty();
    let mut changed = false;
    if let Some(object) = root.as_object_mut() {
        if object.remove("anthropicBaseUrl").is_some() {
            changed = true;
        }
    }
    let Some(env) = root
        .as_object_mut()
        .and_then(|object| object.get_mut("env"))
        .and_then(|value| value.as_object_mut())
    else {
        if !changed {
            return Ok(false);
        }
        fs::write(path, serde_json::to_string_pretty(&root)?)
            .with_context(|| format!("failed to write {}", path.display()))?;
        return Ok(true);
    };
    for key in [
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
    ] {
        if env.remove(key).is_some() {
            changed = true;
        }
    }
    if enforce_models {
        if let Some(chosen) = chosen {
            let current = env
                .get("ANTHROPIC_MODEL")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if current != chosen {
                env.insert("ANTHROPIC_MODEL".to_string(), json!(chosen));
                changed = true;
            }
        }
    }
    let stale: Vec<String> = env
        .iter()
        .filter_map(|(key, value)| {
            if !enforce_models || !CLAUDE_MODEL_KEYS.contains(&key.as_str()) {
                return None;
            }
            let model = value.as_str().unwrap_or("");
            if model.is_empty() || crate::setup::account_model_allowed(allowed, model) {
                None
            } else {
                Some(key.clone())
            }
        })
        .collect();
    for key in stale {
        match chosen {
            Some(id) => {
                env.insert(key, json!(id));
            }
            None => {
                env.remove(&key);
            }
        }
        changed = true;
    }
    if !changed {
        return Ok(false);
    }
    fs::write(path, serde_json::to_string_pretty(&root)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

pub fn repair_claude_subscription_configs(
    cwd: &std::path::Path,
    chosen: Option<&str>,
    allowed: &[String],
) -> AnyhowResult<()> {
    for path in claude_subscription_settings_paths(cwd) {
        repair_claude_subscription_file(&path, chosen, allowed)?;
    }
    Ok(())
}

pub fn apply_claude_code(gateway_url: &str, api_key: &str) -> AnyhowResult<RuntimeSetupResult> {
    apply_claude_code_with_model(gateway_url, api_key, None)
}

pub fn apply_claude_code_with_model(
    gateway_url: &str,
    api_key: &str,
    model: Option<&str>,
) -> AnyhowResult<RuntimeSetupResult> {
    let path = home_join(".claude/settings.json");
    let backup_path = backup_file(&path)?;
    ensure_parent(&path)?;

    let mut root = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    let env = root
        .as_object_mut()
        .context("Claude settings root must be an object")?
        .entry("env")
        .or_insert_with(|| json!({}));
    if let Some(env_obj) = env.as_object_mut() {
        env_obj.insert("ANTHROPIC_BASE_URL".to_string(), json!(gateway_url));
        // Claude Code sends ANTHROPIC_AUTH_TOKEN when both are set. A leftover
        // token (GLM's own installer writes one) ignores the key just saved
        // and comes back as 401「令牌已过期或验证不正确」.
        env_obj.insert("ANTHROPIC_API_KEY".to_string(), json!(api_key));
        env_obj.insert("ANTHROPIC_AUTH_TOKEN".to_string(), json!(api_key));
        if let Some(model_id) = model.map(str::trim).filter(|m| !m.is_empty()) {
            env_obj.insert("ANTHROPIC_MODEL".to_string(), json!(model_id));
            env_obj.insert(
                "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
                json!(model_id),
            );
            env_obj.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(), json!(model_id));
            env_obj.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(), json!(model_id));
            env_obj.insert("CLAUDE_CODE_SUBAGENT_MODEL".to_string(), json!(model_id));
        }
    }
    root.as_object_mut()
        .expect("object")
        .insert("anthropicBaseUrl".to_string(), json!(gateway_url));

    fs::write(&path, serde_json::to_string_pretty(&root)?)?;

    Ok(RuntimeSetupResult {
        runtime_id: "claude-code".to_string(),
        display_name: "Claude Code".to_string(),
        applied: true,
        config_path: Some(path.display().to_string()),
        backup_path: backup_path.map(|p| p.display().to_string()),
        message: format!(
            "set env.ANTHROPIC_BASE_URL to {gateway_url} (Anthropic Messages path) and API key"
        ),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_keeps_a_claude_model_and_send_replaces_a_foreign_one() {
        let allowed = vec![
            "claude-sonnet-5".to_string(),
            "claude-haiku-4-5".to_string(),
        ];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{"anthropicBaseUrl":"https://example.test","env":{"ANTHROPIC_BASE_URL":"https://example.test","ANTHROPIC_API_KEY":"secret","ANTHROPIC_MODEL":"glm-5.3","ANTHROPIC_DEFAULT_HAIKU_MODEL":"claude-haiku-4-5","OTHER":"keep"}}"#,
        )
        .unwrap();
        assert!(repair_claude_subscription_file(&path, None, &allowed).unwrap());
        let cleared = fs::read_to_string(&path).unwrap();
        assert!(!cleared.contains("glm-5.3"));
        assert!(!cleared.contains("example.test"));
        assert!(!cleared.contains("secret"));
        assert!(cleared.contains("claude-haiku-4-5"));
        assert!(cleared.contains("keep"));

        fs::write(&path, r#"{"env":{"ANTHROPIC_MODEL":"gpt-5.6-sol"}}"#).unwrap();
        assert!(repair_claude_subscription_file(&path, Some("claude-sonnet-5"), &allowed).unwrap());
        let repaired = fs::read_to_string(&path).unwrap();
        assert!(repaired.contains("claude-sonnet-5"));
        assert!(!repaired.contains("gpt-5.6-sol"));

        fs::write(&path, "{").unwrap();
        assert!(repair_claude_subscription_file(&path, Some("claude-sonnet-5"), &allowed).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{");
    }
}
