use std::path::{Path, PathBuf};

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{discover_binary, home_join, read_version_result};

pub struct CursorAdapter;

fn looks_like_cursor_agent(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.contains("cursor-agent")
        || text.ends_with("/.local/bin/agent")
        || text.ends_with("\\agent.exe")
}

fn discovery_from_cli(path: PathBuf) -> AdapterDiscovery {
    let version = read_version_result(&path).ok().flatten();
    AdapterDiscovery {
        installed: true,
        version,
        binary_path: Some(path),
    }
}

fn discovery_present(path: PathBuf) -> AdapterDiscovery {
    AdapterDiscovery {
        installed: true,
        version: None,
        binary_path: Some(path),
    }
}

fn desktop_app_candidates() -> Vec<PathBuf> {
    let mut paths = vec![home_join("Applications/Cursor.app")];
    paths.extend(system_desktop_app_candidates());
    paths
}

#[cfg(test)]
fn system_desktop_app_candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(all(not(test), target_os = "macos"))]
fn system_desktop_app_candidates() -> Vec<PathBuf> {
    vec![PathBuf::from("/Applications/Cursor.app")]
}

#[cfg(all(not(test), target_os = "windows"))]
fn system_desktop_app_candidates() -> Vec<PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .map(PathBuf::from)
        .map(|local| {
            vec![
                local.join("Programs/cursor/Cursor.exe"),
                local.join("Programs/Cursor/Cursor.exe"),
            ]
        })
        .unwrap_or_default()
}

#[cfg(all(not(test), target_os = "linux"))]
fn system_desktop_app_candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/opt/Cursor/cursor"),
        PathBuf::from("/usr/share/cursor/cursor"),
        home_join(".local/share/applications/cursor.desktop"),
    ]
}

#[cfg(all(
    not(test),
    not(any(target_os = "macos", target_os = "windows", target_os = "linux"))
))]
fn system_desktop_app_candidates() -> Vec<PathBuf> {
    Vec::new()
}

fn desktop_app_path() -> Option<PathBuf> {
    desktop_app_candidates()
        .into_iter()
        .find(|path| path.exists())
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
