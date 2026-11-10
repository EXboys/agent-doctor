use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_yaml::Value;

use crate::adapter::{
    AdapterDiscovery, ApplyReport, RuntimeAdapter, RuntimeModelPreset, RuntimeModelState,
    RuntimeProfile,
};
use crate::adapters::util::{discover_binary, home_join, yaml_has_tag};

pub const DEEPSEEK_HARNESS_RUNTIME_ID: &str = "deepseek-harness";
pub const DEEPSEEK_HARNESS_CLI: &str = "dsh";
pub const DEEPSEEK_HARNESS_VERSION: &str = "0.1.5-rc.3";
pub const DEEPSEEK_HARNESS_NPM_PACKAGE: &str = "@deepseek-ai/dsh";
/// Node floor used when the installed package's `engines.node` cannot be read.
/// dsh 0.1.5 uses `import.meta.main` (22.18+), `zlib.createZstdCompress`
/// (22.15+), and undici, which wants 22.19.
pub const DEEPSEEK_HARNESS_NODE_MIN: &str = "22.19.0";
pub const DEEPSEEK_BASE_URL_ENV: &str = "DEEPSEEK_BASE_URL";
pub const DEEPSEEK_API_KEY_ENV: &str = "DEEPSEEK_API_KEY";
/// The one provider route dsh's `llm-deepseek` plugin registers; every model
/// id — catalogued or custom — rides it.
pub const DEEPSEEK_OFFICIAL_PROVIDER_ROUTE: &str = "deepseek-official";
/// Endpoint dsh defaults to when neither the environment nor its settings
/// carry a base URL.
pub const DEEPSEEK_OFFICIAL_BASE_URL: &str = "https://api.deepseek.com";

/// cordis patch entries that pin the default model. `agent-default-model`
/// owns the agent-facing selection; the `acp` app layer pins its own model
/// for ACP sessions, so both are overridden together. The home patch file is
/// the last layer of the composed tree, so these entries win over the
/// shipped defaults.
const AGENT_DEFAULT_MODEL_PATCH_ID: &str = "agent-default-model";
const ACP_SESSION_PATCH_ID: &str = "acp";

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

    /// Read the home patch file the way dsh does: a top-level YAML array.
    ///
    /// Same contract as the MCP wiring's reader: tagged files must be edited
    /// by hand (a rewrite would silently drop their `!!js` expressions), an
    /// unknown shape is refused instead of clobbered, and a missing file is
    /// just an empty patch list.
    fn read_patch_list(path: &Path) -> Result<Vec<Value>> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if yaml_has_tag(&raw) {
            anyhow::bail!(
                "{} uses YAML tags (like !!js) that Agent Doctor cannot rewrite without dropping them; edit that file by hand",
                path.display()
            );
        }
        let root: Value = serde_yaml::from_str(&raw).with_context(|| {
            format!(
                "{} is not valid YAML; fix or remove that file by hand so Agent Doctor does not overwrite your edits",
                path.display()
            )
        })?;
        match root {
            Value::Sequence(sequence) => Ok(sequence),
            Value::Null => Ok(Vec::new()),
            _ => anyhow::bail!(
                "{} must be a top-level YAML array of patch entries",
                path.display()
            ),
        }
    }

    fn patch_entry_id(entry: &Value) -> Option<&str> {
        entry
            .as_mapping()?
            .get(Value::String("id".into()))?
            .as_str()
    }

    /// The model id the patch layer pins, `agent-default-model` first.
    pub fn patched_model(path: &Path) -> Result<Option<String>> {
        for id in [AGENT_DEFAULT_MODEL_PATCH_ID, ACP_SESSION_PATCH_ID] {
            for entry in Self::read_patch_list(path)? {
                if Self::patch_entry_id(&entry) != Some(id) {
                    continue;
                }
                let model = entry
                    .as_mapping()
                    .and_then(|map| map.get(Value::String("config".into())))
                    .and_then(|config| config.get(Value::String("model".into())))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|model| !model.is_empty());
                if let Some(model) = model {
                    return Ok(Some(model.to_string()));
                }
            }
        }
        Ok(None)
    }

    /// Pin (or, with an empty id, unpin) the default model in the home patch
    /// file. Entries the rest of Agent Doctor wrote — the MCP plugin inserts,
    /// mostly — are preserved untouched.
    pub fn apply_model_patch(path: &Path, model: &str) -> Result<()> {
        let model = model.trim();
        let mut patches = Self::read_patch_list(path)?;
        let mut changed = false;
        if model.is_empty() {
            let before = patches.len();
            patches.retain(|entry| {
                !matches!(
                    Self::patch_entry_id(entry),
                    Some(AGENT_DEFAULT_MODEL_PATCH_ID) | Some(ACP_SESSION_PATCH_ID)
                )
            });
            changed = patches.len() != before;
        } else {
            let desired = |id: &str| -> Value {
                let mut config = serde_yaml::Mapping::new();
                config.insert(
                    Value::String("provider".into()),
                    Value::String(DEEPSEEK_OFFICIAL_PROVIDER_ROUTE.into()),
                );
                config.insert(Value::String("model".into()), Value::String(model.into()));
                let mut entry = serde_yaml::Mapping::new();
                entry.insert(Value::String("id".into()), Value::String(id.into()));
                entry.insert(Value::String("config".into()), Value::Mapping(config));
                Value::Mapping(entry)
            };
            for id in [AGENT_DEFAULT_MODEL_PATCH_ID, ACP_SESSION_PATCH_ID] {
                let entry = desired(id);
                if let Some(slot) = patches
                    .iter_mut()
                    .find(|entry| Self::patch_entry_id(entry) == Some(id))
                {
                    if *slot != entry {
                        *slot = entry;
                        changed = true;
                    }
                } else {
                    patches.push(entry);
                    changed = true;
                }
            }
        }
        if !changed {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let rendered = serde_yaml::to_string(&Value::Sequence(patches))
            .with_context(|| format!("failed to serialize {}", path.display()))?;
        fs::write(path, rendered).with_context(|| format!("failed to write {}", path.display()))
    }

    /// Keep the chat's chosen model in dsh's patch layer. A no-op when the
    /// patch already carries it, so a turn costs no write.
    pub fn ensure_model_patch(&self, model: &str) -> Result<()> {
        Self::apply_model_patch(&Self::cordis_patch_path(), model)
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

    fn read_model(&self) -> Result<Option<RuntimeModelState>> {
        let Some(model) = Self::patched_model(&Self::cordis_patch_path())? else {
            return Ok(None);
        };
        Ok(Some(RuntimeModelState {
            provider: Some(DEEPSEEK_OFFICIAL_PROVIDER_ROUTE.to_string()),
            model: Some(model),
            // A custom endpoint arrives through the launch environment, not
            // the patch layer, so surface whatever this process resolves.
            base_url: self
                .read_profile()
                .ok()
                .and_then(|profile| profile.gateway_url),
        }))
    }

    fn apply_model(&self, preset: &RuntimeModelPreset) -> Result<ApplyReport> {
        let path = Self::cordis_patch_path();
        Self::apply_model_patch(&path, &preset.model)?;
        Ok(ApplyReport {
            runtime_id: DEEPSEEK_HARNESS_RUNTIME_ID.to_string(),
            config_path: path.display().to_string(),
            backup_path: None,
            restart_hint: "start a new DeepSeek Harness chat to pick up the model".to_string(),
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
    use tempfile::TempDir;

    fn patch_file(dir: &Path) -> PathBuf {
        dir.join(".dsh").join("cordis.patch.yml")
    }

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
        assert_eq!(DEEPSEEK_HARNESS_VERSION, "0.1.5-rc.3");
        assert_eq!(DEEPSEEK_HARNESS_NPM_PACKAGE, "@deepseek-ai/dsh");
        assert!(!DeepSeekHarnessAdapter.config_paths_required());
    }

    #[test]
    fn apply_model_pins_both_patch_entries_and_preserves_others() {
        let dir = TempDir::new().unwrap();
        let path = patch_file(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "- insert:\n  - id: mcp-computer\n    name: '@deepseek-ai/dsh-mcp-client'\n    config:\n      serverName: computer\n",
        )
        .unwrap();

        DeepSeekHarnessAdapter::apply_model_patch(&path, "my-model").unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        assert!(
            raw.contains("mcp-computer"),
            "managed inserts survive: {raw}"
        );
        assert!(raw.contains("my-model"), "{raw}");
        let list = DeepSeekHarnessAdapter::read_patch_list(&path).unwrap();
        let model_entry = list
            .iter()
            .find(|entry| {
                DeepSeekHarnessAdapter::patch_entry_id(entry) == Some("agent-default-model")
            })
            .unwrap();
        let config = model_entry
            .as_mapping()
            .unwrap()
            .get(Value::String("config".into()))
            .unwrap();
        assert_eq!(
            config
                .get(Value::String("provider".into()))
                .and_then(Value::as_str),
            Some(DEEPSEEK_OFFICIAL_PROVIDER_ROUTE)
        );
        assert_eq!(
            config
                .get(Value::String("model".into()))
                .and_then(Value::as_str),
            Some("my-model")
        );
        assert!(list
            .iter()
            .any(|entry| DeepSeekHarnessAdapter::patch_entry_id(entry) == Some("acp")));
    }

    #[test]
    fn re_applying_the_same_model_skips_the_write() {
        let dir = TempDir::new().unwrap();
        let path = patch_file(dir.path());
        DeepSeekHarnessAdapter::apply_model_patch(&path, "my-model").unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        DeepSeekHarnessAdapter::apply_model_patch(&path, "my-model").unwrap();
        assert_eq!(before, fs::metadata(&path).unwrap().modified().unwrap());
    }

    #[test]
    fn empty_model_unpins() {
        let dir = TempDir::new().unwrap();
        let path = patch_file(dir.path());
        DeepSeekHarnessAdapter::apply_model_patch(&path, "my-model").unwrap();
        DeepSeekHarnessAdapter::apply_model_patch(&path, "").unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("my-model"), "{raw}");
        assert_eq!(DeepSeekHarnessAdapter::patched_model(&path).unwrap(), None);
    }

    #[test]
    fn read_model_reports_the_pinned_model() {
        let dir = TempDir::new().unwrap();
        let path = patch_file(dir.path());
        DeepSeekHarnessAdapter::apply_model_patch(&path, "my-model").unwrap();
        // home()/cordis_patch_path() resolve DSH_HOME; root the adapter at the
        // temp dir for the duration of the read. Env mutation is racy across
        // the test binary, so serialize on a lock and restore the old value.
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap();
        let previous = std::env::var_os("DSH_HOME");
        let set_home = |value: Option<std::ffi::OsString>| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match value {
                Some(value) => unsafe { std::env::set_var("DSH_HOME", value) },
                None => unsafe { std::env::remove_var("DSH_HOME") },
            }))
            .unwrap();
        };
        set_home(Some(dir.path().join(".dsh").into()));
        let state = DeepSeekHarnessAdapter.read_model().unwrap();
        set_home(previous);
        assert_eq!(
            state.map(|state| (state.provider, state.model)),
            Some((
                Some(DEEPSEEK_OFFICIAL_PROVIDER_ROUTE.to_string()),
                Some("my-model".to_string())
            ))
        );
    }

    #[test]
    fn a_tagged_patch_file_is_refused_not_rewritten() {
        let dir = TempDir::new().unwrap();
        let path = patch_file(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "- id: x\n  config:\n    model: !!js process.env.M\n").unwrap();
        assert!(DeepSeekHarnessAdapter::apply_model_patch(&path, "m").is_err());
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("!!js"), "hand-edited file untouched: {raw}");
    }

    #[test]
    fn an_unknown_shape_is_refused_not_clobbered() {
        let dir = TempDir::new().unwrap();
        let path = patch_file(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "plugins:\n  - id: keep-me\n").unwrap();
        assert!(DeepSeekHarnessAdapter::apply_model_patch(&path, "m").is_err());
        assert!(fs::read_to_string(&path).unwrap().contains("keep-me"));
    }
}
