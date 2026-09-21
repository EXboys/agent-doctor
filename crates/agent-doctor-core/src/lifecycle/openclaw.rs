//! OpenClaw install/update via official installers (https://openclaw.ai/install.sh).
//!
//! Uses `--no-onboard --no-prompt` so repair can install the CLI without launching
//! the interactive onboarding wizard.

use anyhow::{Context, Result};

use super::runner::run_shell_command;

pub const OPENCLAW_INSTALL_SCRIPT_URL: &str = "https://openclaw.ai/install.sh";

#[cfg(target_os = "windows")]
pub const OPENCLAW_INSTALL_PS1_URL: &str = "https://openclaw.ai/install.ps1";

/// Unix install: curl to temp file, then bash (not `curl | bash` — safer under WSL/sub-shells).
const OPENCLAW_INSTALL_UNIX: &str =
    "bash -c 'tmp=$(mktemp) && curl -fsSL --proto \"=https\" --tlsv1.2 \
    https://openclaw.ai/install.sh -o $tmp && bash $tmp --no-onboard --no-prompt; \
    status=$?; rm -f $tmp; exit $status'";

const OPENCLAW_UPDATE_UNIX: &str =
    "openclaw update || bash -c 'tmp=$(mktemp) && curl -fsSL --proto \"=https\" --tlsv1.2 \
    https://openclaw.ai/install.sh -o $tmp && bash $tmp --no-onboard --no-prompt; \
    status=$?; rm -f $tmp; exit $status'";

#[cfg(target_os = "windows")]
const OPENCLAW_INSTALL_WINDOWS: &str = r#"powershell -NoProfile -ExecutionPolicy Bypass -Command "& ([scriptblock]::Create((iwr -useb https://openclaw.ai/install.ps1))) -NoOnboard -NoPrompt""#;

#[cfg(target_os = "windows")]
const OPENCLAW_UPDATE_WINDOWS: &str = r#"openclaw update || powershell -NoProfile -ExecutionPolicy Bypass -Command "& ([scriptblock]::Create((iwr -useb https://openclaw.ai/install.ps1))) -NoOnboard -NoPrompt""#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenClawLifecycleAction {
    Install,
    Update,
}

pub fn openclaw_shell_command(action: OpenClawLifecycleAction) -> String {
    match action {
        OpenClawLifecycleAction::Install => openclaw_install_shell_command(),
        OpenClawLifecycleAction::Update => openclaw_update_shell_command(),
    }
}

pub fn openclaw_install_shell_command() -> String {
    #[cfg(target_os = "windows")]
    {
        OPENCLAW_INSTALL_WINDOWS.to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        OPENCLAW_INSTALL_UNIX.to_string()
    }
}

pub fn openclaw_update_shell_command() -> String {
    #[cfg(target_os = "windows")]
    {
        OPENCLAW_UPDATE_WINDOWS.to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        OPENCLAW_UPDATE_UNIX.to_string()
    }
}

pub fn openclaw_doctor_fix_shell_command() -> &'static str {
    "openclaw doctor --fix --yes --non-interactive"
}

pub fn run_openclaw_doctor_fix() -> Result<()> {
    run_shell_command(openclaw_doctor_fix_shell_command()).context("OpenClaw config repair failed")
}

pub fn run_openclaw_lifecycle(action: OpenClawLifecycleAction) -> Result<()> {
    let command_line = openclaw_shell_command(action);
    run_shell_command(&command_line).with_context(|| {
        format!(
            "OpenClaw {} failed",
            match action {
                OpenClawLifecycleAction::Install => "install",
                OpenClawLifecycleAction::Update => "update",
            }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_install_uses_temp_file_and_no_onboard() {
        let cmd = openclaw_install_shell_command();
        assert!(cmd.contains("mktemp"));
        assert!(cmd.contains("openclaw.ai/install.sh"));
        assert!(cmd.contains("--no-onboard"));
        assert!(cmd.contains("--no-prompt"));
    }

    #[test]
    fn unix_update_tries_cli_first() {
        let cmd = openclaw_update_shell_command();
        assert!(cmd.starts_with("openclaw update"));
        assert!(cmd.contains("||"));
    }
}
