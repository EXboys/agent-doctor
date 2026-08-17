use std::fs;
use std::path::PathBuf;

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{discover_binary, home_join};

pub struct OpenClawAdapter;

impl RuntimeAdapter for OpenClawAdapter {
    fn id(&self) -> &'static str {
        "openclaw"
    }

    fn display_name(&self) -> &'static str {
        "OpenClaw"
    }

    fn discover(&self) -> AdapterDiscovery {
        discover_binary("openclaw")
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        vec![home_join(".openclaw/openclaw.json")]
    }

    fn optional_config_paths(&self) -> Vec<PathBuf> {
        // Secrets overlay. Missing is not a defect; wiring/scaffold creates it when needed.
        vec![home_join(".openclaw/.env")]
    }

    fn read_profile(&self) -> anyhow::Result<RuntimeProfile> {
        let path = home_join(".openclaw/openclaw.json");
        if !path.exists() {
            return Ok(RuntimeProfile {
                gateway_url: None,
                key_source: None,
            });
        }

        let raw = fs::read_to_string(&path)?;
        let value: serde_json::Value = serde_json::from_str(&raw)?;
        let gateway_url = configured_base_url(&value);

        Ok(RuntimeProfile {
            gateway_url,
            key_source: Some(format!("{}", path.display())),
        })
    }
}

/// Resolve the LLM base URL OpenClaw is configured to use.
///
/// Prefer the provider named in `agents.defaults.model.primary` (`slot/model`),
/// then Agent Doctor slots (`evotown` / `personal` / legacy `agent-doctor`),
/// then any other provider `baseUrl`, then legacy invalid keys for migration reads.
pub fn configured_base_url(value: &serde_json::Value) -> Option<String> {
    use crate::setup::{OPENCLAW_PERSONAL_SLOT, OPENCLAW_PROVIDER_ID, OPENCLAW_TEAM_SLOT};

    if let Some(primary) = value
        .pointer("/agents/defaults/model/primary")
        .and_then(serde_json::Value::as_str)
    {
        if let Some((slot, _)) = primary.split_once('/') {
            if let Some(url) = value
                .pointer(&format!("/models/providers/{slot}/baseUrl"))
                .and_then(serde_json::Value::as_str)
                .filter(|u| !u.trim().is_empty())
            {
                return Some(url.to_string());
            }
        }
    }

    for slot in [
        OPENCLAW_TEAM_SLOT,
        OPENCLAW_PERSONAL_SLOT,
        OPENCLAW_PROVIDER_ID,
    ] {
        if let Some(url) = value
            .pointer(&format!("/models/providers/{slot}/baseUrl"))
            .and_then(serde_json::Value::as_str)
            .filter(|u| !u.trim().is_empty())
        {
            return Some(url.to_string());
        }
    }

    if let Some(providers) = value
        .pointer("/models/providers")
        .and_then(|v| v.as_object())
    {
        for provider in providers.values() {
            if let Some(url) = provider
                .get("baseUrl")
                .and_then(serde_json::Value::as_str)
                .filter(|u| !u.trim().is_empty())
            {
                return Some(url.to_string());
            }
        }
    }

    value
        .pointer("/gateway/url")
        .or_else(|| value.pointer("/evotown/url"))
        .and_then(serde_json::Value::as_str)
        .filter(|u| !u.trim().is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_file_is_optional_not_required() {
        let required = OpenClawAdapter.config_paths();
        let optional = OpenClawAdapter.optional_config_paths();
        assert!(required.iter().any(|path| {
            path.file_name().and_then(|name| name.to_str()) == Some("openclaw.json")
        }));
        assert!(required.iter().all(|path| {
            path.file_name().and_then(|name| name.to_str()) != Some(".env")
        }));
        assert!(optional.iter().any(|path| {
            path.file_name().and_then(|name| name.to_str()) == Some(".env")
        }));
    }
}
