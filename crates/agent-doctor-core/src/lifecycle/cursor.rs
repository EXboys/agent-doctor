//! Cursor Agent CLI native installer (https://cursor.com/install).

use anyhow::{Context, Result};

use super::runner::run_shell_command;

pub const CURSOR_INSTALL_SCRIPT_URL: &str = "https://cursor.com/install";

const CURSOR_INSTALL_UNIX: &str =
    "bash -c 'tmp=$(mktemp) && curl -fsSL --proto \"=https\" --tlsv1.2 \
    https://cursor.com/install -o $tmp && bash $tmp; \
    status=$?; rm -f $tmp; exit $status'";

const CURSOR_UPDATE_UNIX: &str =
    "agent update || bash -c 'tmp=$(mktemp) && curl -fsSL --proto \"=https\" --tlsv1.2 \
    https://cursor.com/install -o $tmp && bash $tmp; \
    status=$?; rm -f $tmp; exit $status'";

#[cfg(target_os = "windows")]
const CURSOR_INSTALL_WINDOWS: &str = r#"powershell -NoProfile -ExecutionPolicy Bypass -Command "irm 'https://cursor.com/install?win32=true' | iex""#;

#[cfg(target_os = "windows")]
const CURSOR_UPDATE_WINDOWS: &str = r#"agent update || powershell -NoProfile -ExecutionPolicy Bypass -Command "irm 'https://cursor.com/install?win32=true' | iex""#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorLifecycleAction {
    Install,
    Update,
}

pub fn cursor_install_shell_command() -> String {
    #[cfg(target_os = "windows")]
    {
        CURSOR_INSTALL_WINDOWS.to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        CURSOR_INSTALL_UNIX.to_string()
    }
}

pub fn cursor_update_shell_command() -> String {
    #[cfg(target_os = "windows")]
    {
        CURSOR_UPDATE_WINDOWS.to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        CURSOR_UPDATE_UNIX.to_string()
    }
}

pub fn run_cursor_lifecycle(action: CursorLifecycleAction) -> Result<()> {
    let command = match action {
        CursorLifecycleAction::Install => cursor_install_shell_command(),
        CursorLifecycleAction::Update => cursor_update_shell_command(),
    };
    run_shell_command(&command).with_context(|| {
        format!(
            "Cursor {} failed",
            match action {
                CursorLifecycleAction::Install => "install",
                CursorLifecycleAction::Update => "update",
            }
        )
    })
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
