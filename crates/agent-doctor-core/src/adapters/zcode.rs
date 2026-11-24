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
    linux_paths: &[
        "~/.local/bin/ZCode.AppImage",
        "/opt/ZCode/zcode",
        "/usr/share/zcode/zcode",
    ],
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

/// Official ZCode desktop app (`ZCode.app` / `ZCode.exe` / AppImage).
pub fn zcode_desktop_path() -> Option<PathBuf> {
    crate::adapters::util::desktop_app_path(&DESKTOP_APP)
}

fn desktop_app_path() -> Option<PathBuf> {
    zcode_desktop_path()
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
        if let Some(headless) = headless_from_cli_candidate(&path) {
            return Some(headless);
        }
    }
    desktop_app_path().and_then(|app| headless_inside_app(&app))
}

fn headless_from_cli_candidate(path: &Path) -> Option<ZcodeHeadless> {
    if is_app_image(path) {
        return None;
    }
    // `ZCode.exe` on Windows (or `/usr/bin/zcode -> /opt/ZCode/zcode` on Linux)
    // is the Electron desktop binary, not a standalone CLI: running it without
    // `ELECTRON_RUN_AS_NODE=1` and `resources/glm/zcode.cjs` hits the desktop
    // single-instance lock and exits immediately without printing a login URL.
    if let Some(bundled) = headless_inside_app(path) {
        return Some(bundled);
    }
    if crate::exec::skips_version_probe(path) {
        return None;
    }
    Some(ZcodeHeadless {
        program: path.to_path_buf(),
        script: None,
    })
}

/// Locate the bundled `zcode-builtin.json` next to `zcode.cjs` inside a desktop install.
pub fn zcode_bundled_provider_config(script: &Path) -> Option<PathBuf> {
    let dir = script.parent()?;
    [
        dir.join("provider").join("zcode-builtin.json"),
        dir.join("../config/provider/zcode-builtin.json"),
        dir.join("../../../../../config/provider/zcode-builtin.json"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

/// Build a ready-to-add-args `Command` for either standalone `zcode` or bundled `zcode.cjs`.
pub fn zcode_headless_command(launch: &ZcodeHeadless) -> std::process::Command {
    let mut cmd = crate::exec::command_for_path(&launch.program);
    cmd.env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .env_remove("ZCODE_PERSONAL_PROVIDER_CONFIG_FILE");
    if let Some(script) = &launch.script {
        cmd.env("ELECTRON_RUN_AS_NODE", "1");
        cmd.arg(script);
        if let Some(builtin) = zcode_bundled_provider_config(script) {
            cmd.env("ZCODE_BUILTIN_PROVIDER_CONFIG_FILE", builtin);
        }
    }
    cmd
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
    if app
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
    {
        // The main app binary shows up in the Dock as a blank "exec" window.
        // ZCode itself asks through the helper, which stays in the background.
        let program = macos_headless_program(app);
        let script = app.join("Contents/Resources/glm/zcode.cjs");
        return (program.is_file() && script.is_file()).then_some(ZcodeHeadless {
            program,
            script: Some(script),
        });
    }
    #[cfg(unix)]
    let resolved = if app
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        std::fs::canonicalize(app).unwrap_or_else(|_| app.to_path_buf())
    } else {
        app.to_path_buf()
    };
    #[cfg(not(unix))]
    let resolved = app.to_path_buf();
    if is_app_image(&resolved) || !resolved.is_file() {
        return None;
    }
    let parent = resolved.parent()?;
    let script = parent.join("resources").join("glm").join("zcode.cjs");
    script.is_file().then_some(ZcodeHeadless {
        program: resolved,
        script: Some(script),
    })
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

    #[test]
    fn desktop_exe_on_path_resolves_bundled_node_script_instead_of_gui_launch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("Programs/ZCode");
        let exe = dir.join("ZCode.exe");
        let script = dir.join("resources/glm/zcode.cjs");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(&exe, b"").unwrap();
        fs::write(&script, b"").unwrap();

        let found = headless_from_cli_candidate(&exe).expect("bundled headless");
        assert_eq!(found.program, exe);
        assert_eq!(found.script.as_deref(), Some(script.as_path()));
    }

    #[test]
    fn gui_subsystem_exe_without_bundled_script_is_not_treated_as_cli() {
        let temp = tempfile::tempdir().expect("tempdir");
        let exe = temp.path().join("ZCode.exe");
        // Minimal PE header with IMAGE_SUBSYSTEM_WINDOWS_GUI (2) at optional+68.
        let mut pe = vec![0_u8; 0x80 + 4 + 20 + 70];
        pe[0] = b'M';
        pe[1] = b'Z';
        pe[0x3C..0x40].copy_from_slice(&(0x80_u32).to_le_bytes());
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        let optional = 0x80 + 4 + 20;
        pe[optional..optional + 2].copy_from_slice(&0x20B_u16.to_le_bytes());
        pe[optional + 68..optional + 70].copy_from_slice(&2_u16.to_le_bytes());
        fs::write(&exe, pe).unwrap();

        assert_eq!(headless_from_cli_candidate(&exe), None);
    }
}
