use std::path::PathBuf;

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{
    desktop_app_candidates, desktop_app_path, discovery_from_cli, discovery_present, find_binary,
    home_join, profile_from_config_paths, DesktopAppSpec,
};

pub struct WorkbuddyAdapter;

const DESKTOP_APP: DesktopAppSpec = DesktopAppSpec {
    macos_apps: &["WorkBuddy.app"],
    windows_local: &[
        "Programs/WorkBuddy/WorkBuddy.exe",
        "Programs/workbuddy/WorkBuddy.exe",
    ],
    windows_machine: &["WorkBuddy/WorkBuddy.exe"],
    linux_paths: &[],
};

const DESKTOP_REQUIRED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// The official WorkBuddy command line (`codebuddy`). The desktop app is not this binary.
pub fn workbuddy_cli_path() -> Option<PathBuf> {
    if let Some(path) = find_binary("codebuddy") {
        return Some(path);
    }
    let unix_home = home_join(".codebuddy/bin/codebuddy");
    if unix_home.is_file() {
        return Some(unix_home);
    }
    let win_home = home_join("AppData/Local/codebuddy/bin/codebuddy.exe");
    if win_home.is_file() {
        return Some(win_home);
    }
    None
}

/// Official WorkBuddy desktop client (`WorkBuddy.app` / `WorkBuddy.exe`).
pub fn workbuddy_desktop_path() -> Option<PathBuf> {
    desktop_app_path(&DESKTOP_APP)
}

pub(crate) fn workbuddy_desktop_installs() -> Vec<PathBuf> {
    desktop_app_candidates(&DESKTOP_APP)
}

pub(crate) fn missing_workbuddy_bundle_reason(
    has_cli: bool,
    has_desktop: bool,
    desktop_required: bool,
) -> Option<&'static str> {
    if !desktop_required {
        return (!has_cli).then_some("WorkBuddy 命令行还没装。");
    }
    match (has_cli, has_desktop) {
        (true, true) => None,
        (false, false) => Some("WorkBuddy 命令行和桌面软件都还没装。"),
        (false, true) => Some("WorkBuddy 桌面软件已在本机，命令行还没装。"),
        (true, false) => Some("WorkBuddy 命令行已装好，桌面软件还没装。"),
    }
}

/// Non-empty when WorkBuddy is missing either its CLI or (on macOS/Windows) its desktop app.
pub fn workbuddy_missing_install_part() -> Option<&'static str> {
    missing_workbuddy_bundle_reason(
        workbuddy_cli_path().is_some(),
        workbuddy_desktop_path().is_some(),
        DESKTOP_REQUIRED,
    )
}

impl RuntimeAdapter for WorkbuddyAdapter {
    fn id(&self) -> &'static str {
        "workbuddy"
    }

    fn display_name(&self) -> &'static str {
        "WorkBuddy"
    }

    fn discover(&self) -> AdapterDiscovery {
        if let Some(cli) = workbuddy_cli_path() {
            return discovery_from_cli(cli);
        }
        if let Some(app) = workbuddy_desktop_path() {
            return discovery_present(app);
        }
        AdapterDiscovery {
            installed: false,
            version: None,
            binary_path: None,
        }
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        vec![
            home_join(".codebuddy/settings.json"),
            home_join(".codebuddy/models.json"),
        ]
    }

    fn read_profile(&self) -> anyhow::Result<RuntimeProfile> {
        let mut profile = profile_from_config_paths(self.config_paths());
        let settings = home_join(".codebuddy/settings.json");
        if let Ok(raw) = std::fs::read_to_string(&settings) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(url) = val
                    .pointer("/env/CODEBUDDY_BASE_URL")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    profile.gateway_url = Some(url.to_string());
                }
            }
        }
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::util::with_test_home;
    use std::fs;

    #[test]
    fn desktop_platforms_require_both_cli_and_desktop_app() {
        assert!(missing_workbuddy_bundle_reason(false, false, true).is_some());
        assert_eq!(
            missing_workbuddy_bundle_reason(true, false, true),
            Some("WorkBuddy 命令行已装好，桌面软件还没装。")
        );
        assert_eq!(
            missing_workbuddy_bundle_reason(false, true, true),
            Some("WorkBuddy 桌面软件已在本机，命令行还没装。")
        );
        assert_eq!(missing_workbuddy_bundle_reason(true, true, true), None);
        assert_eq!(missing_workbuddy_bundle_reason(true, false, false), None);
    }

    #[test]
    fn finds_workbuddy_app_under_home_applications() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("Applications/WorkBuddy.app")).unwrap();
        with_test_home(temp.path(), || {
            assert_eq!(
                workbuddy_desktop_path(),
                Some(home_join("Applications/WorkBuddy.app"))
            );
        });
    }
}
