use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use crate::adapter::AdapterDiscovery;
use crate::exec::{
    run_output, skips_version_probe, VERSION_PROBE_DIAGNOSE_TIMEOUT, VERSION_PROBE_TIMEOUT,
};

#[cfg(test)]
thread_local! {
    /// Per-test home override so unit tests never mutate the process `HOME`.
    static TEST_HOME_OVERRIDE: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
    /// Per-test config-root override so unit tests never rewrite the real
    /// `~/Library/Application Support/agent-doctor/workspaces.yaml`.
    static TEST_CONFIG_OVERRIDE: std::cell::RefCell<Option<PathBuf>> =
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
    if relative == ".hermes" {
        return hermes_root();
    }
    if let Some(rest) = relative.strip_prefix(".hermes/") {
        return hermes_root().join(rest);
    }
    home_dir().join(relative)
}

/// Data directory of the Hermes install that is actually on this computer.
///
/// A profile has to live inside that directory. Anywhere else, Hermes treats
/// the launch as borrowed and builds a second Python environment, which fails
/// on some Windows machines before any reply. The official installer uses
/// `%LOCALAPPDATA%\hermes`; an install that already lives in `~\.hermes` stays
/// there.
pub fn hermes_root() -> PathBuf {
    let root = hermes_root_path();
    adopt_legacy_hermes_home(&root);
    root
}

/// `HERMES_HOME` for one project profile, under [`hermes_root`].
pub fn hermes_profile_dir(profile: &str) -> PathBuf {
    hermes_root().join("profiles").join(profile)
}

fn hermes_root_path() -> PathBuf {
    #[cfg(test)]
    {
        if TEST_HOME_OVERRIDE.with(|cell| cell.borrow().is_some()) {
            return home_dir().join(".hermes");
        }
        // Unit tests must not scan or adopt the developer's real Hermes.
        hermes_root_for(
            cfg!(windows),
            &home_dir(),
            local_app_data_dir().as_deref(),
            false,
            false,
            false,
            false,
        )
    }
    #[cfg(not(test))]
    {
        static CACHE: OnceLock<PathBuf> = OnceLock::new();
        CACHE
            .get_or_init(|| {
                detect_hermes_root(
                    cfg!(windows),
                    &home_dir(),
                    local_app_data_dir().as_deref(),
                    find_binary("hermes").as_deref(),
                )
            })
            .clone()
    }
}

fn detect_hermes_root(
    windows: bool,
    home: &Path,
    local_app_data: Option<&Path>,
    binary: Option<&Path>,
) -> PathBuf {
    if windows {
        if let Some(binary) = binary {
            if let Some((root, _)) = hermes_install_location(binary) {
                return root;
            }
        }
    }
    let native = windows_native_hermes_root(home, local_app_data);
    let legacy = home.join(".hermes");
    hermes_root_for(
        windows,
        home,
        local_app_data,
        hermes_install_owns(&native),
        hermes_install_owns(&legacy),
        binary.is_some_and(|path| path_is_inside(path, &native)),
        binary.is_some_and(|path| path_is_inside(path, &legacy)),
    )
}

/// The data directory that owns this `hermes` command, and the checkout folder
/// when that checkout sits beside it.
///
/// Walks only the command's parent folders. A custom location such as
/// `D:\Tools\MyHermes` counts when it contains `tools/`, `bin/hermes`, or a
/// `hermes-agent` checkout. A folder that only has a failed `installs/` tree
/// does not.
pub(crate) fn hermes_install_location(binary: &Path) -> Option<(PathBuf, Option<PathBuf>)> {
    let mut checkout = None;
    for ancestor in binary.ancestors().skip(1) {
        if ancestor.parent().is_none() || is_broad_directory(ancestor) {
            break;
        }
        if hermes_install_owns(ancestor) {
            return Some((ancestor.to_path_buf(), checkout));
        }
        if checkout.is_none() && is_hermes_checkout(ancestor) {
            checkout = Some(ancestor.to_path_buf());
        }
    }
    let checkout = checkout?;
    if let Some(parent) = checkout.parent() {
        let sibling = parent.join("hermes");
        if hermes_install_owns(&sibling) {
            return Some((sibling, Some(checkout)));
        }
    }
    Some((checkout.clone(), Some(checkout)))
}

fn is_hermes_checkout(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    name.eq_ignore_ascii_case("hermes-agent")
        && (path.join("venv").is_dir()
            || path.join(".git").is_dir()
            || path.join("pyproject.toml").is_file())
}

fn is_broad_directory(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    matches!(
        name.to_ascii_lowercase().as_str(),
        "users" | "appdata" | "local" | "localappdata" | "windows" | "program files"
    )
}

fn windows_native_hermes_root(home: &Path, local_app_data: Option<&Path>) -> PathBuf {
    if let Some(local) = local_app_data {
        if !local.as_os_str().is_empty() {
            return local.join("hermes");
        }
    }
    home.join("AppData").join("Local").join("hermes")
}

/// The directory that published this install: launchers, `tools/`, or the checkout.
/// A borrowed data root only has `installs/` from a failed rebuild, and does not count.
pub(crate) fn hermes_install_owns(root: &Path) -> bool {
    if root.join("tools").is_dir() || root.join("hermes-agent").is_dir() {
        return true;
    }
    ["hermes.cmd", "hermes.exe", "hermes"]
        .into_iter()
        .any(|name| root.join("bin").join(name).is_file())
}

fn path_is_inside(path: &Path, root: &Path) -> bool {
    let path = normalize_path_for_set(path);
    let root = normalize_path_for_set(root);
    let root = root.trim_end_matches(['\\', '/']);
    path == root || path.starts_with(&format!("{root}/")) || path.starts_with(&format!("{root}\\"))
}

fn local_app_data_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// Pick the install the running `hermes` belongs to.
///
/// `native_owns` / `legacy_owns` mean that directory published the program
/// (`tools/`, `hermes-agent/`, or `bin/hermes`). A second folder that only
/// contains a failed `installs/` tree is not an owner. When both own an
/// install, the folder that contains the `hermes` command wins.
pub(crate) fn hermes_root_for(
    windows: bool,
    home: &Path,
    local_app_data: Option<&Path>,
    native_owns: bool,
    legacy_owns: bool,
    binary_inside_native: bool,
    binary_inside_legacy: bool,
) -> PathBuf {
    if !windows {
        return home.join(".hermes");
    }
    let native = windows_native_hermes_root(home, local_app_data);
    let legacy = home.join(".hermes");
    if binary_inside_native && (native_owns || !legacy_owns) {
        return native;
    }
    if binary_inside_legacy && (legacy_owns || !native_owns) {
        return legacy;
    }
    if native_owns {
        return native;
    }
    if legacy_owns {
        return legacy;
    }
    native
}

fn adopt_legacy_hermes_home(root: &Path) {
    #[cfg(test)]
    {
        if TEST_HOME_OVERRIDE.with(|cell| cell.borrow().is_none()) {
            return;
        }
    }
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let legacy = home_dir().join(".hermes");
        adopt_legacy_hermes_tree(&legacy, root);
    });
}

/// Copy a pre-Windows-fix `~\.hermes` into the installer home without touching
/// the Python environment that already lives there.
pub(crate) fn adopt_legacy_hermes_tree(legacy: &Path, root: &Path) {
    if paths_same(legacy, root) || !legacy.is_dir() {
        return;
    }
    let _ = copy_hermes_tree(legacy, root);
}

fn paths_same(left: &Path, right: &Path) -> bool {
    normalize_path_for_set(left) == normalize_path_for_set(right)
}

const HERMES_TREE_SKIP: &[&str] = &[
    "installs",
    "tools",
    "venv",
    "environments",
    "node_modules",
    ".git",
    "__pycache__",
    "bin",
    "python",
    "uv",
    "runtime",
    "git",
    "node",
];

fn copy_hermes_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if HERMES_TREE_SKIP
            .iter()
            .any(|skip| name_text.eq_ignore_ascii_case(skip))
        {
            continue;
        }
        let dest = to.join(&name);
        if entry.path().is_dir() {
            copy_hermes_tree(&entry.path(), &dest)?;
        } else if !dest.exists() {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

/// `claude.cmd` inside Hermes's own virtualenv is not Claude Code.
fn is_foreign_venv_shim(path: &Path) -> bool {
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if stem == "hermes" || stem == "hermes-agent" {
        return false;
    }
    let text = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    text.contains("/hermes-agent/venv/") || text.contains("/hermes/venv/")
}

/// Platform config directory (`~/Library/Application Support` on macOS).
///
/// Tests may override via [`with_test_home`], which also redirects this so
/// workspace registry writes stay inside the temp home.
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(test)]
    {
        if let Some(path) = TEST_CONFIG_OVERRIDE.with(|cell| cell.borrow().clone()) {
            return Some(path);
        }
    }
    dirs::config_dir()
}

/// Run `f` with [`home_dir`] / [`home_join`] / [`config_dir`] rooted at `home`
/// for this thread only.
#[cfg(test)]
pub(crate) fn with_test_home<T>(home: &Path, f: impl FnOnce() -> T) -> T {
    let config = home.join("Library/Application Support");
    TEST_HOME_OVERRIDE.with(|home_cell| {
        TEST_CONFIG_OVERRIDE.with(|config_cell| {
            let previous_home = home_cell.replace(Some(home.to_path_buf()));
            let previous_config = config_cell.replace(Some(config));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            *home_cell.borrow_mut() = previous_home;
            *config_cell.borrow_mut() = previous_config;
            match result {
                Ok(value) => value,
                Err(payload) => std::panic::resume_unwind(payload),
            }
        })
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
    assign_merged_path(vec![dir.to_path_buf()]);
}

/// Windows rejects one environment value at 32,767 UTF-16 code units,
/// including the trailing NUL (`SetEnvironmentVariableW`). The whole block
/// shares that cap. Past it, `std::env::set_var` panics. The scan task shows
/// that panic, then the same setup runs on the UI thread, the panic is not
/// caught, and the window closes a few seconds after launch.
///
/// macOS and Linux have no 32,767 cap. `setenv` still panics when the OS
/// rejects the value, and that panic on the UI thread closes the window too.
/// Stay under a fraction of `ARG_MAX`.
fn path_value_limit() -> usize {
    #[cfg(windows)]
    {
        32_766
    }
    #[cfg(not(windows))]
    {
        128 * 1024
    }
}

fn path_value_len(value: &std::ffi::OsStr) -> usize {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        value.encode_wide().count()
    }
    #[cfg(not(windows))]
    {
        value.len()
    }
}

fn path_value_fits(value: &std::ffi::OsStr) -> bool {
    path_value_len(value) <= path_value_limit()
}

#[cfg(any(windows, test))]
fn normalize_windows_path_key(value: &str) -> String {
    let mut value = value.trim().trim_matches('"').trim().replace('/', "\\");
    value.make_ascii_lowercase();
    while value.len() > 3 && value.ends_with('\\') {
        value.pop();
    }
    value
}

fn dedupe_path_dirs(dirs: &mut Vec<PathBuf>) {
    let mut seen = BTreeSet::new();
    dirs.retain(|path| {
        if path.as_os_str().is_empty() {
            return false;
        }
        seen.insert(normalize_path_for_set(path))
    });
}

/// Prepend `prefix`, then as much of the inherited PATH as fits.
///
/// The platform rejects one environment value past the limit, and `set_var`
/// panics, which closes the window. `prefix` is the directories this app
/// adds so Claude and Node resolve; those stay in front. System directories
/// stay as well. The bulky remainder is cut from the end, and every cut is
/// logged to `AgentDoctor/logs/path.log`.
fn assign_merged_path(prefix: Vec<PathBuf>) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());

    let current = std::env::var_os("PATH").unwrap_or_default();
    let inherited: Vec<PathBuf> = std::env::split_paths(&current).collect();

    let mut wanted = prefix.clone();
    wanted.extend(inherited.iter().cloned());
    dedupe_path_dirs(&mut wanted);

    let dirs = select_path_dirs(&prefix, &inherited);
    let dropped: Vec<String> = wanted
        .iter()
        .filter(|dir| !dirs.contains(dir))
        .map(|dir| dir.display().to_string())
        .collect();
    if !dropped.is_empty() {
        log_path_event(&format!(
            "cap cut {} dir(s): {}",
            dropped.len(),
            dropped.join("; ")
        ));
    }
    if !set_path_if_changed(&dirs) {
        log_path_event("PATH value rejected; kept the previous PATH");
    }
}

fn select_path_dirs(prefix: &[PathBuf], inherited: &[PathBuf]) -> Vec<PathBuf> {
    let prefix: Vec<PathBuf> = prefix
        .iter()
        .filter(|dir| !dir.as_os_str().is_empty())
        .cloned()
        .collect();
    let mut protected = Vec::new();
    let mut flexible = Vec::new();
    for dir in inherited {
        if dir.as_os_str().is_empty() {
            continue;
        }
        if is_protected_path_dir(dir) {
            protected.push(dir.clone());
        } else {
            flexible.push(dir.clone());
        }
    }

    let prefix_keep = largest_prefix_that_fits(
        |count| {
            let mut dirs: Vec<PathBuf> = prefix.iter().take(count).cloned().collect();
            dirs.extend(protected.iter().cloned());
            dirs_fit(&dirs)
        },
        prefix.len(),
    );

    if prefix_keep == 0 && !dirs_fit(&protected) {
        let protected_keep = largest_prefix_that_fits(
            |count| dirs_fit(&protected.iter().take(count).cloned().collect::<Vec<_>>()),
            protected.len(),
        );
        let mut dirs: Vec<PathBuf> = protected.into_iter().take(protected_keep).collect();
        dedupe_path_dirs(&mut dirs);
        return dirs;
    }

    let mut dirs: Vec<PathBuf> = prefix.into_iter().take(prefix_keep).collect();
    dirs.extend(protected);
    let flex_keep = largest_prefix_that_fits(
        |count| {
            let mut with_tail = dirs.clone();
            with_tail.extend(flexible.iter().take(count).cloned());
            dirs_fit(&with_tail)
        },
        flexible.len(),
    );
    dirs.extend(flexible.into_iter().take(flex_keep));
    dedupe_path_dirs(&mut dirs);
    dirs
}

/// Largest `count` in `0..=len` for which `fits(count)` is true.
/// `fits(0)` is the head with nothing taken from the candidate list.
fn largest_prefix_that_fits(mut fits: impl FnMut(usize) -> bool, len: usize) -> usize {
    let mut low = 0;
    let mut high = len;
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if fits(mid) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    low
}

fn dirs_fit(dirs: &[PathBuf]) -> bool {
    let mut owned = dirs.to_vec();
    dedupe_path_dirs(&mut owned);
    let Ok(joined) = std::env::join_paths(owned.iter().filter(|path| !path.as_os_str().is_empty()))
    else {
        return false;
    };
    path_value_fits(&joined)
}

fn is_protected_path_dir(path: &Path) -> bool {
    #[cfg(windows)]
    {
        let key = normalize_windows_path_key(&path.to_string_lossy());
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
        let root = normalize_windows_path_key(&root);
        let system32 = format!("{root}\\system32");
        let syswow64 = format!("{root}\\syswow64");
        return key == root
            || key == system32
            || key.starts_with(&format!("{system32}\\"))
            || key == syswow64
            || key.starts_with(&format!("{syswow64}\\"));
    }
    #[cfg(not(windows))]
    {
        matches!(
            path.to_string_lossy().as_ref(),
            "/bin" | "/usr/bin" | "/sbin" | "/usr/sbin"
        )
    }
}

fn set_path_if_changed(dirs: &[PathBuf]) -> bool {
    let Ok(joined) = std::env::join_paths(dirs.iter().filter(|path| !path.as_os_str().is_empty()))
    else {
        return false;
    };
    if !path_value_fits(&joined) {
        return false;
    }
    if std::env::var_os("PATH").unwrap_or_default() == joined {
        return true;
    }
    set_path_value(&joined)
}

fn set_path_value(value: &std::ffi::OsStr) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: the key is the fixed string PATH. `value` comes from
        // `join_paths`, so it has no interior NUL, and callers already
        // rejected values the platform will not store. On Windows this is
        // sound beside other threads. On Unix it is not; this runs once under
        // the PATH lock, and a rejection is caught so it cannot close the window.
        unsafe { std::env::set_var("PATH", value) }
    }))
    .is_ok()
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
            if path_is_file(&candidate) && !is_foreign_venv_shim(&candidate) {
                return Some(candidate);
            }
        }
        let candidate = dir.join(name);
        if path_is_file(&candidate) && !is_foreign_venv_shim(&candidate) {
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
            if path_is_file(&candidate)
                && !is_foreign_venv_shim(&candidate)
                && seen.insert(normalize_path_for_set(&candidate))
            {
                found.push(candidate);
            }
        }
        let candidate = dir.join(name);
        if path_is_file(&candidate)
            && !is_foreign_venv_shim(&candidate)
            && seen.insert(normalize_path_for_set(&candidate))
        {
            found.push(candidate);
        }
    }
    found
}

fn normalize_path_for_set(path: &Path) -> String {
    // Avoid canonicalize(): on Windows it can block for tens of seconds on a
    // disconnected network PATH entry. Trailing slashes and quotes must not
    // count as a second copy, or User + Machine + the live PATH blows past
    // the Windows environment-block cap.
    #[cfg(windows)]
    {
        return normalize_windows_path_key(&path.to_string_lossy());
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
/// omits nvm/npm shims that a developer terminal has. Prepend the managed
/// install locations so discovery matches `cargo run` from a shell.
///
/// The registry PATH is deliberately not read. The inherited PATH already is
/// the logon Machine+User merge, discovery falls back to
/// [`static_common_binary_dirs`], and npm-global resolves through
/// `npm prefix -g`. Merging the registry copies (0.1.87–0.1.90) only
/// shadowed the live PATH and pushed it toward the environment cap.
pub(crate) fn ensure_managed_runtime_path() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        merge_managed_dirs_into_path();
    });
}

/// Re-merge managed dirs into PATH after an install: the npm prefix probe
/// and the managed node root are re-read, so a freshly installed CLI
/// resolves without a restart.
pub fn refresh_managed_runtime_path() {
    invalidate_npm_global_bin_dir_cache();
    merge_managed_dirs_into_path();
}

fn merge_managed_dirs_into_path() {
    // Seed PATH with the dirs child processes need to resolve node/npm.
    // Windows keeps the prefix to the two managed dirs — `find_binary` falls
    // back to `common_binary_dirs()` for discovery, so a longer prefix only
    // spends the 32,767-character budget and shadows the live PATH. Unix
    // shebangs (`#!/usr/bin/env node`) still resolve through PATH, so the
    // static list stays there.
    #[cfg(windows)]
    let mut prefix = vec![managed_nodejs_root()];
    #[cfg(not(windows))]
    let mut prefix = static_common_binary_dirs();
    assign_merged_path(prefix.clone());

    invalidate_npm_global_bin_dir_cache();
    if let Some(npm_bin) = npm_global_bin_dir() {
        prefix.insert(0, npm_bin);
        assign_merged_path(prefix);
    }
}

/// Tiny append-only trail for PATH surgery. Every silent degrade in the
/// 0.1.89–0.1.90 incident was invisible; this file is the post-mortem record.
fn log_path_event(line: &str) {
    use std::io::Write;

    let Some(file) = path_log_file() else {
        return;
    };
    if file.exists()
        && std::fs::metadata(&file)
            .map(|meta| meta.len() > 256 * 1024)
            .unwrap_or(false)
    {
        // Bounded: start a fresh trail instead of growing forever.
        let _ = std::fs::remove_file(&file);
    }
    let Ok(mut handle) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
    else {
        return;
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let _ = writeln!(handle, "[{stamp}] {line}");
}

fn path_log_file() -> Option<PathBuf> {
    let dir = if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(base).join("AgentDoctor")
    } else {
        dirs::data_local_dir()?.join("AgentDoctor")
    }
    .join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("path.log"))
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
    let mut dirs = Vec::new();
    // POSIX prefixes are not Windows directories. Putting them on PATH only
    // spends the 32,767-character block.
    #[cfg(not(windows))]
    {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/opt/homebrew/sbin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.extend(homebrew_node_bin_dirs());
    }

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
#[cfg(not(windows))]
fn homebrew_node_bin_dirs() -> Vec<PathBuf> {
    static CACHE: OnceLock<Vec<PathBuf>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
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
        })
        .clone()
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
    // Doctor sweeps every runtime; prefer the common `--version` flag only so a
    // hanging CLI cannot burn 3× VERSION_PROBE_TIMEOUT serially.
    let version = binary_path
        .as_ref()
        .and_then(|path| read_version(path, &["--version"]));

    AdapterDiscovery {
        installed,
        version,
        binary_path,
    }
}

fn read_version(binary: &PathBuf, flags: &[&str]) -> Option<String> {
    read_version_result_with_flags(binary, flags, VERSION_PROBE_TIMEOUT).unwrap_or_default()
}

pub fn read_version_result(binary: &PathBuf) -> Result<Option<String>, String> {
    // Discovery sweeps stay on the short budget. A diagnose of one runtime
    // can wait for a cold first start.
    read_version_result_with_flags(
        binary,
        &["--version", "-V", "version"],
        VERSION_PROBE_DIAGNOSE_TIMEOUT,
    )
}

fn read_version_result_with_flags(
    binary: &PathBuf,
    flags: &[&str],
    timeout: Duration,
) -> Result<Option<String>, String> {
    // Cursor.exe and similar desktop apps stay open after a version check.
    // The file on disk is enough to know they are installed.
    if skips_version_probe(binary) {
        return Ok(None);
    }
    let mut last_error = None;
    for flag in flags {
        match run_output(binary, &[flag], timeout) {
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
    fn windows_hermes_root_follows_the_install_that_owns_the_program() {
        let home = PathBuf::from(r"C:\Users\Administrator");
        let local = PathBuf::from(r"C:\Users\Administrator\AppData\Local");
        let native = local.join("hermes");
        let legacy = home.join(".hermes");
        assert_eq!(
            hermes_root_for(true, &home, Some(&local), true, false, true, false),
            native
        );
        assert_eq!(
            hermes_root_for(true, &home, Some(&local), false, true, false, true),
            legacy,
            "an install that already lives in the user directory stays there"
        );
        assert_eq!(
            hermes_root_for(true, &home, Some(&local), false, false, false, false),
            native
        );
        assert_eq!(
            hermes_root_for(false, &home, Some(&local), true, true, true, true),
            legacy
        );
    }

    #[test]
    fn failed_rebuild_directory_is_not_the_hermes_install() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let borrowed = home.join(".hermes");
        fs::create_dir_all(borrowed.join("installs").join("dead")).unwrap();
        assert!(!hermes_install_owns(&borrowed));
        let local = temp.path().join("Local");
        let owner = local.join("hermes");
        fs::create_dir_all(owner.join("hermes-agent")).unwrap();
        let command = owner.join("bin").join("hermes.cmd");
        fs::create_dir_all(command.parent().unwrap()).unwrap();
        fs::write(&command, "@echo off\n").unwrap();
        assert!(hermes_install_owns(&owner));
        assert_eq!(
            detect_hermes_root(true, &home, Some(&local), Some(&command)),
            owner
        );
    }

    #[test]
    fn custom_hermes_directory_is_the_install_that_owns_the_command() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let local = temp.path().join("Local");
        let custom = temp.path().join("Tools").join("MyHermes");
        let command = custom.join("bin").join("hermes.cmd");
        fs::create_dir_all(command.parent().unwrap()).unwrap();
        fs::write(&command, "@echo off\n").unwrap();
        fs::create_dir_all(custom.join("tools")).unwrap();

        assert_eq!(
            detect_hermes_root(true, &home, Some(&local), Some(&command)),
            custom
        );
    }

    #[test]
    fn adopts_legacy_profile_without_the_failed_python_env() {
        let temp = tempfile::tempdir().expect("tempdir");
        let legacy = temp.path().join(".hermes");
        let profile = legacy.join("profiles").join("default");
        fs::create_dir_all(profile.join("skills")).unwrap();
        fs::write(profile.join("config.yaml"), "model: glm\n").unwrap();
        fs::create_dir_all(legacy.join("installs").join("dead")).unwrap();
        fs::write(
            legacy.join("installs").join("dead").join("facts.json"),
            "{}",
        )
        .unwrap();
        let root = temp.path().join("Local").join("hermes");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("config.yaml"), "official: true\n").unwrap();

        adopt_legacy_hermes_tree(&legacy, &root);

        assert_eq!(
            fs::read_to_string(root.join("profiles/default/config.yaml")).unwrap(),
            "model: glm\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("config.yaml")).unwrap(),
            "official: true\n"
        );
        assert!(!root.join("installs").exists());
    }

    #[test]
    fn skips_claude_shim_inside_hermes_venv() {
        let temp = tempfile::tempdir().expect("tempdir");
        let shim_dir = temp.path().join("hermes-agent/venv/Scripts");
        fs::create_dir_all(&shim_dir).unwrap();
        write_executable(&shim_dir.join("claude"));
        let real_dir = temp.path().join("Claude");
        fs::create_dir_all(&real_dir).unwrap();
        let real = real_dir.join("claude");
        write_executable(&real);

        let found = find_binary_in_dirs("claude", &[shim_dir.clone(), real_dir.clone()]);
        assert_eq!(found, Some(real));
        let all = find_all_binary_in_dirs("claude", &[shim_dir, real_dir]);
        assert_eq!(all.len(), 1);
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
        #[cfg(not(windows))]
        assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
        #[cfg(windows)]
        assert!(
            !dirs
                .iter()
                .any(|dir| dir == &PathBuf::from("/usr/local/bin")),
            "POSIX prefixes must not consume the Windows PATH block"
        );
    }

    #[test]
    fn windows_path_keys_ignore_case_quotes_and_trailing_slashes() {
        assert_eq!(
            normalize_windows_path_key(r#""C:\Program Files\nodejs\"#),
            r"c:\program files\nodejs"
        );
        assert_eq!(
            normalize_windows_path_key(r"C:\Program Files\nodejs"),
            normalize_windows_path_key("c:/program files/nodejs/")
        );
        assert_eq!(normalize_windows_path_key(r"C:\"), r"c:\");
    }

    fn sample_protected_dir() -> PathBuf {
        #[cfg(windows)]
        {
            let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
            PathBuf::from(root).join("System32")
        }
        #[cfg(not(windows))]
        {
            PathBuf::from("/usr/bin")
        }
    }

    #[test]
    fn oversized_path_is_not_offered_to_the_operating_system() {
        assert!(path_value_fits(std::ffi::OsStr::new(
            "C:\\Windows\\system32"
        )));
        let huge = "A".repeat(path_value_limit() + 8);
        assert!(!path_value_fits(std::ffi::OsStr::new(&huge)));

        let prefix: Vec<_> = (0..8_000)
            .map(|index| PathBuf::from(format!("/agent-doctor-path-prefix/{index}/bin")))
            .collect();
        let protected = sample_protected_dir();
        let selected = select_path_dirs(&prefix, std::slice::from_ref(&protected));
        let joined = std::env::join_paths(&selected).expect("join");
        assert!(path_value_fits(&joined));
        assert!(selected.len() < prefix.len() + 1);
        assert!(selected.contains(&prefix[0]));
        assert!(!selected.contains(&prefix[prefix.len() - 1]));
        assert!(selected.contains(&protected));
    }

    #[test]
    fn claude_and_node_dirs_stay_when_inherited_path_is_too_long() {
        let prefix = vec![
            PathBuf::from("/agent-doctor/.claude/local"),
            PathBuf::from("/agent-doctor/nodejs"),
        ];
        let protected = sample_protected_dir();
        let mut inherited = vec![protected.clone()];
        for index in 0..5_000 {
            inherited.push(PathBuf::from(format!("/agent-doctor-bulky/{index:04}/bin")));
        }
        let selected = select_path_dirs(&prefix, &inherited);
        let joined = std::env::join_paths(&selected).expect("join");
        assert!(path_value_fits(&joined));
        assert_eq!(selected[0], prefix[0]);
        assert!(selected.contains(&prefix[1]));
        assert!(selected.contains(&protected));
        assert!(!selected.contains(inherited.last().unwrap()));
    }

    #[test]
    fn late_inherited_dirs_are_cut_first() {
        let protected = sample_protected_dir();
        let mut inherited = vec![protected.clone(), PathBuf::from("/live/keep")];
        for index in 0..5_000 {
            inherited.push(PathBuf::from(format!("/agent-doctor-bulky/{index:04}/bin")));
        }
        inherited.push(PathBuf::from("/late/install"));
        let selected = select_path_dirs(&[], &inherited);
        let joined = std::env::join_paths(&selected).expect("join");
        assert!(path_value_fits(&joined));
        assert!(selected.contains(&protected));
        assert!(selected.contains(&PathBuf::from("/live/keep")));
        assert!(!selected.contains(&PathBuf::from("/late/install")));
    }
}
