//! Cursor Agent CLI native installer (https://cursor.com/install).

use anyhow::Result;

use super::shell_script::ShellScriptLifecycle;

pub const CURSOR_INSTALL_SCRIPT_URL: &str = "https://cursor.com/install";

const SHELL: ShellScriptLifecycle = ShellScriptLifecycle {
    display_name: "Cursor",
    update_cli: "agent",
    update_bash_args: "",
    unix_script_url,
    windows_irm_arg,
};

fn unix_script_url() -> &'static str {
    CURSOR_INSTALL_SCRIPT_URL
}

fn windows_irm_arg() -> &'static str {
    // The query string needs quoting inside the PowerShell command.
    "'https://cursor.com/install?win32=true'"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorLifecycleAction {
    Install,
    Update,
}

pub fn cursor_install_shell_command() -> String {
    SHELL.install_command()
}

pub fn cursor_update_shell_command() -> String {
    SHELL.update_command()
}

pub fn run_cursor_lifecycle(action: CursorLifecycleAction) -> Result<()> {
    match action {
        CursorLifecycleAction::Install => SHELL.run_install(),
        CursorLifecycleAction::Update => SHELL.run_update(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_uses_official_native_script() {
        let cmd = cursor_install_shell_command();
        assert!(cmd.contains("cursor.com/install"));
        assert!(!cmd.contains("npm install"));
    }
}
