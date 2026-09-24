//! WorkBuddy / CodeBuddy native installer.

use anyhow::{Context, Result};

use super::download_route::use_china_mirrors;
use super::runner::run_shell_command;

pub const WORKBUDDY_INSTALL_SCRIPT_URL: &str = "https://www.codebuddy.cn/cli/install.sh";
pub const WORKBUDDY_INSTALL_SCRIPT_URL_INTL: &str = "https://copilot.tencent.com/cli/install.sh";

#[cfg(target_os = "windows")]
pub const WORKBUDDY_INSTALL_PS1_URL: &str = "https://www.codebuddy.cn/cli/install.ps1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbuddyLifecycleAction {
    Install,
    Update,
}

fn unix_script_url() -> &'static str {
    if use_china_mirrors() {
        WORKBUDDY_INSTALL_SCRIPT_URL
    } else {
        WORKBUDDY_INSTALL_SCRIPT_URL_INTL
    }
}

fn unix_install_command(force: bool) -> String {
    let url = unix_script_url();
    let args = if force { "--force" } else { "" };
    format!(
        "bash -c 'tmp=$(mktemp) && curl -fsSL --proto \"=https\" --tlsv1.2 \
        {url} -o $tmp && bash $tmp {args}; \
        status=$?; rm -f $tmp; exit $status'"
    )
}

pub fn workbuddy_install_shell_command() -> String {
    #[cfg(target_os = "windows")]
    {
        r#"powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://www.codebuddy.cn/cli/install.ps1 | iex""#
            .to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        unix_install_command(false)
    }
}

pub fn workbuddy_update_shell_command() -> String {
    #[cfg(target_os = "windows")]
    {
        r#"codebuddy update || powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://www.codebuddy.cn/cli/install.ps1 | iex""#
            .to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        format!("codebuddy update || {}", unix_install_command(true))
    }
}

pub fn run_workbuddy_lifecycle(action: WorkbuddyLifecycleAction) -> Result<()> {
    let command = match action {
        WorkbuddyLifecycleAction::Install => workbuddy_install_shell_command(),
        WorkbuddyLifecycleAction::Update => workbuddy_update_shell_command(),
    };
    run_shell_command(&command).with_context(|| {
        format!(
            "WorkBuddy {} failed",
            match action {
                WorkbuddyLifecycleAction::Install => "install",
                WorkbuddyLifecycleAction::Update => "update",
            }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_uses_official_native_script() {
        let cmd = workbuddy_install_shell_command();
        assert!(
            cmd.contains("codebuddy.cn/cli/install")
                || cmd.contains("copilot.tencent.com/cli/install")
        );
        assert!(!cmd.contains("npm install"));
    }
}
