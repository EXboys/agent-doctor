use std::process::Child;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::{PromptSessionStatus, MAX_CAPTURE_CHARS, SUMMARY_CHARS};

pub(crate) fn push_capped(acc: &Arc<Mutex<String>>, line: &str) {
    if let Ok(mut guard) = acc.lock() {
        if guard.len() >= MAX_CAPTURE_CHARS {
            return;
        }
        let remain = MAX_CAPTURE_CHARS.saturating_sub(guard.len());
        if line.len() <= remain {
            if !guard.is_empty() {
                guard.push('\n');
            }
            guard.push_str(line);
        } else {
            guard.push_str(&line[..remain]);
        }
    }
}

pub(crate) fn combine_output(stdout: &str, stderr: &str) -> String {
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout.to_string(),
        (true, false) => stderr.to_string(),
        (false, false) => format!("{stdout}\n{stderr}"),
    }
}

pub(crate) fn summarize(combined: &str, status: &PromptSessionStatus, runtime: &str) -> String {
    let trimmed = combined.trim();
    if trimmed.is_empty() {
        return format!("{runtime} ask ended ({status:?}) with no captured output");
    }
    if trimmed.chars().count() <= SUMMARY_CHARS {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(SUMMARY_CHARS).collect();
    format!("{truncated}…")
}

pub(crate) fn force_stop_child(child: &mut Child, pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("pkill")
            .args(["-P", &pid.to_string()])
            .status();
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) fn join_reader(handle: thread::JoinHandle<()>, budget: Duration) {
    let start = Instant::now();
    while start.elapsed() < budget {
        if handle.is_finished() {
            let _ = handle.join();
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

pub(crate) fn is_runtime_stderr_noise(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    [
        "openai codex",
        "reading additional input from stdin",
        "workdir:",
        "approval:",
        "model:",
        "provider:",
        "session id:",
        "-------",
        "user",
        "assistant",
    ]
    .iter()
    .any(|p| lower.starts_with(p) || lower == *p)
}

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
