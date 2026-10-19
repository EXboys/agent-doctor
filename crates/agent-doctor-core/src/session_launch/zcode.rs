use std::path::Path;
use std::process::Command;
#[cfg(target_os = "windows")]
use std::process::Stdio;

use anyhow::{Context, Result};

use crate::adapters::{locate_zcode, ZcodeInstall};

use super::terminal::open_in_terminal;
use super::{OpenSessionMethod, OpenSessionReport};

pub(crate) fn open_zcode(cwd: &Path) -> Result<OpenSessionReport> {
    match locate_zcode() {
        Some(ZcodeInstall::App(path)) => open_zcode_app(&path, cwd),
        Some(ZcodeInstall::Cli(path)) => {
            let owned = path.to_string_lossy().into_owned();
            open_in_terminal("zcode", &[&owned], cwd, None)
        }
        None => open_in_terminal("zcode", &["zcode"], cwd, None),
    }
}

fn open_zcode_app(app: &Path, cwd: &Path) -> Result<OpenSessionReport> {
    launch_zcode_app(app)?;
    Ok(OpenSessionReport {
        runtime: "zcode".into(),
        method: OpenSessionMethod::App,
        cwd: cwd.display().to_string(),
        target: "ZCode".into(),
        detail: "Opened ZCode.".into(),
    })
}

fn launch_zcode_app(app: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open")
            .arg(app)
            .status()
            .context("failed to open ZCode")?;
        if !status.success() {
            anyhow::bail!("ZCode is on this computer but did not open");
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        Command::new(app)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .context("failed to start ZCode")?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        Command::new(app).spawn().context("failed to start ZCode")?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = app;
        anyhow::bail!("opening ZCode is not supported on this platform")
    }
}
