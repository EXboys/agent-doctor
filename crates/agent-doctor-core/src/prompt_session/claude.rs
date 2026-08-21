//! Claude Code ask backend (`claude -p` + optional control_request permissions).

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
    apply_claude_env, apply_overlay_env, collect_overlay_env, format_command_display,
};
use super::mcp_ensure::{ensure_browser_mcp_for_ask, wants_browser_mcp};
use super::util::{
    combine_output, command_from_cli, force_stop_child, is_runtime_stderr_noise, join_reader,
    push_capped, summarize,
};
use super::{
    next_session_id, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, PromptSessionStatus, MAX_TIMEOUT_SEC, MIN_TIMEOUT_SEC,
};
use crate::session_launch::resolve_session_cwd;

pub struct ClaudeAskBackend;

impl AskBackend for ClaudeAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_claude(options, cancel, control, on_event)
    }
}

fn run_claude(
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    control: Option<PromptSessionControl>,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<PromptSessionReport> {
    let session_id = next_session_id();
    let runtime = "claude-code".to_string();

    let prompt = options.prompt.trim();
    if prompt.is_empty() {
        bail!("prompt must not be empty");
    }

    let cwd = resolve_session_cwd(options.cwd.as_deref());
    if !cwd.exists() {
        bail!("session cwd does not exist: {}", cwd.display());
    }

    let timeout_sec = options.timeout_sec.clamp(MIN_TIMEOUT_SEC, MAX_TIMEOUT_SEC);
    let interactive = !options.dangerously_skip_permissions && control.is_some();
    let resume_session_id = options
        .resume_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let overlay = collect_overlay_env();
    let browser_mcp = wants_browser_mcp(options);
    if browser_mcp {
        if let Some(note) = ensure_browser_mcp_for_ask("claude-code", &cwd, &overlay) {
            on_event(PromptSessionEvent::Status {
                session_id: session_id.clone(),
                phase: "mcp".into(),
                message: note,
            });
        }
    }

    let effective_prompt = if browser_mcp {
        format!(
            "{}\n\n{}",
            super::mcp_ensure::browser_mcp_tool_instructions(),
            prompt
        )
    } else {
        prompt.to_string()
    };

    let mut cmd = build_claude_command(
        &effective_prompt,
        &cwd,
        options.dangerously_skip_permissions,
        interactive,
        resume_session_id,
        &overlay,
    )?;
    let command_display = format_command_display(&cmd);

    on_event(PromptSessionEvent::Started {
        session_id: session_id.clone(),
        runtime: runtime.clone(),
        cwd: cwd.display().to_string(),
        command: command_display,
    });

    let started = Instant::now();
    let mut child = cmd.spawn().context("failed to spawn claude-code")?;

    if interactive {
        if let Some(stdin) = child.stdin.take() {
            if let Some(control) = control.as_ref() {
                control.attach_stdin(stdin);
                let user_msg = serde_json::json!({
                    "type": "user",
                    "message": {
                        "role": "user",
                        "content": [{ "type": "text", "text": effective_prompt }]
                    }
                });
                if let Err(err) = control.write_line(&user_msg.to_string()) {
                    control.close();
                    return Err(err).context("failed to send Claude ask prompt on stdin");
                }
            }
        }
    }

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

    let result = pump_claude(
        &session_id,
        &mut child,
        timeout_sec,
        cancel.handle(),
        control.as_ref(),
        &mut emit,
    );

    if let Some(control) = control.as_ref() {
        control.close();
    }

    let duration_ms = started.elapsed().as_millis() as u64;
    let report = match result {
        Ok((status, exit_code, stdout, stderr)) => {
            let recovered = if display_text
                .lock()
                .map(|g| g.trim().is_empty())
                .unwrap_or(true)
            {
                extract_claude_result_text(&stdout)
            } else {
                None
            };
            if let Some(text) = recovered.as_ref() {
                if let Ok(mut guard) = display_text.lock() {
                    *guard = text.clone();
                }
                emit(PromptSessionEvent::Delta {
                    session_id: session_id.clone(),
                    text: text.clone(),
                });
            }
            let display = display_text.lock().map(|g| g.clone()).unwrap_or_default();
            let combined = if display.trim().is_empty() {
                combine_output(&stdout, &stderr)
            } else {
                display
            };
            let summary = summarize(&combined, &status, &runtime);
            let runtime_thread_id = extract_claude_session_id(&stdout)
                .or_else(|| resume_session_id.map(str::to_string));
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

fn build_claude_command(
    prompt: &str,
    cwd: &Path,
    skip_permissions: bool,
    interactive_permissions: bool,
    resume_session_id: Option<&str>,
    overlay: &std::collections::HashMap<String, String>,
) -> Result<Command> {
    let bin = std::env::var("AGENT_DOCTOR_CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
    let mut cmd = command_from_cli(&bin);
    cmd.arg("-p")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--include-partial-messages")
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mcp_config = cwd.join(".mcp.json");
    if mcp_config.is_file() {
        cmd.arg("--mcp-config").arg(&mcp_config);
    }
    if let Some(sid) = resume_session_id {
        cmd.arg("--resume").arg(sid);
    }
    if skip_permissions {
        cmd.arg(prompt)
            .arg("--dangerously-skip-permissions")
            .stdin(Stdio::null());
    } else if interactive_permissions {
        let ask_settings = serde_json::json!({
            "permissions": {
                "ask": [
                    "Bash",
                    "Edit",
                    "Write",
                    "MultiEdit",
                    "NotebookEdit",
                    "WebFetch",
                    "WebSearch",
                    "mcp__browser__*"
                ]
            }
        });
        cmd.arg("--input-format")
            .arg("stream-json")
            .arg("--permission-prompt-tool")
            .arg("stdio")
            .arg("--settings")
            .arg(ask_settings.to_string())
            .stdin(Stdio::piped());
    } else {
        cmd.arg(prompt).stdin(Stdio::null());
    }
    apply_overlay_env(&mut cmd, overlay);
    apply_claude_env(&mut cmd, overlay);
    Ok(cmd)
}

fn pump_claude<F>(
    session_id: &str,
    child: &mut Child,
    timeout_sec: u64,
    cancel: Arc<AtomicBool>,
    control: Option<&PromptSessionControl>,
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
                for event in parse_claude_stream_line(session_id, &line, control) {
                    on_event(event);
                }
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
            Err(error) => return Err(error).context("failed waiting for prompt session"),
        }
    };

    join_reader(stdout_handle, Duration::from_millis(500));
    join_reader(stderr_handle, Duration::from_millis(500));
    drain(on_event);

    let stdout = stdout_acc.lock().map(|g| g.clone()).unwrap_or_default();
    let stderr = stderr_acc.lock().map(|g| g.clone()).unwrap_or_default();
    Ok((status, exit_code, stdout, stderr))
}

fn permission_detail(
    tool_name: &str,
    request: &serde_json::Value,
    input: &serde_json::Value,
) -> String {
    let description = request
        .get("description")
        .or_else(|| request.get("tool_use_description"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(desc) = description {
        return desc.to_string();
    }

    let pick_str = |keys: &[&str]| -> Option<String> {
        for key in keys {
            if let Some(s) = input.get(*key).and_then(|v| v.as_str()).map(str::trim) {
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
        None
    };

    let lower = tool_name.to_ascii_lowercase();
    if lower.contains("bash") || lower == "shell" {
        if let Some(cmd) = pick_str(&["command", "cmd"]) {
            return cmd;
        }
    }
    if lower.contains("write") || lower.contains("edit") {
        if let Some(path) = pick_str(&["file_path", "path", "filePath"]) {
            return path;
        }
    }
    if let Some(path) = pick_str(&["file_path", "path", "filePath", "url", "query"]) {
        return path;
    }
    if let Some(cmd) = pick_str(&["command", "cmd"]) {
        return cmd;
    }

    let compact = input.to_string();
    if compact.len() <= 280 {
        compact
    } else {
        format!("{}…", &compact[..277])
    }
}

fn parse_claude_stream_line(
    session_id: &str,
    line: &str,
    control: Option<&PromptSessionControl>,
) -> Vec<PromptSessionEvent> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        if line.trim().is_empty() {
            return Vec::new();
        }
        return vec![PromptSessionEvent::StdoutLine {
            session_id: session_id.to_string(),
            line: line.to_string(),
        }];
    };

    let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match event_type {
        "system" => {
            let subtype = value.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
            if subtype == "status" {
                let status = value
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("working");
                let message = match status {
                    "requesting" => "正在请求模型…",
                    other => other,
                };
                vec![PromptSessionEvent::Status {
                    session_id: session_id.to_string(),
                    phase: status.to_string(),
                    message: message.to_string(),
                }]
            } else {
                Vec::new()
            }
        }
        "control_request" => {
            let request_id = value
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let request = value
                .get("request")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let subtype = request
                .get("subtype")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if subtype == "can_use_tool" {
                let tool_name = request
                    .get("tool_name")
                    .or_else(|| request.get("display_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool")
                    .to_string();
                let input = request
                    .get("input")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                let detail = permission_detail(&tool_name, &request, &input);
                if let Some(control) = control {
                    control.remember_claude_tool_input(&request_id, input.clone());
                }
                vec![
                    PromptSessionEvent::Status {
                        session_id: session_id.to_string(),
                        phase: "permission".into(),
                        message: format!("等待确认：{tool_name}"),
                    },
                    PromptSessionEvent::PermissionRequest {
                        session_id: session_id.to_string(),
                        request_id,
                        tool_name,
                        detail,
                        input_json: input.to_string(),
                    },
                ]
            } else if !request_id.is_empty() {
                // Initialize / mode-switch style control requests — auto-ack.
                if let Some(control) = control {
                    let _ = control.auto_ack_claude_control(&request_id);
                }
                Vec::new()
            } else {
                Vec::new()
            }
        }
        "assistant" => {
            let mut out = Vec::new();
            let content = value
                .pointer("/message/content")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for block in content {
                let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match block_type {
                    "thinking" => {
                        out.push(PromptSessionEvent::Status {
                            session_id: session_id.to_string(),
                            phase: "thinking".into(),
                            message: "正在思考…".into(),
                        });
                    }
                    "text" => {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                out.push(PromptSessionEvent::Status {
                                    session_id: session_id.to_string(),
                                    phase: "writing".into(),
                                    message: "正在生成回复…".into(),
                                });
                                out.push(PromptSessionEvent::Delta {
                                    session_id: session_id.to_string(),
                                    text: text.to_string(),
                                });
                            }
                        }
                    }
                    "tool_use" => {
                        let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                        out.push(PromptSessionEvent::Status {
                            session_id: session_id.to_string(),
                            phase: "tool".into(),
                            message: format!("调用工具 {name}…"),
                        });
                    }
                    _ => {}
                }
            }
            out
        }
        "stream_event" => {
            let mut out = Vec::new();
            // content_block_delta.text / delta.text / event.delta.text
            let delta = value
                .pointer("/event/delta/text")
                .or_else(|| value.pointer("/event/delta/partial_json"))
                .or_else(|| value.pointer("/delta/text"))
                .and_then(|v| v.as_str());
            if let Some(delta) = delta.filter(|t| !t.is_empty()) {
                out.push(PromptSessionEvent::Delta {
                    session_id: session_id.to_string(),
                    text: delta.to_string(),
                });
            }
            let event_type = value
                .pointer("/event/type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if event_type == "content_block_start" {
                let block_type = value
                    .pointer("/event/content_block/type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if block_type == "tool_use" {
                    let name = value
                        .pointer("/event/content_block/name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("tool");
                    out.push(PromptSessionEvent::Status {
                        session_id: session_id.to_string(),
                        phase: "tool".into(),
                        message: format!("调用工具 {name}…"),
                    });
                }
            }
            out
        }
        // Final envelope — surface errors immediately; successful `result` text is
        // applied after pump when live deltas were missing (avoids duplicating
        // assistant text that already streamed).
        "result" => {
            let is_error = value
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if is_error {
                let msg = value
                    .get("result")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Claude reported an error");
                vec![
                    PromptSessionEvent::Status {
                        session_id: session_id.to_string(),
                        phase: "error".into(),
                        message: msg.to_string(),
                    },
                    PromptSessionEvent::StderrLine {
                        session_id: session_id.to_string(),
                        line: msg.to_string(),
                    },
                ]
            } else {
                vec![PromptSessionEvent::Status {
                    session_id: session_id.to_string(),
                    phase: "writing".into(),
                    message: "正在整理回复…".into(),
                }]
            }
        }
        _ => Vec::new(),
    }
}

/// Pull the final answer out of Claude Code stream-json when live deltas were empty.
fn extract_claude_session_id(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(sid) = value
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(sid.to_string());
        }
    }
    None
}

fn extract_claude_result_text(stdout: &str) -> Option<String> {
    for line in stdout.lines().rev() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("result") {
            continue;
        }
        if value
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            continue;
        }
        if let Some(text) = value
            .get("result")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(text.to_string());
        }
    }

    // Fallback: concatenate assistant text blocks from the stream.
    let mut parts = Vec::new();
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }
        let Some(content) = value.pointer("/message/content").and_then(|v| v.as_array()) else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(|v| v.as_str()) != Some("text") {
                continue;
            }
            if let Some(text) = block
                .get("text")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                parts.push(text.to_string());
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
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
    fn extracts_result_text_from_claude_stream() {
        let stdout = r#"
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"partial"}]}}
{"type":"result","is_error":false,"result":"最终合同摘要"}
"#;
        assert_eq!(
            extract_claude_result_text(stdout).as_deref(),
            Some("最终合同摘要")
        );
    }

    #[test]
    fn parse_result_error_emits_stderr() {
        let events = parse_claude_stream_line(
            "s1",
            r#"{"type":"result","is_error":true,"result":"boom"}"#,
            None,
        );
        assert!(events.iter().any(|e| matches!(
            e,
            PromptSessionEvent::StderrLine { line, .. } if line == "boom"
        )));
    }

    #[test]
    fn parse_control_request_emits_permission() {
        let control = PromptSessionControl::new();
        let line = r#"{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls -la"}}}"#;
        let events = parse_claude_stream_line("s1", line, Some(&control));
        assert!(events.iter().any(|e| matches!(
            e,
            PromptSessionEvent::PermissionRequest {
                request_id,
                tool_name,
                detail,
                ..
            } if request_id == "req-1" && tool_name == "Bash" && detail == "ls -la"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn streams_stdout_and_succeeds() {
        let _guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        let bin = write_fake_bin(
            dir.path(),
            "fake-claude",
            "#!/bin/bash\necho line-one\necho line-two\nexit 0\n",
        );
        std::env::set_var("AGENT_DOCTOR_CLAUDE_BIN", &bin);
        let events = StdMutex::new(Vec::new());
        let report = ClaudeAskBackend
            .run(
                &PromptSessionOptions {
                    runtime: "claude-code".into(),
                    prompt: "hello".into(),
                    cwd: Some(dir.path().to_path_buf()),
                    timeout_sec: 30,
                    dangerously_skip_permissions: false,
                    full_auto: false,
                    resume_thread_id: None,
                    selected_mcps: Vec::new(),
                },
                PromptSessionCancel::new(),
                None,
                &mut |ev| events.lock().unwrap().push(ev),
            )
            .expect("session");
        std::env::remove_var("AGENT_DOCTOR_CLAUDE_BIN");
        assert_eq!(report.status, PromptSessionStatus::Succeeded);
        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(
            e,
            PromptSessionEvent::StdoutLine { line, .. } if line == "line-one"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn interactive_permission_allow_via_control() {
        let _guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        let bin = write_fake_bin(
            dir.path(),
            "fake-claude-perm",
            r##"#!/usr/bin/env python3
import json, sys
_ = sys.stdin.readline()
req = {
  "type": "control_request",
  "request_id": "req-allow-1",
  "request": {
    "subtype": "can_use_tool",
    "tool_name": "Bash",
    "input": {"command": "echo hi"}
  }
}
print(json.dumps(req), flush=True)
line = sys.stdin.readline()
resp = json.loads(line)
assert resp.get("type") == "control_response"
assert resp["response"]["response"]["behavior"] == "allow"
print(json.dumps({"type":"assistant","message":{"content":[{"type":"text","text":"allowed-ok"}]}}), flush=True)
print(json.dumps({"type":"result","is_error":False,"result":"allowed-ok"}), flush=True)
"##,
        );
        std::env::set_var("AGENT_DOCTOR_CLAUDE_BIN", &bin);
        let control = PromptSessionControl::new();
        let control_for_reply = control.clone();
        thread::spawn(move || {
            for _ in 0..100 {
                thread::sleep(Duration::from_millis(30));
                if control_for_reply
                    .respond_permission("req-allow-1", true)
                    .is_ok()
                {
                    return;
                }
            }
        });
        let events = StdMutex::new(Vec::new());
        let report = ClaudeAskBackend
            .run(
                &PromptSessionOptions {
                    runtime: "claude-code".into(),
                    prompt: "run bash".into(),
                    cwd: Some(dir.path().to_path_buf()),
                    timeout_sec: 15,
                    dangerously_skip_permissions: false,
                    full_auto: false,
                    resume_thread_id: None,
                    selected_mcps: Vec::new(),
                },
                PromptSessionCancel::new(),
                Some(control),
                &mut |ev| events.lock().unwrap().push(ev),
            )
            .expect("session");
        std::env::remove_var("AGENT_DOCTOR_CLAUDE_BIN");
        assert_eq!(report.status, PromptSessionStatus::Succeeded);
        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(
            e,
            PromptSessionEvent::PermissionRequest { tool_name, .. } if tool_name == "Bash"
        )));
    }
}
