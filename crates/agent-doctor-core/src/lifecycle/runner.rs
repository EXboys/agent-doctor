use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct ShellCapture {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

impl ShellCapture {
    pub fn combined_output(&self) -> String {
        if self.stderr.trim().is_empty() {
            self.stdout.clone()
        } else if self.stdout.trim().is_empty() {
            self.stderr.clone()
        } else {
            format!("{}\n{}", self.stdout.trim(), self.stderr.trim())
        }
    }
}

pub(crate) fn run_shell_command(command_line: &str) -> Result<()> {
    match run_shell_command_capturing(command_line) {
        Ok(capture) if capture.success => Ok(()),
        Ok(capture) => Err(finish_lifecycle_error(&capture)),
        Err(error) => Err(error),
    }
}

pub fn run_shell_command_capturing(command_line: &str) -> Result<ShellCapture> {
    run_shell_command_streaming(command_line, |_| {})
}

/// Run a shell command, streaming each stdout/stderr line to `on_line`.
pub fn run_shell_command_streaming<F>(command_line: &str, on_line: F) -> Result<ShellCapture>
where
    F: FnMut(&str),
{
    crate::adapters::util::ensure_managed_runtime_path();

    #[cfg(unix)]
    let mut child = Command::new("bash")
        .arg("-c")
        .arg(command_line)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start install shell")?;

    #[cfg(windows)]
    let mut child = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        Command::new("cmd")
            .args(["/C", command_line])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .context("failed to start install shell")?
    };

    let stdout = child.stdout.take().context("missing stdout pipe")?;
    let stderr = child.stderr.take().context("missing stderr pipe")?;

    let queue: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let stdout_acc: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let stderr_acc: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    let queue_out = Arc::clone(&queue);
    let acc_out = Arc::clone(&stdout_acc);
    let stdout_handle = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(mut acc) = acc_out.lock() {
                acc.push_str(&line);
                acc.push('\n');
            }
            if let Ok(mut q) = queue_out.lock() {
                q.push(line);
            }
        }
    });

    let queue_err = Arc::clone(&queue);
    let acc_err = Arc::clone(&stderr_acc);
    let stderr_handle = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Ok(mut acc) = acc_err.lock() {
                acc.push_str(&line);
                acc.push('\n');
            }
            if let Ok(mut q) = queue_err.lock() {
                q.push(line);
            }
        }
    });

    let mut on_line = on_line;
    let status = loop {
        let drained = {
            let mut guard = queue.lock().unwrap_or_else(|error| error.into_inner());
            guard.drain(..).collect::<Vec<_>>()
        };
        for line in drained {
            on_line(&line);
        }

        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(80)),
            Err(error) => return Err(error).context("failed waiting for install shell"),
        }
    };

    let _ = stdout_handle.join();
    let _ = stderr_handle.join();

    let drained = {
        let mut guard = queue.lock().unwrap_or_else(|error| error.into_inner());
        guard.drain(..).collect::<Vec<_>>()
    };
    for line in drained {
        on_line(&line);
    }

    let stdout = stdout_acc
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let stderr = stderr_acc
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();

    Ok(ShellCapture {
        success: status.success(),
        stdout,
        stderr,
        exit_code: status.code(),
    })
}

fn finish_lifecycle_error(capture: &ShellCapture) -> anyhow::Error {
    let raw = capture.combined_output();
    let detail = last_lines(&raw, 8);
    if detail.is_empty() {
        anyhow::anyhow!("installer exited with status {:?}", capture.exit_code)
    } else {
        anyhow::anyhow!("{detail}")
    }
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

pub fn write_install_log(runtime_id: &str, capture: &ShellCapture) -> Result<PathBuf> {
    let root = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agent-doctor")
        .join("logs");
    std::fs::create_dir_all(&root).context("failed to create log directory")?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let path = root.join(format!("install-{runtime_id}-{timestamp}.log"));

    let mut file = std::fs::File::create(&path).context("failed to create install log")?;
    writeln!(file, "exit_code={:?}", capture.exit_code)?;
    writeln!(file, "--- stdout ---")?;
    write!(file, "{}", capture.stdout)?;
    writeln!(file, "--- stderr ---")?;
    write!(file, "{}", capture.stderr)?;

    Ok(path)
}
