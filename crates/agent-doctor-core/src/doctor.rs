use std::any::Any;
use std::thread;

use serde::{Deserialize, Serialize};

use crate::adapter::{RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::ensure_managed_runtime_path;
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
    /// Set when this runtime's probe crashed. The other runtimes still report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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

    // Seed PATH on this thread before parallel discovery (GUI apps often lack Homebrew).
    ensure_managed_runtime_path();

    let adapters = all_adapters();
    let runtimes = inspect_runtimes(&adapters);

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

fn inspect_runtimes(adapters: &[Box<dyn RuntimeAdapter>]) -> Vec<RuntimeDoctorResult> {
    thread::scope(|scope| {
        let handles: Vec<_> = adapters
            .iter()
            .map(|adapter| {
                (
                    adapter,
                    scope.spawn(move || inspect_runtime(adapter.as_ref())),
                )
            })
            .collect();

        handles
            .into_iter()
            .map(|(adapter, handle)| match handle.join() {
                Ok(result) => result,
                // One adapter's probe crashing must not take the whole report
                // down: surface it on that runtime and keep the others.
                Err(payload) => probe_crashed(adapter.as_ref(), &panic_message(payload.as_ref())),
            })
            .collect()
    })
}

fn inspect_runtime(adapter: &dyn RuntimeAdapter) -> RuntimeDoctorResult {
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
        error: None,
    }
}

fn probe_crashed(adapter: &dyn RuntimeAdapter, message: &str) -> RuntimeDoctorResult {
    RuntimeDoctorResult {
        id: adapter.id().to_string(),
        display_name: adapter.display_name().to_string(),
        installed: false,
        version: None,
        binary_path: None,
        config_paths: Vec::new(),
        profile: RuntimeProfile {
            gateway_url: None,
            key_source: None,
        },
        error: Some(format!("runtime probe crashed: {message}")),
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "unknown panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::adapter::AdapterDiscovery;

    #[test]
    fn run_doctor_returns_all_registry_runtimes() {
        let report = run_doctor();
        assert_eq!(report.runtimes.len(), all_adapters().len());
        assert!(report.runtimes.iter().any(|runtime| !runtime.id.is_empty()));
    }

    struct Crashing;

    impl RuntimeAdapter for Crashing {
        fn id(&self) -> &'static str {
            "crashing"
        }
        fn display_name(&self) -> &'static str {
            "Crashing Runtime"
        }
        fn discover(&self) -> AdapterDiscovery {
            panic!("probe exploded")
        }
        fn config_paths(&self) -> Vec<PathBuf> {
            Vec::new()
        }
        fn read_profile(&self) -> anyhow::Result<RuntimeProfile> {
            Ok(RuntimeProfile {
                gateway_url: None,
                key_source: None,
            })
        }
    }

    struct Quiet;

    impl RuntimeAdapter for Quiet {
        fn id(&self) -> &'static str {
            "quiet"
        }
        fn display_name(&self) -> &'static str {
            "Quiet Runtime"
        }
        fn discover(&self) -> AdapterDiscovery {
            AdapterDiscovery {
                installed: true,
                version: Some("1.0".into()),
                binary_path: None,
            }
        }
        fn config_paths(&self) -> Vec<PathBuf> {
            Vec::new()
        }
        fn read_profile(&self) -> anyhow::Result<RuntimeProfile> {
            Ok(RuntimeProfile {
                gateway_url: None,
                key_source: None,
            })
        }
    }

    #[test]
    fn a_crashing_probe_only_fails_its_own_runtime() {
        let adapters: Vec<Box<dyn RuntimeAdapter>> = vec![Box::new(Crashing), Box::new(Quiet)];
        let results = inspect_runtimes(&adapters);
        assert_eq!(results.len(), 2);

        let crashed = &results[0];
        assert_eq!(crashed.id, "crashing");
        assert!(!crashed.installed);
        let error = crashed.error.as_deref().expect("crash is reported");
        assert!(error.contains("probe exploded"), "{error}");

        let quiet = &results[1];
        assert_eq!(quiet.id, "quiet");
        assert!(quiet.installed);
        assert_eq!(quiet.version.as_deref(), Some("1.0"));
        assert!(quiet.error.is_none());
    }

    #[test]
    fn crash_field_is_omitted_from_json_when_absent() {
        let adapters: Vec<Box<dyn RuntimeAdapter>> = vec![Box::new(Quiet)];
        let json = serde_json::to_string(&inspect_runtimes(&adapters)[0]).unwrap();
        assert!(!json.contains("\"error\""), "{json}");
    }
}
