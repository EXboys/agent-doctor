//! Rule-based install/update for npm-distributed CLIs (Claude Code, Codex).

use anyhow::{Context, Result};

use super::download_route::use_china_mirrors;
use super::npm_target::resolve_active_npm_prefix;

const CLAUDE_NPM_PACKAGE: &str = "@anthropic-ai/claude-code";
const CODEX_NPM_PACKAGE: &str = "@openai/codex";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpmCliLifecycleAction {
    Install,
    Update,
}

pub fn claude_code_install_shell_command() -> String {
    claude_code_install_shell_command_routed(use_china_mirrors())
}

fn claude_code_install_shell_command_routed(china: bool) -> String {
    // claude.ai does not open from mainland China. Skip it there and use npm,
    // which this app already points at the China registry.
    install_command(
        china,
        "https://claude.ai/install.sh",
        "https://claude.ai/install.ps1",
        CLAUDE_NPM_PACKAGE,
    )
}

pub fn claude_code_update_shell_command() -> String {
    npm_global_install_command(&format!("{CLAUDE_NPM_PACKAGE}@latest"))
}

pub fn codex_install_shell_command() -> String {
    codex_install_shell_command_routed(use_china_mirrors())
}

fn codex_install_shell_command_routed(china: bool) -> String {
    install_command(
        china,
        "https://chatgpt.com/codex/install.sh",
        "https://chatgpt.com/codex/install.ps1",
        CODEX_NPM_PACKAGE,
    )
}

/// Mainland installs skip the official host. Elsewhere the official script runs
/// first, and npm remains if that download fails.
fn install_command(china: bool, unix_script: &str, windows_script: &str, package: &str) -> String {
    let npm = npm_global_install_command(package);
    if china {
        return npm;
    }
    #[cfg(windows)]
    {
        let _ = unix_script;
        format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -Command \"irm {windows_script} | iex\" || {npm}"
        )
    }
    #[cfg(not(windows))]
    {
        let _ = windows_script;
        format!("set -o pipefail; curl -fsSL {unix_script} | bash || {npm}")
    }
}

pub fn codex_update_shell_command() -> String {
    npm_global_install_command(&format!("{CODEX_NPM_PACKAGE}@latest"))
}

fn npm_global_install_command(package: &str) -> String {
    format!("npm install -g {package}")
}

fn run_npm_cli_lifecycle(
    binary_name: &str,
    package: &str,
    package_update: &str,
    action: NpmCliLifecycleAction,
    label: &str,
) -> Result<()> {
    crate::lifecycle::nodejs::ensure_npm()
        .with_context(|| format!("Node.js / npm is required to install {label}"))?;
    let active = resolve_active_npm_prefix(binary_name)?;
    let package_spec = match action {
        NpmCliLifecycleAction::Install => package,
        NpmCliLifecycleAction::Update => package_update,
    };
    let prefix = if active.prefix.as_os_str().is_empty() {
        super::npm_target::default_npm_global_prefix().context("无法确定 npm 安装目录。")?
    } else {
        active.prefix
    };
    super::npm_target::run_npm_global("install", package_spec, &prefix).with_context(|| {
        format!(
            "{label} {} failed",
            match action {
                NpmCliLifecycleAction::Install => "install",
                NpmCliLifecycleAction::Update => "update",
            }
        )
    })
}

pub fn run_claude_code_lifecycle(action: NpmCliLifecycleAction) -> Result<()> {
    run_npm_cli_lifecycle(
        "claude",
        CLAUDE_NPM_PACKAGE,
        &format!("{CLAUDE_NPM_PACKAGE}@latest"),
        action,
        "Claude Code",
    )
}

pub fn run_codex_lifecycle(action: NpmCliLifecycleAction) -> Result<()> {
    run_npm_cli_lifecycle(
        "codex",
        CODEX_NPM_PACKAGE,
        &format!("{CODEX_NPM_PACKAGE}@latest"),
        action,
        "Codex",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainland_install_skips_the_blocked_official_host() {
        assert_eq!(
            claude_code_install_shell_command_routed(true),
            "npm install -g @anthropic-ai/claude-code"
        );
        assert_eq!(
            codex_install_shell_command_routed(true),
            "npm install -g @openai/codex"
        );
    }

    #[test]
    fn other_regions_try_the_official_script_then_npm() {
        let claude = claude_code_install_shell_command_routed(false);
        assert!(
            claude.contains("https://claude.ai/install.sh")
                || claude.contains("https://claude.ai/install.ps1")
        );
        assert!(claude.contains("|| npm install -g @anthropic-ai/claude-code"));
        let codex = codex_install_shell_command_routed(false);
        assert!(
            codex.contains("https://chatgpt.com/codex/install.sh")
                || codex.contains("https://chatgpt.com/codex/install.ps1")
        );
        assert!(codex.contains("|| npm install -g @openai/codex"));
    }

    #[test]
    fn update_pins_latest_tag() {
        assert!(codex_update_shell_command().ends_with("@latest"));
        assert!(claude_code_update_shell_command().ends_with("@latest"));
    }
}
