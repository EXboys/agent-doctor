//! OpenClaw ask backend (`openclaw agent exec --message` + optional `--json` / `--session-id`).

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::Value;

use super::backend::AskBackend;
use super::control::PromptSessionControl;
use super::env::{apply_overlay_env, collect_overlay_env, format_command_display};
use super::util::{
    combine_output, force_stop_child, is_runtime_stderr_noise, join_reader, push_capped, summarize,
};
use super::{
    next_session_id, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, PromptSessionStatus, MAX_TIMEOUT_SEC, MIN_TIMEOUT_SEC,
};
use crate::session_launch::resolve_session_cwd;

pub struct OpenClawAskBackend;

impl AskBackend for OpenClawAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        _control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_openclaw(options, cancel, on_event)
    }
}

fn run_openclaw(
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<PromptSessionReport> {
    let session_id = next_session_id();
    let runtime = "openclaw".to_string();

    let prompt = options.prompt.trim();
    if prompt.is_empty() {
        bail!("prompt must not be empty");
    }

    let cwd = resolve_session_cwd(options.cwd.as_deref());
    if !cwd.exists() {
        bail!("session cwd does not exist: {}", cwd.display());
    }

    let timeout_sec = options.timeout_sec.clamp(MIN_TIMEOUT_SEC, MAX_TIMEOUT_SEC);
    let resume_session_id = options
        .resume_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let mut cmd = build_openclaw_command(prompt, &cwd, resume_session_id, timeout_sec)?;
    let command_display = format_command_display(&cmd);

    on_event(PromptSessionEvent::Started {
        session_id: session_id.clone(),
        runtime: runtime.clone(),
        cwd: cwd.display().to_string(),
        command: command_display,
    });

    let started = Instant::now();
    let mut child = cmd.spawn().context("failed to spawn openclaw")?;

    let display_text = Arc::new(Mutex::new(String::new()));
    let display_for_cb = Arc::clone(&display_text);
    let mut emit = |event: PromptSessionEvent| {
        match &event {
            PromptSessionEvent::Delta { text, .. } => {
                if let Ok(mut guard) = display_for_cb.lock() {
                    guard.push_str(text);
                }
            }
            PromptSessionEvent::StdoutLine { line, .. } => {
                if let Ok(mut guard) = display_for_cb.lock() {
                    if !guard.is_empty() {
                        guard.push('\n');
                    }
                    guard.push_str(line);
                }
            }
            _ => {}
        }
        on_event(event);
    };

    let result = pump_lines(
        &session_id,
        &mut child,
        timeout_sec,
        cancel.handle(),
        &mut emit,
    );

    let duration_ms = started.elapsed().as_millis() as u64;
    let report = match result {
        Ok((status, exit_code, stdout, stderr)) => {
            let parsed = parse_openclaw_json_output(&stdout);
            let reply = parsed
                .as_ref()
                .and_then(|v| extract_openclaw_reply(v))
                .unwrap_or_default();
            let runtime_thread_id = parsed
                .as_ref()
                .and_then(|v| extract_openclaw_session_id(v))
                .or_else(|| resume_session_id.map(str::to_string));

            let display = display_text.lock().map(|g| g.clone()).unwrap_or_default();
            let combined = if !reply.trim().is_empty() {
                reply.clone()
            } else if !display.trim().is_empty() {
                display
            } else {
                combine_output(&stdout, &stderr)
            };

            if status == PromptSessionStatus::Succeeded && !reply.trim().is_empty() {
                emit(PromptSessionEvent::Delta {
                    session_id: session_id.clone(),
                    text: reply,
                });
            }

            let summary = summarize(&combined, &status, &runtime);
            emit(PromptSessionEvent::Completed {
                session_id: session_id.clone(),
                status: status.clone(),
                exit_code,
                summary: summary.clone(),
            });
            PromptSessionReport {
                session_id,
                runtime,
                cwd: cwd.display().to_string(),
                status,
                exit_code,
                summary: summary.clone(),
                log_excerpt: combined,
                duration_ms,
                runtime_thread_id,
            }
        }
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            let summary = format!("{err:#}");
            emit(PromptSessionEvent::Completed {
                session_id: session_id.clone(),
                status: PromptSessionStatus::Failed,
                exit_code: None,
                summary: summary.clone(),
            });
            PromptSessionReport {
                session_id,
                runtime,
                cwd: cwd.display().to_string(),
                status: PromptSessionStatus::Failed,
                exit_code: None,
                summary: summary.clone(),
                log_excerpt: summary,
                duration_ms,
                runtime_thread_id: resume_session_id.map(str::to_string),
            }
        }
    };
    Ok(report)
}

fn build_openclaw_command(
    prompt: &str,
    cwd: &Path,
    resume_session_id: Option<&str>,
    timeout_sec: u64,
) -> Result<Command> {
    let overlay = collect_overlay_env();
    let bin = std::env::var("AGENT_DOCTOR_OPENCLAW_BIN").unwrap_or_else(|_| "openclaw".into());
    let mut cmd = Command::new(&bin);
    cmd.arg("agent")
        .arg("exec")
        .arg("--message")
        .arg(prompt)
        .arg("--cwd")
        .arg(cwd.display().to_string())
        .arg("--json")
        .arg("--timeout")
        .arg(timeout_sec.to_string())
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    if let Some(sid) = resume_session_id {
        cmd.arg("--session-id").arg(sid);
    }
    apply_overlay_env(&mut cmd, &overlay);
    Ok(cmd)
}

fn pump_lines<F>(
    session_id: &str,
    child: &mut Child,
    timeout_sec: u64,
    cancel: Arc<AtomicBool>,
    on_event: &mut F,
) -> Result<(PromptSessionStatus, Option<i32>, String, String)>
where
    F: FnMut(PromptSessionEvent),
{
    let pid = child.id();
    let queue = Arc::new(Mutex::new(Vec::<(bool, String)>::new()));
    let stdout_acc = Arc::new(Mutex::new(String::new()));
    let stderr_acc = Arc::new(Mutex::new(String::new()));

    let stdout = child.stdout.take().context("missing stdout pipe")?;
    let stderr = child.stderr.take().context("missing stderr pipe")?;

    let q_out = Arc::clone(&queue);
    let acc_out = Arc::clone(&stdout_acc);
    let stdout_handle = thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines().flatten() {
            push_capped(&acc_out, &line);
            if let Ok(mut guard) = q_out.lock() {
                guard.push((true, line));
            }
        }
    });

    let q_err = Arc::clone(&queue);
    let acc_err = Arc::clone(&stderr_acc);
    let stderr_handle = thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stderr);
        for line in reader.lines().flatten() {
            push_capped(&acc_err, &line);
            if let Ok(mut guard) = q_err.lock() {
                guard.push((false, line));
            }
        }
    });

    let drain = |on_event: &mut F| {
        let drained = {
            let mut guard = queue.lock().unwrap_or_else(|e| e.into_inner());
            guard.drain(..).collect::<Vec<_>>()
        };
        for (is_stdout, line) in drained {
            if is_stdout {
                // JSON payloads are parsed at the end; skip noisy intermediate lines in UI.
                if line.trim_start().starts_with('{') {
                    continue;
                }
                on_event(PromptSessionEvent::StdoutLine {
                    session_id: session_id.to_string(),
                    line,
                });
            } else if !is_runtime_stderr_noise(&line) {
                on_event(PromptSessionEvent::StderrLine {
                    session_id: session_id.to_string(),
                    line,
                });
            }
        }
    };

    let deadline = Instant::now() + Duration::from_secs(timeout_sec);
    let (status, exit_code) = loop {
        drain(on_event);
        if cancel.load(Ordering::SeqCst) {
            force_stop_child(child, pid);
            break (PromptSessionStatus::Cancelled, None);
        }
        if Instant::now() >= deadline {
            force_stop_child(child, pid);
            break (PromptSessionStatus::TimedOut, None);
        }
        match child.try_wait() {
            Ok(Some(wait_status)) => {
                let code = wait_status.code();
                let status = if wait_status.success() {
                    PromptSessionStatus::Succeeded
                } else {
                    PromptSessionStatus::Failed
                };
                break (status, code);
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(error) => return Err(error).context("failed waiting for openclaw ask"),
        }
    };

    join_reader(stdout_handle, Duration::from_millis(500));
    join_reader(stderr_handle, Duration::from_millis(500));
    drain(on_event);

    let stdout = stdout_acc.lock().map(|g| g.clone()).unwrap_or_default();
    let stderr = stderr_acc.lock().map(|g| g.clone()).unwrap_or_default();
    Ok((status, exit_code, stdout, stderr))
}

fn parse_openclaw_json_output(stdout: &str) -> Option<Value> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    // Prefer the last JSON object in mixed output.
    for line in trimmed.lines().rev() {
        let line = line.trim();
        if line.starts_with('{') {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                return Some(value);
            }
        }
    }
    None
}

fn extract_openclaw_reply(value: &Value) -> Option<String> {
    const KEYS: &[&str] = &[
        "reply",
        "message",
        "text",
        "result",
        "output",
        "content",
        "response",
    ];
    for key in KEYS {
        if let Some(text) = value.get(*key).and_then(value_to_text) {
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
    }
    if let Some(obj) = value.get("data") {
        return extract_openclaw_reply(obj);
    }
    if let Some(obj) = value.get("result") {
        if obj.is_object() {
            return extract_openclaw_reply(obj);
        }
    }
    None
}

fn extract_openclaw_session_id(value: &Value) -> Option<String> {
    for key in ["sessionId", "session_id", "session", "id"] {
        if let Some(sid) = value
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(sid.to_string());
        }
    }
    value
        .get("data")
        .and_then(extract_openclaw_session_id)
        .or_else(|| value.get("result").and_then(extract_openclaw_session_id))
}

fn value_to_text(value: &Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = value.as_array() {
        let parts: Vec<String> = arr
            .iter()
            .filter_map(|item| {
                if let Some(s) = item.as_str() {
                    Some(s.to_string())
                } else if let Some(s) = item.get("text").and_then(|v| v.as_str()) {
                    Some(s.to_string())
                } else {
                    None
                }
            })
            .collect();
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_session::util::TEST_ENV_LOCK;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;
    use tempfile::tempdir;

    #[cfg(unix)]
    fn write_fake_bin(dir: &Path, name: &str, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, script).expect("write fake bin");
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[test]
    fn parses_json_reply_and_session() {
        let raw = r#"{"reply":"openclaw-ok","sessionId":"oc-1"}"#;
        let value = parse_openclaw_json_output(raw).unwrap();
        assert_eq!(extract_openclaw_reply(&value).as_deref(), Some("openclaw-ok"));
        assert_eq!(
            extract_openclaw_session_id(&value).as_deref(),
            Some("oc-1")
        );
    }

    #[cfg(unix)]
    #[test]
    fn streams_json_reply_and_succeeds() {
        let _guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        let bin = write_fake_bin(
            dir.path(),
            "fake-openclaw",
            "#!/bin/bash\necho '{\"reply\":\"openclaw-ok\",\"sessionId\":\"oc-sid\"}'\nexit 0\n",
        );
        std::env::set_var("AGENT_DOCTOR_OPENCLAW_BIN", &bin);
        let events = StdMutex::new(Vec::new());
        let report = OpenClawAskBackend
            .run(
                &PromptSessionOptions {
                    runtime: "openclaw".into(),
                    prompt: "hello".into(),
                    cwd: Some(dir.path().to_path_buf()),
                    timeout_sec: 30,
                    dangerously_skip_permissions: false,
                    full_auto: true,
                    resume_thread_id: None,
                },
                PromptSessionCancel::new(),
                None,
                &mut |ev| events.lock().unwrap().push(ev),
            )
            .expect("session");
        std::env::remove_var("AGENT_DOCTOR_OPENCLAW_BIN");
        assert_eq!(report.status, PromptSessionStatus::Succeeded);
        assert_eq!(report.runtime_thread_id.as_deref(), Some("oc-sid"));
        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(
            e,
            PromptSessionEvent::Delta { text, .. } if text.contains("openclaw-ok")
        )));
    }
}
