use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::adapters::{DEEPSEEK_HARNESS_NPM_PACKAGE, DEEPSEEK_HARNESS_VERSION};

use super::npm_target::resolve_active_npm_prefix;

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

/// Version of the `dsh` found on this computer, read from its npm package.
pub fn installed_deepseek_harness_version() -> Option<String> {
    crate::adapters::util::ensure_managed_runtime_path();
    let bin = crate::adapters::util::find_binary(crate::adapters::DEEPSEEK_HARNESS_CLI)?;
    let prefix = super::npm_target::prefix_for_binary(&bin)?;
    let package = Path::new(DEEPSEEK_HARNESS_NPM_PACKAGE);
    [
        prefix.join("lib").join("node_modules").join(package),
        prefix.join("node_modules").join(package),
    ]
    .iter()
    .find_map(|dir| fs::read_to_string(dir.join("package.json")).ok())
    .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
    .and_then(|manifest| manifest.get("version")?.as_str().map(str::to_string))
}

/// Older than the pinned build. Those builds ship no `acp` profile, so chat cannot start.
pub fn deepseek_harness_is_outdated(installed: &str) -> bool {
    release_core(installed) < release_core(DEEPSEEK_HARNESS_VERSION)
}

fn release_core(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .map_while(|part| part.parse().ok())
        .collect()
}

pub fn run_deepseek_harness_lifecycle(action: DeepSeekHarnessLifecycleAction) -> Result<()> {
    crate::lifecycle::nodejs::ensure_npm()
        .context("Node.js / npm is required to install DeepSeek Harness")?;
    let active = resolve_active_npm_prefix("dsh")?;
    let package = format!("{DEEPSEEK_HARNESS_NPM_PACKAGE}@{DEEPSEEK_HARNESS_VERSION}");
    let prefix = if active.prefix.as_os_str().is_empty() {
        super::npm_target::default_npm_global_prefix().context("无法确定 npm 安装目录。")?
    } else {
        active.prefix
    };
    super::npm_target::run_npm_global("install", &package, &prefix).with_context(|| {
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
        let expected = "npm install --global @deepseek-ai/dsh@0.1.5-rc.3";
        assert_eq!(deepseek_harness_install_shell_command(), expected);
        assert_eq!(deepseek_harness_update_shell_command(), expected);
    }

    #[test]
    fn builds_without_the_chat_profile_count_as_outdated() {
        assert!(deepseek_harness_is_outdated("0.1.0-rc.6"));
        assert!(deepseek_harness_is_outdated("0.1.1-rc.2"));
        assert!(!deepseek_harness_is_outdated("0.1.5-rc.3"));
        assert!(!deepseek_harness_is_outdated("0.2.1-rc.2"));
    }
}
