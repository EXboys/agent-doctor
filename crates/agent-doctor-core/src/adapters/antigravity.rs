//! Antigravity CLI (`agy`).
//!
//! The binary is installed by Google's script into `~/.local/bin/agy`
//! (Windows: `%LOCALAPPDATA%\agy\bin\agy.exe`). Settings live in
//! `~/.gemini/antigravity-cli/settings.json`.
//!
//! The desktop app (`Antigravity.app` / `Antigravity.exe`) is a different
//! program. A shell script named `agy` that only launches that app is not
//! the CLI this chat runs (`agy acp`).

use std::path::{Path, PathBuf};

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{
    desktop_app_path, discovery_from_cli, discovery_present, find_all_binaries, home_join,
    is_test_home_active, DesktopAppSpec,
};

pub const ANTIGRAVITY_RUNTIME_ID: &str = "antigravity";

pub struct AntigravityAdapter;

pub fn antigravity_settings_path() -> PathBuf {
    home_join(".gemini/antigravity-cli/settings.json")
}

const DESKTOP_APP: DesktopAppSpec = DesktopAppSpec {
    macos_apps: &["Antigravity.app"],
    windows_local: &[
        "Programs/antigravity/Antigravity.exe",
        "Programs/Antigravity/Antigravity.exe",
    ],
    windows_machine: &["Antigravity/Antigravity.exe", "antigravity/Antigravity.exe"],
    linux_paths: &[],
};

/// Official `agy` binary. The desktop app and a shell script that only opens
/// it do not count: chat runs `agy acp`.
pub fn agy_cli_path() -> Option<PathBuf> {
    for path in known_cli_files() {
        if usable_cli(&path) {
            return Some(path);
        }
    }
    if is_test_home_active() {
        return None;
    }
    find_all_binaries("agy")
        .into_iter()
        .find(|path| usable_cli(path))
}

/// Antigravity desktop app, when the CLI is not what is installed.
pub fn antigravity_desktop_path() -> Option<PathBuf> {
    desktop_app_path(&DESKTOP_APP)
}

/// Set when the desktop app is on this computer and the CLI is not.
/// The CLI by itself is enough; this app does not install the desktop app.
pub fn antigravity_missing_install_part() -> Option<&'static str> {
    if agy_cli_path().is_some() {
        return None;
    }
    antigravity_desktop_path().map(|_| "Antigravity 桌面软件已在本机，命令行还没装。")
}

fn known_cli_files() -> Vec<PathBuf> {
    let mut paths = vec![
        home_join(".local/bin/agy"),
        home_join(".local/bin/agy.exe"),
        home_join("AppData/Local/agy/bin/agy.exe"),
    ];
    if !is_test_home_active() {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            paths.push(PathBuf::from(local).join("agy").join("bin").join("agy.exe"));
        }
    }
    paths
}

fn usable_cli(path: &Path) -> bool {
    path.is_file() && !is_ide_launcher(path)
}

/// Homebrew and the IDE used to publish a script named `agy` that only starts
/// `Antigravity.app`. That file is a few kilobytes. The real CLI is the large
/// Go binary.
fn is_ide_launcher(path: &Path) -> bool {
    let text = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    if text.contains("/antigravity.app/")
        || text.contains("/programs/antigravity/")
        || text.contains("agy.wrapper")
        || text.contains("/antigravity-ide/")
    {
        return true;
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if meta.len() > 256 * 1024 {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    head.contains("antigravity.app")
        || head.contains("antigravity.exe")
        || head.contains("antigravity-ide")
}

/// API-key mode is ignored until settings say `modelProvider` is `gemini`.
/// Other keys in the file stay.
pub fn enable_gemini_key_provider() {
    let path = antigravity_settings_path();
    let mut value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert(
        "modelProvider".into(),
        serde_json::Value::String("gemini".into()),
    );
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(raw) = serde_json::to_string_pretty(&value) {
        let _ = std::fs::write(path, raw);
    }
}

/// Drop API-key mode so the next turn uses the Google login on this computer.
/// Other settings stay. A file that is not in key mode is left as it is.
pub fn disable_gemini_key_provider() {
    let path = antigravity_settings_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if object
        .get("modelProvider")
        .and_then(serde_json::Value::as_str)
        != Some("gemini")
    {
        return;
    }
    object.remove("modelProvider");
    if let Ok(pretty) = serde_json::to_string_pretty(&value) {
        let _ = std::fs::write(path, pretty);
    }
}

impl RuntimeAdapter for AntigravityAdapter {
    fn id(&self) -> &'static str {
        ANTIGRAVITY_RUNTIME_ID
    }

    fn display_name(&self) -> &'static str {
        "Antigravity"
    }

    fn discover(&self) -> AdapterDiscovery {
        if let Some(cli) = agy_cli_path() {
            return discovery_from_cli(cli);
        }
        if let Some(app) = antigravity_desktop_path() {
            return discovery_present(app);
        }
        AdapterDiscovery {
            installed: false,
            version: None,
            binary_path: None,
        }
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        vec![antigravity_settings_path()]
    }

    fn config_paths_required(&self) -> bool {
        false
    }

    fn read_profile(&self) -> anyhow::Result<RuntimeProfile> {
        let path = antigravity_settings_path();
        if !path.is_file() {
            return Ok(RuntimeProfile::missing());
        }
        let raw = std::fs::read_to_string(&path)?;
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        let uses_key = value
            .get("modelProvider")
            .and_then(serde_json::Value::as_str)
            == Some("gemini");
        Ok(RuntimeProfile {
            gateway_url: None,
            key_source: if uses_key {
                Some(path.display().to_string())
            } else {
                None
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::util::with_test_home;

    #[test]
    fn key_mode_can_be_cleared_without_dropping_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        with_test_home(dir.path(), || {
            let path = antigravity_settings_path();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                r#"{"modelProvider":"gemini","trustedWorkspaces":["/tmp"]}"#,
            )
            .unwrap();
            disable_gemini_key_provider();
            let raw = std::fs::read_to_string(&path).unwrap();
            assert!(!raw.contains("modelProvider"));
            assert!(raw.contains("trustedWorkspaces"));
            disable_gemini_key_provider();
            assert!(std::fs::read_to_string(&path)
                .unwrap()
                .contains("trustedWorkspaces"));
        });
    }

    fn write_cli(path: &std::path::Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
    }

    #[test]
    fn user_local_bin_counts_as_the_cli() {
        let dir = tempfile::tempdir().unwrap();
        with_test_home(dir.path(), || {
            let bin = home_join(".local/bin/agy");
            write_cli(&bin);
            assert_eq!(agy_cli_path(), Some(bin));
            assert_eq!(antigravity_missing_install_part(), None);
            assert!(AntigravityAdapter.discover().installed);
        });
    }

    #[test]
    fn windows_local_appdata_bin_counts_as_the_cli() {
        let dir = tempfile::tempdir().unwrap();
        with_test_home(dir.path(), || {
            let bin = home_join("AppData/Local/agy/bin/agy.exe");
            write_cli(&bin);
            assert_eq!(agy_cli_path(), Some(bin));
            assert!(AntigravityAdapter.discover().installed);
        });
    }

    #[test]
    fn desktop_app_is_seen_when_the_cli_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        with_test_home(dir.path(), || {
            let app = home_join("Applications/Antigravity.app");
            std::fs::create_dir_all(&app).unwrap();
            assert!(agy_cli_path().is_none());
            assert_eq!(antigravity_desktop_path(), Some(app.clone()));
            assert_eq!(
                antigravity_missing_install_part(),
                Some("Antigravity 桌面软件已在本机，命令行还没装。")
            );
            let found = AntigravityAdapter.discover();
            assert!(found.installed);
            assert_eq!(found.binary_path, Some(app));
        });
    }

    #[test]
    fn ide_wrapper_named_agy_is_not_the_cli() {
        let dir = tempfile::tempdir().unwrap();
        with_test_home(dir.path(), || {
            let wrapper = home_join(".local/bin/agy");
            std::fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
            std::fs::write(
                &wrapper,
                b"#!/bin/sh\nexec '/Applications/Antigravity.app/Contents/Resources/app/bin/antigravity' \"$@\"\n",
            )
            .unwrap();
            let app = home_join("Applications/Antigravity.app");
            std::fs::create_dir_all(&app).unwrap();
            assert!(agy_cli_path().is_none());
            assert_eq!(
                antigravity_missing_install_part(),
                Some("Antigravity 桌面软件已在本机，命令行还没装。")
            );
            let real = home_join("AppData/Local/agy/bin/agy.exe");
            write_cli(&real);
            assert_eq!(agy_cli_path(), Some(real));
            assert_eq!(antigravity_missing_install_part(), None);
        });
    }
}
