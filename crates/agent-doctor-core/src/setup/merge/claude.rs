use std::fs;

use anyhow::{Context, Result as AnyhowResult};
use serde_json::json;

use crate::adapters::util::home_join;
use crate::setup::{backup_file, ensure_parent, RuntimeSetupResult};

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
