//! Codex ask backend via `codex app-server` (JSON-RPC over stdio).

use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::backend::AskBackend;
use super::control::PromptSessionControl;
use super::env::{
    apply_codex_env, apply_overlay_env, codex_provider_config_args, collect_overlay_env,
    format_command_display, prepare_codex_home, resolve_codex_overlay,
};
use super::util::{
    combine_output, force_stop_child, humanize_runtime_error, is_runtime_stderr_noise, join_reader,
    push_capped, summarize,
};
use super::{
    next_session_id, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, PromptSessionStatus, MAX_TIMEOUT_SEC, MIN_TIMEOUT_SEC,
};
use crate::session_launch::resolve_session_cwd;

static RPC_SEQ: AtomicU64 = AtomicU64::new(1);

pub struct CodexAskBackend;

impl AskBackend for CodexAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_codex_app_server(options, cancel, control, on_event)
    }
}

fn next_rpc_id() -> u64 {
    RPC_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Interactive Ask approval policy.
///
/// Use `on-request` (not `untrusted` / UnlessTrusted): the latter rejects
/// `require_escalated` and breaks agent turns. Trusted projects may still
/// auto-approve safe commands; sandbox escapes / explicit approvals still hit UI.
pub(crate) fn interactive_approval_policy() -> Value {
    json!("on-request")
}

pub(crate) fn elevated_approval_policy() -> Value {
    json!("never")
}

/// `thread/start` SandboxMode — kebab-case enum.
pub(crate) fn thread_sandbox_mode() -> &'static str {
    "workspace-write"
}

/// `turn/start` SandboxPolicy — camelCase `type`.
pub(crate) fn turn_sandbox_policy(cwd: &str) -> Value {
    json!({
        "type": "workspaceWrite",
        "writableRoots": [cwd],
        "networkAccess": true
    })
}

fn codex_ask_developer_instructions() -> String {
    "Do not use require_escalated or ask for elevated permissions. \
     Use ordinary shell/file/network tools; the host UI will show Allow/Deny when approval is required. \
     When creating or editing files with apply_patch, every hunk MUST start with one of: \
     '*** Add File: {path}', '*** Delete File: {path}', or '*** Update File: {path}'. \
     Never put file contents on the hunk header line. Example to add a file:\n\
     *** Begin Patch\n\
     *** Add File: path/to/file.txt\n\
     +line one\n\
     +line two\n\
     *** End Patch\n\
     Prefer apply_patch for file writes; if apply_patch fails validation, fix the hunk headers and retry \
     (or fall back to a simple shell write of the file contents)."
        .to_string()
}

fn run_codex_app_server(
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    control: Option<PromptSessionControl>,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<PromptSessionReport> {
    let session_id = next_session_id();
    let runtime = "codex".to_string();

    let prompt = options.prompt.trim();
    if prompt.is_empty() {
        bail!("prompt must not be empty");
    }

    let cwd = resolve_session_cwd(options.cwd.as_deref());
    if !cwd.exists() {
        bail!("session cwd does not exist: {}", cwd.display());
    }

    let timeout_sec = options.timeout_sec.clamp(MIN_TIMEOUT_SEC, MAX_TIMEOUT_SEC);
    let overlay = collect_overlay_env();
    prepare_codex_home(&overlay);

    let mut cmd = build_app_server_command(&cwd, &overlay)?;
    let command_display = format_command_display(&cmd);

    on_event(PromptSessionEvent::Started {
        session_id: session_id.clone(),
        runtime: runtime.clone(),
        cwd: cwd.display().to_string(),
        command: command_display,
    });

    let started = Instant::now();
    let mut child = cmd.spawn().context("failed to spawn codex app-server")?;

    let has_ui_control = control.is_some();
    let control = control.unwrap_or_default();
    if let Some(stdin) = child.stdin.take() {
        control.attach_stdin(stdin);
    } else {
        bail!("codex app-server missing stdin pipe");
    }

    let interactive = !options.full_auto && has_ui_control;
    let approval_policy = if interactive {
        interactive_approval_policy()
    } else {
        elevated_approval_policy()
    };
    let resume_thread_id = options
        .resume_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

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

    let result = (|| -> Result<(
        (PromptSessionStatus, Option<i32>, String, String),
        Option<String>,
    )> {
        // initialize
        let init_id = next_rpc_id();
        control.write_line(
            &json!({
                "method": "initialize",
                "id": init_id,
                "params": {
                    "clientInfo": {
                        "name": "agent_doctor",
                        "title": "Agent Doctor",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            })
            .to_string(),
        )?;
        control.write_line(&json!({ "method": "initialized", "params": {} }).to_string())?;

        // thread/start or thread/resume
        let thread_id_rpc = next_rpc_id();
        let thread_params = if let Some(ref tid) = resume_thread_id {
            json!({
                "threadId": tid,
                "cwd": cwd.display().to_string(),
                "approvalPolicy": approval_policy.clone(),
                "sandbox": thread_sandbox_mode(),
                "approvalsReviewer": "user",
                "developerInstructions": interactive.then(|| codex_ask_developer_instructions()),
            })
        } else {
            json!({
                "cwd": cwd.display().to_string(),
                "approvalPolicy": approval_policy.clone(),
                "sandbox": thread_sandbox_mode(),
                "serviceName": "agent_doctor_ask",
                "approvalsReviewer": "user",
                "developerInstructions": interactive.then(|| codex_ask_developer_instructions()),
            })
        };
        let thread_method = if resume_thread_id.is_some() {
            "thread/resume"
        } else {
            "thread/start"
        };
        control.write_line(
            &json!({
                "method": thread_method,
                "id": thread_id_rpc,
                "params": thread_params
            })
            .to_string(),
        )?;

        let mut state = PumpState {
            session_id: session_id.clone(),
            waiting_thread: Some(thread_id_rpc),
            waiting_turn: None,
            thread_id: resume_thread_id.clone(),
            turn_done: false,
            interactive,
            cwd: cwd.display().to_string(),
            prompt: prompt.to_string(),
            approval_policy,
            saw_agent_delta: false,
        };

        let outcome = pump_app_server(
            &mut child,
            timeout_sec,
            cancel.handle(),
            &control,
            &mut state,
            &mut emit,
        )?;
        Ok((outcome, state.thread_id))
    })();

    control.close();
    let duration_ms = started.elapsed().as_millis() as u64;

    let report = match result {
        Ok(((status, exit_code, stdout, stderr), runtime_thread_id)) => {
            let display = display_text.lock().map(|g| g.clone()).unwrap_or_default();
            let combined = if display.trim().is_empty() {
                combine_output(&stdout, &stderr)
            } else {
                display
            };
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
            let summary = humanize_runtime_error(&format!("{err:#}"));
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
                runtime_thread_id: resume_thread_id,
            }
        }
    };
    Ok(report)
}

fn build_app_server_command(cwd: &std::path::Path, overlay: &std::collections::HashMap<String, String>) -> Result<Command> {
    let bin = std::env::var("AGENT_DOCTOR_CODEX_BIN").unwrap_or_else(|_| "codex".into());
    let mut cmd = Command::new(bin);
    cmd.arg("app-server");
    for arg in codex_provider_config_args(resolve_codex_overlay(overlay).as_ref()) {
        cmd.arg(arg);
    }
    cmd.current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_overlay_env(&mut cmd, overlay);
    apply_codex_env(&mut cmd, overlay);
    Ok(cmd)
}

struct PumpState {
    session_id: String,
    waiting_thread: Option<u64>,
    waiting_turn: Option<u64>,
    thread_id: Option<String>,
    turn_done: bool,
    interactive: bool,
    cwd: String,
    prompt: String,
    approval_policy: Value,
    saw_agent_delta: bool,
}

fn pump_app_server<F>(
    child: &mut Child,
    timeout_sec: u64,
    cancel: Arc<AtomicBool>,
    control: &PromptSessionControl,
    state: &mut PumpState,
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

    let deadline = Instant::now() + Duration::from_secs(timeout_sec);
    let mut status = PromptSessionStatus::Succeeded;
    let mut exit_code: Option<i32> = Some(0);

    loop {
        let drained = {
            let mut guard = queue.lock().unwrap_or_else(|e| e.into_inner());
            guard.drain(..).collect::<Vec<_>>()
        };
        for (is_stdout, line) in drained {
            if is_stdout {
                handle_rpc_line(line, control, state, on_event)?;
            } else if !is_runtime_stderr_noise(&line) {
                on_event(PromptSessionEvent::StderrLine {
                    session_id: state.session_id.clone(),
                    line: humanize_runtime_error(&line),
                });
            }
        }

        if state.turn_done {
            force_stop_child(child, pid);
            break;
        }

        if cancel.load(Ordering::SeqCst) {
            force_stop_child(child, pid);
            status = PromptSessionStatus::Cancelled;
            exit_code = None;
            break;
        }
        if Instant::now() >= deadline {
            force_stop_child(child, pid);
            status = PromptSessionStatus::TimedOut;
            exit_code = None;
            break;
        }

        match child.try_wait() {
            Ok(Some(wait_status)) => {
                exit_code = wait_status.code();
                status = if wait_status.success() && state.turn_done {
                    PromptSessionStatus::Succeeded
                } else if wait_status.success() {
                    // Process exited before turn completed — treat as failure.
                    PromptSessionStatus::Failed
                } else {
                    PromptSessionStatus::Failed
                };
                break;
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(error) => return Err(error).context("failed waiting for codex app-server"),
        }
    }

    join_reader(stdout_handle, Duration::from_millis(500));
    join_reader(stderr_handle, Duration::from_millis(500));
    // Final drain
    let drained = {
        let mut guard = queue.lock().unwrap_or_else(|e| e.into_inner());
        guard.drain(..).collect::<Vec<_>>()
    };
    for (is_stdout, line) in drained {
        if is_stdout {
            let _ = handle_rpc_line(line, control, state, on_event);
        }
    }

    let stdout = stdout_acc.lock().map(|g| g.clone()).unwrap_or_default();
    let stderr = stderr_acc.lock().map(|g| g.clone()).unwrap_or_default();
    Ok((status, exit_code, stdout, stderr))
}

fn handle_rpc_line<F>(
    line: String,
    control: &PromptSessionControl,
    state: &mut PumpState,
    on_event: &mut F,
) -> Result<()>
where
    F: FnMut(PromptSessionEvent),
{
    let Ok(value) = serde_json::from_str::<Value>(&line) else {
        if !line.trim().is_empty() {
            on_event(PromptSessionEvent::StdoutLine {
                session_id: state.session_id.clone(),
                line,
            });
        }
        return Ok(());
    };

    // JSON-RPC response
    if let Some(id) = value.get("id") {
        if value.get("result").is_some() || value.get("error").is_some() {
            return handle_rpc_response(&value, id, control, state, on_event);
        }
    }

    // Server-initiated request (approvals)
    if let (Some(id), Some(method)) = (value.get("id"), value.get("method").and_then(|m| m.as_str()))
    {
        return handle_server_request(method, id, value.get("params"), control, state, on_event);
    }

    // Notification
    if let Some(method) = value.get("method").and_then(|m| m.as_str()) {
        handle_notification(method, value.get("params"), state, on_event);
    }

    Ok(())
}

fn handle_rpc_response<F>(
    value: &Value,
    id: &Value,
    control: &PromptSessionControl,
    state: &mut PumpState,
    on_event: &mut F,
) -> Result<()>
where
    F: FnMut(PromptSessionEvent),
{
    let id_num = id.as_u64().or_else(|| id.as_i64().map(|n| n as u64));

    if let Some(err) = value.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("codex app-server error");
        on_event(PromptSessionEvent::StderrLine {
            session_id: state.session_id.clone(),
            line: humanize_runtime_error(msg),
        });
        if state.waiting_thread == id_num || state.waiting_turn == id_num {
            state.turn_done = true;
        }
        return Ok(());
    }

    if state.waiting_thread == id_num {
        state.waiting_thread = None;
        let thread_id = value
            .pointer("/result/thread/id")
            .or_else(|| value.pointer("/result/thread/sessionId"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .context("thread/start missing thread id")?;
        state.thread_id = Some(thread_id.clone());

        let turn_id = next_rpc_id();
        state.waiting_turn = Some(turn_id);
        on_event(PromptSessionEvent::Status {
            session_id: state.session_id.clone(),
            phase: "requesting".into(),
            message: "正在请求模型…".into(),
        });
        control.write_line(
            &json!({
                "method": "turn/start",
                "id": turn_id,
                "params": {
                    "threadId": thread_id,
                    "input": [{ "type": "text", "text": state.prompt }],
                    "cwd": state.cwd,
                    "approvalPolicy": state.approval_policy,
                    // SandboxPolicy.type uses camelCase (unlike thread `sandbox` SandboxMode kebab-case).
                    "sandboxPolicy": turn_sandbox_policy(&state.cwd)
                }
            })
            .to_string(),
        )?;
        return Ok(());
    }

    if state.waiting_turn == id_num {
        state.waiting_turn = None;
        // Turn accepted; completion arrives via turn/completed notification.
    }

    Ok(())
}

fn handle_server_request<F>(
    method: &str,
    id: &Value,
    params: Option<&Value>,
    control: &PromptSessionControl,
    state: &mut PumpState,
    on_event: &mut F,
) -> Result<()>
where
    F: FnMut(PromptSessionEvent),
{
    let request_id = match id {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };

    match method {
        "item/commandExecution/requestApproval"
        | "item/fileChange/requestApproval"
        | "item/permissions/requestApproval" => {
            if !state.interactive {
                let _ = control.decline_codex_rpc(id);
                return Ok(());
            }
            let params = params.cloned().unwrap_or(Value::Null);
            let (tool_name, detail, input_json) = permission_from_codex(method, &params);
            control.remember_codex_rpc(&request_id);
            on_event(PromptSessionEvent::Status {
                session_id: state.session_id.clone(),
                phase: "permission".into(),
                message: format!("等待确认：{tool_name}"),
            });
            on_event(PromptSessionEvent::PermissionRequest {
                session_id: state.session_id.clone(),
                request_id,
                tool_name,
                detail,
                input_json,
            });
        }
        other => {
            on_event(PromptSessionEvent::Status {
                session_id: state.session_id.clone(),
                phase: "info".into(),
                message: format!("auto-decline unsupported request: {other}"),
            });
            let _ = control.decline_codex_rpc(id);
        }
    }
    Ok(())
}

fn handle_notification<F>(
    method: &str,
    params: Option<&Value>,
    state: &mut PumpState,
    on_event: &mut F,
) where
    F: FnMut(PromptSessionEvent),
{
    let params = params.cloned().unwrap_or(Value::Null);
    match method {
        "turn/started" => {
            on_event(PromptSessionEvent::Status {
                session_id: state.session_id.clone(),
                phase: "requesting".into(),
                message: "正在请求模型…".into(),
            });
        }
        "turn/completed" | "turn/finished" => {
            state.turn_done = true;
            on_event(PromptSessionEvent::Status {
                session_id: state.session_id.clone(),
                phase: "writing".into(),
                message: "本轮完成".into(),
            });
        }
        "turn/failed" => {
            state.turn_done = true;
            let msg = params
                .pointer("/error/message")
                .or_else(|| params.pointer("/turn/error/message"))
                .and_then(|v| v.as_str())
                .unwrap_or("Codex turn failed");
            on_event(PromptSessionEvent::StderrLine {
                session_id: state.session_id.clone(),
                line: humanize_runtime_error(msg),
            });
        }
        "item/agentMessage/delta" => {
            if let Some(text) = params
                .get("delta")
                .or_else(|| params.pointer("/item/text"))
                .and_then(|v| v.as_str())
            {
                if !text.is_empty() {
                    // Do NOT emit Status on every delta — the chat UI seals the
                    // assistant bubble on phase changes, which otherwise stores
                    // one localStorage message per token.
                    state.saw_agent_delta = true;
                    on_event(PromptSessionEvent::Delta {
                        session_id: state.session_id.clone(),
                        text: text.to_string(),
                    });
                }
            }
        }
        "item/completed" | "item/started" => {
            let item = params.get("item").cloned().unwrap_or(Value::Null);
            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match item_type {
                // Prefer streamed deltas; only fall back to the completed payload
                // when no delta arrived (some turns emit text only on completed).
                "agent_message" if method == "item/completed" => {
                    if !state.saw_agent_delta {
                        if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                on_event(PromptSessionEvent::Delta {
                                    session_id: state.session_id.clone(),
                                    text: text.to_string(),
                                });
                            }
                        }
                    }
                }
                "agent_message" => {}
                "commandExecution" | "fileChange" | "mcpToolCall" => {
                    // Emit once on start — completed would duplicate the same chip in UI.
                    if method == "item/started" {
                        let label = item
                            .get("command")
                            .and_then(|v| {
                                if let Some(s) = v.as_str() {
                                    Some(s.to_string())
                                } else if let Some(arr) = v.as_array() {
                                    Some(
                                        arr.iter()
                                            .filter_map(|x| x.as_str())
                                            .collect::<Vec<_>>()
                                            .join(" "),
                                    )
                                } else {
                                    None
                                }
                            })
                            .or_else(|| {
                                item.get("tool")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string)
                            })
                            .unwrap_or_else(|| item_type.to_string());
                        let label = shorten_tool_label(&label);
                        on_event(PromptSessionEvent::Status {
                            session_id: state.session_id.clone(),
                            phase: "tool".into(),
                            message: label,
                        });
                    }
                }
                "error" => {
                    if let Some(msg) = item.get("message").and_then(|v| v.as_str()) {
                        on_event(PromptSessionEvent::StderrLine {
                            session_id: state.session_id.clone(),
                            line: humanize_runtime_error(msg),
                        });
                    }
                }
                _ => {}
            }
        }
        "error" => {
            if let Some(msg) = params.get("message").and_then(|v| v.as_str()) {
                on_event(PromptSessionEvent::StderrLine {
                    session_id: state.session_id.clone(),
                    line: humanize_runtime_error(msg),
                });
            }
        }
        _ => {}
    }
}

pub(crate) fn permission_from_codex(method: &str, params: &Value) -> (String, String, String) {
    let tool_name = match method {
        "item/fileChange/requestApproval" => "FileChange",
        "item/permissions/requestApproval" => "Permissions",
        _ => "Bash",
    }
    .to_string();

    let detail = params
        .get("reason")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            params.get("command").and_then(|v| {
                if let Some(s) = v.as_str() {
                    Some(s.to_string())
                } else if let Some(arr) = v.as_array() {
                    Some(
                        arr.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(" "),
                    )
                } else {
                    None
                }
            })
        })
        .or_else(|| {
            params
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(|c| format!("cwd: {c}"))
        })
        .unwrap_or_else(|| params.to_string());

    (tool_name, detail, params.to_string())
}

/// Collapse shell wrappers like `/bin/zsh -lc 'pwd'` → `pwd` for compact UI chips.
fn shorten_tool_label(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return t.to_string();
    }
    // `/bin/zsh -lc 'cmd'` or `zsh -lc "cmd"`
    if let Some(idx) = t.find(" -lc ") {
        let rest = t[idx + 5..].trim();
        let unquoted = rest
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .or_else(|| rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
            .unwrap_or(rest)
            .trim();
        if !unquoted.is_empty() {
            return unquoted.to_string();
        }
    }
    // argv form: /bin/zsh -lc pwd
    let parts: Vec<&str> = t.split_whitespace().collect();
    if parts.len() >= 3 && parts[1] == "-lc" {
        return parts[2..].join(" ");
    }
    t.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortens_zsh_lc_wrappers() {
        assert_eq!(shorten_tool_label("/bin/zsh -lc pwd"), "pwd");
        assert_eq!(shorten_tool_label("/bin/zsh -lc 'ls -la'"), "ls -la");
        assert_eq!(shorten_tool_label("echo hi"), "echo hi");
    }

    #[test]
    fn permission_from_command_approval() {
        let params = json!({
            "command": ["curl", "-s", "https://example.com"],
            "reason": "network",
            "cwd": "/tmp"
        });
        let (tool, detail, _) =
            permission_from_codex("item/commandExecution/requestApproval", &params);
        assert_eq!(tool, "Bash");
        assert_eq!(detail, "network");
    }

    #[test]
    fn interactive_policy_is_on_request_not_unless_trusted() {
        assert_eq!(interactive_approval_policy(), json!("on-request"));
        assert_eq!(elevated_approval_policy(), json!("never"));
        assert_eq!(thread_sandbox_mode(), "workspace-write");
        let policy = turn_sandbox_policy("/tmp/proj");
        assert_eq!(policy["type"], "workspaceWrite");
        assert_eq!(policy["writableRoots"][0], "/tmp/proj");
    }

    #[cfg(unix)]
    #[test]
    fn app_server_permission_allow_via_control() {
        use crate::prompt_session::util::TEST_ENV_LOCK;
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::sync::Mutex as StdMutex;
        use tempfile::tempdir;

        let _guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        fn write_fake_bin(dir: &Path, name: &str, script: &str) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join(name);
            fs::write(&path, script).expect("write fake bin");
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).unwrap();
            path
        }

        let dir = tempdir().unwrap();
        let bin = write_fake_bin(
            dir.path(),
            "fake-codex",
            r##"#!/usr/bin/env python3
import json, sys

def read():
    line = sys.stdin.readline()
    if not line:
        return None
    return json.loads(line)

# initialize
msg = read()
assert msg["method"] == "initialize"
print(json.dumps({"id": msg["id"], "result": {"userAgent": "fake"}}), flush=True)
msg = read()
assert msg["method"] == "initialized"

# thread/start
msg = read()
assert msg["method"] == "thread/start"
assert msg["params"]["approvalPolicy"] == "on-request"
assert msg["params"]["sandbox"] == "workspace-write"
print(json.dumps({"id": msg["id"], "result": {"thread": {"id": "thr_1"}}}), flush=True)

# turn/start
msg = read()
assert msg["method"] == "turn/start"
assert msg["params"]["sandboxPolicy"]["type"] == "workspaceWrite"
print(json.dumps({"id": msg["id"], "result": {"turn": {"id": "turn_1", "status": "inProgress"}}}), flush=True)
print(json.dumps({"method": "turn/started", "params": {"turn": {"id": "turn_1"}}}), flush=True)
print(json.dumps({
  "id": 99,
  "method": "item/commandExecution/requestApproval",
  "params": {"command": ["echo", "hi"], "reason": "run echo"}
}), flush=True)

# wait for decision
msg = read()
assert msg["id"] == 99
assert msg["result"]["decision"] == "accept"

print(json.dumps({
  "method": "item/completed",
  "params": {"item": {"id": "i1", "type": "agent_message", "text": "codex-ok"}}
}), flush=True)
print(json.dumps({"method": "turn/completed", "params": {"turn": {"id": "turn_1"}}}), flush=True)
import time
time.sleep(5)
"##,
        );
        std::env::set_var("AGENT_DOCTOR_CODEX_BIN", &bin);

        let control = PromptSessionControl::new();
        let events = StdMutex::new(Vec::new());
        let control_for_reply = control.clone();
        thread::spawn(move || {
            for _ in 0..200 {
                thread::sleep(Duration::from_millis(20));
                if control_for_reply.respond_permission("99", true).is_ok() {
                    return;
                }
            }
        });

        let report = CodexAskBackend
            .run(
                &PromptSessionOptions {
                    runtime: "codex".into(),
                    prompt: "run echo".into(),
                    cwd: Some(dir.path().to_path_buf()),
                    timeout_sec: 10,
                    dangerously_skip_permissions: false,
                    full_auto: false,
                    resume_thread_id: None,
                },
                PromptSessionCancel::new(),
                Some(control),
                &mut |ev| events.lock().unwrap().push(ev),
            )
            .expect("session");
        std::env::remove_var("AGENT_DOCTOR_CODEX_BIN");
        assert_eq!(
            report.status,
            PromptSessionStatus::Succeeded,
            "summary={} events={:?}",
            report.summary,
            events
                .lock()
                .unwrap()
                .iter()
                .map(|e| match e {
                    PromptSessionEvent::Started { .. } => "started".into(),
                    PromptSessionEvent::Status { message, .. } => format!("status:{message}"),
                    PromptSessionEvent::Delta { text, .. } => format!("delta:{text}"),
                    PromptSessionEvent::PermissionRequest { tool_name, .. } => {
                        format!("perm:{tool_name}")
                    }
                    PromptSessionEvent::StderrLine { line, .. } => format!("err:{line}"),
                    PromptSessionEvent::Completed { status, .. } => format!("done:{status:?}"),
                    _ => "other".into(),
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(report.runtime_thread_id.as_deref(), Some("thr_1"));
        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(
            e,
            PromptSessionEvent::PermissionRequest { tool_name, .. } if tool_name == "Bash"
        )));
        assert!(evs.iter().any(|e| matches!(
            e,
            PromptSessionEvent::Delta { text, .. } if text.contains("codex-ok")
        )));
    }
}
