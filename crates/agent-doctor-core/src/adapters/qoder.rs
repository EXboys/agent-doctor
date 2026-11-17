use std::path::{Path, PathBuf};

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{
    desktop_app_candidates, desktop_app_path, discovery_from_cli, discovery_present, find_binary,
    home_join, profile_from_config_paths, DesktopAppSpec,
};

pub struct QoderAdapter;

/// Where a Qoder install actually lives on this computer.
///
/// The current installer publishes `qodercli` (symlink in `~/.local/bin`,
/// binary under `~/.qoder/bin/qodercli/`). Older installs still use `qoder`.
/// The desktop IDE is a separate app. Current CN builds ship as
/// `Qoder CN IDE.app`; older names (`Qoder CN.app` / `Qoder IDE.app` /
/// `Qoder.app`) still count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QoderInstall {
    Cli(PathBuf),
    App(PathBuf),
}

const DESKTOP_APP: DesktopAppSpec = DesktopAppSpec {
    macos_apps: &[
        "Qoder CN IDE.app",
        "Qoder CN.app",
        "Qoder IDE.app",
        "Qoder.app",
    ],
    windows_local: &[
        "Programs/Qoder CN IDE/Qoder CN IDE.exe",
        "Programs/Qoder CN/Qoder CN.exe",
        "Programs/Qoder IDE/Qoder IDE.exe",
        "Programs/Qoder/Qoder.exe",
    ],
    windows_machine: &[
        "Qoder CN IDE/Qoder CN IDE.exe",
        "Qoder CN/Qoder CN.exe",
        "Qoder IDE/Qoder IDE.exe",
        "Qoder/Qoder.exe",
    ],
    linux_paths: &[
        "/usr/share/qoder-cn-ide/qoder-cn-ide",
        "/usr/share/qoder-ide/qoder-ide",
        "/usr/bin/qoder-ide",
        "/opt/Qoder/qoder",
        "/usr/share/qoder/qoder",
        "~/.local/share/qoder-ide/qoder-ide",
    ],
};

const DESKTOP_REQUIRED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

pub(crate) fn locate_qoder() -> Option<QoderInstall> {
    choose_qoder_install(cli_on_path(), versioned_cli_path(), qoder_desktop_path())
}

/// Official Qoder command line. The desktop app does not count.
pub fn qoder_cli_path() -> Option<PathBuf> {
    match locate_qoder() {
        Some(QoderInstall::Cli(path)) => Some(path),
        _ => None,
    }
}

/// Official Qoder desktop IDE (`Qoder CN IDE` / legacy `Qoder CN` / `Qoder IDE` / `Qoder`).
pub fn qoder_desktop_path() -> Option<PathBuf> {
    desktop_app_path(&DESKTOP_APP)
}

pub(crate) fn qoder_desktop_installs() -> Vec<PathBuf> {
    desktop_app_candidates(&DESKTOP_APP)
}

pub(crate) fn missing_qoder_bundle_reason(
    has_cli: bool,
    has_desktop: bool,
    desktop_required: bool,
) -> Option<&'static str> {
    if !desktop_required {
        return (!has_cli).then_some("Qoder 命令行还没装。");
    }
    match (has_cli, has_desktop) {
        (true, true) => None,
        (false, false) => Some("Qoder 命令行和桌面软件都还没装。"),
        (false, true) => Some("Qoder 桌面软件已在本机，命令行还没装。"),
        (true, false) => Some("Qoder 命令行已装好，桌面软件还没装。"),
    }
}

/// Non-empty when Qoder is missing either its CLI or (on macOS/Windows) its desktop app.
pub fn qoder_missing_install_part() -> Option<&'static str> {
    missing_qoder_bundle_reason(
        qoder_cli_path().is_some(),
        qoder_desktop_path().is_some(),
        DESKTOP_REQUIRED,
    )
}

fn choose_qoder_install(
    cli: Option<PathBuf>,
    versioned: Option<PathBuf>,
    app: Option<PathBuf>,
) -> Option<QoderInstall> {
    if let Some(path) = cli.or(versioned) {
        return Some(QoderInstall::Cli(path));
    }
    app.map(QoderInstall::App)
}

fn cli_on_path() -> Option<PathBuf> {
    // Current installer name first. `qoder` is the older command.
    for name in ["qodercli", "qoderclicn", "qoder"] {
        if let Some(path) = find_binary(name) {
            return Some(path);
        }
    }
    None
}

fn versioned_cli_path() -> Option<PathBuf> {
    let root = home_join(".qoder/bin");
    let mut found = Vec::new();
    collect_cli_files(&root, &mut found);
    collect_cli_files(&root.join("qodercli"), &mut found);
    collect_cli_files(&root.join("qoderclicn"), &mut found);
    collect_cli_files(&root.join("qoder"), &mut found);
    found.sort();
    found.pop()
}

fn collect_cli_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if looks_like_qoder_cli(&path) {
            out.push(path);
        }
    }
}

fn looks_like_qoder_cli(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name.starts_with('.') {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    // Versioned names look like `qodercli-0.2.7`, so a dotted suffix is not an
    // extension. Only drop known non-binaries and Windows script suffixes.
    const SKIP_SUFFIX: &[&str] = &[".sha256", ".json", ".txt", ".md", ".log", ".sig"];
    if SKIP_SUFFIX.iter().any(|suffix| lower.ends_with(suffix)) {
        return false;
    }
    let stem = lower
        .strip_suffix(".exe")
        .or_else(|| lower.strip_suffix(".cmd"))
        .or_else(|| lower.strip_suffix(".bat"))
        .unwrap_or(&lower);
    stem == "qoder"
        || stem == "qodercli"
        || stem == "qoderclicn"
        || stem.starts_with("qodercli-")
        || stem.starts_with("qoderclicn-")
        || stem.starts_with("qoder-")
}

impl RuntimeAdapter for QoderAdapter {
    fn id(&self) -> &'static str {
        "qoder"
    }

    fn display_name(&self) -> &'static str {
        "Qoder"
    }

    fn discover(&self) -> AdapterDiscovery {
        match locate_qoder() {
            Some(QoderInstall::Cli(path)) => discovery_from_cli(path),
            Some(QoderInstall::App(path)) => discovery_present(path),
            None => AdapterDiscovery {
                installed: false,
                version: None,
                binary_path: None,
            },
        }
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        vec![home_join(".qoder/settings.json")]
    }

    fn read_profile(&self) -> anyhow::Result<RuntimeProfile> {
        let mut profile = profile_from_config_paths(self.config_paths());
        let settings = home_join(".qoder/settings.json");
        if let Ok(raw) = std::fs::read_to_string(&settings) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(url) = val
                    .pointer("/providers/agent-doctor/baseUrl")
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
    fn cli_wins_over_versioned_binary_and_app() {
        let cli = PathBuf::from("/tmp/qodercli");
        let versioned = PathBuf::from("/tmp/.qoder/bin/qodercli/qodercli-0.2.7");
        let app = PathBuf::from("/Applications/Qoder IDE.app");
        assert_eq!(
            choose_qoder_install(Some(cli.clone()), Some(versioned), Some(app)),
            Some(QoderInstall::Cli(cli))
        );
    }

    #[test]
    fn versioned_binary_wins_over_app() {
        let versioned = PathBuf::from("/tmp/.qoder/bin/qodercli/qodercli-0.2.7");
        let app = PathBuf::from("/Applications/Qoder.app");
        assert_eq!(
            choose_qoder_install(None, Some(versioned.clone()), Some(app)),
            Some(QoderInstall::Cli(versioned))
        );
    }

    #[test]
    fn app_counts_when_no_cli_for_locate() {
        let app = PathBuf::from("/Applications/Qoder IDE.app");
        assert_eq!(
            choose_qoder_install(None, None, Some(app.clone())),
            Some(QoderInstall::App(app))
        );
    }

    #[test]
    fn versioned_qodercli_is_found_under_home() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin = temp.path().join(".qoder/bin/qodercli/qodercli-0.2.7");
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        fs::write(&bin, b"").unwrap();
        let marker = bin.parent().unwrap().join(".qodercli-install-resource");
        fs::write(&marker, b"script").unwrap();
        fs::write(bin.parent().unwrap().join("qodercli-0.2.7.sha256"), b"abc").unwrap();
        with_test_home(temp.path(), || {
            assert_eq!(versioned_cli_path(), Some(bin));
        });
    }

    #[test]
    fn legacy_home_binary_is_found() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin = temp.path().join(".qoder/bin/qoder");
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        fs::write(&bin, b"").unwrap();
        with_test_home(temp.path(), || {
            assert_eq!(versioned_cli_path(), Some(bin));
        });
    }

    #[test]
    fn desktop_platforms_require_both_cli_and_desktop_app() {
        assert!(missing_qoder_bundle_reason(false, false, true).is_some());
        assert_eq!(
            missing_qoder_bundle_reason(true, false, true),
            Some("Qoder 命令行已装好，桌面软件还没装。")
        );
        assert_eq!(
            missing_qoder_bundle_reason(false, true, true),
            Some("Qoder 桌面软件已在本机，命令行还没装。")
        );
        assert_eq!(missing_qoder_bundle_reason(true, true, true), None);
        assert_eq!(missing_qoder_bundle_reason(true, false, false), None);
    }

    #[test]
    fn qoder_desktop_apps_are_found_under_home() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("Applications/Qoder IDE.app")).unwrap();
        with_test_home(temp.path(), || {
            assert_eq!(
                qoder_desktop_path(),
                Some(home_join("Applications/Qoder IDE.app"))
            );
            assert!(QoderAdapter.discover().installed);
            if DESKTOP_REQUIRED && cli_on_path().is_none() {
                assert!(qoder_missing_install_part().is_some());
            }
        });
    }

    #[test]
    fn qoder_cn_ide_app_wins_over_legacy_apps() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("Applications/Qoder CN IDE.app")).unwrap();
        fs::create_dir_all(temp.path().join("Applications/Qoder CN.app")).unwrap();
        fs::create_dir_all(temp.path().join("Applications/Qoder IDE.app")).unwrap();
        fs::create_dir_all(temp.path().join("Applications/Qoder.app")).unwrap();
        with_test_home(temp.path(), || {
            assert_eq!(
                qoder_desktop_path(),
                Some(home_join("Applications/Qoder CN IDE.app"))
            );
            assert_eq!(missing_qoder_bundle_reason(true, true, true), None);
        });
    }

    #[test]
    fn legacy_qoder_cn_app_still_counts() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("Applications/Qoder CN.app")).unwrap();
        with_test_home(temp.path(), || {
            assert_eq!(
                qoder_desktop_path(),
                Some(home_join("Applications/Qoder CN.app"))
            );
        });
    }
}
