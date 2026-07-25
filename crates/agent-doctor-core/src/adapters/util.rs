use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::adapter::AdapterDiscovery;

pub fn home_join(relative: &str) -> PathBuf {
    dirs::home_dir().expect("home directory").join(relative)
}

pub fn find_binary(name: &str) -> Option<PathBuf> {
    find_in_path(name)
        .or_else(|| find_binary_in_dirs(name, &common_binary_dirs()))
        .or_else(|| find_with_where_exe(name))
}

pub fn find_all_binaries(name: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(path_var) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path_var));
    }
    dirs.extend(common_binary_dirs());
    find_all_binary_in_dirs(name, &dirs)
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    find_binary_in_dirs(name, &std::env::split_paths(&path_var).collect::<Vec<_>>())
}

fn find_binary_in_dirs(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in dirs {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(target_os = "windows")]
        {
            let exe_candidate = dir.join(format!("{name}.exe"));
            if exe_candidate.is_file() {
                return Some(exe_candidate);
            }
        }
    }
    None
}

fn find_all_binary_in_dirs(name: &str, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for dir in dirs {
        let candidate = dir.join(name);
        if candidate.is_file() && seen.insert(normalize_path_for_set(&candidate)) {
            found.push(candidate);
        }
        #[cfg(target_os = "windows")]
        {
            let exe_candidate = dir.join(format!("{name}.exe"));
            if exe_candidate.is_file() && seen.insert(normalize_path_for_set(&exe_candidate)) {
                found.push(exe_candidate);
            }
        }
    }
    found
}

fn normalize_path_for_set(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn common_binary_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];

    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".cargo/bin"));
        dirs.push(home.join("bin"));
        dirs.push(home.join(".npm-global/bin"));
    }

    if let Some(npm_bin) = npm_global_bin_dir() {
        dirs.push(npm_bin);
    }

    dirs
}

fn npm_global_bin_dir() -> Option<PathBuf> {
    let output = Command::new("npm").args(["prefix", "-g"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if prefix.is_empty() {
        return None;
    }
    Some(PathBuf::from(prefix).join("bin"))
}

/// Use `where.exe` on Windows to find executables that may be in restricted
/// directories (e.g. WindowsApps) where read_dir() would fail.
#[cfg(target_os = "windows")]
fn find_with_where_exe(name: &str) -> Option<PathBuf> {
    let output = Command::new("where").arg(name).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next()?.trim();
    if first_line.is_empty() {
        return None;
    }
    let candidate = PathBuf::from(first_line);
    candidate.is_file().then_some(candidate)
}

#[cfg(not(target_os = "windows"))]
fn find_with_where_exe(_name: &str) -> Option<PathBuf> {
    None
}

pub fn discover_binary(name: &str) -> AdapterDiscovery {
    let binary_path = find_binary(name);
    let installed = binary_path.is_some();
    let version = binary_path
        .as_ref()
        .and_then(|path| read_version(path, &["--version", "-V", "version"]));

    AdapterDiscovery {
        installed,
        version,
        binary_path,
    }
}

fn read_version(binary: &PathBuf, flags: &[&str]) -> Option<String> {
    for flag in flags {
        let output = Command::new(binary).arg(flag).output().ok()?;
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let line = text.lines().next()?.trim();
        if !line.is_empty() {
            return Some(line.to_string());
        }
    }
    None
}

pub fn read_version_result(binary: &PathBuf) -> Result<Option<String>, String> {
    let mut last_error = None;
    for flag in ["--version", "-V", "version"] {
        match Command::new(binary).arg(flag).output() {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let line = stdout
                    .lines()
                    .chain(stderr.lines())
                    .map(str::trim)
                    .find(|line| !line.is_empty());
                return Ok(line.map(str::to_string));
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                last_error = Some(format!("{flag} exited with {}", output.status));
                if !stderr.trim().is_empty() {
                    last_error = Some(format!("{flag}: {}", stderr.trim()));
                }
            }
            Err(error) => {
                last_error = Some(format!("{flag}: {error}"));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "version command failed".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn write_executable(path: &PathBuf) {
        fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).unwrap();
        }
        #[cfg(windows)]
        {
            let _ = fs::metadata(path);
        }
    }

    #[test]
    fn finds_binary_in_supplemental_dirs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin = temp.path().join("agent-doctor-probe");
        write_executable(&bin);

        let found = find_binary_in_dirs("agent-doctor-probe", &[temp.path().to_path_buf()]);
        assert_eq!(found, Some(bin));
    }

    #[test]
    fn finds_all_binaries_without_duplicates() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin = temp.path().join("agent-doctor-probe-all");
        write_executable(&bin);

        let found = find_all_binary_in_dirs(
            "agent-doctor-probe-all",
            &[temp.path().to_path_buf(), temp.path().to_path_buf()],
        );
        assert_eq!(found, vec![bin]);
    }

    #[test]
    fn common_binary_dirs_includes_home_local_bin() {
        let dirs = common_binary_dirs();
        let home = dirs::home_dir().expect("home");
        assert!(dirs.contains(&home.join(".local/bin")));
        assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
    }
}
