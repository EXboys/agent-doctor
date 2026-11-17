use std::path::{Path, PathBuf};

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{
    discover_binary, discovery_from_cli, discovery_present, home_join, DesktopAppSpec,
};

pub struct CursorAdapter;

/// `cursor-agent` installs as `agent` under `~/.local/bin`, so the binary name
/// alone cannot tell it from other tools'.
fn looks_like_cursor_agent(path: &Path) -> bool {
    // Normalize separators so Windows `\.local\bin\agent` matches the same
    // suffix check as Unix `/.local/bin/agent`.
    let text = path.to_string_lossy().replace('\\', "/");
    let lower = text.to_ascii_lowercase();
    lower.contains("cursor-agent")
        || text.ends_with("/.local/bin/agent")
        || (lower.ends_with("/agent.exe") && lower.contains("cursor"))
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

/// Official Cursor desktop app (`Cursor.app` / `Cursor.exe`).
pub fn cursor_desktop_path() -> Option<PathBuf> {
    crate::adapters::util::desktop_app_path(&DESKTOP_APP)
}

fn desktop_app_path() -> Option<PathBuf> {
    cursor_desktop_path()
}

/// Ask needs the Cursor Agent CLI (`agent`). The desktop IDE alone is not enough.
pub fn cursor_missing_install_part() -> Option<&'static str> {
    if cursor_cli_path().is_some() {
        return None;
    }
    if cursor_desktop_path().is_some() {
        Some("Cursor 桌面软件已在本机，命令行还没装。")
    } else {
        Some("Cursor 命令行还没装。")
    }
}

/// The official Cursor command line. The desktop app is not this binary.
pub fn cursor_cli_path() -> Option<PathBuf> {
    let local_agent = home_join(".local/bin/agent");
    if local_agent.is_file() && looks_like_cursor_agent(&local_agent) {
        return Some(local_agent);
    }
    if crate::adapters::util::is_test_home_active() {
        return None;
    }
    let named = discover_binary("cursor-agent");
    if named.binary_path.is_some() {
        return named.binary_path;
    }
    let agent = discover_binary("agent");
    if agent
        .binary_path
        .as_deref()
        .is_some_and(looks_like_cursor_agent)
    {
        return agent.binary_path;
    }
    None
}

/// Install locations the uninstall step may remove. Settings under `~/.cursor` are not included.
pub(crate) fn cursor_desktop_installs() -> Vec<PathBuf> {
    crate::adapters::util::desktop_app_candidates(&DESKTOP_APP)
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
        if !crate::adapters::util::is_test_home_active() {
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
        }
        if let Some(app) = desktop_app_path() {
            return discovery_present(app);
        }
        // Uninstall deliberately keeps settings (`~/.cursor`) and CLI data
        // (`~/.local/share/cursor-agent`), so neither is evidence of a usable
        // install — only binaries and the desktop app count.
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
    fn leftover_settings_alone_do_not_count_as_installed() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(temp.path().join(".cursor")).expect("cursor settings");
        fs::create_dir_all(temp.path().join(".local/share/cursor-agent")).expect("cli data");
        with_test_home(temp.path(), || {
            let found = CursorAdapter.discover();
            assert!(!found.installed);
            assert_eq!(found.binary_path, None);
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
            assert_eq!(
                cursor_missing_install_part(),
                Some("Cursor 桌面软件已在本机，命令行还没装。")
            );
        });
    }

    #[test]
    fn agent_cli_clears_the_missing_part() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin = temp.path().join(".local/bin/agent");
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        fs::write(&bin, b"").unwrap();
        with_test_home(temp.path(), || {
            assert_eq!(cursor_cli_path(), Some(home_join(".local/bin/agent")));
            assert_eq!(cursor_missing_install_part(), None);
        });
    }

    #[test]
    fn windows_local_agent_path_counts_as_cursor_cli() {
        assert!(looks_like_cursor_agent(Path::new(
            r"C:\Users\runner\AppData\Local\Temp\.tmp\home\.local\bin\agent"
        )));
        assert!(looks_like_cursor_agent(Path::new(
            r"C:\Users\runner\AppData\Local\Programs\cursor\resources\app\bin\agent.exe"
        )));
        assert!(!looks_like_cursor_agent(Path::new(
            r"C:\Windows\System32\agent.exe"
        )));
    }
}
