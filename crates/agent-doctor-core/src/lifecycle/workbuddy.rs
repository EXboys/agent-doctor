//! WorkBuddy / CodeBuddy native installer.

use anyhow::Result;

use super::shell_script::ShellScriptLifecycle;

pub const WORKBUDDY_INSTALL_SCRIPT_URL: &str = "https://www.codebuddy.cn/cli/install.sh";
pub const WORKBUDDY_INSTALL_SCRIPT_URL_INTL: &str = "https://copilot.tencent.com/cli/install.sh";

pub const WORKBUDDY_INSTALL_PS1_URL: &str = "https://www.codebuddy.cn/cli/install.ps1";

const SHELL: ShellScriptLifecycle = ShellScriptLifecycle {
    display_name: "WorkBuddy",
    update_cli: "codebuddy",
    update_bash_args: " --force",
    unix_script_url,
    windows_irm_arg,
};

/// The mirror decides which region's install script runs.
fn unix_script_url() -> &'static str {
    #[cfg(not(target_os = "windows"))]
    {
        if super::download_route::use_china_mirrors() {
            WORKBUDDY_INSTALL_SCRIPT_URL
        } else {
            WORKBUDDY_INSTALL_SCRIPT_URL_INTL
        }
    }
    #[cfg(target_os = "windows")]
    {
        WORKBUDDY_INSTALL_SCRIPT_URL
    }
}

fn windows_irm_arg() -> &'static str {
    WORKBUDDY_INSTALL_PS1_URL
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbuddyLifecycleAction {
    Install,
    Update,
}

pub fn workbuddy_install_shell_command() -> String {
    SHELL.install_command()
}

pub fn workbuddy_update_shell_command() -> String {
    SHELL.update_command()
}

pub fn run_workbuddy_lifecycle(action: WorkbuddyLifecycleAction) -> Result<()> {
    match action {
        WorkbuddyLifecycleAction::Install => SHELL.run_install(),
        WorkbuddyLifecycleAction::Update => SHELL.run_update(),
    }
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
