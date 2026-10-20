use std::path::Path;
use std::process::Command;
#[cfg(target_os = "windows")]
use std::process::Stdio;

use anyhow::{Context, Result};

use crate::adapters::{locate_qoder, QoderInstall};

use super::terminal::open_in_terminal;
use super::{OpenSessionMethod, OpenSessionReport};

pub(crate) fn open_qoder(cwd: &Path, prompt: Option<&str>) -> Result<OpenSessionReport> {
    match locate_qoder() {
        Some(QoderInstall::Cli(path)) => {
            let owned = path.to_string_lossy().into_owned();
            open_in_terminal("qoder", &[&owned], cwd, prompt)
        }
        Some(QoderInstall::App(path)) => open_qoder_app(&path, cwd),
        None => open_in_terminal("qoder", &["qodercli"], cwd, prompt),
    }
}

fn open_qoder_app(app: &Path, cwd: &Path) -> Result<OpenSessionReport> {
    launch_qoder_app(app)?;
    Ok(OpenSessionReport {
        runtime: "qoder".into(),
        method: OpenSessionMethod::App,
        cwd: cwd.display().to_string(),
        target: "Qoder".into(),
        detail: "Opened Qoder.".into(),
    })
}

fn launch_qoder_app(app: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open") // raw-spawn-ok
            .arg(app)
            .status()
            .context("failed to open Qoder")?;
        if !status.success() {
            anyhow::bail!("Qoder is on this computer but did not open");
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        Command::new(app) // raw-spawn-ok
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .context("failed to start Qoder")?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        Command::new(app).spawn().context("failed to start Qoder")?; // raw-spawn-ok
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = app;
        anyhow::bail!("opening Qoder is not supported on this platform")
    }
}
