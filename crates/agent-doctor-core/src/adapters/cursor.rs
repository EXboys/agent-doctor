use std::path::{Path, PathBuf};

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{
    discover_binary, discovery_from_cli, discovery_present, home_join, DesktopAppSpec,
};

pub struct CursorAdapter;

/// `cursor-agent` installs as `agent` under `~/.local/bin`, so the binary name
/// alone cannot tell it from other tools'.
fn looks_like_cursor_agent(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.contains("cursor-agent")
        || text.ends_with("/.local/bin/agent")
        || text.ends_with("\\agent.exe")
}

const DESKTOP_APP: DesktopAppSpec = DesktopAppSpec {
    macos_apps: &["Cursor.app"],
    windows_local: &["Programs/cursor/Cursor.exe", "Programs/Cursor/Cursor.exe"],
    windows_machine: &[],
    linux_paths: &[
        "/opt/Cursor/cursor",
        "/usr/share/cursor/cursor",
        "~/.local/share/applications/cursor.desktop",
    ],
};

fn desktop_app_path() -> Option<PathBuf> {
    crate::adapters::util::desktop_app_path(&DESKTOP_APP)
}

impl RuntimeAdapter for CursorAdapter {
    fn id(&self) -> &'static str {
        "cursor"
    }

    fn display_name(&self) -> &'static str {
        "Cursor"
    }

    fn discover(&self) -> AdapterDiscovery {
        let local_agent = home_join(".local/bin/agent");
        if local_agent.is_file() {
            return discovery_from_cli(local_agent);
        }
        let cursor = discover_binary("cursor");
        if cursor.installed {
            return cursor;
        }
        let agent = discover_binary("agent");
        if agent
            .binary_path
            .as_deref()
            .is_some_and(looks_like_cursor_agent)
        {
            return agent;
        }
        if let Some(app) = desktop_app_path() {
            return discovery_present(app);
        }
        let cursor_home = home_join(".cursor");
        if cursor_home.is_dir() {
            return discovery_present(cursor_home);
        }
        if home_join(".local/share/cursor-agent").is_dir() {
            return discovery_present(local_agent);
        }
        AdapterDiscovery {
            installed: false,
            version: None,
            binary_path: None,
        }
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        vec![home_join(".cursor/cli-config.json")]
    }

    fn config_paths_required(&self) -> bool {
        false
    }

    fn read_profile(&self) -> anyhow::Result<RuntimeProfile> {
        Ok(RuntimeProfile {
            gateway_url: None,
            key_source: home_join(".cursor")
                .is_dir()
                .then(|| home_join(".cursor").display().to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::util::with_test_home;
    use std::fs;

    #[test]
    fn cursor_home_dir_counts_as_installed() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(temp.path().join(".cursor")).expect("cursor home");
        with_test_home(temp.path(), || {
            let found = CursorAdapter.discover();
            assert!(found.installed);
            assert_eq!(found.binary_path, Some(home_join(".cursor")));
        });
    }

    #[test]
    fn user_applications_app_counts_as_installed() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("Applications/Cursor.app")).expect("cursor app");
        with_test_home(temp.path(), || {
            let found = CursorAdapter.discover();
            assert!(found.installed);
            assert_eq!(
                found.binary_path,
                Some(home_join("Applications/Cursor.app"))
            );
        });
    }
}
