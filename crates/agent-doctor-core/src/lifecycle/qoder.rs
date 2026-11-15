//! Qoder CLI via its official npm package (`@qoder-ai/qodercli`).

use anyhow::Result;

use super::npm_cli::{run_npm_cli_lifecycle, NpmCliLifecycleAction};

pub const QODER_NPM_PACKAGE: &str = "@qoder-ai/qodercli";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QoderLifecycleAction {
    Install,
    Update,
}

pub fn qoder_install_shell_command() -> String {
    super::npm_cli::npm_global_install_command(QODER_NPM_PACKAGE)
}

pub fn qoder_update_shell_command() -> String {
    super::npm_cli::npm_global_install_command(&format!("{QODER_NPM_PACKAGE}@latest"))
}

pub fn run_qoder_lifecycle(action: QoderLifecycleAction) -> Result<()> {
    let npm_action = match action {
        QoderLifecycleAction::Install => NpmCliLifecycleAction::Install,
        QoderLifecycleAction::Update => NpmCliLifecycleAction::Update,
    };
    run_npm_cli_lifecycle(
        "qodercli",
        QODER_NPM_PACKAGE,
        &format!("{QODER_NPM_PACKAGE}@latest"),
        npm_action,
        "Qoder",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_uses_the_official_npm_package() {
        assert_eq!(
            qoder_install_shell_command(),
            "npm install -g @qoder-ai/qodercli"
        );
        assert!(qoder_update_shell_command().ends_with("@latest"));
    }
}
