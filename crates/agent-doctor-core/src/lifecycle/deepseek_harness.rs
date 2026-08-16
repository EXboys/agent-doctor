use anyhow::{Context, Result};

use crate::adapters::{DEEPSEEK_HARNESS_NPM_PACKAGE, DEEPSEEK_HARNESS_VERSION};

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
    run_shell_command(&deepseek_harness_shell_command(action)).with_context(|| {
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
