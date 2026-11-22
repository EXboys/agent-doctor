//! Pi coding agent (`pi`, npm `@earendil-works/pi-coding-agent`).
//!
//! Config lives in `~/.pi/agent` unless `PI_CODING_AGENT_DIR` points somewhere else.
//! Tests pin the directory under the temporary home so a developer machine is left alone.

use std::path::PathBuf;

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeModelState, RuntimeProfile};
use crate::adapters::util::{discover_binary, home_join, is_test_home_active};

pub const PI_RUNTIME_ID: &str = "pi";
pub const PI_NPM_PACKAGE: &str = "@earendil-works/pi-coding-agent";
/// Pi's own `engines.node` floor. An older Node exits before the program starts.
pub const PI_NODE_MIN: &str = "22.19.0";

pub struct PiAdapter;

/// Directory that holds `models.json`, `auth.json`, and `settings.json`.
pub fn pi_agent_dir() -> PathBuf {
    if !is_test_home_active() {
        if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
            let trimmed = dir.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
    }
    home_join(".pi/agent")
}

impl RuntimeAdapter for PiAdapter {
    fn id(&self) -> &'static str {
        PI_RUNTIME_ID
    }

    fn display_name(&self) -> &'static str {
        "Pi"
    }

    fn discover(&self) -> AdapterDiscovery {
        discover_binary("pi")
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        let root = pi_agent_dir();
        vec![
            root.join("models.json"),
            root.join("auth.json"),
            root.join("settings.json"),
        ]
    }

    fn config_paths_required(&self) -> bool {
        false
    }

    fn read_profile(&self) -> anyhow::Result<RuntimeProfile> {
        let models = pi_agent_dir().join("models.json");
        if !models.exists() {
            return Ok(RuntimeProfile::missing());
        }
        let raw = std::fs::read_to_string(&models)?;
        let value: serde_json::Value = serde_json::from_str(&raw)?;
        let gateway_url = value
            .pointer("/providers/agent-doctor/baseUrl")
            .and_then(serde_json::Value::as_str)
            .filter(|url| !url.trim().is_empty())
            .map(str::to_string);
        let auth = pi_agent_dir().join("auth.json");
        let key_source = if auth.is_file() {
            Some(auth.display().to_string())
        } else if gateway_url.is_some() {
            Some(models.display().to_string())
        } else {
            None
        };
        Ok(RuntimeProfile {
            gateway_url,
            key_source,
        })
    }

    fn read_model(&self) -> anyhow::Result<Option<RuntimeModelState>> {
        let settings_path = pi_agent_dir().join("settings.json");
        if !settings_path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&settings_path)?;
        let value: serde_json::Value = serde_json::from_str(&raw)?;
        let provider = value
            .get("defaultProvider")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let model = value
            .get("defaultModel")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let base_url = std::fs::read_to_string(pi_agent_dir().join("models.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|models| {
                let slot = provider.as_deref().unwrap_or("agent-doctor");
                models
                    .pointer(&format!("/providers/{slot}/baseUrl"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            });
        if provider.is_none() && model.is_none() && base_url.is_none() {
            return Ok(None);
        }
        Ok(Some(RuntimeModelState {
            provider,
            model,
            base_url,
        }))
    }
}
