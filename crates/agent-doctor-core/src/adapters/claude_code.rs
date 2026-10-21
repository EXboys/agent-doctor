use std::fs;
use std::path::PathBuf;

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{discover_binary, home_join};

pub struct ClaudeCodeAdapter;

impl RuntimeAdapter for ClaudeCodeAdapter {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn discover(&self) -> AdapterDiscovery {
        discover_binary("claude")
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        vec![home_join(".claude/settings.json")]
    }

    fn read_profile(&self) -> anyhow::Result<RuntimeProfile> {
        let path = home_join(".claude/settings.json");
        if !path.exists() {
            return Ok(RuntimeProfile::missing());
        }

        let raw = fs::read_to_string(&path)?;
        let value: serde_json::Value = serde_json::from_str(&raw)?;
        let gateway_url = value
            .pointer("/env/ANTHROPIC_BASE_URL")
            .or_else(|| value.pointer("/anthropicBaseUrl"))
            .and_then(|v| v.as_str())
            .map(str::to_string);

        Ok(RuntimeProfile {
            gateway_url,
            key_source: Some(format!("{}", path.display())),
        })
    }
}
