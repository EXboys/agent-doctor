use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_yaml::{Mapping, Value};

use crate::adapter::{
    AdapterDiscovery, ApplyReport, RuntimeAdapter, RuntimeModelPreset, RuntimeModelState,
    RuntimeProfile,
};
use crate::adapters::util::{discover_binary, home_join};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HermesApiKeyStatus {
    pub env_var: String,
    pub configured: bool,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HermesSettings {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key_env: Option<String>,
    pub api_key_configured: bool,
    pub api_key_hint: Option<String>,
}

pub struct HermesAdapter;

impl HermesAdapter {
    fn config_path() -> PathBuf {
        home_join(".hermes/config.yaml")
    }

    fn secrets_path() -> PathBuf {
        home_join(".hermes/.env")
    }

    pub fn provider_api_key_env(provider: &str) -> Option<String> {
        let provider = provider.trim().to_lowercase();
        let known = match provider.as_str() {
            // Hermes resolves bare `provider: custom` as an OpenAI-compatible
            // endpoint and reads OPENAI_API_KEY at runtime.
            "custom" => Some("OPENAI_API_KEY"),
            "deepseek" => Some("DEEPSEEK_API_KEY"),
            "openai" => Some("OPENAI_API_KEY"),
            "anthropic" => Some("ANTHROPIC_API_KEY"),
            "openrouter" => Some("OPENROUTER_API_KEY"),
            "google" | "gemini" => Some("GOOGLE_API_KEY"),
            "groq" => Some("GROQ_API_KEY"),
            "together" => Some("TOGETHER_API_KEY"),
            "xai" | "grok" => Some("XAI_API_KEY"),
            "mistral" => Some("MISTRAL_API_KEY"),
            "ollama" | "local" => None,
            _ => None,
        };
        known.map(str::to_string).or_else(|| {
            if provider.is_empty() {
                None
            } else {
                Some(format!("{}_API_KEY", provider.to_uppercase()))
            }
        })
    }

    pub(crate) fn effective_provider(provider: &str, base_url: &str) -> String {
        let configured = provider.trim().to_lowercase();
        if !configured.is_empty() {
            return configured;
        }
        let url = base_url.trim().to_lowercase();
        if url.contains("api.deepseek.com") {
            return "deepseek".to_string();
        }
        if url.contains("api.openai.com") {
            return "openai".to_string();
        }
        if url.contains("openrouter.ai") {
            return "openrouter".to_string();
        }
        if url.contains("generativelanguage.googleapis.com") {
            return "google".to_string();
        }
        if url.contains("localhost:11434") || url.contains("127.0.0.1:11434") {
            return "ollama".to_string();
        }
        configured
    }

    fn mask_api_key(value: &str) -> String {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        if trimmed.len() <= 8 {
            return "****".to_string();
        }
        format!(
            "{}****{}",
            &trimmed[..3.min(trimmed.len())],
            &trimmed[trimmed.len().saturating_sub(4)..]
        )
    }

    fn read_env_value(path: &Path, key: &str) -> Result<Option<String>> {
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            if name.trim() == key {
                return Ok(Some(
                    value
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string(),
                ));
            }
        }
        Ok(None)
    }

    fn write_env_value(path: &Path, key: &str, value: &str) -> Result<()> {
        if path.exists() {
            let original = fs::read_to_string(path)?;
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("system time before UNIX epoch")?
                .as_secs();
            let backup_path = path.with_extension(format!("env.bak.{ts}"));
            fs::write(&backup_path, original)?;
        } else if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut lines: Vec<String> = if path.exists() {
            fs::read_to_string(path)?
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

        fs::write(path, format!("{}\n", lines.join("\n")))?;
        Ok(())
    }

    pub fn read_api_key_status(provider: &str) -> Result<Option<HermesApiKeyStatus>> {
        let Some(env_var) = Self::provider_api_key_env(provider) else {
            return Ok(None);
        };
        let path = Self::secrets_path();
        let value = Self::read_env_value(&path, &env_var)?;
        let configured = value.as_ref().is_some_and(|entry| !entry.trim().is_empty());
        Ok(Some(HermesApiKeyStatus {
            env_var,
            configured,
            hint: value
                .filter(|entry| !entry.trim().is_empty())
                .map(|entry| Self::mask_api_key(&entry)),
        }))
    }

    pub fn apply_api_key(provider: &str, api_key: &str) -> Result<()> {
        let env_var = Self::provider_api_key_env(provider)
            .with_context(|| format!("provider '{provider}' does not use an API key"))?;
        if api_key.trim().is_empty() {
            anyhow::bail!("API key cannot be empty");
        }
        Self::write_env_value(&Self::secrets_path(), &env_var, api_key.trim())
    }

    pub fn read_settings(&self) -> Result<HermesSettings> {
        let model = self.read_model()?.unwrap_or(RuntimeModelState {
            provider: None,
            model: None,
            base_url: None,
        });
        let base_url = model.base_url.unwrap_or_default();
        let provider = Self::effective_provider(&model.provider.unwrap_or_default(), &base_url);
        let api_key = Self::read_api_key_status(&provider)?;
        Ok(HermesSettings {
            provider: provider.clone(),
            model: model.model.unwrap_or_default(),
            base_url,
            api_key_env: api_key.as_ref().map(|status| status.env_var.clone()),
            api_key_configured: api_key.as_ref().is_some_and(|status| status.configured),
            api_key_hint: api_key.and_then(|status| status.hint),
        })
    }

    fn load_config_value(path: &PathBuf) -> Result<Value> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_yaml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn apply_settings(
        &self,
        preset: &RuntimeModelPreset,
        api_key: Option<&str>,
    ) -> Result<ApplyReport> {
        let path = Self::config_path();
        if !path.exists() {
            anyhow::bail!(
                "Hermes config not found at {} — run Hermes once or create the file first",
                path.display()
            );
        }

        let raw = fs::read_to_string(&path)?;
        let mut root = Self::load_config_value(&path)?;
        let model = root
            .as_mapping_mut()
            .context("Hermes config root must be a mapping")?;

        let model_map = model
            .entry(Value::from("model"))
            .or_insert_with(|| Value::Mapping(Mapping::new()));
        let model_map = model_map
            .as_mapping_mut()
            .context("Hermes config model section must be a mapping")?;

        model_map.insert(
            Value::from("provider"),
            Value::from(preset.provider.as_str()),
        );
        model_map.insert(Value::from("default"), Value::from(preset.model.as_str()));
        model_map.insert(
            Value::from("base_url"),
            Value::from(preset.base_url.as_str()),
        );

        let backup_path = backup_config(&path, &raw)?;
        let updated = serde_yaml::to_string(&root)?;
        fs::write(&path, updated)?;

        let wrote_key = api_key.is_some_and(|value| !value.trim().is_empty());
        if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
            Self::apply_api_key(&preset.provider, api_key)?;
        }

        let mut restart_hint =
            "Restart Hermes or start a new session for model changes to take effect.".to_string();
        if wrote_key {
            restart_hint.push_str(" API key saved to ~/.hermes/.env.");
        }

        Ok(ApplyReport {
            runtime_id: self.id().to_string(),
            config_path: path.display().to_string(),
            backup_path: Some(backup_path.display().to_string()),
            restart_hint,
        })
    }
}

impl RuntimeAdapter for HermesAdapter {
    fn id(&self) -> &'static str {
        "hermes"
    }

    fn display_name(&self) -> &'static str {
        "Hermes"
    }

    fn discover(&self) -> AdapterDiscovery {
        discover_binary("hermes")
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        vec![Self::config_path()]
    }

    fn optional_config_paths(&self) -> Vec<PathBuf> {
        // Created by key scaffold / wiring. Missing is not a defect on its own.
        vec![Self::secrets_path()]
    }

    fn read_profile(&self) -> Result<RuntimeProfile> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(RuntimeProfile {
                gateway_url: None,
                key_source: None,
            });
        }

        let model = self.read_model()?.unwrap_or(RuntimeModelState {
            provider: None,
            model: None,
            base_url: None,
        });

        let secrets = Self::secrets_path();
        let key_source = if secrets.exists() {
            secrets.display().to_string()
        } else {
            path.display().to_string()
        };

        Ok(RuntimeProfile {
            gateway_url: model.base_url,
            key_source: Some(key_source),
        })
    }

    fn read_model(&self) -> Result<Option<RuntimeModelState>> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(None);
        }

        let value = Self::load_config_value(&path)?;
        let model = value.get("model");
        Ok(Some(RuntimeModelState {
            provider: model
                .and_then(|m| m.get("provider"))
                .and_then(Value::as_str)
                .map(str::to_string),
            model: model
                .and_then(|m| m.get("default"))
                .and_then(Value::as_str)
                .map(str::to_string),
            base_url: model
                .and_then(|m| m.get("base_url"))
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
                .map(str::to_string),
        }))
    }

    fn apply_model(&self, preset: &RuntimeModelPreset) -> Result<ApplyReport> {
        self.apply_settings(preset, None)
    }
}

fn backup_config(path: &Path, original: &str) -> Result<PathBuf> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time before UNIX epoch")?
        .as_secs();
    let backup_path = path.with_extension(format!("yaml.bak.{ts}"));
    fs::write(&backup_path, original)?;
    Ok(backup_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn preserves_explicit_custom_and_infers_missing_provider() {
        assert_eq!(
            HermesAdapter::effective_provider("custom", "https://api.deepseek.com/v1"),
            "custom"
        );
        assert_eq!(
            HermesAdapter::effective_provider("", "https://api.deepseek.com/v1"),
            "deepseek"
        );
        assert_eq!(
            HermesAdapter::effective_provider("", "http://127.0.0.1:11434/v1"),
            "ollama"
        );
        assert_eq!(
            HermesAdapter::effective_provider("anthropic", "https://api.deepseek.com/v1"),
            "anthropic"
        );
        assert_eq!(
            HermesAdapter::provider_api_key_env("custom").as_deref(),
            Some("OPENAI_API_KEY")
        );
    }

    #[test]
    fn apply_model_updates_hermes_yaml() {
        let temp = TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.yaml");
        fs::write(
            &config_path,
            "model:\n  default: old-model\n  provider: old\n  base_url: https://old.example/v1\n",
        )
        .unwrap();

        let preset = RuntimeModelPreset {
            provider: "deepseek".to_string(),
            model: "deepseek-v4-flash".to_string(),
            base_url: "https://api.deepseek.com/v1".to_string(),
        };

        let mut root: Value =
            serde_yaml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        let model = root.as_mapping_mut().unwrap();
        let model_map = model
            .entry(Value::from("model"))
            .or_insert_with(|| Value::Mapping(Mapping::new()));
        let model_map = model_map.as_mapping_mut().unwrap();
        model_map.insert(
            Value::from("provider"),
            Value::from(preset.provider.as_str()),
        );
        model_map.insert(Value::from("default"), Value::from(preset.model.as_str()));
        model_map.insert(
            Value::from("base_url"),
            Value::from(preset.base_url.as_str()),
        );
        fs::write(&config_path, serde_yaml::to_string(&root).unwrap()).unwrap();

        let updated = fs::read_to_string(&config_path).unwrap();
        assert!(updated.contains("deepseek-v4-flash"));
        assert!(updated.contains("https://api.deepseek.com/v1"));
    }

    #[test]
    fn writes_and_reads_api_key_env_lines() {
        let temp = TempDir::new().expect("tempdir");
        let env_path = temp.path().join(".env");
        HermesAdapter::write_env_value(&env_path, "DEEPSEEK_API_KEY", "sk-test-secret-key")
            .unwrap();
        let value = HermesAdapter::read_env_value(&env_path, "DEEPSEEK_API_KEY").unwrap();
        assert_eq!(value.as_deref(), Some("sk-test-secret-key"));
    }

    #[test]
    fn env_file_is_optional_not_required() {
        let required = HermesAdapter.config_paths();
        let optional = HermesAdapter.optional_config_paths();
        assert!(required
            .iter()
            .all(|path| { path.file_name().and_then(|name| name.to_str()) != Some(".env") }));
        assert!(optional
            .iter()
            .any(|path| { path.file_name().and_then(|name| name.to_str()) == Some(".env") }));
    }
}
