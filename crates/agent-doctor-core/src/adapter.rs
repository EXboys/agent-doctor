use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterDiscovery {
    pub installed: bool,
    pub version: Option<String>,
    pub binary_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeProfile {
    pub gateway_url: Option<String>,
    pub key_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeModelState {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeModelPreset {
    pub provider: String,
    pub model: String,
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyReport {
    pub runtime_id: String,
    pub config_path: String,
    pub backup_path: Option<String>,
    pub restart_hint: String,
}

pub trait RuntimeAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn discover(&self) -> AdapterDiscovery;
    fn config_paths(&self) -> Vec<PathBuf>;
    fn config_paths_required(&self) -> bool {
        true
    }
    /// Credentials or overlays that may be absent (no warn, no repair).
    fn optional_config_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn all_config_paths(&self) -> Vec<PathBuf> {
        let mut paths = self.config_paths();
        paths.extend(self.optional_config_paths());
        paths
    }
    fn read_profile(&self) -> anyhow::Result<RuntimeProfile>;

    fn read_model(&self) -> anyhow::Result<Option<RuntimeModelState>> {
        Ok(None)
    }

    fn apply_model(&self, _preset: &RuntimeModelPreset) -> anyhow::Result<ApplyReport> {
        anyhow::bail!("{} does not support model presets yet", self.display_name())
    }
}
