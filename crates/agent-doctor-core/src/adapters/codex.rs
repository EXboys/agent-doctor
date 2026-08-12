use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use toml::Value;

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeModelState, RuntimeProfile};
use crate::adapters::util::{discover_binary, home_join};

pub struct CodexAdapter;

impl CodexAdapter {
    fn config_path() -> PathBuf {
        home_join(".codex/config.toml")
    }

    fn auth_path() -> PathBuf {
        home_join(".codex/auth.json")
    }

    fn load_config_value(path: &Path) -> Result<Value> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
    }

    fn provider_id(value: &Value) -> Option<String> {
        value
            .get("model_provider")
            .and_then(Value::as_str)
            .filter(|provider| !provider.is_empty())
            .map(str::to_string)
    }

    fn provider_table<'a>(value: &'a Value, provider: &str) -> Option<&'a Value> {
        value.get("model_providers")?.get(provider)
    }

    fn provider_base_url(value: &Value, provider: &str) -> Option<String> {
        Self::provider_table(value, provider)
            .and_then(|table| table.get("base_url"))
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .map(str::to_string)
            .or_else(|| {
                value
                    .get("openai_base_url")
                    .and_then(Value::as_str)
                    .filter(|url| !url.is_empty())
                    .map(str::to_string)
            })
    }

    fn provider_env_key(value: &Value, provider: &str) -> Option<String> {
        Self::provider_table(value, provider)?
            .get("env_key")
            .and_then(Value::as_str)
            .filter(|key| !key.is_empty())
            .map(str::to_string)
    }

    fn read_model_from_value(value: &Value) -> RuntimeModelState {
        let provider = Self::provider_id(value);
        let base_url = provider
            .as_deref()
            .and_then(|provider| Self::provider_base_url(value, provider));

        RuntimeModelState {
            provider,
            model: value
                .get("model")
                .and_then(Value::as_str)
                .filter(|model| !model.is_empty())
                .map(str::to_string),
            base_url,
        }
    }

    fn key_source(value: &Value, provider: Option<&str>, auth_path: &Path) -> Option<String> {
        provider
            .and_then(|provider| Self::provider_env_key(value, provider))
            .or_else(|| {
                if auth_path.exists() {
                    Some(auth_path.display().to_string())
                } else {
                    None
                }
            })
    }
}

impl RuntimeAdapter for CodexAdapter {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn display_name(&self) -> &'static str {
        "Codex CLI"
    }

    fn discover(&self) -> AdapterDiscovery {
        discover_binary("codex")
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        vec![Self::config_path(), Self::auth_path()]
    }

    fn read_profile(&self) -> Result<RuntimeProfile> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(RuntimeProfile {
                gateway_url: None,
                key_source: None,
            });
        }

        let value = Self::load_config_value(&path)?;
        let model = Self::read_model_from_value(&value);
        let key_source = Self::key_source(&value, model.provider.as_deref(), &Self::auth_path())
            .or_else(|| Some(path.display().to_string()));

        Ok(RuntimeProfile {
            gateway_url: model.base_url,
            key_source,
        })
    }

    fn read_model(&self) -> Result<Option<RuntimeModelState>> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(None);
        }

        let value = Self::load_config_value(&path)?;
        Ok(Some(Self::read_model_from_value(&value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_model_provider_and_base_url() {
        let value: Value = toml::from_str(
            r#"
model = "gpt-5-codex"
model_provider = "company"

[model_providers.company]
name = "Company Gateway"
base_url = "https://gateway.example/v1"
env_key = "COMPANY_API_KEY"
"#,
        )
        .expect("parse codex config");

        let model = CodexAdapter::read_model_from_value(&value);
        assert_eq!(model.provider.as_deref(), Some("company"));
        assert_eq!(model.model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(
            model.base_url.as_deref(),
            Some("https://gateway.example/v1")
        );
        assert_eq!(
            CodexAdapter::key_source(
                &value,
                model.provider.as_deref(),
                Path::new("/tmp/auth.json")
            )
            .as_deref(),
            Some("COMPANY_API_KEY")
        );
    }
}
