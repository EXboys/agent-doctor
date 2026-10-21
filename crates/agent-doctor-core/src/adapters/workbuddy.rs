use std::path::PathBuf;

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{discover_binary, home_join, profile_from_config_paths};

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
        Ok(profile_from_config_paths(self.config_paths()))
    }
}
