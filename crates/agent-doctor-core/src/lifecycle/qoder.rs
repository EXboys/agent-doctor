//! Qoder native installer (https://qoder.com/install).

use anyhow::{Context, Result};

use super::runner::run_shell_command;

pub const QODER_INSTALL_SCRIPT_URL: &str = "https://qoder.com/install";

#[cfg(target_os = "windows")]
pub const QODER_INSTALL_PS1_URL: &str = "https://qoder.com/install.ps1";

const QODER_INSTALL_UNIX: &str =
    "bash -c 'tmp=$(mktemp) && curl -fsSL --proto \"=https\" --tlsv1.2 \
    https://qoder.com/install -o $tmp && bash $tmp; \
    status=$?; rm -f $tmp; exit $status'";

const QODER_UPDATE_UNIX: &str =
    "qoder update || bash -c 'tmp=$(mktemp) && curl -fsSL --proto \"=https\" --tlsv1.2 \
    https://qoder.com/install -o $tmp && bash $tmp --force; \
    status=$?; rm -f $tmp; exit $status'";

#[cfg(target_os = "windows")]
const QODER_INSTALL_WINDOWS: &str = r#"powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://qoder.com/install.ps1 | iex""#;

#[cfg(target_os = "windows")]
const QODER_UPDATE_WINDOWS: &str = r#"qoder update || powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://qoder.com/install.ps1 | iex""#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QoderLifecycleAction {
    Install,
    Update,
}

pub fn qoder_install_shell_command() -> String {
    #[cfg(target_os = "windows")]
    {
        QODER_INSTALL_WINDOWS.to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        QODER_INSTALL_UNIX.to_string()
    }
}

pub fn qoder_update_shell_command() -> String {
    #[cfg(target_os = "windows")]
    {
        QODER_UPDATE_WINDOWS.to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        QODER_UPDATE_UNIX.to_string()
    }
}

pub fn run_qoder_lifecycle(action: QoderLifecycleAction) -> Result<()> {
    let command = match action {
        QoderLifecycleAction::Install => qoder_install_shell_command(),
        QoderLifecycleAction::Update => qoder_update_shell_command(),
    };
    run_shell_command(&command).with_context(|| {
        format!(
            "Qoder {} failed",
            match action {
                QoderLifecycleAction::Install => "install",
                QoderLifecycleAction::Update => "update",
            }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_uses_official_native_script() {
        let cmd = qoder_install_shell_command();
        assert!(cmd.contains("qoder.com/install"));
        assert!(!cmd.contains("npm install"));
    }
}
