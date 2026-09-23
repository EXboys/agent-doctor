//! Which npm global prefix install / update / uninstall should use.
//!
//! Doctor probes the first discoverable CLI on PATH. Lifecycle must target the
//! npm prefix that owns that binary — not whatever `npm` happens to use by default
//! (Homebrew Node vs user ~/.local vs managed Node often disagree).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::adapters::util::{ensure_managed_runtime_path, find_all_binaries, find_binary};

/// Active npm `--prefix` for a CLI, plus whether we already see an install.
#[derive(Debug, Clone)]
pub struct ActiveNpmPrefix {
    pub prefix: PathBuf,
    /// Detected CLI path when an install already exists on PATH.
    pub detected_binary: Option<PathBuf>,
}

pub fn shell_single_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// Infer the npm `--prefix` that owns a global CLI binary (symlink into node_modules).
pub fn npm_prefix_owning_binary(bin: &Path) -> Option<PathBuf> {
    let resolved = std::fs::canonicalize(bin).unwrap_or_else(|_| bin.to_path_buf());
    let raw = resolved.to_string_lossy();
    #[cfg(windows)]
    let marker = "\\node_modules\\";
    #[cfg(not(windows))]
    let marker = "/node_modules/";
    let idx = raw.find(marker)?;
    let before = PathBuf::from(&raw[..idx]);
    // Unix global layout: <prefix>/lib/node_modules/<pkg>/...
    if before
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("lib"))
    {
        return before.parent().map(Path::to_path_buf);
    }
    // Windows / flat: <prefix>/node_modules/<pkg>/...
    Some(before)
}

pub fn default_npm_global_prefix() -> Option<PathBuf> {
    let npm = find_binary("npm")?;
    let output = std::process::Command::new(&npm)
        .args(["prefix", "-g"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if prefix.is_empty() {
        return None;
    }
    Some(PathBuf::from(prefix))
}

/// Prefer the prefix owning the CLI doctor currently detects; otherwise the
/// default global prefix of the npm on PATH (after ensure_npm when needed).
pub fn resolve_active_npm_prefix(binary_name: &str) -> Result<ActiveNpmPrefix> {
    ensure_managed_runtime_path();
    if let Some(bin) = find_binary(binary_name) {
        if let Some(prefix) = npm_prefix_owning_binary(&bin) {
            return Ok(ActiveNpmPrefix {
                prefix,
                detected_binary: Some(bin),
            });
        }
    }

    crate::lifecycle::nodejs::ensure_npm().map_err(|_| anyhow!("需要本机已安装 npm。"))?;
    let prefix = default_npm_global_prefix().ok_or_else(|| anyhow!("无法确定 npm 安装目录。"))?;
    Ok(ActiveNpmPrefix {
        prefix,
        detected_binary: None,
    })
}

pub fn npm_install_global_command(package: &str, prefix: &Path) -> String {
    format!(
        "npm install -g {package} --prefix {}",
        shell_single_quote(prefix)
    )
}

pub fn npm_uninstall_global_command(package: &str, prefix: &Path) -> String {
    format!(
        "npm uninstall -g {package} --prefix {}",
        shell_single_quote(prefix)
    )
}

/// Other npm-owned copies still visible after the active prefix was removed.
pub fn leftover_npm_cli_binaries(binary_name: &str) -> Vec<PathBuf> {
    ensure_managed_runtime_path();
    find_all_binaries(binary_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(unix)]
    #[test]
    fn npm_prefix_from_homebrew_style_layout() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let pkg = root.path().join("lib/node_modules/@deepseek-ai/dsh/lib");
        fs::create_dir_all(&pkg).unwrap();
        let target = pkg.join("bin.js");
        fs::write(&target, b"#!/usr/bin/env node\n").unwrap();
        let bin_dir = root.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let shim = bin_dir.join("dsh");
        std::os::unix::fs::symlink(&target, &shim).unwrap();
        let mut perms = fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&target, perms).unwrap();

        let prefix = npm_prefix_owning_binary(&shim).expect("prefix");
        let expected = root
            .path()
            .canonicalize()
            .unwrap_or_else(|_| root.path().to_path_buf());
        assert_eq!(prefix, expected);
    }

    #[test]
    fn install_command_pins_prefix() {
        let cmd =
            npm_install_global_command("@deepseek-ai/dsh@0.1.0-rc.6", Path::new("/opt/homebrew"));
        assert!(cmd.contains("npm install -g @deepseek-ai/dsh@0.1.0-rc.6"));
        assert!(cmd.contains("--prefix '/opt/homebrew'"));
    }
}
