//! Short-lived local process helper for probes and identity checks.
//!
//! Windows GUI apps (Tauri) can hang forever on `Command::output()` when the
//! child is a `.cmd` shim (`npm`, `claude`, …) or a GUI exe that never exits.
//! Always use a timeout, closed stdin, and [`DETACHED_PROCESS`].
//!
//! `CREATE_NO_WINDOW` is not enough when Windows Terminal is the default
//! terminal: each short probe still paints a window and closes it. Opening
//! the app runs one probe per installed command, so that looks like several
//! flashes in a row. A detached process is not handed to that terminal.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Default budget for `--version` / `npm prefix` / CLI identity probes.
pub const SHORT_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Tighter budget for runtime `--version` during doctor / discovery sweeps.
pub const VERSION_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// Diagnose probes one runtime. A new Mac often spends several seconds on the
/// first start (Gatekeeper, a cold Python CLI). Killing that at 1.5s looks
/// like the program was never installed.
pub const VERSION_PROBE_DIAGNOSE_TIMEOUT: Duration = Duration::from_secs(12);

#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x00000008;
#[derive(Debug)]
pub enum RunError {
    Io(std::io::Error),
    TimedOut { timeout: Duration },
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::TimedOut { timeout } => {
                write!(f, "timed out after {}s", timeout.as_secs_f32().max(0.1))
            }
        }
    }
}

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::TimedOut { .. } => None,
        }
    }
}

impl RunError {
    pub fn timed_out(&self) -> bool {
        matches!(self, Self::TimedOut { .. })
    }
}

/// Run `program args…` and capture stdout/stderr, killing the tree on timeout.
pub fn run_output(
    program: impl AsRef<Path>,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, RunError> {
    let program = program.as_ref();
    let mut cmd = build_command(program, args);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_no_window(&mut cmd);

    let child = cmd.spawn().map_err(RunError::Io)?;
    let pid = child.id();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(err)) => Err(RunError::Io(err)),
        Err(_) => {
            kill_process_tree(pid);
            let _ = rx.recv_timeout(Duration::from_millis(400));
            Err(RunError::TimedOut { timeout })
        }
    }
}

type ProbeKey = (PathBuf, Vec<String>, u64, Option<std::time::SystemTime>);

static PROBE_CACHE: std::sync::Mutex<Option<std::collections::HashMap<ProbeKey, Output>>> =
    std::sync::Mutex::new(None);

/// [`run_output`] for read-only probes (`--version`, `npm prefix -g`).
///
/// Opening the app sweeps every runtime several times in its first minute,
/// and on Windows each child can paint a terminal. A successful answer is kept
/// until the program file changes or [`clear_probe_cache`] runs. A bare name
/// (resolved through PATH at spawn time) and failures are never kept.
pub fn run_probe_cached(
    program: impl AsRef<Path>,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, RunError> {
    let program = program.as_ref();
    let Some(key) = probe_key(program, args) else {
        return run_output(program, args, timeout);
    };
    if let Some(hit) = probe_cache_lock()
        .as_ref()
        .and_then(|cache| cache.get(&key).cloned())
    {
        return Ok(hit);
    }
    let output = run_output(program, args, timeout)?;
    if output.status.success() {
        probe_cache_lock()
            .get_or_insert_with(Default::default)
            .insert(key, output.clone());
    }
    Ok(output)
}

/// Forget every cached probe, e.g. after an install changed what is on disk.
pub fn clear_probe_cache() {
    *probe_cache_lock() = None;
}

fn probe_cache_lock(
) -> std::sync::MutexGuard<'static, Option<std::collections::HashMap<ProbeKey, Output>>> {
    PROBE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn probe_key(program: &Path, args: &[&str]) -> Option<ProbeKey> {
    if !program.is_absolute() {
        return None;
    }
    let meta = std::fs::metadata(program).ok()?;
    Some((
        program.to_path_buf(),
        args.iter().map(|arg| arg.to_string()).collect(),
        meta.len(),
        meta.modified().ok(),
    ))
}

fn build_command(program: &Path, args: &[&str]) -> Command {
    #[cfg(windows)]
    {
        // npm global CLIs are `.cmd` shims. `cmd /C shim.cmd` has no console, so
        // Windows gives the `node.exe` that the shim starts its own visible one.
        // Launch node itself so DETACHED_PROCESS applies to the process that runs.
        if let Some((node, script)) = npm_shim_launch(program) {
            let mut cmd = Command::new(node);
            cmd.arg(script);
            cmd.args(args);
            return cmd;
        }
        // Same window, different target: the shim starts `claude.exe` (a console
        // program). `cmd /C` has no console, so Windows opens one for that exe,
        // prints the failure there, and closes it when the process exits.
        if let Some(exe) = shim_exe_target(program) {
            let mut cmd = Command::new(exe);
            cmd.args(args);
            return cmd;
        }
        let ext = program
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "cmd" || ext == "bat" {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg(program).args(args);
            return cmd;
        }
    }
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd
}

/// Version probes must not start a desktop app. GUI executables ignore
/// `DETACHED_PROCESS` and often stay open after the stub exits.
pub fn skips_version_probe(path: &Path) -> bool {
    // Desktop apps (Cursor.exe and shims that only start them) ignore
    // DETACHED_PROCESS and stay open. Presence of the file is enough.
    if launch_opens_gui_window(path) {
        return true;
    }
    #[cfg(windows)]
    {
        // A batch file we cannot turn into the real program has to run through
        // cmd.exe. The program that script starts then gets its own terminal.
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if (ext == "cmd" || ext == "bat")
            && npm_shim_launch(path).is_none()
            && shim_exe_target(path).is_none()
        {
            return true;
        }
    }
    false
}

fn launch_opens_gui_window(path: &Path) -> bool {
    if is_gui_subsystem_exe(path) {
        return true;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "cmd" && ext != "bat" {
        return false;
    }
    // A node shim prints `--version` and exits. A shim that starts Cursor.exe
    // (or another GUI) leaves that window behind.
    if npm_shim_launch(path).is_some() {
        return false;
    }
    cmd_references_gui_exe(path)
}

fn is_gui_subsystem_exe(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "exe" {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    pe_subsystem_is_gui(&bytes)
}

/// `IMAGE_SUBSYSTEM_WINDOWS_GUI` is 2. Console images are 3.
fn pe_subsystem_is_gui(bytes: &[u8]) -> bool {
    if bytes.len() < 0x40 || bytes[0] != b'M' || bytes[1] != b'Z' {
        return false;
    }
    let e_lfanew = u32::from_le_bytes(bytes[0x3C..0x40].try_into().unwrap_or([0; 4])) as usize;
    let pe = e_lfanew;
    // PE signature + COFF (20) + optional header through Subsystem (70).
    if bytes.len() < pe + 4 + 20 + 70 {
        return false;
    }
    if &bytes[pe..pe + 4] != b"PE\0\0" {
        return false;
    }
    let optional = pe + 24;
    let magic = u16::from_le_bytes(bytes[optional..optional + 2].try_into().unwrap_or([0; 2]));
    if magic != 0x10B && magic != 0x20B {
        return false;
    }
    let subsystem = u16::from_le_bytes(
        bytes[optional + 68..optional + 70]
            .try_into()
            .unwrap_or([0; 2]),
    );
    subsystem == 2
}

/// npm writes some global `.cmd` shims as UTF-16. Reading them as UTF-8 fails,
/// and the fallback `cmd /C` then gives `node.exe` its own terminal window.
fn read_cmd_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        return Some(String::from_utf16_lossy(&units));
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        return Some(String::from_utf16_lossy(&units));
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    String::from_utf8(bytes.to_vec()).ok()
}

/// npm `cmd-shim`: the batch file's job is `node.exe` plus a `.js` entry.
/// The node is resolved absolutely by [`resolve_shim_node`], so a truncated
/// PATH cannot break the launch.
fn npm_shim_launch(cmd_path: &Path) -> Option<(PathBuf, PathBuf)> {
    let text = read_cmd_text(cmd_path)?;
    let dir = cmd_path.parent()?;
    let rel = npm_shim_script_rel(&text)?;
    let script = resolve_under(dir, &rel);
    let ext = script
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(ext.as_str(), "js" | "cjs" | "mjs") || !script.is_file() {
        return None;
    }
    let node = resolve_shim_node(dir);
    Some((node, script))
}

/// The `node.exe` that runs an npm shim, as an absolute path when one exists.
///
/// A long-PATH machine can shed the nodejs dirs from this process's PATH, and
/// the shim runs whatever `node` the child then resolves — every npm CLI dies
/// while node is installed. Resolve absolutely and keep the bare name (PATH at
/// spawn time) only as the last resort.
fn resolve_shim_node(shim_dir: &Path) -> PathBuf {
    if let Some(node) = node_candidates_for_shim(shim_dir)
        .into_iter()
        .find(|candidate| candidate.is_file())
    {
        return node;
    }
    PathBuf::from("node")
}

/// Ordered candidates: node shipped beside the shim (the app's managed npm
/// prefix keeps one there), the app-managed runtime, then well-known machine
/// installs.
fn node_candidates_for_shim(shim_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![shim_dir.join("node.exe")];
    candidates.push(crate::adapters::util::managed_nodejs_root().join("node.exe"));
    candidates.extend(
        well_known_node_dirs()
            .into_iter()
            .map(|dir| dir.join("node.exe")),
    );
    candidates
}

/// Directories a standard Windows node install keeps `node.exe` in.
fn well_known_node_dirs() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut dirs = Vec::new();
        for key in ["ProgramFiles", "ProgramW6432"] {
            if let Ok(root) = std::env::var(key) {
                if !root.is_empty() {
                    dirs.push(PathBuf::from(root).join("nodejs"));
                }
            }
        }
        // nvm-for-windows points NVM_SYMLINK at the active node dir itself.
        if let Ok(root) = std::env::var("NVM_SYMLINK") {
            if !root.is_empty() {
                dirs.push(PathBuf::from(root));
            }
        }
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join(r"scoop\apps\nodejs\current"));
            dirs.push(home.join(r"AppData\Local\Programs\nodejs"));
        }
        dirs
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn npm_shim_script_rel(text: &str) -> Option<String> {
    // Node's own `npm.cmd` / `npx.cmd` keep the entry in a variable
    // (`SET "NPM_CLI_JS=%~dp0\…\npm-cli.js"`) and launch `"%NODE_EXE%"  "%NPM_CLI_JS%" %*`.
    // The line that forwards `%*` names the real entry; earlier helper scripts do not.
    let vars = dp0_script_vars(text);
    for line in text.lines().filter(|line| line.contains("%*")) {
        for token in quoted_tokens(line) {
            if let Some(rel) = strip_dp0(token).filter(|rel| is_js_entry(rel)) {
                return Some(rel);
            }
            let name = token.strip_prefix('%').and_then(|t| t.strip_suffix('%'));
            if let Some(rel) = name.and_then(|name| vars.get(&name.to_ascii_uppercase())) {
                return Some(rel.clone());
            }
        }
    }
    for line in text.lines() {
        if let Some(rel) = quoted_dp0_script(line) {
            return Some(rel);
        }
    }
    None
}

fn quoted_dp0_script(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'"' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let end_rel = line[start..].find('"')?;
        let end = start + end_rel;
        let token = &line[start..end];
        if let Some(rel) = strip_dp0(token).filter(|rel| is_js_entry(rel)) {
            return Some(rel);
        }
        i = end + 1;
    }
    None
}

fn is_js_entry(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    lower.ends_with(".js") || lower.ends_with(".cjs") || lower.ends_with(".mjs")
}

/// `SET "NAME=%~dp0\x.js"` lines, keyed by upper-case name (batch variables ignore case).
fn dp0_script_vars(text: &str) -> std::collections::HashMap<String, String> {
    let mut vars = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(head) = line.get(..4) else {
            continue;
        };
        if !head.eq_ignore_ascii_case("set ") {
            continue;
        }
        let rest = line[4..].trim();
        let rest = match rest.strip_prefix('"') {
            Some(quoted) => quoted.split('"').next().unwrap_or(""),
            None => rest,
        };
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        if let Some(rel) = strip_dp0(value.trim()).filter(|rel| is_js_entry(rel)) {
            vars.insert(name.trim().to_ascii_uppercase(), rel);
        }
    }
    vars
}

/// The program a `.cmd` shim actually starts, when that program is an `.exe`
/// next to the shim (the native Claude install) rather than a node script.
/// Callers are Windows-only; tests on other platforms still resolve the path.
#[cfg_attr(not(windows), allow(dead_code))]
fn shim_exe_target(cmd_path: &Path) -> Option<PathBuf> {
    let text = read_cmd_text(cmd_path)?;
    let dir = cmd_path.parent()?;
    for token in exe_tokens(&text) {
        let lower = token.to_ascii_lowercase();
        if !lower.ends_with(".exe") {
            continue;
        }
        if lower == "node.exe" || lower.ends_with("\\node.exe") || lower.ends_with("/node.exe") {
            continue;
        }
        let Some(resolved) = resolve_shim_token(dir, &token) else {
            continue;
        };
        if resolved.is_file() && !is_gui_subsystem_exe(&resolved) {
            return Some(resolved);
        }
    }
    None
}

#[cfg_attr(not(windows), allow(dead_code))]
fn exe_tokens(text: &str) -> Vec<String> {
    let mut found: Vec<String> = quoted_tokens(text)
        .into_iter()
        .map(str::to_string)
        .collect();
    for word in text.split_whitespace() {
        let word = word.trim_matches(|c: char| c == '"' || c == '\'');
        if word.to_ascii_lowercase().ends_with(".exe") && !found.iter().any(|token| token == word) {
            found.push(word.to_string());
        }
    }
    found
}

#[cfg_attr(not(windows), allow(dead_code))]
fn resolve_shim_token(dir: &Path, token: &str) -> Option<PathBuf> {
    if let Some(rel) = strip_dp0(token) {
        return Some(resolve_under(dir, &rel));
    }
    if token.contains('%') {
        return None;
    }
    let path = PathBuf::from(token);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(resolve_under(dir, token))
    }
}

fn strip_dp0(token: &str) -> Option<String> {
    let rest = token
        .strip_prefix("%~dp0%")
        .or_else(|| token.strip_prefix("%dp0%"))
        .or_else(|| token.strip_prefix("%~dp0"));
    let rest = rest?;
    let rest = rest.trim_start_matches(['\\', '/']);
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

fn resolve_under(dir: &Path, rel: &str) -> PathBuf {
    let mut path = dir.to_path_buf();
    for part in rel.split(['\\', '/']) {
        if part.is_empty() || part == "." {
            continue;
        }
        path.push(part);
    }
    path
}

fn cmd_references_gui_exe(cmd_path: &Path) -> bool {
    let Some(text) = read_cmd_text(cmd_path) else {
        return false;
    };
    let Some(dir) = cmd_path.parent() else {
        return false;
    };
    for token in quoted_tokens(&text) {
        if !token.to_ascii_lowercase().ends_with(".exe") {
            continue;
        }
        let resolved = if let Some(rel) = strip_dp0(token) {
            resolve_under(dir, &rel)
        } else if token.contains("%~dp0") || token.contains("%dp0%") {
            continue;
        } else {
            let path = PathBuf::from(token);
            if path.is_absolute() {
                path
            } else {
                resolve_under(dir, token)
            }
        };
        if is_gui_subsystem_exe(&resolved) {
            return true;
        }
    }
    false
}

fn quoted_tokens(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            if let Some(end_rel) = text[start..].find('"') {
                let end = start + end_rel;
                tokens.push(&text[start..end]);
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    tokens
}

/// Build a spawnable CLI command. Windows starts npm `.cmd` shims as `node`
/// directly and detaches the console, so a GUI app does not flash a terminal.
pub fn command_for_path(program: &Path) -> Command {
    let mut cmd = build_command(program, &[]);
    apply_no_window(&mut cmd);
    cmd
}

fn apply_no_window(_cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        _cmd.creation_flags(DETACHED_PROCESS);
        log_spawn(_cmd);
    }
}

/// One line per console child in `%LOCALAPPDATA%\AgentDoctor\logs\spawn.log`.
/// A terminal flash names no program; this file does.
#[cfg(windows)]
fn log_spawn(cmd: &Command) {
    use std::io::Write;

    static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    let Some(base) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let dir = PathBuf::from(base).join("AgentDoctor").join("logs");
    let args: Vec<String> = cmd
        .get_args()
        .map(|arg| spawn_log_arg(&arg.to_string_lossy()))
        .collect();
    let line = format!(
        "[{}] {} {}\n",
        local_stamp(),
        cmd.get_program().to_string_lossy(),
        args.join(" ")
    );

    // Probes run on several threads; one locked write keeps each line whole.
    let _guard = WRITE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let file = dir.join("spawn.log");
    if std::fs::metadata(&file).is_ok_and(|meta| meta.len() > 256 * 1024) {
        let _ = std::fs::remove_file(&file);
    }
    if let Ok(mut handle) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
    {
        let _ = handle.write_all(line.as_bytes());
    }
}

/// `2026-10-20 11:34:42.517`, local time.
#[cfg(windows)]
fn local_stamp() -> String {
    // SAFETY: GetLocalTime only writes the returned SYSTEMTIME.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

/// Paths and flags stay readable. A long bare token may be a key, and prompt
/// text does not belong in a log, so both are cut.
#[cfg_attr(not(windows), allow(dead_code))]
fn spawn_log_arg(arg: &str) -> String {
    let looks_like_path = arg.contains('\\') || arg.contains('/');
    if !looks_like_path && !arg.contains(' ') && arg.chars().count() > 24 {
        return "<hidden>".to_string();
    }
    let limit = if looks_like_path { 160 } else { 40 };
    if arg.chars().count() > limit {
        let head: String = arg.chars().take(limit).collect();
        format!("{head}…")
    } else {
        arg.to_string()
    }
}

pub fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(DETACHED_PROCESS)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_out_a_sleeping_process() {
        #[cfg(windows)]
        let result = run_output(
            "ping",
            &["-n", "8", "127.0.0.1"],
            Duration::from_millis(300),
        );
        #[cfg(not(windows))]
        let result = run_output("sleep", &["8"], Duration::from_millis(300));

        assert!(
            result.as_ref().err().is_some_and(RunError::timed_out),
            "expected timeout, got {result:?}"
        );
    }

    #[test]
    fn captures_quick_success() {
        #[cfg(windows)]
        let output = run_output("cmd", &["/C", "echo ok"], SHORT_PROBE_TIMEOUT).expect("echo");
        #[cfg(not(windows))]
        let output = run_output("echo", &["ok"], SHORT_PROBE_TIMEOUT).expect("echo");

        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("ok"));
    }

    #[test]
    fn npm_shim_launches_node_script() {
        let temp = tempfile::tempdir().expect("tempdir");
        let script = temp
            .path()
            .join("node_modules")
            .join("claude")
            .join("cli.js");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::write(&script, "console.log('1.0.0')\n").unwrap();
        let cmd = temp.path().join("claude.cmd");
        std::fs::write(
            &cmd,
            "@ECHO off\r\n\"%_prog%\" \"%dp0%\\node_modules\\claude\\cli.js\" %*\r\n",
        )
        .unwrap();

        let (node, launched) = npm_shim_launch(&cmd).expect("shim");
        assert!(
            node == Path::new("node") || (node.is_absolute() && node.is_file()),
            "node must be the bare name or an absolute existing file: {node:?}"
        );
        assert_eq!(launched, script);
        assert!(!launch_opens_gui_window(&cmd));

        let mut wide = Vec::new();
        wide.extend_from_slice(&[0xFF, 0xFE]);
        for unit in
            "@ECHO off\r\n\"%_prog%\" \"%dp0%\\node_modules\\claude\\cli.js\" %*\r\n".encode_utf16()
        {
            wide.extend_from_slice(&unit.to_le_bytes());
        }
        let wide_cmd = temp.path().join("claude-wide.cmd");
        std::fs::write(&wide_cmd, wide).unwrap();
        let (node, launched) = npm_shim_launch(&wide_cmd).expect("utf-16 shim");
        assert_eq!(launched, script);
        assert!(
            node == Path::new("node") || (node.is_absolute() && node.is_file()),
            "node must be the bare name or an absolute existing file: {node:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn probe_cache_runs_an_unchanged_program_once() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let counter = temp.path().join("runs");
        let script = temp.path().join("probe.sh");
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho x >> '{}'\necho 1.0.0\n", counter.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let runs = || {
            std::fs::read_to_string(&counter)
                .map(|text| text.lines().count())
                .unwrap_or(0)
        };

        let first = run_probe_cached(&script, &["--version"], SHORT_PROBE_TIMEOUT).expect("first");
        let second =
            run_probe_cached(&script, &["--version"], SHORT_PROBE_TIMEOUT).expect("second");
        assert_eq!(first.stdout, second.stdout);
        assert_eq!(runs(), 1);

        run_probe_cached(&script, &["-V"], SHORT_PROBE_TIMEOUT).expect("other args");
        assert_eq!(runs(), 2);

        clear_probe_cache();
        run_probe_cached(&script, &["--version"], SHORT_PROBE_TIMEOUT).expect("after clear");
        assert_eq!(runs(), 3);
    }

    #[test]
    fn spawn_log_hides_keys_and_long_text() {
        assert_eq!(spawn_log_arg("--version"), "--version");
        assert_eq!(
            spawn_log_arg("sk-abcdefghijklmnopqrstuvwxyz123456"),
            "<hidden>"
        );
        assert_eq!(
            spawn_log_arg(r"C:\Users\Admin\AppData\Local\AgentDoctor\runtime\nodejs\npm.cmd"),
            r"C:\Users\Admin\AppData\Local\AgentDoctor\runtime\nodejs\npm.cmd"
        );
        let prompt = "please fix the config file and then restart the gateway now";
        assert!(spawn_log_arg(prompt).ends_with('…'));
    }

    #[test]
    fn node_bundled_npm_cmd_launches_npm_cli() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("node.exe"), b"MZ stub").unwrap();
        let bin = temp.path().join("node_modules").join("npm").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("npm-cli.js"), "").unwrap();
        std::fs::write(bin.join("npm-prefix.js"), "").unwrap();
        let cmd = temp.path().join("npm.cmd");
        std::fs::write(
            &cmd,
            ":: Created by npm, please don't edit manually.\r\n\
             @ECHO OFF\r\n\
             \r\n\
             SETLOCAL\r\n\
             \r\n\
             SET \"NODE_EXE=%~dp0\\node.exe\"\r\n\
             IF NOT EXIST \"%NODE_EXE%\" (\r\n\
             \x20 SET \"NODE_EXE=node\"\r\n\
             )\r\n\
             \r\n\
             SET \"NPM_PREFIX_JS=%~dp0\\node_modules\\npm\\bin\\npm-prefix.js\"\r\n\
             SET \"NPM_CLI_JS=%~dp0\\node_modules\\npm\\bin\\npm-cli.js\"\r\n\
             FOR /F \"delims=\" %%F IN ('CALL \"%NODE_EXE%\" \"%NPM_PREFIX_JS%\"') DO (\r\n\
             \x20 SET \"NPM_PREFIX_NPM_CLI_JS=%%F\\node_modules\\npm\\bin\\npm-cli.js\"\r\n\
             )\r\n\
             IF EXIST \"%NPM_PREFIX_NPM_CLI_JS%\" (\r\n\
             \x20 SET \"NPM_CLI_JS=%NPM_PREFIX_NPM_CLI_JS%\"\r\n\
             )\r\n\
             \r\n\
             \"%NODE_EXE%\"  \"%NPM_CLI_JS%\" %*\r\n",
        )
        .unwrap();

        let (node, launched) = npm_shim_launch(&cmd).expect("npm.cmd");
        assert_eq!(node, temp.path().join("node.exe"));
        assert_eq!(launched, bin.join("npm-cli.js"));
    }

    #[test]
    fn shim_prefers_the_node_beside_it() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("node.exe"), b"MZ stub").unwrap();
        let script = temp
            .path()
            .join("node_modules")
            .join("claude")
            .join("cli.js");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::write(&script, "console.log('1.0.0')\n").unwrap();
        let cmd = temp.path().join("claude.cmd");
        std::fs::write(
            &cmd,
            "@ECHO off\r\n\"%_prog%\" \"%dp0%\\node_modules\\claude\\cli.js\" %*\r\n",
        )
        .unwrap();

        let (node, launched) = npm_shim_launch(&cmd).expect("shim");
        assert_eq!(node, temp.path().join("node.exe"));
        assert_eq!(launched, script);
    }

    #[test]
    fn exe_shim_launches_the_program_not_cmd() {
        let temp = tempfile::tempdir().expect("tempdir");
        let exe = temp
            .path()
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code")
            .join("bin")
            .join("claude.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"not a pe").unwrap();
        let cmd = temp.path().join("claude.cmd");
        std::fs::write(
            &cmd,
            "@ECHO off\r\n\"%_prog%\" \"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe\" %*\r\n",
        )
        .unwrap();

        assert!(npm_shim_launch(&cmd).is_none());
        assert_eq!(shim_exe_target(&cmd).as_deref(), Some(exe.as_path()));
        assert!(!skips_version_probe(&cmd));

        #[cfg(windows)]
        {
            let built = build_command(&cmd, &["--version"]);
            assert_eq!(built.get_program(), exe.as_os_str());
        }
    }

    #[test]
    fn gui_exe_and_gui_shim_skip_version_probe() {
        let temp = tempfile::tempdir().expect("tempdir");
        let exe = temp.path().join("Cursor.exe");
        std::fs::write(&exe, gui_pe_bytes()).unwrap();
        assert!(is_gui_subsystem_exe(&exe));

        let console = temp.path().join("node.exe");
        std::fs::write(&console, console_pe_bytes()).unwrap();
        assert!(!is_gui_subsystem_exe(&console));

        let cmd = temp.path().join("cursor.cmd");
        std::fs::write(&cmd, "@echo off\r\n\"%~dp0Cursor.exe\" %*\r\n").unwrap();
        assert!(launch_opens_gui_window(&cmd));
        assert!(launch_opens_gui_window(&exe));
    }

    fn gui_pe_bytes() -> Vec<u8> {
        pe_bytes(2)
    }

    fn console_pe_bytes() -> Vec<u8> {
        pe_bytes(3)
    }

    fn pe_bytes(subsystem: u16) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x100];
        bytes[0] = b'M';
        bytes[1] = b'Z';
        let e_lfanew: u32 = 0x80;
        bytes[0x3C..0x40].copy_from_slice(&e_lfanew.to_le_bytes());
        let pe = e_lfanew as usize;
        bytes[pe..pe + 4].copy_from_slice(b"PE\0\0");
        let optional = pe + 24;
        bytes[optional..optional + 2].copy_from_slice(&0x20B_u16.to_le_bytes());
        bytes[optional + 68..optional + 70].copy_from_slice(&subsystem.to_le_bytes());
        bytes
    }
}
