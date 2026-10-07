//! Short-lived local process helper for probes and identity checks.
//!
//! Windows GUI apps (Tauri) can hang forever on `Command::output()` when the
//! child is a `.cmd` shim (`npm`, `claude`, …) or a GUI exe that never exits.
//! Always use a timeout, closed stdin, and `CREATE_NO_WINDOW`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Default budget for `--version` / `npm prefix` / CLI identity probes.
pub const SHORT_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Tighter budget for runtime `--version` during doctor / discovery sweeps.
pub const VERSION_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

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

fn build_command(program: &Path, args: &[&str]) -> Command {
    #[cfg(windows)]
    {
        // npm global CLIs are `.cmd` shims. `cmd /C shim.cmd` has no console, so
        // Windows gives the `node.exe` that the shim starts its own visible one.
        // Launch node itself so CREATE_NO_WINDOW applies to the process that runs.
        if let Some((node, script)) = npm_shim_launch(program) {
            let mut cmd = Command::new(node);
            cmd.arg(script);
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
/// `CREATE_NO_WINDOW` and often stay open after the stub exits.
pub fn skips_version_probe(path: &Path) -> bool {
    // Desktop apps (Cursor.exe and shims that only start them) ignore
    // CREATE_NO_WINDOW and stay open. Presence of the file is enough.
    launch_opens_gui_window(path)
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

/// npm `cmd-shim`: the batch file's job is `node.exe` plus a `.js` entry.
fn npm_shim_launch(cmd_path: &Path) -> Option<(PathBuf, PathBuf)> {
    let text = std::fs::read_to_string(cmd_path).ok()?;
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
    let beside = dir.join("node.exe");
    let node = if beside.is_file() {
        beside
    } else {
        PathBuf::from("node")
    };
    Some((node, script))
}

fn npm_shim_script_rel(text: &str) -> Option<String> {
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
        if let Some(rel) = strip_dp0(token) {
            let lower = rel.to_ascii_lowercase();
            if lower.ends_with(".js") || lower.ends_with(".cjs") || lower.ends_with(".mjs") {
                return Some(rel);
            }
        }
        i = end + 1;
    }
    None
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
    let Ok(text) = std::fs::read_to_string(cmd_path) else {
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
/// directly and hides the console, so a GUI app does not flash a terminal.
pub fn command_for_path(program: &Path) -> Command {
    let mut cmd = build_command(program, &[]);
    apply_no_window(&mut cmd);
    cmd
}

fn apply_no_window(_cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        _cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

pub fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
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
        assert_eq!(node, PathBuf::from("node"));
        assert_eq!(launched, script);
        assert!(!launch_opens_gui_window(&cmd));
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
