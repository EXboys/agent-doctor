use serde::{Deserialize, Serialize};

use crate::adapter::RuntimeProfile;
use crate::presets::load_profiles;
use crate::profile::agent_profile_path;
use crate::profile::read_company_profile;
use crate::runtime::all_adapters;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDoctorResult {
    pub id: String,
    pub display_name: String,
    pub installed: bool,
    pub version: Option<String>,
    pub binary_path: Option<String>,
    pub config_paths: Vec<String>,
    pub profile: RuntimeProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub profile_env_path: Option<String>,
    pub profile_env_exists: bool,
    pub company_gateway_url: Option<String>,
    pub active_preset: Option<String>,
    pub runtimes: Vec<RuntimeDoctorResult>,
}

pub fn run_doctor() -> DoctorReport {
    let profile_env_path = agent_profile_path();
    let profile_env_exists = profile_env_path
        .as_ref()
        .map(|path| path.exists())
        .unwrap_or(false);

    let runtimes = all_adapters()
        .iter()
        .map(|adapter| {
            let discovery = adapter.discover();
            let profile = adapter.read_profile().unwrap_or(RuntimeProfile {
                gateway_url: None,
                key_source: None,
            });

            RuntimeDoctorResult {
                id: adapter.id().to_string(),
                display_name: adapter.display_name().to_string(),
                installed: discovery.installed,
                version: discovery.version,
                binary_path: discovery.binary_path.map(|path| path.display().to_string()),
                config_paths: adapter
                    .all_config_paths()
                    .into_iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                profile,
            }
        })
        .collect();

    let active_preset = load_profiles().ok().and_then(|doc| doc.active);
    let company_gateway_url = read_company_profile()
        .ok()
        .flatten()
        .and_then(|profile| profile.gateway_url);

    DoctorReport {
        profile_env_path: profile_env_path.map(|path| path.display().to_string()),
        profile_env_exists,
        company_gateway_url,
        active_preset,
        runtimes,
    }
}
