//! Antigravity CLI (`agy`).
//!
//! The binary is installed by Google's script into `~/.local/bin/agy`
//! (Windows: `%LOCALAPPDATA%\agy\bin\agy.exe`). Settings live in
//! `~/.gemini/antigravity-cli/settings.json`.

use std::path::PathBuf;

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{discover_binary, home_join};

pub const ANTIGRAVITY_RUNTIME_ID: &str = "antigravity";

pub struct AntigravityAdapter;

pub fn antigravity_settings_path() -> PathBuf {
    home_join(".gemini/antigravity-cli/settings.json")
}

pub fn agy_cli_path() -> Option<PathBuf> {
    crate::adapters::util::find_binary("agy")
}

/// API-key mode is ignored until settings say `modelProvider` is `gemini`.
/// Other keys in the file stay.
pub fn enable_gemini_key_provider() {
    let path = antigravity_settings_path();
    let mut value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert(
        "modelProvider".into(),
        serde_json::Value::String("gemini".into()),
    );
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(raw) = serde_json::to_string_pretty(&value) {
        let _ = std::fs::write(path, raw);
    }
}

/// Drop API-key mode so the next turn uses the Google login on this computer.
/// Other settings stay. A file that is not in key mode is left as it is.
pub fn disable_gemini_key_provider() {
    let path = antigravity_settings_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if object
        .get("modelProvider")
        .and_then(serde_json::Value::as_str)
        != Some("gemini")
    {
        return;
    }
    object.remove("modelProvider");
    if let Ok(pretty) = serde_json::to_string_pretty(&value) {
        let _ = std::fs::write(path, pretty);
    }
}

impl RuntimeAdapter for AntigravityAdapter {
    fn id(&self) -> &'static str {
        ANTIGRAVITY_RUNTIME_ID
    }

    fn display_name(&self) -> &'static str {
        "Antigravity"
    }

    fn discover(&self) -> AdapterDiscovery {
        discover_binary("agy")
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        vec![antigravity_settings_path()]
    }

    fn config_paths_required(&self) -> bool {
        false
    }

    fn read_profile(&self) -> anyhow::Result<RuntimeProfile> {
        let path = antigravity_settings_path();
        if !path.is_file() {
            return Ok(RuntimeProfile::missing());
        }
        let raw = std::fs::read_to_string(&path)?;
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        let uses_key = value
            .get("modelProvider")
            .and_then(serde_json::Value::as_str)
            == Some("gemini");
        Ok(RuntimeProfile {
            gateway_url: None,
            key_source: if uses_key {
                Some(path.display().to_string())
            } else {
                None
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::util::with_test_home;

    #[test]
    fn key_mode_can_be_cleared_without_dropping_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        with_test_home(dir.path(), || {
            let path = antigravity_settings_path();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                r#"{"modelProvider":"gemini","trustedWorkspaces":["/tmp"]}"#,
            )
            .unwrap();
            disable_gemini_key_provider();
            let raw = std::fs::read_to_string(&path).unwrap();
            assert!(!raw.contains("modelProvider"));
            assert!(raw.contains("trustedWorkspaces"));
            disable_gemini_key_provider();
            assert!(std::fs::read_to_string(&path)
                .unwrap()
                .contains("trustedWorkspaces"));
        });
    }
}
