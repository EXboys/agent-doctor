use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use crate::adapter::AdapterDiscovery;
use crate::exec::{run_output, SHORT_PROBE_TIMEOUT};

#[cfg(test)]
thread_local! {
    /// Per-test home override so unit tests never mutate the process `HOME`.
    static TEST_HOME_OVERRIDE: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Resolve the user home directory (tests may override via [`with_test_home`]).
pub fn home_dir() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(path) = TEST_HOME_OVERRIDE.with(|cell| cell.borrow().clone()) {
            return path;
        }
    }
    dirs::home_dir().expect("home directory")
}

pub fn home_join(relative: &str) -> PathBuf {
    home_dir().join(relative)
}

/// Run `f` with [`home_dir`] / [`home_join`] rooted at `home` for this thread only.
#[cfg(test)]
pub(crate) fn with_test_home<T>(home: &Path, f: impl FnOnce() -> T) -> T {
    TEST_HOME_OVERRIDE.with(|cell| {
        let previous = cell.replace(Some(home.to_path_buf()));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        *cell.borrow_mut() = previous;
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    })
}

/// User-local Node install used when the machine has no npm (no admin / winget).
pub fn managed_nodejs_root() -> PathBuf {
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(dirs::data_local_dir)
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("AgentDoctor").join("runtime").join("nodejs")
    }
    #[cfg(not(windows))]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/share/agent-doctor/runtime/nodejs")
    }
}

pub(crate) fn prepend_user_path(dir: &Path) {
    if dir.as_os_str().is_empty() {
        return;
    }
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![dir.to_path_buf()];
    dirs.extend(std::env::split_paths(&current));
    if let Ok(joined) = std::env::join_paths(dirs.iter().filter(|p| !p.as_os_str().is_empty())) {
        // SAFETY: process-local PATH update so child npm/node shims resolve.
        std::env::set_var("PATH", joined);
    }
}

pub fn find_binary(name: &str) -> Option<PathBuf> {
    ensure_managed_runtime_path();
    find_in_path(name)
        .or_else(|| find_binary_in_dirs(name, &common_binary_dirs()))
        .or_else(|| find_with_where_exe(name))
}

pub fn find_all_binaries(name: &str) -> Vec<PathBuf> {
    ensure_managed_runtime_path();
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
        if is_slow_network_path(dir) {
            continue;
        }
        for candidate in windows_shim_candidates(dir, name) {
            if path_is_file(&candidate) {
                return Some(candidate);
            }
        }
        let candidate = dir.join(name);
        if path_is_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn find_all_binary_in_dirs(name: &str, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for dir in dirs {
        if is_slow_network_path(dir) {
            continue;
        }
        for candidate in windows_shim_candidates(dir, name) {
            if path_is_file(&candidate) && seen.insert(normalize_path_for_set(&candidate)) {
                found.push(candidate);
            }
        }
        let candidate = dir.join(name);
        if path_is_file(&candidate) && seen.insert(normalize_path_for_set(&candidate)) {
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

/// UNC / disconnected mapped drives make `is_file()` hang for a long time.
fn is_slow_network_path(path: &Path) -> bool {
    #[cfg(windows)]
    {
        let value = path.to_string_lossy();
        value.starts_with("\\\\") || value.starts_with("//")
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

fn path_is_file(path: &Path) -> bool {
    drive_prefix_reachable(path) && path.is_file()
}

fn drive_prefix_reachable(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::collections::HashMap;
        use std::sync::Mutex;
        static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();

        let value = path.to_string_lossy().replace('/', "\\");
        if value.starts_with("\\\\") {
            return false;
        }
        let bytes = value.as_bytes();
        if bytes.len() < 2 || bytes[1] != b':' {
            return true;
        }
        let root = format!("{}\\", &value[..2]);
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(guard) = cache.lock() {
            if let Some(ok) = guard.get(&root) {
                return *ok;
            }
        }
        let ok = path_exists_bounded(Path::new(&root));
        if let Ok(mut guard) = cache.lock() {
            guard.insert(root, ok);
        }
        ok
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        true
    }
}

#[cfg(windows)]
fn path_exists_bounded(path: &Path) -> bool {
    use std::sync::mpsc;
    use std::thread;
    let (tx, rx) = mpsc::channel();
    let path = path.to_path_buf();
    thread::spawn(move || {
        let _ = tx.send(path.exists());
    });
    rx.recv_timeout(Duration::from_millis(250)).unwrap_or(false)
}

/// GUI apps (double-clicked `.exe`) inherit Explorer's PATH, which often
/// omits nvm/npm shims that a developer terminal has. Prepend well-known
/// install locations so discovery matches `cargo run` from a shell.
pub(crate) fn ensure_managed_runtime_path() {
    ensure_windows_user_path();
    #[cfg(not(windows))]
    {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            merge_managed_dirs_into_path();
        });
    }
}

/// Re-merge managed dirs into PATH after an install (installers often mutate the
/// user PATH in the registry; GUI processes keep a stale copy).
pub fn refresh_managed_runtime_path() {
    invalidate_npm_global_bin_dir_cache();
    #[cfg(windows)]
    {
        merge_windows_persistent_path_into_env();
    }
    merge_managed_dirs_into_path();
}

fn merge_managed_dirs_into_path() {
    // Seed PATH with static known dirs first so `npm`/`node` shebangs resolve
    // before we query `npm prefix -g` (GUI apps often lack Homebrew on PATH).
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = static_common_binary_dirs();
    dirs.extend(std::env::split_paths(&current));
    let mut seen = BTreeSet::new();
    dirs.retain(|p| {
        if p.as_os_str().is_empty() {
            return false;
        }
        seen.insert(normalize_path_for_set(p))
    });
    if let Ok(joined) = std::env::join_paths(dirs) {
        // SAFETY: process-local PATH update for discovery / child installers.
        std::env::set_var("PATH", joined);
    }

    invalidate_npm_global_bin_dir_cache();
    if let Some(npm_bin) = npm_global_bin_dir() {
        prepend_user_path(&npm_bin);
    }
}

pub(crate) fn ensure_windows_user_path() {
    #[cfg(windows)]
    {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            merge_windows_persistent_path_into_env();
            merge_managed_dirs_into_path();
        });
    }
}

#[cfg(windows)]
fn merge_windows_persistent_path_into_env() {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for scope in ["User", "Machine"] {
        dirs.extend(windows_env_path_dirs(scope));
    }
    if dirs.is_empty() {
        return;
    }
    let current = std::env::var_os("PATH").unwrap_or_default();
    dirs.extend(std::env::split_paths(&current));
    let mut seen = BTreeSet::new();
    dirs.retain(|p| {
        if p.as_os_str().is_empty() {
            return false;
        }
        seen.insert(normalize_path_for_set(p))
    });
    if let Ok(joined) = std::env::join_paths(dirs) {
        std::env::set_var("PATH", joined);
    }
}

#[cfg(windows)]
fn windows_env_path_dirs(scope: &str) -> Vec<PathBuf> {
    let script = format!("[Environment]::GetEnvironmentVariable('Path','{scope}')");
    let output = run_output(
        Path::new("powershell"),
        &["-NoProfile", "-NonInteractive", "-Command", &script],
        Duration::from_secs(8),
    )
    .ok();
    let Some(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    raw.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn common_binary_dirs() -> Vec<PathBuf> {
    let mut dirs = static_common_binary_dirs();
    if let Some(npm_bin) = npm_global_bin_dir() {
        dirs.push(npm_bin);
    }
    dirs
}

/// Well-known bin dirs that do not require invoking `npm` (safe while seeding PATH).
fn static_common_binary_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
    ];

    dirs.extend(homebrew_node_bin_dirs());

    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".cargo/bin"));
        dirs.push(home.join("bin"));
        dirs.push(home.join(".npm-global/bin"));
        dirs.push(home.join(".claude/local"));
        dirs.push(home.join(".hermes/bin"));
        dirs.push(home.join("AppData/Local/hermes"));
        #[cfg(windows)]
        {
            dirs.push(home.join(r"AppData\Roaming\npm"));
            dirs.push(home.join(r"AppData\Local\Programs\nodejs"));
            dirs.push(home.join(r"AppData\Local\hermes"));
            dirs.push(home.join(r"scoop\shims"));
            dirs.push(home.join(r".local\bin"));
        }
    }

    let managed = managed_nodejs_root();
    dirs.push(managed.clone());
    dirs.push(managed.join("bin"));

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

    dirs
}

/// Homebrew kegs like `node@24` put `node`/`npm` under `opt/node@NN/bin`.
fn homebrew_node_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for root in ["/opt/homebrew/opt", "/usr/local/opt"] {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "node" || name.starts_with("node@") {
                dirs.push(entry.path().join("bin"));
            }
        }
    }
    dirs
}

static NPM_PREFIX_CACHE: std::sync::Mutex<Option<Option<PathBuf>>> = std::sync::Mutex::new(None);

fn npm_global_bin_dir() -> Option<PathBuf> {
    let mut guard = NPM_PREFIX_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        *guard = Some(npm_global_bin_dir_uncached());
    }
    guard.clone().flatten()
}

fn invalidate_npm_global_bin_dir_cache() {
    *NPM_PREFIX_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

fn npm_global_bin_dir_uncached() -> Option<PathBuf> {
    // Prefer an absolute npm path so GUI shells without Homebrew still work.
    // `#!/usr/bin/env node` still needs node on PATH — callers must seed PATH first.
    let npm =
        find_binary_in_dirs("npm", &static_common_binary_dirs()).or_else(|| find_in_path("npm"))?;
    let output = run_output(&npm, &["prefix", "-g"], Duration::from_secs(5)).ok()?;
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
    path_is_file(&candidate).then_some(candidate)
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

    #[cfg(windows)]
    #[test]
    fn skips_unc_and_unreachable_drive_prefix() {
        assert!(is_slow_network_path(std::path::Path::new(
            r"\\nas\share\bin"
        )));
        assert!(!is_slow_network_path(std::path::Path::new(r"C:\Users\bin")));
        assert!(!drive_prefix_reachable(std::path::Path::new(
            r"\\nas\share\bin"
        )));
    }

    #[test]
    fn common_binary_dirs_includes_home_local_bin() {
        let dirs = common_binary_dirs();
        assert!(
            dirs.iter()
                .any(|d| d.ends_with(".local/bin") || d.ends_with(r".local\bin")),
            "expected a …/.local/bin entry, got {dirs:?}"
        );
        assert!(
            dirs.iter()
                .any(|d| d.ends_with("nodejs") || d.ends_with(r"nodejs")),
            "expected managed nodejs dir, got {dirs:?}"
        );
        assert!(
            dirs.iter()
                .any(|d| d.ends_with(".claude/local") || d.ends_with(r".claude\local")),
            "expected a …/.claude/local entry, got {dirs:?}"
        );
        assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
    }
}
