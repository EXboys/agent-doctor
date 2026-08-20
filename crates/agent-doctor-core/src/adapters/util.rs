use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use crate::adapter::AdapterDiscovery;
use crate::exec::{run_output, SHORT_PROBE_TIMEOUT};

pub fn home_join(relative: &str) -> PathBuf {
    dirs::home_dir().expect("home directory").join(relative)
}

pub fn find_binary(name: &str) -> Option<PathBuf> {
    ensure_windows_user_path();
    find_in_path(name)
        .or_else(|| find_binary_in_dirs(name, &common_binary_dirs()))
        .or_else(|| find_with_where_exe(name))
}

pub fn find_all_binaries(name: &str) -> Vec<PathBuf> {
    ensure_windows_user_path();
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

fn windows_shim_candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        // Prefer .cmd/.exe over the npm shebang file named `claude` (no extension),
        // which CreateProcess cannot run and which we used to try first.
        return [".cmd", ".exe", ".bat"]
            .into_iter()
            .map(|ext| dir.join(format!("{name}{ext}")))
            .collect();
    }
    #[cfg(not(windows))]
    {
        let _ = (dir, name);
        Vec::new()
    }
}

fn find_binary_in_dirs(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in dirs {
        for candidate in windows_shim_candidates(dir, name) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn find_all_binary_in_dirs(name: &str, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for dir in dirs {
        for candidate in windows_shim_candidates(dir, name) {
            if candidate.is_file() && seen.insert(normalize_path_for_set(&candidate)) {
                found.push(candidate);
            }
        }
        let candidate = dir.join(name);
        if candidate.is_file() && seen.insert(normalize_path_for_set(&candidate)) {
            found.push(candidate);
        }
    }
    found
}

fn normalize_path_for_set(path: &Path) -> String {
    // Avoid canonicalize(): on Windows it can block for tens of seconds on a
    // disconnected network PATH entry.
    #[cfg(windows)]
    {
        return path
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase();
    }
    #[cfg(not(windows))]
    {
        path.canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .display()
            .to_string()
    }
}

/// GUI apps (double-clicked `.exe`) inherit Explorer's PATH, which often
/// omits nvm/npm shims that a developer terminal has. Prepend well-known
/// Windows install locations so discovery matches `cargo run` from a shell.
fn ensure_windows_user_path() {
    #[cfg(windows)]
    {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            let current = std::env::var_os("PATH").unwrap_or_default();
            let mut dirs = common_binary_dirs();
            dirs.extend(std::env::split_paths(&current));
            if let Ok(joined) =
                std::env::join_paths(dirs.iter().filter(|p| !p.as_os_str().is_empty()))
            {
                std::env::set_var("PATH", joined);
            }
        });
    }
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
        dirs.push(home.join(".claude/local"));
        #[cfg(windows)]
        {
            dirs.push(home.join(r"AppData\Roaming\npm"));
            dirs.push(home.join(r"AppData\Local\Programs\nodejs"));
            dirs.push(home.join(r"scoop\shims"));
        }
    }

    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
        for key in ["ProgramFiles", "ProgramW6432"] {
            if let Ok(root) = std::env::var(key) {
                dirs.push(PathBuf::from(root).join("nodejs"));
            }
        }
        if let Ok(nvm) = std::env::var("NVM_SYMLINK") {
            dirs.push(PathBuf::from(nvm));
        }
        if let Ok(nvm_home) = std::env::var("NVM_HOME") {
            dirs.push(PathBuf::from(nvm_home));
        }
    }

    if let Some(npm_bin) = npm_global_bin_dir() {
        dirs.push(npm_bin);
    }

    dirs
}

fn npm_global_bin_dir() -> Option<PathBuf> {
    static CACHED: OnceLock<Option<PathBuf>> = OnceLock::new();
    CACHED.get_or_init(npm_global_bin_dir_uncached).clone()
}

fn npm_global_bin_dir_uncached() -> Option<PathBuf> {
    let output = run_output(Path::new("npm"), &["prefix", "-g"], Duration::from_secs(5)).ok()?;
    if !output.status.success() {
        return None;
    }
    let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if prefix.is_empty() {
        return None;
    }
    let prefix = PathBuf::from(prefix);
    #[cfg(windows)]
    {
        // `npm prefix -g` is already the directory that holds .cmd shims.
        return Some(prefix);
    }
    #[cfg(not(windows))]
    {
        Some(prefix.join("bin"))
    }
}

/// Use `where.exe` on Windows to find executables that may be in restricted
/// directories (e.g. WindowsApps) where read_dir() would fail.
#[cfg(target_os = "windows")]
fn find_with_where_exe(name: &str) -> Option<PathBuf> {
    let output = run_output(Path::new("where"), &[name], Duration::from_secs(3)).ok()?;
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
    read_version_result_with_flags(binary, flags).unwrap_or_default()
}

pub fn read_version_result(binary: &PathBuf) -> Result<Option<String>, String> {
    read_version_result_with_flags(binary, &["--version", "-V", "version"])
}

fn read_version_result_with_flags(
    binary: &PathBuf,
    flags: &[&str],
) -> Result<Option<String>, String> {
    let mut last_error = None;
    for flag in flags {
        match run_output(binary, &[flag], SHORT_PROBE_TIMEOUT) {
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
            Err(error) if error.timed_out() => {
                return Err(format!("{flag} {error}"));
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
        // Prefer suffix check: other tests temporarily mutate HOME in parallel.
        assert!(
            dirs.iter()
                .any(|d| d.ends_with(".local/bin") || d.ends_with(r".local\bin")),
            "expected a …/.local/bin entry, got {dirs:?}"
        );
        assert!(
            dirs.iter()
                .any(|d| d.ends_with(".claude/local") || d.ends_with(r".claude\local")),
            "expected a …/.claude/local entry, got {dirs:?}"
        );
        assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
    }
}
