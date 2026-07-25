//! Rule-based install/update for npm-distributed CLIs (Claude Code, Codex).

use anyhow::{Context, Result};

use super::runner::run_shell_command;

const CLAUDE_NPM_PACKAGE: &str = "@anthropic-ai/claude-code";
const CODEX_NPM_PACKAGE: &str = "@openai/codex";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpmCliLifecycleAction {
    Install,
    Update,
}

pub fn claude_code_install_shell_command() -> String {
    npm_global_install_command(CLAUDE_NPM_PACKAGE)
}

pub fn claude_code_update_shell_command() -> String {
    npm_global_install_command(&format!("{CLAUDE_NPM_PACKAGE}@latest"))
}

pub fn codex_install_shell_command() -> String {
    npm_global_install_command(CODEX_NPM_PACKAGE)
}

pub fn codex_update_shell_command() -> String {
    npm_global_install_command(&format!("{CODEX_NPM_PACKAGE}@latest"))
}

fn npm_global_install_command(package: &str) -> String {
    format!("npm install -g {package}")
}

pub fn run_claude_code_lifecycle(action: NpmCliLifecycleAction) -> Result<()> {
    let command = match action {
        NpmCliLifecycleAction::Install => claude_code_install_shell_command(),
        NpmCliLifecycleAction::Update => claude_code_update_shell_command(),
    };
    run_shell_command(&command).with_context(|| {
        format!(
            "Claude Code {} failed",
            match action {
                NpmCliLifecycleAction::Install => "install",
                NpmCliLifecycleAction::Update => "update",
            }
        )
    })
}

pub fn run_codex_lifecycle(action: NpmCliLifecycleAction) -> Result<()> {
    let command = match action {
        NpmCliLifecycleAction::Install => codex_install_shell_command(),
        NpmCliLifecycleAction::Update => codex_update_shell_command(),
    };
    run_shell_command(&command).with_context(|| {
        format!(
            "Codex {} failed",
            match action {
                NpmCliLifecycleAction::Install => "install",
                NpmCliLifecycleAction::Update => "update",
            }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_install_uses_official_npm_package() {
        assert_eq!(
            claude_code_install_shell_command(),
            "npm install -g @anthropic-ai/claude-code"
        );
    }

    #[test]
    fn codex_install_uses_official_npm_package() {
        assert_eq!(
            codex_install_shell_command(),
            "npm install -g @openai/codex"
        );
    }

    #[test]
    fn update_pins_latest_tag() {
        assert!(codex_update_shell_command().ends_with("@latest"));
        assert!(claude_code_update_shell_command().ends_with("@latest"));
    }
}
