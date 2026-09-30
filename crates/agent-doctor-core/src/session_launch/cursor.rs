use std::path::Path;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::path::PathBuf;
use std::process::Command;
#[cfg(target_os = "windows")]
use std::process::Stdio;

use anyhow::{bail, Context, Result};

#[cfg(windows)]
use crate::profile::read_company_profile;
#[cfg(windows)]
use crate::setup::anthropic_gateway_url_from_evotown_base;

use super::*;

pub(crate) fn open_cursor_app(cwd: &Path) -> Result<OpenSessionReport> {
    launch_cursor_app()?;
    Ok(OpenSessionReport {
        runtime: "cursor".into(),
        method: OpenSessionMethod::App,
        cwd: cwd.display().to_string(),
        target: "Cursor".into(),
        detail: "Opened Cursor.".into(),
    })
}

pub(crate) fn launch_cursor_app() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open")
            .args(["-a", "Cursor"])
            .status()
            .context("failed to open Cursor")?;
        if status.success() {
            return Ok(());
        }
        let app = PathBuf::from("/Applications/Cursor.app");
        if app.exists() {
            let status = Command::new("open")
                .arg(&app)
                .status()
                .context("failed to open Cursor.app")?;
            if status.success() {
                return Ok(());
            }
        }
        bail!("Cursor is on this computer but did not open");
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            for rel in ["Programs/cursor/Cursor.exe", "Programs/Cursor/Cursor.exe"] {
                let exe = PathBuf::from(&local).join(rel);
                if exe.is_file() {
                    Command::new(&exe)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .creation_flags(CREATE_NO_WINDOW)
                        .spawn()
                        .context("failed to start Cursor")?;
                    return Ok(());
                }
            }
        }
        Command::new("cmd")
            .args(["/C", "start", "", "cursor"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .context("failed to start Cursor")?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        if Command::new("cursor")
            .spawn()
            .map(|_| true)
            .unwrap_or(false)
        {
            return Ok(());
        }
        let status = Command::new("xdg-open")
            .arg("cursor://")
            .status()
            .context("failed to open Cursor")?;
        if status.success() {
            return Ok(());
        }
        bail!("Cursor is on this computer but did not open");
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        bail!("opening Cursor is not supported on this platform");
    }
}
