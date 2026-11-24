//! Read the user's login shell environment, the way VS Code and Paseo do.
//!
//! The shell runs the user's whole `~/.zshrc`: conda, nvm, oh-my-zsh update
//! prompts, sometimes `tmux` auto-attach. So it runs with stdin closed, in its
//! own process group, under a hard time budget, and its output is cut between
//! two random markers. Any failure means "no shell variables", never a hang.

use std::time::Duration;

/// Whole budget for both attempts (interactive, then non-interactive).
pub(crate) const CAPTURE_BUDGET: Duration = Duration::from_secs(6);

#[cfg(not(unix))]
pub(crate) fn capture(_budget: Duration) -> Option<super::Vars> {
    None
}

#[cfg(unix)]
pub(crate) use unix::capture;

#[cfg(unix)]
pub(crate) mod unix {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use crate::Vars;

    /// Lets a user's rc file skip slow setup, like VS Code's
    /// `VSCODE_RESOLVING_ENVIRONMENT`.
    const RESOLVING_ENV: &str = "AGENT_DOCTOR_RESOLVING_ENVIRONMENT";
    const BOOTSTRAP_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

    pub(crate) fn capture(budget: Duration) -> Option<Vars> {
        capture_with(&login_shell(), &[], budget)
    }

    /// `extra_env` is for tests (`ZDOTDIR`, `HOME`).
    pub(crate) fn capture_with(
        shell: &Path,
        extra_env: &[(&str, &str)],
        budget: Duration,
    ) -> Option<Vars> {
        let name = shell.file_name()?.to_str()?;
        if !matches!(name, "zsh" | "bash" | "sh" | "dash" | "ksh" | "fish") {
            return None;
        }
        let mark = marker();
        let script = format!("printf '%s\\n' {mark}; /usr/bin/env; printf '%s\\n' {mark}");
        let started = Instant::now();
        let attempts: [&[&str]; 2] = [&["-i", "-l", "-c"], &["-l", "-c"]];
        for (index, flags) in attempts.iter().enumerate() {
            let left = budget.saturating_sub(started.elapsed());
            if left.is_zero() {
                break;
            }
            let slice = if index == 0 { left / 2 } else { left };
            let Some(stdout) = run(shell, flags, &script, extra_env, slice) else {
                continue;
            };
            if let Some(vars) = parse(&stdout, &mark) {
                return Some(vars);
            }
        }
        None
    }

    fn login_shell() -> PathBuf {
        if let Some(shell) = std::env::var_os("SHELL").map(PathBuf::from) {
            if shell.is_absolute() && shell.exists() {
                return shell;
            }
        }
        if let Some(shell) = passwd_shell() {
            return shell;
        }
        PathBuf::from(if cfg!(target_os = "macos") {
            "/bin/zsh"
        } else {
            "/bin/sh"
        })
    }

    fn passwd_shell() -> Option<PathBuf> {
        // SAFETY: getpwuid returns a pointer into static storage or null; the
        // string is copied out before any other passwd call.
        unsafe {
            let entry = libc::getpwuid(libc::getuid());
            if entry.is_null() || (*entry).pw_shell.is_null() {
                return None;
            }
            let shell = std::ffi::CStr::from_ptr((*entry).pw_shell)
                .to_str()
                .ok()?
                .to_string();
            let path = PathBuf::from(shell);
            (path.is_absolute() && path.exists() && !path.ends_with("false")).then_some(path)
        }
    }

    fn marker() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        format!("ADENV{:x}{:x}", std::process::id(), nanos)
    }

    fn run(
        shell: &Path,
        flags: &[&str],
        script: &str,
        extra_env: &[(&str, &str)],
        timeout: Duration,
    ) -> Option<String> {
        let path = match std::env::var("PATH") {
            Ok(current) if !current.is_empty() => format!("{BOOTSTRAP_PATH}:{current}"),
            _ => BOOTSTRAP_PATH.to_string(),
        };
        // Unix-only and stdin-closed: no console window exists on these platforms.
        let mut cmd = Command::new(shell); // raw-spawn-ok
        cmd.args(flags)
            .arg(script)
            .env("PATH", path)
            .env(RESOLVING_ENV, "1")
            .env("DISABLE_AUTO_UPDATE", "true")
            .env("ZSH_TMUX_AUTOSTART", "false")
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        for (key, value) in extra_env {
            cmd.env(key, value);
        }
        let mut child = cmd.spawn().ok()?;
        let group = child.id() as libc::pid_t;

        let buffer = Arc::new(Mutex::new(Vec::new()));
        let mut stdout = child.stdout.take()?;
        let sink = Arc::clone(&buffer);
        let reader = std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            loop {
                match stdout.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if let Ok(mut buf) = sink.lock() {
                            buf.extend_from_slice(&chunk[..read]);
                        }
                    }
                }
            }
        });

        let deadline = Instant::now() + timeout;
        let exited = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => break None,
            }
        };
        // A background job the rc file started can hold stdout open after the
        // shell exits. Give it a moment, then end the whole group.
        let drain_until = Instant::now() + Duration::from_millis(500);
        while exited.is_some() && !reader.is_finished() && Instant::now() < drain_until {
            std::thread::sleep(Duration::from_millis(10));
        }
        if exited.is_none() || !reader.is_finished() {
            // SAFETY: plain kill(2) on the process group this call created.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
            let _ = child.wait();
        }
        let status = exited?;
        if !status.success() {
            return None;
        }
        let bytes = buffer.lock().ok()?.clone();
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub(crate) fn parse(stdout: &str, mark: &str) -> Option<Vars> {
        let mut lines = stdout.lines();
        lines.by_ref().find(|line| line.trim() == mark)?;
        let mut vars = Vars::new();
        for line in lines {
            if line.trim() == mark {
                return Some(vars);
            }
            if let Some((key, value)) = line.split_once('=') {
                if !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                    vars.insert(key.to_string(), value.to_string());
                }
            }
        }
        None
    }
}
