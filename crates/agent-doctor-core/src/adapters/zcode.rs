use std::path::{Path, PathBuf};

use crate::adapter::{AdapterDiscovery, RuntimeAdapter, RuntimeProfile};
use crate::adapters::util::{
    discovery_present, find_binary, home_join, profile_from_config_paths, DesktopAppSpec,
};

pub struct ZcodeAdapter;

const DESKTOP_APP: DesktopAppSpec = DesktopAppSpec {
    macos_apps: &["ZCode.app"],
    windows_local: &["Programs/ZCode/ZCode.exe", "Programs/zcode/ZCode.exe"],
    windows_machine: &["ZCode/ZCode.exe"],
    linux_paths: &["~/.local/bin/ZCode.AppImage"],
};

/// ZCode is the desktop app from zai-org/ZCode. A terminal `zcode` is optional.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZcodeInstall {
    App(PathBuf),
    Cli(PathBuf),
}

pub fn locate_zcode() -> Option<ZcodeInstall> {
    if let Some(app) = desktop_app_path() {
        return Some(ZcodeInstall::App(app));
    }
    cli_path().map(ZcodeInstall::Cli)
}

fn desktop_app_path() -> Option<PathBuf> {
    crate::adapters::util::desktop_app_path(&DESKTOP_APP)
}

/// How to start a headless `zcode --prompt` without opening the window.
///
/// A `zcode` on PATH is the CLI. The desktop install keeps the same script inside
/// the app (`Contents/Resources/glm/zcode.cjs`), run by the app binary as Node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZcodeHeadless {
    pub program: PathBuf,
    pub script: Option<PathBuf>,
}

pub fn locate_zcode_headless() -> Option<ZcodeHeadless> {
    if let Some(path) = cli_path() {
        if !is_app_image(&path) {
            return Some(ZcodeHeadless {
                program: path,
                script: None,
            });
        }
    }
    desktop_app_path().and_then(|app| headless_inside_app(&app))
}

fn macos_headless_program(app: &Path) -> PathBuf {
    let helper = app.join("Contents/Frameworks/ZCode Helper.app/Contents/MacOS/ZCode Helper");
    if helper.is_file() {
        helper
    } else {
        app.join("Contents/MacOS/ZCode")
    }
}

fn headless_inside_app(app: &Path) -> Option<ZcodeHeadless> {
    let (program, script) = if app
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
    {
        (
            // The main app binary shows up in the Dock as a blank "exec" window.
            // ZCode itself asks through the helper, which stays in the background.
            macos_headless_program(app),
            app.join("Contents/Resources/glm/zcode.cjs"),
        )
    } else if app
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        let parent = app.parent()?;
        (
            app.to_path_buf(),
            parent.join("resources").join("glm").join("zcode.cjs"),
        )
    } else {
        return None;
    };
    if program.is_file() && script.is_file() {
        Some(ZcodeHeadless {
            program,
            script: Some(script),
        })
    } else {
        None
    }
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
    crate::adapters::util::discovery_from_cli(path)
}

fn is_app_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("AppImage"))
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
        Ok(profile_from_config_paths(self.config_paths()))
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

    #[test]
    fn bundled_app_script_is_the_headless_cli() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app = temp.path().join("Applications/ZCode.app");
        let program = app.join("Contents/MacOS/ZCode");
        let script = app.join("Contents/Resources/glm/zcode.cjs");
        fs::create_dir_all(program.parent().unwrap()).unwrap();
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(&program, b"").unwrap();
        fs::write(&script, b"").unwrap();
        with_test_home(temp.path(), || {
            if find_binary("zcode").is_some() {
                return;
            }
            let found = locate_zcode_headless().expect("headless");
            assert_eq!(found.program, program);
            assert_eq!(found.script.as_deref(), Some(script.as_path()));
        });
    }

    #[test]
    fn headless_uses_the_background_helper_when_present() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app = temp.path().join("Applications/ZCode.app");
        let main = app.join("Contents/MacOS/ZCode");
        let helper = app.join("Contents/Frameworks/ZCode Helper.app/Contents/MacOS/ZCode Helper");
        let script = app.join("Contents/Resources/glm/zcode.cjs");
        for path in [&main, &helper, &script] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"").unwrap();
        }
        with_test_home(temp.path(), || {
            if find_binary("zcode").is_some() {
                return;
            }
            let found = locate_zcode_headless().expect("headless");
            assert_eq!(found.program, helper);
        });
    }
}
