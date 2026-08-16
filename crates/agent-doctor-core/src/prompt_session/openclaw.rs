//! OpenClaw ask backend (`openclaw agent --local --agent … --session-id … --json`).
//!
//! Always pass session selectors (`--agent` + `--session-id`). JSON goes to stdout and
//! diagnostics to stderr — Ask surfaces only the assistant reply, not CLI chrome.

use std::fs;
use std::path::{Path, PathBuf};
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
    // Always own a session id so Ask can resume natively and avoid stuffing chat history
    // into `--message` (which previously polluted OpenClaw into `NO_REPLY`).
    let openclaw_session_id = resume_session_id
        .map(str::to_string)
        .unwrap_or_else(fresh_openclaw_session_id);

    let overlay = collect_overlay_env();
    if let Some(note) = ensure_openclaw_agent_ready(&overlay) {
        on_event(PromptSessionEvent::Status {
            session_id: session_id.clone(),
            phase: "workspace".into(),
            message: note,
        });
    }
    let mut prompt_text = prompt.to_string();
    if super::mcp_ensure::wants_browser_mcp(options) {
        if let Some(note) =
            super::mcp_ensure::ensure_browser_mcp_for_ask("openclaw", &cwd, &overlay)
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

    let mut cmd = build_openclaw_command(&prompt_text, &cwd, &openclaw_session_id, timeout_sec)?;
    let command_display = format_command_display(&cmd);

    on_event(PromptSessionEvent::Started {
        session_id: session_id.clone(),
        runtime: runtime.clone(),
        cwd: cwd.display().to_string(),
        command: command_display,
    });

    let started = Instant::now();
    let mut child = cmd.spawn().context("failed to spawn openclaw")?;
    let agent = resolve_openclaw_agent(&overlay);
    let tool_trace_path = openclaw_session_jsonl_path(&agent, &openclaw_session_id);
    let mut emitted_tools = std::collections::HashSet::new();
    let mut tool_trace_len = 0;

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
        tool_trace_path.as_deref(),
        &mut emitted_tools,
        &mut tool_trace_len,
        &mut emit,
    );

    let duration_ms = started.elapsed().as_millis() as u64;
    let report = match result {
        Ok((status, exit_code, stdout, stderr)) => {
            let parsed = parse_openclaw_json_output(&stdout);
            let reply = parsed
                .as_ref()
                .and_then(extract_openclaw_reply)
                .unwrap_or_default();
            let runtime_thread_id = parsed
                .as_ref()
                .and_then(extract_openclaw_session_id)
                .unwrap_or_else(|| openclaw_session_id.clone());

            // `openclaw --json` only returns the final reply. Tool chips live in the
            // session jsonl (`browser__browser_navigate`), so surface them before the
            // assistant text — Ask verify used to miss the MCP pathway entirely.
            emit_openclaw_turn_tools(
                &session_id,
                &agent,
                &runtime_thread_id,
                &mut emitted_tools,
                &mut emit,
            );

            let combined = if !reply.trim().is_empty() {
                reply.clone()
            } else if status != PromptSessionStatus::Succeeded {
                extract_openclaw_error_text(&stderr)
                    .or_else(|| extract_openclaw_error_text(&stdout))
                    .unwrap_or_else(|| summarize_openclaw_failure(&stderr, &stdout))
            } else {
                String::new()
            };

            if !reply.trim().is_empty() {
                emit(PromptSessionEvent::Delta {
                    session_id: session_id.clone(),
                    text: reply,
                });
            } else if status != PromptSessionStatus::Succeeded && !combined.trim().is_empty() {
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
                runtime_thread_id: Some(runtime_thread_id),
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
                runtime_thread_id: Some(openclaw_session_id),
            }
        }
    };
    Ok(report)
}

fn fresh_openclaw_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    // UUID-shaped id accepted by `--session-id`.
    let a = (nanos ^ (pid << 64)) as u64;
    let b = (nanos.wrapping_mul(0x9e37_79b9_7f4a_7c15)) as u64;
    format!(
        "{:08x}-{:04x}-4{:03x}-a{:03x}-{:012x}",
        (a >> 32) as u32,
        ((a >> 16) & 0xffff) as u16,
        (a & 0x0fff) as u16,
        ((b >> 48) & 0x0fff) as u16,
        b & 0xffff_ffff_ffff
    )
}

fn resolve_openclaw_agent(overlay: &std::collections::HashMap<String, String>) -> String {
    overlay
        .get("OPENCLAW_AGENT_ID")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("AGENT_DOCTOR_OPENCLAW_AGENT")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "main".into())
}

/// Bind/create workspace OpenClaw agent before Ask spawn (fixes missing agents.list).
fn ensure_openclaw_agent_ready(
    overlay: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let agent = resolve_openclaw_agent(overlay);
    if agent == "main" {
        return None;
    }
    let workspace = overlay
        .get("OPENCLAW_WORKSPACE")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            crate::workspace::load_workspaces().ok().and_then(|doc| {
                doc.active
                    .as_ref()
                    .and_then(|name| doc.workspaces.get(name))
                    .map(|entry| entry.openclaw_workspace.clone())
            })
        })?;

    match crate::workspace::backends::bind_openclaw(&agent, &workspace) {
        Ok(report) => Some(format!(
            "OpenClaw agent ready: {} ({})",
            agent, report.detail
        )),
        Err(err) => Some(format!(
            "OpenClaw agent '{agent}' missing and bind failed ({err}); Ask may fall back if spawn fails"
        )),
    }
}

fn build_openclaw_command(
    prompt: &str,
    cwd: &Path,
    session_id: &str,
    timeout_sec: u64,
) -> Result<Command> {
    let overlay = collect_overlay_env();
    let bin = std::env::var("AGENT_DOCTOR_OPENCLAW_BIN").unwrap_or_else(|_| "openclaw".into());
    let agent = resolve_openclaw_agent(&overlay);
    let mut cmd = Command::new(&bin);
    cmd.arg("agent")
        .arg("--local")
        .arg("--agent")
        .arg(&agent)
        .arg("--session-id")
        .arg(session_id)
        .arg("--message")
        .arg(prompt)
        .arg("--json")
        .arg("--timeout")
        .arg(timeout_sec.to_string())
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    apply_overlay_env(&mut cmd, &overlay);
    Ok(cmd)
}

fn pump_lines<F>(
    session_id: &str,
    child: &mut Child,
    timeout_sec: u64,
    cancel: Arc<AtomicBool>,
    tool_trace_path: Option<&Path>,
    emitted_tools: &mut std::collections::HashSet<String>,
    tool_trace_len: &mut u64,
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
                // JSON is parsed at completion; never stream fragments into the chat bubble.
                continue;
            }
            if is_openclaw_stderr_noise(&line) || is_runtime_stderr_noise(&line) {
                continue;
            }
            on_event(PromptSessionEvent::StderrLine {
                session_id: session_id.to_string(),
                line,
            });
        }
    };

    let deadline = Instant::now() + Duration::from_secs(timeout_sec);
    let mut next_tool_poll = Instant::now();
    let (status, exit_code) = loop {
        drain(on_event);
        if Instant::now() >= next_tool_poll {
            if let Some(path) = tool_trace_path {
                emit_openclaw_tools_from_path(
                    session_id,
                    path,
                    emitted_tools,
                    tool_trace_len,
                    on_event,
                );
            }
            next_tool_poll = Instant::now() + Duration::from_millis(120);
        }
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
    if let Some(path) = tool_trace_path {
        emit_openclaw_tools_from_path(session_id, path, emitted_tools, tool_trace_len, on_event);
    }

    let stdout = stdout_acc.lock().map(|g| g.clone()).unwrap_or_default();
    let stderr = stderr_acc.lock().map(|g| g.clone()).unwrap_or_default();
    Ok((status, exit_code, stdout, stderr))
}

fn emit_openclaw_turn_tools<F>(
    session_id: &str,
    agent: &str,
    runtime_thread_id: &str,
    emitted_tools: &mut std::collections::HashSet<String>,
    emit: &mut F,
) where
    F: FnMut(PromptSessionEvent),
{
    let Some(path) = resolve_openclaw_session_jsonl(agent, runtime_thread_id) else {
        return;
    };
    let mut trace_len = 0;
    emit_openclaw_tools_from_path(session_id, &path, emitted_tools, &mut trace_len, emit);
}

fn openclaw_session_jsonl_path(agent: &str, session_id: &str) -> Option<PathBuf> {
    let agent = agent.trim();
    let session_id = session_id.trim();
    if agent.is_empty()
        || session_id.is_empty()
        || agent.contains(['/', '\\'])
        || session_id.contains(['/', '\\'])
    {
        return None;
    }
    let home = dirs::home_dir()?;
    let path = home
        .join(".openclaw")
        .join("agents")
        .join(agent)
        .join("sessions")
        .join(format!("{session_id}.jsonl"));
    Some(path)
}

fn resolve_openclaw_session_jsonl(agent: &str, session_id: &str) -> Option<PathBuf> {
    let path = openclaw_session_jsonl_path(agent, session_id)?;
    path.is_file().then_some(path)
}

fn emit_openclaw_tools_from_path<F>(
    session_id: &str,
    path: &Path,
    emitted_tools: &mut std::collections::HashSet<String>,
    trace_len: &mut u64,
    emit: &mut F,
) where
    F: FnMut(PromptSessionEvent),
{
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    let current_len = metadata.len();
    if current_len == *trace_len {
        return;
    }
    *trace_len = current_len;
    for name in collect_last_turn_openclaw_tools(path) {
        if !emitted_tools.insert(name.clone()) {
            continue;
        }
        emit(PromptSessionEvent::Status {
            session_id: session_id.to_string(),
            phase: "tool".into(),
            message: format!("调用工具 {name}…"),
        });
    }
}

/// Tools used after the last user message in an OpenClaw session jsonl.
fn collect_last_turn_openclaw_tools(path: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    collect_last_turn_openclaw_tools_from(&text)
}

fn collect_last_turn_openclaw_tools_from(text: &str) -> Vec<String> {
    let mut tools = Vec::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let message = value.get("message").unwrap_or(&value);
        let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "user" {
            tools.clear();
            continue;
        }
        if let Some(arr) = message.get("content").and_then(|v| v.as_array()) {
            for item in arr {
                let ty = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if ty == "toolCall" || ty == "tool_use" || ty == "functionCall" {
                    if let Some(name) = item
                        .get("name")
                        .or_else(|| item.get("toolName"))
                        .or_else(|| item.pointer("/function/name"))
                        .and_then(|v| v.as_str())
                    {
                        if !name.is_empty() {
                            tools.push(name.to_string());
                        }
                    }
                }
            }
        }
        if role == "toolResult" || role == "tool" {
            for key in [
                message.get("toolName"),
                message.get("tool_name"),
                message.pointer("/details/mcpTool"),
            ] {
                if let Some(name) = key
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    tools.push(name.to_string());
                }
            }
        }
    }
    tools
}

fn parse_openclaw_json_output(stdout: &str) -> Option<Value> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    // Pretty-printed JSON: take the first balanced `{ … }` block.
    if let Some(start) = trimmed.find('{') {
        let slice = &trimmed[start..];
        let mut depth = 0i32;
        for (i, ch) in slice.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(value) = serde_json::from_str::<Value>(&slice[..=i]) {
                            return Some(value);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
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

fn normalize_openclaw_reply_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.eq_ignore_ascii_case("NO_REPLY")
        || trimmed.eq_ignore_ascii_case("no_reply")
        || trimmed == "∅"
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn extract_openclaw_reply(value: &Value) -> Option<String> {
    // Prefer visible payloads / final text over deep meta dumps.
    if let Some(arr) = value.get("payloads").and_then(|v| v.as_array()) {
        let parts: Vec<String> = arr
            .iter()
            .filter_map(|item| item.get("text").and_then(value_to_text))
            .filter_map(|s| normalize_openclaw_reply_text(&s))
            .collect();
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    const KEYS: &[&str] = &[
        "final",
        "reply",
        "message",
        "text",
        "result",
        "output",
        "content",
        "response",
        "finalAssistantVisibleText",
        "finalAssistantRawText",
    ];
    for key in KEYS {
        if let Some(text) = value.get(*key).and_then(value_to_text) {
            if let Some(normalized) = normalize_openclaw_reply_text(&text) {
                return Some(normalized);
            }
        }
    }
    for nest in ["data", "result", "meta"] {
        if let Some(obj) = value.get(nest) {
            if obj.is_object() {
                // Avoid walking the huge systemPromptReport tree for false positives.
                if nest == "meta" {
                    if let Some(text) = obj
                        .get("finalAssistantVisibleText")
                        .or_else(|| obj.get("finalAssistantRawText"))
                        .and_then(value_to_text)
                        .and_then(|t| normalize_openclaw_reply_text(&t))
                    {
                        return Some(text);
                    }
                    continue;
                }
                if let Some(text) = extract_openclaw_reply(obj) {
                    return Some(text);
                }
            }
        }
    }
    None
}

fn extract_openclaw_session_id(value: &Value) -> Option<String> {
    for key in ["sessionId", "session_id", "session"] {
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
        .get("meta")
        .and_then(|m| m.get("agentMeta"))
        .and_then(extract_openclaw_session_id)
        .or_else(|| value.get("meta").and_then(extract_openclaw_session_id))
        .or_else(|| value.get("data").and_then(extract_openclaw_session_id))
        .or_else(|| value.get("result").and_then(extract_openclaw_session_id))
}

fn extract_openclaw_error_text(text: &str) -> Option<String> {
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || is_openclaw_stderr_noise(line) {
            continue;
        }
        if trimmed.starts_with("Error:")
            || trimmed.starts_with("OpenClaw does not recognize")
            || trimmed.contains("HTTP 401")
            || trimmed.contains("does not recognize option")
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

fn summarize_openclaw_failure(stderr: &str, stdout: &str) -> String {
    let compact = combine_output(stderr, stdout);
    let trimmed = compact.trim();
    if trimmed.is_empty() {
        "openclaw failed".into()
    } else if trimmed.len() > 400 {
        format!("{}…", &trimmed[..400])
    } else {
        trimmed.to_string()
    }
}

fn is_openclaw_stderr_noise(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("[agents/")
        || lower.starts_with("[provider-")
        || lower.starts_with("[model-")
        || lower.contains("tool policy removed")
        || lower.contains("model-fetch")
        || lower.contains("provider-transport")
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
                } else {
                    item.get("text")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
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
        assert_eq!(
            extract_openclaw_reply(&value).as_deref(),
            Some("openclaw-ok")
        );
        assert_eq!(extract_openclaw_session_id(&value).as_deref(), Some("oc-1"));
    }

    #[test]
    fn parses_local_json_payloads_and_meta_session() {
        let raw = r#"{
          "payloads":[{"text":"hey","mediaUrl":null}],
          "meta":{"agentMeta":{"sessionId":"oc-meta-1"}}
        }"#;
        let value = parse_openclaw_json_output(raw).unwrap();
        assert_eq!(extract_openclaw_reply(&value).as_deref(), Some("hey"));
        assert_eq!(
            extract_openclaw_session_id(&value).as_deref(),
            Some("oc-meta-1")
        );
    }

    #[test]
    fn ignores_no_reply_sentinel() {
        let raw = r#"{
          "payloads":[],
          "meta":{"finalAssistantVisibleText":"NO_REPLY","agentMeta":{"sessionId":"oc-2"}}
        }"#;
        let value = parse_openclaw_json_output(raw).unwrap();
        assert_eq!(extract_openclaw_reply(&value), None);
        assert_eq!(extract_openclaw_session_id(&value).as_deref(), Some("oc-2"));
    }

    #[test]
    fn last_turn_tools_from_openclaw_session_jsonl() {
        let raw = r#"
{"type":"message","message":{"role":"user","content":"old turn with browser_navigate"}}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"exec"}]}}
{"type":"message","message":{"role":"user","content":"请用 browser MCP"}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"ok"},{"type":"toolCall","name":"browser__browser_navigate","arguments":{"url":"https://example.com/"}}]}}
{"type":"message","message":{"role":"toolResult","toolName":"browser__browser_navigate","details":{"mcpServer":"browser","mcpTool":"browser_navigate"}}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Example Domain"}]}}
"#;
        let tools = collect_last_turn_openclaw_tools_from(raw);
        assert_eq!(
            tools,
            vec![
                "browser__browser_navigate".to_string(),
                "browser__browser_navigate".to_string(),
                "browser_navigate".to_string(),
            ]
        );
    }

    #[test]
    fn emits_new_openclaw_tools_as_session_trace_grows() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        fs::write(
            &path,
            concat!(
                "{\"message\":{\"role\":\"user\",\"content\":\"test\"}}\n",
                "{\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"toolCall\",\"name\":\"browser_navigate\"}]}}\n",
            ),
        )
        .unwrap();

        let mut seen = std::collections::HashSet::new();
        let mut trace_len = 0;
        let mut events = Vec::new();
        emit_openclaw_tools_from_path("s1", &path, &mut seen, &mut trace_len, &mut |event| {
            events.push(event)
        });
        assert_eq!(events.len(), 1);

        fs::write(
            &path,
            concat!(
                "{\"message\":{\"role\":\"user\",\"content\":\"test\"}}\n",
                "{\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"toolCall\",\"name\":\"browser_navigate\"}]}}\n",
                "{\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"toolCall\",\"name\":\"browser_screenshot\"}]}}\n",
            ),
        )
        .unwrap();
        emit_openclaw_tools_from_path("s1", &path, &mut seen, &mut trace_len, &mut |event| {
            events.push(event)
        });

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[1],
            PromptSessionEvent::Status { message, .. } if message.contains("browser_screenshot")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn streams_json_reply_and_succeeds() {
        let _guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        let bin = write_fake_bin(
            dir.path(),
            "fake-openclaw",
            "#!/bin/bash\necho '{\"payloads\":[{\"text\":\"openclaw-ok\"}],\"meta\":{\"agentMeta\":{\"sessionId\":\"oc-sid\"}}}'\nexit 0\n",
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
                    selected_mcps: Vec::new(),
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
        assert!(!evs
            .iter()
            .any(|e| matches!(e, PromptSessionEvent::StdoutLine { .. })));
    }
}
