use std::path::{Path, PathBuf};

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{find_binary, home_join, read_version_result};

pub struct ZcodeAdapter;

/// ZCode is the desktop app from zai-org/ZCode. A terminal `zcode` is optional.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ZcodeInstall {
    App(PathBuf),
    Cli(PathBuf),
}

pub(crate) fn locate_zcode() -> Option<ZcodeInstall> {
    if let Some(app) = desktop_app_path() {
        return Some(ZcodeInstall::App(app));
    }
    cli_path().map(ZcodeInstall::Cli)
}

fn cli_path() -> Option<PathBuf> {
    if let Some(path) = find_binary("zcode") {
        return Some(path);
    }
    for rel in [".local/bin/zcode", ".local/bin/ZCode.AppImage"] {
        let path = home_join(rel);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn discovery_from_cli(path: PathBuf) -> AdapterDiscovery {
    if is_app_image(&path) {
        return discovery_present(path);
    }
    let version = read_version_result(&path).ok().flatten();
    AdapterDiscovery {
        installed: true,
        version,
        binary_path: Some(path),
    }
}

fn is_app_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("AppImage"))
}

fn discovery_present(path: PathBuf) -> AdapterDiscovery {
    AdapterDiscovery {
        installed: true,
        version: None,
        binary_path: Some(path),
    }
}

fn desktop_app_candidates() -> Vec<PathBuf> {
    let mut paths = vec![home_join("Applications/ZCode.app")];
    paths.extend(system_desktop_app_candidates());
    paths
}

#[cfg(test)]
fn system_desktop_app_candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(all(not(test), target_os = "macos"))]
fn system_desktop_app_candidates() -> Vec<PathBuf> {
    vec![PathBuf::from("/Applications/ZCode.app")]
}

#[cfg(all(not(test), target_os = "windows"))]
fn system_desktop_app_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        paths.push(local.join("Programs/ZCode/ZCode.exe"));
        paths.push(local.join("Programs/zcode/ZCode.exe"));
    }
    for key in ["ProgramFiles", "ProgramW6432"] {
        if let Some(root) = std::env::var_os(key) {
            paths.push(PathBuf::from(root).join("ZCode/ZCode.exe"));
        }
    }
    paths
}

#[cfg(all(not(test), target_os = "linux"))]
fn system_desktop_app_candidates() -> Vec<PathBuf> {
    vec![home_join(".local/bin/ZCode.AppImage")]
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

impl RuntimeAdapter for ZcodeAdapter {
    fn id(&self) -> &'static str {
        "zcode"
    }

    fn display_name(&self) -> &'static str {
        "ZCode"
    }

    fn discover(&self) -> AdapterDiscovery {
        match locate_zcode() {
            Some(ZcodeInstall::App(path)) => discovery_present(path),
            Some(ZcodeInstall::Cli(path)) => discovery_from_cli(path),
            None => AdapterDiscovery {
                installed: false,
                version: None,
                binary_path: None,
            },
        }
    }

    fn config_paths(&self) -> Vec<PathBuf> {
        vec![home_join(".zcode/cli/config.json")]
    }

    fn config_paths_required(&self) -> bool {
        false
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::util::with_test_home;
    use std::fs;

    #[test]
    fn user_applications_app_counts_as_installed() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("Applications/ZCode.app")).expect("app");
        with_test_home(temp.path(), || {
            let found = ZcodeAdapter.discover();
            assert!(found.installed);
            assert_eq!(found.binary_path, Some(home_join("Applications/ZCode.app")));
        });
    }

    #[test]
    fn home_cli_counts_when_no_app() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin = temp.path().join(".local/bin/zcode");
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        fs::write(&bin, b"").unwrap();
        with_test_home(temp.path(), || {
            if find_binary("zcode").is_some() || desktop_app_path().is_some() {
                return;
            }
            let found = ZcodeAdapter.discover();
            assert!(found.installed);
            assert_eq!(found.binary_path, Some(bin));
        });
    }

    #[test]
    fn app_wins_over_cli() {
        let app = PathBuf::from("/Applications/ZCode.app");
        let cli = PathBuf::from("/tmp/zcode");
        assert_eq!(
            choose_for_test(Some(app.clone()), Some(cli)),
            Some(ZcodeInstall::App(app))
        );
    }

    fn choose_for_test(app: Option<PathBuf>, cli: Option<PathBuf>) -> Option<ZcodeInstall> {
        if let Some(app) = app {
            return Some(ZcodeInstall::App(app));
        }
        cli.map(ZcodeInstall::Cli)
    }
}
