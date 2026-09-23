use anyhow::{Context, Result};

use crate::adapters::{DEEPSEEK_HARNESS_NPM_PACKAGE, DEEPSEEK_HARNESS_VERSION};

use super::npm_target::{npm_install_global_command, resolve_active_npm_prefix};
use super::runner::run_shell_command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeepSeekHarnessLifecycleAction {
    Install,
    Update,
}

pub fn deepseek_harness_install_shell_command() -> String {
    format!("npm install --global {DEEPSEEK_HARNESS_NPM_PACKAGE}@{DEEPSEEK_HARNESS_VERSION}")
}

pub fn deepseek_harness_update_shell_command() -> String {
    deepseek_harness_install_shell_command()
}

pub fn deepseek_harness_shell_command(_action: DeepSeekHarnessLifecycleAction) -> String {
    deepseek_harness_install_shell_command()
}

pub fn run_deepseek_harness_lifecycle(action: DeepSeekHarnessLifecycleAction) -> Result<()> {
    crate::lifecycle::nodejs::ensure_npm()
        .context("Node.js / npm is required to install DeepSeek Harness")?;
    let active = resolve_active_npm_prefix("dsh")?;
    let package = format!("{DEEPSEEK_HARNESS_NPM_PACKAGE}@{DEEPSEEK_HARNESS_VERSION}");
    let command = npm_install_global_command(&package, &active.prefix);
    run_shell_command(&command).with_context(|| {
        format!(
            "DeepSeek Harness {} failed",
            match action {
                DeepSeekHarnessLifecycleAction::Install => "install",
                DeepSeekHarnessLifecycleAction::Update => "update",
            }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_and_update_pin_the_official_version() {
        let expected = "npm install --global @deepseek-ai/dsh@0.1.0-rc.6";
        assert_eq!(deepseek_harness_install_shell_command(), expected);
        assert_eq!(deepseek_harness_update_shell_command(), expected);
    }
}
