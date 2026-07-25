use std::fs;

use anyhow::{Context, Result as AnyhowResult};
use serde_json::json;
use serde_yaml::{Mapping, Value as YamlValue};
use toml::Value as TomlValue;

use crate::adapters::util::home_join;
use crate::adapters::HermesAdapter;
use crate::setup::{backup_file, ensure_parent, RuntimeSetupResult, COMPANY_API_KEY_ENV};

pub fn apply_openclaw(gateway_url: &str, _api_key: &str) -> AnyhowResult<RuntimeSetupResult> {
    let path = home_join(".openclaw/openclaw.json");
    let backup_path = backup_file(&path)?;
    ensure_parent(&path)?;

    let mut root = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    if let Some(obj) = root.as_object_mut() {
        let gateway = obj.entry("gateway").or_insert_with(|| json!({}));
        if let Some(gateway_obj) = gateway.as_object_mut() {
            gateway_obj.insert("url".to_string(), json!(gateway_url));
        }
        let evotown = obj.entry("evotown").or_insert_with(|| json!({}));
        if let Some(evotown_obj) = evotown.as_object_mut() {
            evotown_obj.insert("url".to_string(), json!(gateway_url));
        }
    }

    fs::write(&path, serde_json::to_string_pretty(&root)?)?;

    Ok(RuntimeSetupResult {
        runtime_id: "openclaw".to_string(),
        display_name: "OpenClaw".to_string(),
        applied: true,
        config_path: Some(path.display().to_string()),
        backup_path: backup_path.map(|p| p.display().to_string()),
        message: format!("set gateway.url to {gateway_url}"),
    })
}

pub fn apply_hermes(
    gateway_url: &str,
    api_key: &str,
    provider: &str,
) -> AnyhowResult<RuntimeSetupResult> {
    let path = home_join(".hermes/config.yaml");
    let backup_path = backup_file(&path)?;
    ensure_parent(&path)?;

    let mut root: YamlValue = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        serde_yaml::from_str(&raw).unwrap_or_else(|_| YamlValue::Mapping(Mapping::new()))
    } else {
        YamlValue::Mapping(Mapping::new())
    };

    {
        let model = root
            .as_mapping_mut()
            .context("Hermes config root must be a mapping")?
            .entry(YamlValue::from("model"))
            .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
        let model_map = model
            .as_mapping_mut()
            .context("Hermes model section must be a mapping")?;

        // Evotown gateway is OpenAI-compatible; Hermes calls that "custom"
        // (not "openai" — that slug is unknown to Hermes).
        let effective_provider =
            if provider.trim().is_empty() || provider.trim().eq_ignore_ascii_case("openai") {
                "custom"
            } else {
                provider.trim()
            };
        model_map.insert(
            YamlValue::from("provider"),
            YamlValue::from(effective_provider),
        );
        if !model_map.contains_key(YamlValue::from("default")) {
            model_map.insert(YamlValue::from("default"), YamlValue::from("gpt-4o-mini"));
        }
        model_map.insert(YamlValue::from("base_url"), YamlValue::from(gateway_url));
    }

    // Keep title generation on the same gateway (avoid auto → native provider 401).
    if let Some(root_map) = root.as_mapping_mut() {
        let aux = root_map
            .entry(YamlValue::from("auxiliary"))
            .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
        if let Some(aux_map) = aux.as_mapping_mut() {
            let title = aux_map
                .entry(YamlValue::from("title_generation"))
                .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
            if let Some(title_map) = title.as_mapping_mut() {
                title_map.insert(YamlValue::from("provider"), YamlValue::from("custom"));
                title_map.insert(YamlValue::from("base_url"), YamlValue::from(gateway_url));
            }
        }
    }

    let provider_name = root
        .get("model")
        .and_then(|model| model.get("provider"))
        .and_then(YamlValue::as_str)
        .unwrap_or("custom")
        .to_string();

    fs::write(&path, serde_yaml::to_string(&root)?)?;
    // Custom endpoints authenticate via OPENAI_API_KEY.
    let env_provider = if provider_name == "custom" {
        "openai"
    } else {
        provider_name.as_str()
    };
    HermesAdapter::apply_api_key(env_provider, api_key)?;

    Ok(RuntimeSetupResult {
        runtime_id: "hermes".to_string(),
        display_name: "Hermes Agent".to_string(),
        applied: true,
        config_path: Some(path.display().to_string()),
        backup_path: backup_path.map(|p| p.display().to_string()),
        message: format!(
            "set model.provider={provider_name}, base_url={gateway_url}, and updated ~/.hermes/.env"
        ),
    })
}

pub fn apply_claude_code(gateway_url: &str, api_key: &str) -> AnyhowResult<RuntimeSetupResult> {
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
        env_obj.insert("ANTHROPIC_API_KEY".to_string(), json!(api_key));
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
    })
}

pub fn apply_codex(gateway_url: &str, api_key: &str) -> AnyhowResult<RuntimeSetupResult> {
    let path = home_join(".codex/config.toml");
    let backup_path = backup_file(&path)?;
    ensure_parent(&path)?;

    let mut root: TomlValue = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        toml::from_str(&raw).unwrap_or(TomlValue::Table(toml::map::Map::new()))
    } else {
        TomlValue::Table(toml::map::Map::new())
    };

    let table = root
        .as_table_mut()
        .context("Codex config root must be a table")?;

    if !table.contains_key("model") {
        table.insert(
            "model".to_string(),
            TomlValue::String("gpt-4o-mini".to_string()),
        );
    }
    table.insert(
        "model_provider".to_string(),
        TomlValue::String("company".to_string()),
    );

    let mut company = toml::map::Map::new();
    company.insert(
        "name".to_string(),
        TomlValue::String("Company Gateway".to_string()),
    );
    company.insert(
        "base_url".to_string(),
        TomlValue::String(gateway_url.to_string()),
    );
    company.insert(
        "env_key".to_string(),
        TomlValue::String(COMPANY_API_KEY_ENV.to_string()),
    );

    let mut providers = toml::map::Map::new();
    providers.insert("company".to_string(), TomlValue::Table(company));
    table.insert("model_providers".to_string(), TomlValue::Table(providers));

    fs::write(&path, toml::to_string_pretty(&root)?)?;

    write_codex_auth_hint(api_key)?;

    Ok(RuntimeSetupResult {
        runtime_id: "codex".to_string(),
        display_name: "Codex CLI".to_string(),
        applied: true,
        config_path: Some(path.display().to_string()),
        backup_path: backup_path.map(|p| p.display().to_string()),
        message: format!(
            "set model_providers.company.base_url; export {COMPANY_API_KEY_ENV} or source profile.env"
        ),
    })
}

fn write_codex_auth_hint(api_key: &str) -> AnyhowResult<()> {
    let path = home_join(".codex/auth.json");
    if path.exists() {
        return Ok(());
    }
    ensure_parent(&path)?;
    let payload = json!({
        "auth_mode": "apikey",
        "note": "API key is stored in ~/.config/agent-doctor/profile.env — source it or set AGENT_DOCTOR_COMPANY_API_KEY",
        "placeholder": !api_key.is_empty()
    });
    fs::write(&path, serde_json::to_string_pretty(&payload)?)?;
    Ok(())
}
