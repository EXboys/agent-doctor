//! Write the saved provider into Pi without touching its other providers or logins.
//!
//! Pi reads `~/.pi/agent/models.json` (provider + model list), `auth.json`
//! (the key), and `settings.json` (`defaultProvider` / `defaultModel`).
//! The key is stored in `auth.json`, which Pi checks before `models.json`,
//! and copied onto the provider entry so an older Pi that only reads
//! `models.json` still connects.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::adapters::pi_agent_dir;
use crate::setup::{backup_file, ensure_parent, RuntimeSetupResult, PROTOCOL_ANTHROPIC};

pub const PI_PROVIDER_ID: &str = "agent-doctor";

pub fn pi_api_kind(protocol: &str) -> &'static str {
    if protocol.eq_ignore_ascii_case(PROTOCOL_ANTHROPIC) {
        "anthropic-messages"
    } else {
        "openai-completions"
    }
}

/// Base URL Pi's provider entry expects. OpenAI chat completions lose the
/// `/chat/completions` suffix; Anthropic Messages keeps the gateway root.
pub fn pi_base_url(gateway_url: &str, protocol: &str) -> String {
    if protocol.eq_ignore_ascii_case(PROTOCOL_ANTHROPIC) {
        return gateway_url.trim().trim_end_matches('/').to_string();
    }
    let base = super::hermes_chat_base_url(gateway_url);
    let trimmed = base.trim().trim_end_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with("/chat/completions") {
        trimmed[..trimmed.len() - "/chat/completions".len()]
            .trim_end_matches('/')
            .to_string()
    } else {
        trimmed.to_string()
    }
}

/// True when this computer already has the Agent Doctor provider written for Pi.
/// Chat then names that provider. A Pi that only has its own login stays on that login.
pub fn pi_saved_provider_ready() -> bool {
    let path = pi_agent_dir().join("models.json");
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    value
        .pointer(&format!("/providers/{PI_PROVIDER_ID}/baseUrl"))
        .and_then(Value::as_str)
        .is_some_and(|url| {
            let url = url.trim();
            url.starts_with("http://") || url.starts_with("https://")
        })
}

pub fn apply_pi_with_model(
    gateway_url: &str,
    api_key: &str,
    protocol: &str,
    model: Option<&str>,
) -> Result<RuntimeSetupResult> {
    apply_pi_in_dir(&pi_agent_dir(), gateway_url, api_key, protocol, model)
}

pub fn apply_pi_in_dir(
    dir: &Path,
    gateway_url: &str,
    api_key: &str,
    protocol: &str,
    model: Option<&str>,
) -> Result<RuntimeSetupResult> {
    let url = pi_base_url(gateway_url, protocol);
    let key = api_key.trim();
    if url.is_empty() || key.is_empty() {
        return Ok(RuntimeSetupResult {
            runtime_id: "pi".into(),
            display_name: "Pi".into(),
            applied: false,
            message: "skipped — gateway URL or API key is empty".into(),
            ..Default::default()
        });
    }
    let model_id = model
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(super::COMPANY_DEFAULT_MODEL);
    let api = pi_api_kind(protocol);

    let models_path = dir.join("models.json");
    let auth_path = dir.join("auth.json");
    let settings_path = dir.join("settings.json");
    ensure_parent(&models_path)?;
    let models_backup = backup_file(&models_path)?;
    let _auth_backup = backup_file(&auth_path)?;
    let _settings_backup = backup_file(&settings_path)?;

    let mut models = read_object(&models_path)?;
    let providers = object_entry(&mut models, "providers");
    providers.insert(
        PI_PROVIDER_ID.to_string(),
        json!({
            "baseUrl": url,
            "api": api,
            "apiKey": key,
            "models": [{ "id": model_id, "name": model_id }]
        }),
    );
    write_json(&models_path, &Value::Object(models))?;

    let mut auth = read_object(&auth_path)?;
    auth.insert(
        PI_PROVIDER_ID.to_string(),
        json!({ "type": "api_key", "key": key }),
    );
    write_json(&auth_path, &Value::Object(auth))?;

    let mut settings = read_object(&settings_path)?;
    settings.insert(
        "defaultProvider".into(),
        Value::String(PI_PROVIDER_ID.to_string()),
    );
    settings.insert("defaultModel".into(), Value::String(model_id.to_string()));
    write_json(&settings_path, &Value::Object(settings))?;

    Ok(RuntimeSetupResult {
        runtime_id: "pi".into(),
        display_name: "Pi".into(),
        applied: true,
        config_path: Some(models_path.display().to_string()),
        backup_path: models_backup.map(|path| path.display().to_string()),
        message: format!(
            "set Pi provider {PI_PROVIDER_ID} baseUrl={url} model={model_id} api={api}"
        ),
        ..Default::default()
    })
}

fn read_object(path: &Path) -> Result<serde_json::Map<String, Value>> {
    if !path.exists() {
        return Ok(serde_json::Map::new());
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let value: Value =
        serde_json::from_str(&raw).with_context(|| format!("{} is not JSON", path.display()))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => anyhow::bail!("{} must be a JSON object", path.display()),
    }
}

fn object_entry<'a>(
    root: &'a mut serde_json::Map<String, Value>,
    key: &str,
) -> &'a mut serde_json::Map<String, Value> {
    let entry = root.entry(key.to_string()).or_insert_with(|| json!({}));
    if !entry.is_object() {
        *entry = json!({});
    }
    entry.as_object_mut().expect("object")
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::util::with_test_home;

    #[test]
    fn openai_url_drops_chat_completions_suffix() {
        assert_eq!(
            pi_base_url(
                "https://example.test/v1/chat/completions",
                crate::setup::PROTOCOL_OPENAI,
            ),
            "https://example.test/v1"
        );
        assert_eq!(
            pi_api_kind(crate::setup::PROTOCOL_OPENAI),
            "openai-completions"
        );
    }

    #[test]
    fn anthropic_keeps_the_gateway_root() {
        assert_eq!(
            pi_base_url("https://example.test/anthropic/", PROTOCOL_ANTHROPIC),
            "https://example.test/anthropic"
        );
        assert_eq!(pi_api_kind(PROTOCOL_ANTHROPIC), "anthropic-messages");
    }

    #[test]
    fn merge_keeps_other_providers_and_logins() {
        let home = tempfile::tempdir().unwrap();
        with_test_home(home.path(), || {
            let dir = pi_agent_dir();
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("models.json"),
                r#"{"providers":{"openai":{"baseUrl":"https://api.openai.com/v1","api":"openai-completions","models":[{"id":"gpt-4o"}]}},"themeHint":"keep"}"#,
            )
            .unwrap();
            fs::write(
                dir.join("auth.json"),
                r#"{"openai":{"type":"oauth","access":"keep-me"}}"#,
            )
            .unwrap();
            fs::write(
                dir.join("settings.json"),
                r#"{"theme":"dark","defaultProvider":"openai"}"#,
            )
            .unwrap();

            let result = apply_pi_in_dir(
                &dir,
                "https://gateway.example/v1/chat/completions",
                "sk-test",
                crate::setup::PROTOCOL_OPENAI,
                Some("deepseek-v4-flash"),
            )
            .unwrap();
            assert!(result.applied);

            let models: Value =
                serde_json::from_str(&fs::read_to_string(dir.join("models.json")).unwrap())
                    .unwrap();
            assert_eq!(models["themeHint"], "keep");
            assert_eq!(
                models["providers"]["openai"]["baseUrl"],
                "https://api.openai.com/v1"
            );
            assert_eq!(
                models["providers"][PI_PROVIDER_ID]["baseUrl"],
                "https://gateway.example/v1"
            );
            assert_eq!(
                models["providers"][PI_PROVIDER_ID]["api"],
                "openai-completions"
            );
            assert_eq!(models["providers"][PI_PROVIDER_ID]["apiKey"], "sk-test");

            let auth: Value =
                serde_json::from_str(&fs::read_to_string(dir.join("auth.json")).unwrap()).unwrap();
            assert_eq!(auth["openai"]["access"], "keep-me");
            assert_eq!(auth[PI_PROVIDER_ID]["type"], "api_key");
            assert_eq!(auth[PI_PROVIDER_ID]["key"], "sk-test");

            let settings: Value =
                serde_json::from_str(&fs::read_to_string(dir.join("settings.json")).unwrap())
                    .unwrap();
            assert_eq!(settings["theme"], "dark");
            assert_eq!(settings["defaultProvider"], PI_PROVIDER_ID);
            assert_eq!(settings["defaultModel"], "deepseek-v4-flash");
            assert!(pi_saved_provider_ready());
        });
    }

    #[test]
    fn empty_key_does_not_touch_files() {
        let home = tempfile::tempdir().unwrap();
        with_test_home(home.path(), || {
            let dir = pi_agent_dir();
            let result = apply_pi_in_dir(
                &dir,
                "https://example.test/v1",
                "  ",
                crate::setup::PROTOCOL_OPENAI,
                None,
            )
            .unwrap();
            assert!(!result.applied);
            assert!(!dir.join("models.json").exists());
            assert!(!pi_saved_provider_ready());
        });
    }
}
