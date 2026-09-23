use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use super::download_route::apply_china_download_env;

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
    let mut command = Command::new("bash");
    #[cfg(unix)]
    command.arg("-c").arg(command_line);

    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let mut command = Command::new("cmd");
        command
            .args(["/C", command_line])
            .creation_flags(CREATE_NO_WINDOW);
        command
    };

    apply_china_download_env(&mut command);
    prepare_npm_install_env(command_line, &mut command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start install shell")?;

    let stdout = child.stdout.take().context("missing stdout pipe")?;
    let stderr = child.stderr.take().context("missing stderr pipe")?;

    let queue: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let stdout_acc: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let stderr_acc: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    let queue_out = Arc::clone(&queue);
    let acc_out = Arc::clone(&stdout_acc);
    let stdout_handle = thread::spawn(move || {
        push_progress_lines(stdout, &queue_out, &acc_out);
    });

    let queue_err = Arc::clone(&queue);
    let acc_err = Arc::clone(&stderr_acc);
    let stderr_handle = thread::spawn(move || {
        push_progress_lines(stderr, &queue_err, &acc_err);
    });

    let mut on_line = on_line;
    let started = std::time::Instant::now();
    let mut last_output_at = started;
    let mut last_heartbeat_at = started;
    let status = loop {
        let drained = {
            let mut guard = queue.lock().unwrap_or_else(|error| error.into_inner());
            guard.drain(..).collect::<Vec<_>>()
        };
        if !drained.is_empty() {
            last_output_at = std::time::Instant::now();
            for line in drained {
                on_line(&line);
            }
        } else if looks_like_npm_install(command_line)
            && last_output_at.elapsed() >= Duration::from_secs(8)
            && last_heartbeat_at.elapsed() >= Duration::from_secs(8)
        {
            let waited = started.elapsed().as_secs();
            on_line(&format!(
                "仍在下载安装，请稍候…（依赖比较多，已等待 {waited} 秒）"
            ));
            last_heartbeat_at = std::time::Instant::now();
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

fn looks_like_npm_install(command_line: &str) -> bool {
    let lower = command_line.to_ascii_lowercase();
    lower.contains("npm install") || lower.contains("npm uninstall")
}

/// Make long npm installs visible and ignore host-injected npm knobs (e.g. Cursor `devdir`).
fn prepare_npm_install_env(command_line: &str, command: &mut Command) {
    if !looks_like_npm_install(command_line) {
        return;
    }
    command.env_remove("npm_config_devdir");
    command.env_remove("NPM_CONFIG_DEVDIR");
    // Default npm loglevel hides hundreds of DeepSeek dependency fetches.
    command.env("npm_config_loglevel", "info");
    command.env("npm_config_progress", "true");
    command.env("npm_config_fetch_retries", "2");
    command.env("npm_config_fetch_timeout", "120000");
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

fn push_progress_lines<R: Read>(mut reader: R, queue: &Mutex<Vec<String>>, acc: &Mutex<String>) {
    let mut pending = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        pending.extend_from_slice(&buf[..n]);
        drain_progress_chunks(&mut pending, queue, acc);
    }
    if !pending.is_empty() {
        emit_progress_chunk(&pending, queue, acc);
    }
}

/// Git and uv print progress with carriage returns. Split those into updates
/// so the install bar does not sit still until the next newline.
fn drain_progress_chunks(pending: &mut Vec<u8>, queue: &Mutex<Vec<String>>, acc: &Mutex<String>) {
    while let Some(pos) = pending
        .iter()
        .position(|byte| *byte == b'\n' || *byte == b'\r')
    {
        let chunk: Vec<u8> = pending.drain(..=pos).collect();
        let body = chunk
            .strip_suffix(b"\n")
            .or_else(|| chunk.strip_suffix(b"\r"))
            .unwrap_or(&chunk);
        if !body.is_empty() {
            emit_progress_chunk(body, queue, acc);
        }
    }
}

fn emit_progress_chunk(body: &[u8], queue: &Mutex<Vec<String>>, acc: &Mutex<String>) {
    let line = String::from_utf8_lossy(body).trim().to_string();
    if line.is_empty() {
        return;
    }
    if let Ok(mut acc) = acc.lock() {
        acc.push_str(&line);
        acc.push('\n');
    }
    if let Ok(mut queue) = queue.lock() {
        queue.push(line);
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
