//! WorkBuddy CLI via its official npm package (`@tencent-ai/codebuddy-code`).
//!
//! The command it installs is `codebuddy`. The desktop app stays where it is.

use anyhow::Result;

use super::npm_cli::{run_npm_cli_lifecycle, NpmCliLifecycleAction};

pub const WORKBUDDY_NPM_PACKAGE: &str = "@tencent-ai/codebuddy-code";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbuddyLifecycleAction {
    Install,
    Update,
}

pub fn workbuddy_install_shell_command() -> String {
    super::npm_cli::npm_global_install_command(WORKBUDDY_NPM_PACKAGE)
}

pub fn workbuddy_update_shell_command() -> String {
    super::npm_cli::npm_global_install_command(&format!("{WORKBUDDY_NPM_PACKAGE}@latest"))
}

pub fn run_workbuddy_lifecycle(action: WorkbuddyLifecycleAction) -> Result<()> {
    let npm_action = match action {
        WorkbuddyLifecycleAction::Install => NpmCliLifecycleAction::Install,
        WorkbuddyLifecycleAction::Update => NpmCliLifecycleAction::Update,
    };
    run_npm_cli_lifecycle(
        "codebuddy",
        WORKBUDDY_NPM_PACKAGE,
        &format!("{WORKBUDDY_NPM_PACKAGE}@latest"),
        npm_action,
        "WorkBuddy",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_uses_the_official_npm_package() {
        assert_eq!(
            workbuddy_install_shell_command(),
            "npm install -g @tencent-ai/codebuddy-code"
        );
        assert!(workbuddy_update_shell_command().ends_with("@latest"));
    }
}
