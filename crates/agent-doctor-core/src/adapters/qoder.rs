use std::path::{Path, PathBuf};

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{find_binary, home_join, read_version_result};

pub struct QoderAdapter;

/// Where a Qoder install actually lives on this computer.
///
/// The current installer publishes `qodercli` (symlink in `~/.local/bin`,
/// binary under `~/.qoder/bin/qodercli/`). Older installs still use `qoder`.
/// The desktop IDE is a separate app: `Qoder.app`, and since 1.25 `Qoder IDE.app`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QoderInstall {
    Cli(PathBuf),
    App(PathBuf),
}

pub(crate) fn locate_qoder() -> Option<QoderInstall> {
    choose_qoder_install(cli_on_path(), versioned_cli_path(), desktop_app_path())
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
    for name in ["qodercli", "qoder"] {
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
        || stem.starts_with("qodercli-")
        || stem.starts_with("qoder-")
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
    let mut paths = vec![
        home_join("Applications/Qoder IDE.app"),
        home_join("Applications/Qoder.app"),
    ];
    paths.extend(system_desktop_app_candidates());
    paths
}

#[cfg(test)]
fn system_desktop_app_candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(all(not(test), target_os = "macos"))]
fn system_desktop_app_candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications/Qoder IDE.app"),
        PathBuf::from("/Applications/Qoder.app"),
    ]
}

#[cfg(all(not(test), target_os = "windows"))]
fn system_desktop_app_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        paths.push(local.join("Programs/Qoder IDE/Qoder IDE.exe"));
        paths.push(local.join("Programs/Qoder/Qoder.exe"));
    }
    for key in ["ProgramFiles", "ProgramW6432"] {
        if let Some(root) = std::env::var_os(key) {
            let root = PathBuf::from(root);
            paths.push(root.join("Qoder IDE/Qoder IDE.exe"));
            paths.push(root.join("Qoder/Qoder.exe"));
        }
    }
    paths
}

#[cfg(all(not(test), target_os = "linux"))]
fn system_desktop_app_candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/share/qoder-ide/qoder-ide"),
        PathBuf::from("/usr/bin/qoder-ide"),
        PathBuf::from("/opt/Qoder/qoder"),
        PathBuf::from("/usr/share/qoder/qoder"),
        home_join(".local/share/qoder-ide/qoder-ide"),
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
    fn app_counts_when_no_cli() {
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
    fn qoder_ide_app_counts_as_installed() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("Applications/Qoder IDE.app")).unwrap();
        with_test_home(temp.path(), || {
            assert_eq!(
                desktop_app_path(),
                Some(home_join("Applications/Qoder IDE.app"))
            );
            let found = QoderAdapter.discover();
            assert!(found.installed);
            if cli_on_path().is_none() {
                assert_eq!(
                    found.binary_path,
                    Some(home_join("Applications/Qoder IDE.app"))
                );
            }
        });
    }

    #[test]
    fn legacy_qoder_app_counts_as_installed() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("Applications/Qoder.app")).unwrap();
        with_test_home(temp.path(), || {
            assert_eq!(
                desktop_app_path(),
                Some(home_join("Applications/Qoder.app"))
            );
        });
    }

    #[test]
    fn current_app_name_wins_over_legacy_app() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("Applications/Qoder IDE.app")).unwrap();
        fs::create_dir_all(temp.path().join("Applications/Qoder.app")).unwrap();
        with_test_home(temp.path(), || {
            assert_eq!(
                desktop_app_path(),
                Some(home_join("Applications/Qoder IDE.app"))
            );
        });
    }
}
