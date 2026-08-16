use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_yaml::Value;

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{discover_binary, home_join};

pub const DEEPSEEK_HARNESS_RUNTIME_ID: &str = "deepseek-harness";
pub const DEEPSEEK_HARNESS_CLI: &str = "dsh";
pub const DEEPSEEK_HARNESS_VERSION: &str = "0.1.0-rc.6";
pub const DEEPSEEK_HARNESS_NPM_PACKAGE: &str = "@deepseek-ai/dsh";
pub const DEEPSEEK_BASE_URL_ENV: &str = "DEEPSEEK_BASE_URL";
pub const DEEPSEEK_API_KEY_ENV: &str = "DEEPSEEK_API_KEY";

pub struct DeepSeekHarnessAdapter;

impl DeepSeekHarnessAdapter {
    pub fn home() -> PathBuf {
        std::env::var_os("DSH_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home_join(".dsh"))
    }

    pub fn settings_path() -> PathBuf {
        Self::home().join("settings.yaml")
    }

    pub fn credentials_path() -> PathBuf {
        Self::home().join(".credentials.yaml")
    }

    pub fn cordis_patch_path() -> PathBuf {
        Self::home().join("cordis.patch.yml")
    }

    pub fn env_path() -> PathBuf {
        Self::home().join(".env")
    }

    fn yaml_value(path: &Path, key: &str) -> Result<Option<String>> {
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let value: Value = serde_yaml::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(find_yaml_scalar(&value, key)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string))
    }

    fn env_file_has_key(path: &Path, key: &str) -> Result<bool> {
        if !path.exists() {
            return Ok(false);
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Ok(raw.lines().any(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return false;
            }
            let assignment = line.strip_prefix("export ").unwrap_or(line);
            assignment.split_once('=').is_some_and(|(name, value)| {
                name.trim() == key && !value.trim().trim_matches('"').trim_matches('\'').is_empty()
            })
        }))
    }

    fn yaml_paths() -> [PathBuf; 3] {
        [
            Self::settings_path(),
            Self::credentials_path(),
            Self::cordis_patch_path(),
        ]
    }
}

impl RuntimeAdapter for DeepSeekHarnessAdapter {
    fn id(&self) -> &'static str {
        DEEPSEEK_HARNESS_RUNTIME_ID
    }

    fn display_name(&self) -> &'static str {
        "DeepSeek Harness"
    }

    fn discover(&self) -> AdapterDiscovery {
        discover_binary(DEEPSEEK_HARNESS_CLI)
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        Self::yaml_paths().into_iter().collect()
    }

    fn config_paths_required(&self) -> bool {
        false
    }

    fn read_profile(&self) -> Result<RuntimeProfile> {
        let gateway_url = std::env::var(DEEPSEEK_BASE_URL_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                Self::yaml_paths().into_iter().find_map(|path| {
                    Self::yaml_value(&path, DEEPSEEK_BASE_URL_ENV)
                        .ok()
                        .flatten()
                })
            });

        let key_source = if std::env::var(DEEPSEEK_API_KEY_ENV)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
        {
            Some(format!("process environment:{DEEPSEEK_API_KEY_ENV}"))
        } else if Self::env_file_has_key(&Self::env_path(), DEEPSEEK_API_KEY_ENV)? {
            Some(Self::env_path().display().to_string())
        } else {
            Self::yaml_paths().into_iter().find_map(|path| {
                Self::yaml_value(&path, DEEPSEEK_API_KEY_ENV)
                    .ok()
                    .flatten()
                    .map(|_| path.display().to_string())
            })
        };

        Ok(RuntimeProfile {
            gateway_url,
            key_source,
        })
    }
}

fn find_yaml_scalar<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    match value {
        Value::Mapping(mapping) => {
            for (entry_key, entry_value) in mapping {
                if entry_key.as_str() == Some(key) {
                    return entry_value.as_str();
                }
                if let Some(found) = find_yaml_scalar(entry_value, key) {
                    return Some(found);
                }
            }
            None
        }
        Value::Sequence(sequence) => sequence
            .iter()
            .find_map(|entry| find_yaml_scalar(entry, key)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nested_simple_yaml_values() {
        let value: Value = serde_yaml::from_str(
            "provider:\n  env:\n    DEEPSEEK_BASE_URL: https://api.deepseek.com\n",
        )
        .unwrap();
        assert_eq!(
            find_yaml_scalar(&value, DEEPSEEK_BASE_URL_ENV),
            Some("https://api.deepseek.com")
        );
    }

    #[test]
    fn version_and_package_are_pinned() {
        assert_eq!(DEEPSEEK_HARNESS_VERSION, "0.1.0-rc.6");
        assert_eq!(DEEPSEEK_HARNESS_NPM_PACKAGE, "@deepseek-ai/dsh");
        assert!(!DeepSeekHarnessAdapter.config_paths_required());
    }
}
