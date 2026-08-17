//! Short-lived local process helper for probes and identity checks.
//!
//! Windows GUI apps (Tauri) can hang forever on `Command::output()` when the
//! child is a `.cmd` shim (`npm`, `claude`, …) or a GUI exe that never exits.
//! Always use a timeout, closed stdin, and `CREATE_NO_WINDOW`.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Default budget for `--version` / `npm prefix` / CLI identity probes.
pub const SHORT_PROBE_TIMEOUT: Duration = Duration::from_secs(4);

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

fn apply_no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd;
}

fn kill_process_tree(pid: u32) {
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
}
