use std::path::PathBuf;

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{discover_binary, home_join};

pub struct QoderAdapter;

impl RuntimeAdapter for QoderAdapter {
    fn id(&self) -> &'static str {
        "qoder"
    }

    fn display_name(&self) -> &'static str {
        "Qoder"
    }

    fn discover(&self) -> AdapterDiscovery {
        discover_binary("qoder")
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        vec![home_join(".qoder/settings.json")]
    }

    fn read_profile(&self) -> anyhow::Result<RuntimeProfile> {
        Ok(RuntimeProfile {
            gateway_url: None,
            key_source: self
                .config_paths()
                .into_iter()
                .find(|path| path.exists())
                .map(|path| path.display().to_string()),
        })
    }
}
