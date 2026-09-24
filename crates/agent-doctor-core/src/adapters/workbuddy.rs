use std::path::PathBuf;

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{discover_binary, home_join};

pub struct WorkbuddyAdapter;

impl RuntimeAdapter for WorkbuddyAdapter {
    fn id(&self) -> &'static str {
        "workbuddy"
    }

    fn display_name(&self) -> &'static str {
        "WorkBuddy"
    }

    fn discover(&self) -> AdapterDiscovery {
        let found = discover_binary("codebuddy");
        if found.installed {
            return found;
        }
        discover_binary("workbuddy")
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        vec![home_join(".codebuddy/settings.json")]
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
