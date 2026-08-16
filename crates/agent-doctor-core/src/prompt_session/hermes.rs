//! Hermes ask backend (`hermes chat -q -Q` + optional `--resume` / `--yolo`).
//!
//! Quiet mode (`-Q`) prints the final reply on stdout and `session_id:` on stderr —
//! Ask surfaces only the reply, not the interactive CLI chrome.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use super::backend::AskBackend;
use super::control::PromptSessionControl;
use super::env::{
    apply_hermes_env, apply_overlay_env, collect_overlay_env, format_command_display,
    prepare_hermes_home,
};
use super::util::{
    combine_output, force_stop_child, is_runtime_stderr_noise, join_reader, push_capped, summarize,
};
use super::{
    next_session_id, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, PromptSessionStatus, MAX_TIMEOUT_SEC, MIN_TIMEOUT_SEC,
};
use crate::session_launch::resolve_session_cwd;

pub struct HermesAskBackend;

impl AskBackend for HermesAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        _control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_hermes(options, cancel, on_event)
    }
}

fn run_hermes(
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<PromptSessionReport> {
    let session_id = next_session_id();
    let runtime = "hermes".to_string();

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
    let yolo = options.dangerously_skip_permissions || options.full_auto;

    let overlay = collect_overlay_env();
    let mut prompt_text = prompt.to_string();
    if super::mcp_ensure::wants_browser_mcp(options) {
        if let Some(note) = super::mcp_ensure::ensure_browser_mcp_for_ask("hermes", &cwd, &overlay)
        {
            on_event(PromptSessionEvent::Status {
                session_id: session_id.clone(),
                phase: "mcp".into(),
                message: note,
            });
        }
        prompt_text = format!(
            "{}\n\n{}",
            super::mcp_ensure::browser_mcp_tool_instructions(),
            prompt
        );
    }

    let mut cmd = build_hermes_command(&prompt_text, &cwd, resume_session_id, yolo)?;
    let command_display = format_command_display(&cmd);

    on_event(PromptSessionEvent::Started {
        session_id: session_id.clone(),
        runtime: runtime.clone(),
        cwd: cwd.display().to_string(),
        command: command_display,
    });

    let started = Instant::now();
    let mut child = cmd.spawn().context("failed to spawn hermes")?;

    let display_text = Arc::new(Mutex::new(String::new()));
    let display_for_cb = Arc::clone(&display_text);
    let mut emit = |event: PromptSessionEvent| {
        if let PromptSessionEvent::Delta { text, .. } = &event {
            if let Ok(mut guard) = display_for_cb.lock() {
                guard.push_str(text);
            }
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
            let reply = extract_hermes_reply(&stdout);
            let runtime_thread_id = extract_hermes_session_id(&stderr)
                .or_else(|| extract_hermes_session_id(&stdout))
                .or_else(|| extract_hermes_session_id(&combine_output(&stdout, &stderr)))
                .or_else(|| resume_session_id.map(str::to_string));

            let combined = if !reply.trim().is_empty() {
                reply.clone()
            } else if status != PromptSessionStatus::Succeeded {
                extract_hermes_error_text(&stderr).unwrap_or_else(|| {
                    let fallback = combine_output(&stdout, &stderr);
                    extract_hermes_reply(&fallback)
                })
            } else {
                String::new()
            };

            if status == PromptSessionStatus::Succeeded && !reply.trim().is_empty() {
                emit(PromptSessionEvent::Delta {
                    session_id: session_id.clone(),
                    text: reply,
                });
            } else if status != PromptSessionStatus::Succeeded && !combined.trim().is_empty() {
                // Surface a compact failure note into the transcript when no reply landed.
                let display = display_text.lock().map(|g| g.clone()).unwrap_or_default();
                if display.trim().is_empty() {
                    emit(PromptSessionEvent::Delta {
                        session_id: session_id.clone(),
                        text: combined.clone(),
                    });
                }
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

fn build_hermes_command(
    prompt: &str,
    cwd: &Path,
    resume_session_id: Option<&str>,
    yolo: bool,
) -> Result<Command> {
    let overlay = collect_overlay_env();
    prepare_hermes_home(&overlay);
    let bin = std::env::var("AGENT_DOCTOR_HERMES_BIN").unwrap_or_else(|_| "hermes".into());
    let mut cmd = Command::new(&bin);
    cmd.arg("chat")
        .arg("-q")
        .arg(prompt)
        .arg("-Q")
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    if let Some(sid) = resume_session_id {
        cmd.arg("-r").arg(sid);
    }
    if yolo {
        cmd.arg("--yolo");
    }
    apply_overlay_env(&mut cmd, &overlay);
    apply_hermes_env(&mut cmd, &overlay);
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
        for line in reader.lines().map_while(Result::ok) {
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
        for line in reader.lines().map_while(Result::ok) {
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
                // Quiet-mode stdout is the final reply; emit once at completion as Delta.
                continue;
            }
            if is_hermes_stderr_noise(&line) || is_runtime_stderr_noise(&line) {
                continue;
            }
            on_event(PromptSessionEvent::StderrLine {
                session_id: session_id.to_string(),
                line,
            });
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
            Err(error) => return Err(error).context("failed waiting for hermes ask"),
        }
    };

    join_reader(stdout_handle, Duration::from_millis(500));
    join_reader(stderr_handle, Duration::from_millis(500));
    drain(on_event);

    let stdout = stdout_acc.lock().map(|g| g.clone()).unwrap_or_default();
    let stderr = stderr_acc.lock().map(|g| g.clone()).unwrap_or_default();
    Ok((status, exit_code, stdout, stderr))
}

fn extract_hermes_session_id(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(sid) = value
                .get("session_id")
                .or_else(|| value.get("sessionId"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Some(sid.to_string());
            }
        }
        if let Some((key, rest)) = trimmed.split_once(':') {
            let key = key.trim().to_ascii_lowercase().replace(' ', "_");
            if matches!(key.as_str(), "session" | "session_id") {
                let id = rest
                    .trim()
                    .trim_matches(|c: char| c == '`' || c == '"' || c == '\'')
                    .trim();
                // Ignore "Session:        <id>" metadata rows that include trailing labels.
                let id = id.split_whitespace().next().unwrap_or(id);
                if !id.is_empty()
                    && !id.eq_ignore_ascii_case("with")
                    && id.chars().any(|c| c.is_ascii_alphanumeric())
                {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

fn extract_hermes_reply(stdout: &str) -> String {
    if let Some(boxed) = extract_hermes_box_reply(stdout) {
        return boxed;
    }
    let mut kept: Vec<&str> = Vec::new();
    for line in stdout.lines() {
        if is_hermes_cli_chrome(line) {
            continue;
        }
        kept.push(line);
    }
    // Trim leading/trailing blank lines only.
    while kept.first().is_some_and(|l| l.trim().is_empty()) {
        kept.remove(0);
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    kept.join("\n").trim().to_string()
}

fn extract_hermes_box_reply(text: &str) -> Option<String> {
    let mut inside = false;
    let mut parts: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('╭') || trimmed.starts_with("+--") {
            inside = true;
            continue;
        }
        if inside && (trimmed.starts_with('╰') || trimmed.starts_with("+--")) {
            break;
        }
        if inside {
            let content = if let Some(rest) = trimmed.strip_prefix('│') {
                rest.trim_end()
            } else {
                line.trim_end()
            };
            parts.push(content.trim_start().to_string());
        }
    }
    let joined = parts.join("\n").trim().to_string();
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn extract_hermes_error_text(stderr: &str) -> Option<String> {
    let mut lines: Vec<&str> = Vec::new();
    for line in stderr.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || is_hermes_stderr_noise(line) {
            continue;
        }
        if trimmed.starts_with("Error:")
            || trimmed.starts_with("❌")
            || trimmed.contains("HTTP 401")
            || trimmed.contains("AuthenticationError")
            || trimmed.contains("API call failed")
        {
            lines.push(trimmed);
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn is_hermes_cli_chrome(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.starts_with("Query:")
        || trimmed.starts_with("Initializing agent")
        || trimmed.starts_with("Resume this session")
        || trimmed.starts_with("hermes --resume")
        || trimmed.starts_with("hermes chat")
        || trimmed.starts_with("Session:")
        || trimmed.starts_with("Duration:")
        || trimmed.starts_with("Messages:")
        || trimmed.starts_with("↻")
        || trimmed.starts_with("⚠")
        || trimmed.starts_with("⚠️")
        || trimmed.starts_with("🔌")
        || trimmed.starts_with("🌐")
        || trimmed.starts_with("📝")
        || trimmed.starts_with("📋")
        || trimmed.starts_with("💡")
        || trimmed.starts_with("─ ⚕ Hermes")
        || (trimmed.contains('─') && trimmed.chars().filter(|c| *c == '─').count() >= 8)
        || trimmed.starts_with('╭')
        || trimmed.starts_with('╰')
        || trimmed.starts_with('│')
}

fn is_hermes_stderr_noise(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.to_ascii_lowercase().starts_with("session_id:") {
        return true;
    }
    if trimmed.starts_with("⚠") || trimmed.starts_with("⚠️") {
        // Auxiliary / compression notices are not actionable Ask errors.
        if trimmed.contains("auxiliary") || trimmed.contains("OPENROUTER") {
            return true;
        }
    }
    false
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
    fn extracts_session_id_from_banner() {
        assert_eq!(
            extract_hermes_session_id("Session: abc-123\nok").as_deref(),
            Some("abc-123")
        );
        assert_eq!(
            extract_hermes_session_id("session_id: 20260814_225743_6916aa").as_deref(),
            Some("20260814_225743_6916aa")
        );
    }

    #[test]
    fn extracts_reply_from_quiet_and_boxed_output() {
        assert_eq!(extract_hermes_reply("OK\n"), "OK");
        let boxed = r#"Query: 你好
Initializing agent...
────────────────────────────────────────
╭─ ⚕ Hermes ───────────────────────────────────────────────────────────────────╮
你好！我是 Hermes Agent。
╰──────────────────────────────────────────────────────────────────────────────╯
Resume this session with:
hermes --resume 20260814223955a98b96
Session: 20260814223955a98b96
"#;
        assert_eq!(extract_hermes_reply(boxed), "你好！我是 Hermes Agent。");
    }

    #[test]
    fn prepare_hermes_home_writes_model_pointer_and_env() {
        use super::super::env::prepare_hermes_home;
        use std::collections::HashMap;
        let dir = tempdir().unwrap();
        let mut overlay = HashMap::new();
        overlay.insert("HERMES_HOME".into(), dir.path().display().to_string());
        overlay.insert(
            "AGENT_DOCTOR_GATEWAY_URL".into(),
            "https://example.test/v1".into(),
        );
        overlay.insert("OPENAI_API_KEY".into(), "sk-test-key".into());
        overlay.insert("AGENT_DOCTOR_MODEL".into(), "demo-model".into());
        prepare_hermes_home(&overlay);
        let cfg = fs::read_to_string(dir.path().join("config.yaml")).unwrap();
        assert!(cfg.contains("provider: custom"));
        assert!(cfg.contains("https://example.test/v1"));
        assert!(cfg.contains("demo-model"));
        let env = fs::read_to_string(dir.path().join(".env")).unwrap();
        assert!(env.contains("OPENAI_API_KEY=sk-test-key"));
    }

    #[cfg(unix)]
    #[test]
    fn streams_stdout_and_succeeds() {
        let _guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        let bin = write_fake_bin(
            dir.path(),
            "fake-hermes",
            "#!/bin/bash\necho hermes-ok\necho 'session_id: hermes-sid-1' >&2\nexit 0\n",
        );
        std::env::set_var("AGENT_DOCTOR_HERMES_BIN", &bin);
        let events = StdMutex::new(Vec::new());
        let report = HermesAskBackend
            .run(
                &PromptSessionOptions {
                    runtime: "hermes".into(),
                    prompt: "hello".into(),
                    cwd: Some(dir.path().to_path_buf()),
                    timeout_sec: 30,
                    dangerously_skip_permissions: true,
                    full_auto: false,
                    resume_thread_id: None,
                    selected_mcps: Vec::new(),
                },
                PromptSessionCancel::new(),
                None,
                &mut |ev| events.lock().unwrap().push(ev),
            )
            .expect("session");
        std::env::remove_var("AGENT_DOCTOR_HERMES_BIN");
        assert_eq!(report.status, PromptSessionStatus::Succeeded);
        assert_eq!(report.runtime_thread_id.as_deref(), Some("hermes-sid-1"));
        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(
            e,
            PromptSessionEvent::Delta { text, .. } if text.contains("hermes-ok")
        )));
        assert!(!evs
            .iter()
            .any(|e| matches!(e, PromptSessionEvent::StdoutLine { .. })));
    }
}
