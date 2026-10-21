//! Qoder native installer (https://qoder.com/install).

use anyhow::Result;

use super::shell_script::ShellScriptLifecycle;

pub const QODER_INSTALL_SCRIPT_URL: &str = "https://qoder.com/install";

pub const QODER_INSTALL_PS1_URL: &str = "https://qoder.com/install.ps1";

const SHELL: ShellScriptLifecycle = ShellScriptLifecycle {
    display_name: "Qoder",
    update_cli: "qoder",
    update_bash_args: " --force",
    unix_script_url,
    windows_irm_arg,
};

fn unix_script_url() -> &'static str {
    QODER_INSTALL_SCRIPT_URL
}

fn windows_irm_arg() -> &'static str {
    QODER_INSTALL_PS1_URL
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QoderLifecycleAction {
    Install,
    Update,
}

pub fn qoder_install_shell_command() -> String {
    SHELL.install_command()
}

pub fn qoder_update_shell_command() -> String {
    SHELL.update_command()
}

pub fn run_qoder_lifecycle(action: QoderLifecycleAction) -> Result<()> {
    match action {
        QoderLifecycleAction::Install => SHELL.run_install(),
        QoderLifecycleAction::Update => SHELL.run_update(),
    }
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
