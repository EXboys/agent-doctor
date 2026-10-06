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

/// Infer the npm `--prefix` that owns a global CLI binary (symlink into node_modules).
pub fn npm_prefix_owning_binary(bin: &Path) -> Option<PathBuf> {
    let resolved = std::fs::canonicalize(bin).unwrap_or_else(|_| bin.to_path_buf());
    let raw = strip_verbatim_prefix(&resolved.to_string_lossy());
    let marker = if raw.contains('\\') {
        "\\node_modules\\"
    } else {
        "/node_modules/"
    };
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

fn strip_verbatim_prefix(raw: &str) -> String {
    raw.strip_prefix(r"\\?\").unwrap_or(raw).to_string()
}

/// Where this installed file actually lives.
///
/// A Windows global shim sits next to `node_modules` (`…\npm\claude.cmd`), not
/// inside it. That directory is the prefix, wherever the user put it.
fn prefix_for_binary(bin: &Path) -> Option<PathBuf> {
    if let Some(prefix) = npm_prefix_owning_binary(bin) {
        return Some(prefix);
    }
    let dir = bin.parent()?;
    if dir.join("node_modules").is_dir() {
        return Some(dir.to_path_buf());
    }
    prefix_from_npm_beside(bin)
}

/// Ask the npm that sits next to this program, not whichever npm happens to be
/// first on PATH.
fn prefix_from_npm_beside(bin: &Path) -> Option<PathBuf> {
    let dir = bin.parent()?;
    let npm = ["npm.cmd", "npm.exe", "npm.bat", "npm"]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())?;
    let output = std::process::Command::new(&npm)
        .args(["prefix", "-g"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if prefix.is_empty() {
        return None;
    }
    Some(PathBuf::from(strip_verbatim_prefix(&prefix)))
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

/// Prefer the prefix that owns the CLI already on this computer.
///
/// When nothing is installed yet, fall back to that machine's own `npm prefix -g`.
/// A program that is installed some other way does not get a guessed folder.
pub fn resolve_active_npm_prefix(binary_name: &str) -> Result<ActiveNpmPrefix> {
    ensure_managed_runtime_path();
    if let Some(bin) = find_binary(binary_name) {
        if let Some(prefix) = prefix_for_binary(&bin) {
            return Ok(ActiveNpmPrefix {
                prefix,
                detected_binary: Some(bin),
            });
        }
        return Ok(ActiveNpmPrefix {
            prefix: PathBuf::new(),
            detected_binary: Some(bin),
        });
    }

    crate::lifecycle::nodejs::ensure_npm().map_err(|_| anyhow!("需要本机已安装 npm。"))?;
    let prefix = default_npm_global_prefix().ok_or_else(|| anyhow!("无法确定 npm 安装目录。"))?;
    Ok(ActiveNpmPrefix {
        prefix,
        detected_binary: None,
    })
}

/// Run npm inside `prefix`. The folder is the one just discovered for this
/// computer. `--prefix .` stays inside that folder, so Windows does not glue
/// the app's launch folder onto another drive.
pub fn run_npm_global(action: &str, package: &str, prefix: &Path) -> Result<()> {
    if prefix.as_os_str().is_empty() {
        return Err(anyhow!("无法确定 npm 安装目录。"));
    }
    std::fs::create_dir_all(prefix).map_err(|_| anyhow!("安装目录用不了，请再试一次。"))?;
    let command = format!("npm {action} -g {package} --prefix .");
    super::runner::run_shell_command_in(&command, Some(prefix))
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
    fn windows_shim_uses_the_folder_next_to_node_modules() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("node_modules").join("pkg")).unwrap();
        let shim = root.path().join("claude.cmd");
        fs::write(&shim, b"@echo off\r\n").unwrap();

        let prefix = prefix_for_binary(&shim).expect("prefix");
        assert_eq!(prefix, root.path());
    }

    #[test]
    fn a_program_outside_npm_does_not_invent_a_prefix() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("claude.exe");
        fs::write(&bin, b"not npm").unwrap();
        assert!(prefix_for_binary(&bin).is_none());
    }
}
