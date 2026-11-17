//! Open the official desktop window for dual-install runtimes.

use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(target_os = "windows")]
use std::process::Stdio;

use anyhow::{bail, Context, Result};

use super::{OpenSessionMethod, OpenSessionReport};

pub(crate) const DESKTOP_APP_MISSING: &str = "这台电脑上还没有它的桌面软件。";

/// Path to the desktop app for runtimes that ship one alongside the CLI.
pub fn runtime_desktop_app_path(runtime: &str) -> Option<PathBuf> {
    match runtime {
        "cursor" => crate::adapters::cursor_desktop_path(),
        "qoder" => crate::adapters::qoder_desktop_path(),
        "workbuddy" => crate::adapters::workbuddy_desktop_path(),
        "zcode" => crate::adapters::zcode_desktop_path(),
        _ => None,
    }
}

pub fn open_runtime_desktop_app(runtime: &str) -> Result<OpenSessionReport> {
    let Some(path) = runtime_desktop_app_path(runtime) else {
        bail!(DESKTOP_APP_MISSING);
    };
    open_desktop_app(runtime, &path)
}

pub(crate) fn open_desktop_app(runtime: &str, path: &Path) -> Result<OpenSessionReport> {
    if !path.exists() {
        bail!(DESKTOP_APP_MISSING);
    }
    launch_desktop_path(path)?;
    Ok(OpenSessionReport {
        runtime: runtime.into(),
        method: OpenSessionMethod::App,
        cwd: String::new(),
        target: path.display().to_string(),
        detail: format!("opened desktop app {}", path.display()),
    })
}

fn launch_desktop_path(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open") // raw-spawn-ok
            .arg(path)
            .status()
            .context("failed to open desktop app")?;
        if !status.success() {
            bail!("`open` exited with {status}");
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        Command::new("cmd") // raw-spawn-ok
            .args(["/C", "start", "", &path.to_string_lossy()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .context("failed to open desktop app")?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let status = Command::new("xdg-open") // raw-spawn-ok
            .arg(path)
            .status()
            .context("failed to open desktop app")?;
        if !status.success() {
            bail!("`xdg-open` exited with {status}");
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = path;
        bail!("opening desktop apps is not supported on this computer");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dual_runtimes_look_up_a_desktop_app() {
        for id in ["codex", "claude-code", "openclaw", "hermes"] {
            assert_eq!(runtime_desktop_app_path(id), None, "{id}");
        }
        // Presence depends on the machine; the lookup itself must not panic.
        let _ = runtime_desktop_app_path("cursor");
        let _ = runtime_desktop_app_path("qoder");
        let _ = runtime_desktop_app_path("workbuddy");
        let _ = runtime_desktop_app_path("zcode");
    }

    #[test]
    fn missing_app_uses_beginner_copy() {
        let err = open_desktop_app("qoder", Path::new("/tmp/no-such-qoder-app.app")).unwrap_err();
        assert!(err.to_string().contains("桌面软件"));
    }
}
