//! ZCode ask backend.
//!
//! The desktop app does not put `zcode` on PATH. Headless chat is the bundled
//! script, launched as Node by the app binary:
//! `ELECTRON_RUN_AS_NODE=1 ZCode …/glm/zcode.cjs --prompt … --output-format stream-json`.
//! `--prompt` defaults to yolo, so this path does not wait on a permission prompt
//! the ask window cannot answer.

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
use super::plan::{tool_carries_plan, PlanBoard};
use super::tool_step::{tool_result_text, tool_step, tool_step_result};
use super::util::{
    combine_output, finish_oneshot_after_pipes_closed, force_stop_child, format_tool_status,
    is_runtime_log_line, is_runtime_stderr_noise, join_reader, push_capped, stderr_event,
    summarize, tool_activity_detail, tool_chip, turn_error, SessionClock, ToolWatch,
};
use super::{
    next_session_id, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, PromptSessionStatus, PumpResult, MAX_TIMEOUT_SEC, MIN_TIMEOUT_SEC,
};
use agent_doctor_core::adapters::locate_zcode_headless;
use agent_doctor_core::session_env::format_command_display;
use agent_doctor_core::session_launch::resolve_session_cwd;

const ZCODE_MISSING: &str = "没找到 ZCode。先点「安装」。";
const ZCODE_NOT_READY: &str = "ZCode 还不能在这里问答。先打开 ZCode 窗口登录一次，再回来问。";

pub struct ZcodeAskBackend;

impl AskBackend for ZcodeAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        _control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_zcode(options, cancel, on_event)
    }
}

fn run_zcode(
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<PromptSessionReport> {
    let session_id = next_session_id();
    let runtime = "zcode".to_string();
    let prompt = options.prompt.trim();
    if prompt.is_empty() {
        bail!("prompt must not be empty");
    }
    let cwd = resolve_session_cwd(options.cwd.as_deref(), options.workspace_name.as_deref());
    if !cwd.exists() {
        bail!("session cwd does not exist: {}", cwd.display());
    }
    let timeout_sec = options.timeout_sec.clamp(MIN_TIMEOUT_SEC, MAX_TIMEOUT_SEC);
    // Headless `--resume` takes a ZCode session id (`sess_…`). A bare UUID is
    // a Claude session and must not be forwarded.
    let resume_session_id = zcode_resume_id(options.resume_thread_id.as_deref());

    let mut cmd = build_zcode_command(prompt, &cwd, resume_session_id)?;
    let command_display = format_command_display(&cmd);
    on_event(PromptSessionEvent::Started {
        session_id: session_id.clone(),
        runtime: runtime.clone(),
        cwd: cwd.display().to_string(),
        command: command_display,
        client_run_id: String::new(),
    });

    let started = Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|_| anyhow::Error::msg(ZCODE_NOT_READY))?;
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
        Ok((mut status, exit_code, stdout, stderr, timeout)) => {
            let stream = extract_zcode_stream(&stdout);
            let reply = stream
                .text
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_default();
            let runtime_thread_id = stream
                .session_id
                .or_else(|| resume_session_id.map(str::to_string));
            if status == PromptSessionStatus::Succeeded && stream.error.is_some() {
                status = PromptSessionStatus::Failed;
            }
            let failed = status != PromptSessionStatus::Succeeded
                && status != PromptSessionStatus::Cancelled
                && status != PromptSessionStatus::TimedOut;
            let error = if failed && reply.trim().is_empty() {
                let raw = stream
                    .error
                    .unwrap_or_else(|| combine_output(&stdout, &stderr));
                let (message, plain) = if let Some(text) = plain_zcode_error(&raw) {
                    (text, true)
                } else if raw.trim().is_empty() {
                    (ZCODE_NOT_READY.to_string(), true)
                } else {
                    (raw, false)
                };
                Some((message, plain))
            } else {
                None
            };
            let combined = match &error {
                Some((text, _)) => text.clone(),
                None if !reply.trim().is_empty() => reply.clone(),
                None if failed => combine_output(&stdout, &stderr),
                None => String::new(),
            };
            let display = display_text
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default();
            if error.is_none() && display.trim().is_empty() && !reply.trim().is_empty() {
                emit(PromptSessionEvent::Delta {
                    session_id: session_id.clone(),
                    text: reply.clone(),
                });
            }
            if let Some((message, plain)) = error {
                if plain {
                    emit(PromptSessionEvent::TurnError {
                        session_id: session_id.clone(),
                        message,
                        plain: true,
                    });
                } else {
                    emit(turn_error(&session_id, &message));
                }
            }
            let summary = summarize(&combined, &status, &runtime);
            emit(PromptSessionEvent::Completed {
                session_id: session_id.clone(),
                status: status.clone(),
                exit_code,
                summary: summary.clone(),
                timeout,
            });
            PromptSessionReport {
                usage: None,
                session_id,
                runtime,
                cwd: cwd.display().to_string(),
                status,
                exit_code,
                summary,
                log_excerpt: combined,
                duration_ms,
                runtime_thread_id,
            }
        }
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            let summary = format!("{err:#}");
            emit(turn_error(&session_id, &summary));
            emit(PromptSessionEvent::Completed {
                session_id: session_id.clone(),
                status: PromptSessionStatus::Failed,
                exit_code: None,
                summary: summary.clone(),
                timeout: None,
            });
            PromptSessionReport {
                usage: None,
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

fn is_zcode_refresh_note(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("ZCode Built-in ") && !trimmed.contains("刷新失败")
}

fn bundled_provider_config(script: &Path) -> Option<std::path::PathBuf> {
    let dir = script.parent()?;
    [
        dir.join("provider").join("zcode-builtin.json"),
        dir.join("../config/provider/zcode-builtin.json"),
        dir.join("../../../../../config/provider/zcode-builtin.json"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

/// ZCode session ids are `sess_…`. A bare UUID belongs to Claude.
fn zcode_resume_id(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim)
        .filter(|value| value.starts_with("sess_") && value.len() > "sess_".len())
}

fn build_zcode_command(
    prompt: &str,
    cwd: &Path,
    resume_session_id: Option<&str>,
) -> Result<Command> {
    let launch = if let Some(bin) = std::env::var_os("AGENT_DOCTOR_ZCODE_BIN") {
        agent_doctor_core::adapters::ZcodeHeadless {
            program: std::path::PathBuf::from(bin),
            script: std::env::var_os("AGENT_DOCTOR_ZCODE_SCRIPT").map(std::path::PathBuf::from),
        }
    } else {
        locate_zcode_headless().ok_or_else(|| anyhow::Error::msg(ZCODE_MISSING))?
    };
    let mut cmd = agent_doctor_core::exec::command_for_path(&launch.program);
    cmd.env("NO_COLOR", "1").env("FORCE_COLOR", "0");
    if let Some(script) = &launch.script {
        cmd.env("ELECTRON_RUN_AS_NODE", "1");
        cmd.arg(script);
        // The bundled script looks next to itself, then five folders up. A desktop
        // install keeps this file at Contents/Resources/config/provider instead.
        if let Some(builtin) = bundled_provider_config(script) {
            cmd.env("ZCODE_BUILTIN_PROVIDER_CONFIG_FILE", builtin);
        }
    }
    cmd.arg("--prompt")
        .arg(prompt)
        .arg("--cwd")
        .arg(cwd)
        .arg("--output-format")
        .arg("stream-json")
        .arg("--no-color")
        .arg("--surface")
        .arg("terminal")
        .arg("--mode")
        .arg("yolo")
        .arg("--disallowed-tools")
        .arg("AskUserQuestion EnterPlanMode ExitPlanMode")
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    if let Some(session) = resume_session_id {
        cmd.arg("--resume").arg(session);
    }
    Ok(cmd)
}

fn pump_lines<F>(
    session_id: &str,
    child: &mut Child,
    timeout_sec: u64,
    cancel: Arc<AtomicBool>,
    on_event: &mut F,
) -> Result<PumpResult>
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
    let stdout_eof = Arc::new(AtomicBool::new(false));
    let stdout_eof_flag = Arc::clone(&stdout_eof);
    let stdout_handle = thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            push_capped(&acc_out, &line);
            if let Ok(mut guard) = q_out.lock() {
                guard.push((true, line));
            }
        }
        stdout_eof_flag.store(true, Ordering::SeqCst);
    });

    let q_err = Arc::clone(&queue);
    let acc_err = Arc::clone(&stderr_acc);
    let stderr_eof = Arc::new(AtomicBool::new(false));
    let stderr_eof_flag = Arc::clone(&stderr_eof);
    let stderr_handle = thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            push_capped(&acc_err, &line);
            if let Ok(mut guard) = q_err.lock() {
                guard.push((false, line));
            }
        }
        stderr_eof_flag.store(true, Ordering::SeqCst);
    });

    let mut plan_board = PlanBoard::default();
    let mut tools = ToolWatch::default();
    let mut turn = ZcodeTurn::default();
    let mut drain = |on_event: &mut F, tools: &mut ToolWatch| -> bool {
        let drained = {
            let mut guard = queue.lock().unwrap_or_else(|err| err.into_inner());
            guard.drain(..).collect::<Vec<_>>()
        };
        let saw = !drained.is_empty();
        for (is_stdout, line) in drained {
            if is_stdout {
                handle_zcode_stream_line(
                    session_id,
                    &line,
                    &mut turn,
                    &mut plan_board,
                    tools,
                    on_event,
                );
                continue;
            }
            if is_runtime_stderr_noise(&line)
                || is_runtime_log_line(&line)
                || is_zcode_refresh_note(&line)
            {
                continue;
            }
            on_event(stderr_event(session_id, &line));
        }
        saw
    };

    let mut clock = SessionClock::new(timeout_sec);
    let mut pipes_closed_at = None;
    let mut timeout_note = None;
    let (status, exit_code) = loop {
        if drain(on_event, &mut tools) {
            clock.touch();
        }
        clock.set_tool_running(tools.running());
        if cancel.load(Ordering::SeqCst) {
            force_stop_child(child, pid);
            break (PromptSessionStatus::Cancelled, None);
        }
        if clock.expired() || clock.stalled() {
            timeout_note = Some(clock.timeout_note(tools.last_label()));
            force_stop_child(child, pid);
            break (PromptSessionStatus::TimedOut, None);
        }
        if let Some(done) = finish_oneshot_after_pipes_closed(
            child,
            pid,
            &stdout_eof,
            &stderr_eof,
            &mut pipes_closed_at,
        ) {
            break done;
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
            Err(error) => return Err(error).context("ZCode 问答中断了"),
        }
    };
    join_reader(stdout_handle, Duration::from_millis(500));
    join_reader(stderr_handle, Duration::from_millis(500));
    drain(on_event, &mut tools);
    let stdout = stdout_acc
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let stderr = stderr_acc
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    Ok((status, exit_code, stdout, stderr, timeout_note))
}

#[derive(Default)]
struct ZcodeTurn {
    text: String,
    session_id: Option<String>,
    error: Option<String>,
}

fn handle_zcode_stream_line<F>(
    session_id: &str,
    line: &str,
    turn: &mut ZcodeTurn,
    plan_board: &mut PlanBoard,
    tools: &mut ToolWatch,
    on_event: &mut F,
) where
    F: FnMut(PromptSessionEvent),
{
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return;
    };
    if let Some(found) = text_field(&value, &["sessionId", "session_id"]) {
        turn.session_id = Some(found);
    }
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    let payload = value.get("payload").cloned().unwrap_or(Value::Null);
    match kind {
        "session.created" | "session.resumed" | "turn.started" => {
            on_event(PromptSessionEvent::Status {
                session_id: session_id.to_string(),
                phase: "thinking".into(),
                message: "正在准备…".into(),
            });
        }
        "model.streaming" => emit_stream_delta(session_id, &payload, turn, on_event),
        "tool.updated" => emit_tool(session_id, &payload, turn, plan_board, tools, on_event),
        "turn.failed" => {
            if let Some(message) = payload_message(&payload) {
                turn.error = Some(message);
            }
            tools.clear();
        }
        "turn.completed" => tools.clear(),
        "permission.requested" => {
            on_event(PromptSessionEvent::Status {
                session_id: session_id.to_string(),
                phase: "tool".into(),
                message: "ZCode 在等确认。打开 ZCode 窗口点一下。".into(),
            });
        }
        _ => {}
    }
}

fn emit_stream_delta<F>(session_id: &str, payload: &Value, turn: &mut ZcodeTurn, on_event: &mut F)
where
    F: FnMut(PromptSessionEvent),
{
    let stream_kind = payload.get("kind").and_then(Value::as_str).unwrap_or("");
    let delta = text_field(payload, &["delta", "text"]).unwrap_or_default();
    if delta.is_empty() {
        return;
    }
    match stream_kind {
        "reasoning_delta" => on_event(PromptSessionEvent::Thinking {
            session_id: session_id.to_string(),
            text: delta,
        }),
        "text_delta" | "" => {
            turn.text.push_str(&delta);
            on_event(PromptSessionEvent::Delta {
                session_id: session_id.to_string(),
                text: delta,
            });
        }
        _ => {}
    }
}

fn emit_tool<F>(
    session_id: &str,
    payload: &Value,
    _turn: &mut ZcodeTurn,
    plan_board: &mut PlanBoard,
    tools: &mut ToolWatch,
    on_event: &mut F,
) where
    F: FnMut(PromptSessionEvent),
{
    let update = payload.get("kind").and_then(Value::as_str).unwrap_or("");
    let name = text_field(payload, &["toolName", "name", "tool", "tool_name"])
        .unwrap_or_else(|| "tool".into());
    let tool_id =
        text_field(payload, &["toolCallId", "callId", "id", "toolUseId"]).unwrap_or_default();
    let input = payload
        .get("input")
        .or_else(|| payload.get("arguments"))
        .or_else(|| payload.get("args"))
        .cloned()
        .unwrap_or(Value::Null);
    match update {
        "scheduled" | "started" => {
            tools.start(&tool_id);
            if let Some(items) = plan_board.observe_tool(&name, &tool_id, &input) {
                on_event(PromptSessionEvent::Plan {
                    session_id: session_id.to_string(),
                    items,
                });
            } else if !tool_carries_plan(&name) {
                let detail = tool_activity_detail(&name, &input);
                let label = zcode_tool_label(&name);
                tools.remember(&tool_chip(label, &detail));
                on_event(PromptSessionEvent::Status {
                    session_id: session_id.to_string(),
                    phase: "tool".into(),
                    message: format_tool_status(label, &detail),
                });
                if !tool_id.is_empty() {
                    on_event(PromptSessionEvent::Tool {
                        session_id: session_id.to_string(),
                        step: Box::new(tool_step(&tool_id, &name, &input)),
                    });
                }
            }
        }
        "result" | "error" => {
            tools.finish(tool_id.as_str());
            if tool_id.is_empty() {
                return;
            }
            let output = ["output", "result", "content", "message", "error"]
                .iter()
                .find_map(|key| payload.get(*key))
                .map(tool_result_text)
                .unwrap_or_default();
            on_event(PromptSessionEvent::Tool {
                session_id: session_id.to_string(),
                step: Box::new(tool_step_result(&tool_id, &output, update == "error")),
            });
        }
        _ => {}
    }
}

fn zcode_tool_label(name: &str) -> &str {
    match name {
        "bash" | "shell" | "terminal" => "终端",
        "read_file" | "read" => "读文件",
        "write_file" | "write" | "edit_file" | "edit" | "str_replace" => "改文件",
        "web_search" | "search" => "搜索",
        other => other,
    }
}

struct ZcodeStreamFinal {
    text: Option<String>,
    session_id: Option<String>,
    error: Option<String>,
}

fn extract_zcode_stream(stdout: &str) -> ZcodeStreamFinal {
    let mut turn = ZcodeTurn::default();
    let mut plan = PlanBoard::default();
    let mut tools = ToolWatch::default();
    for line in stdout.lines() {
        handle_zcode_stream_line(line, line, &mut turn, &mut plan, &mut tools, &mut |_| {});
    }
    ZcodeStreamFinal {
        text: if turn.text.trim().is_empty() {
            None
        } else {
            Some(turn.text)
        },
        session_id: turn.session_id,
        error: turn.error,
    }
}

fn text_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn payload_message(payload: &Value) -> Option<String> {
    if let Some(text) = text_field(payload, &["message", "reason", "text"]) {
        return Some(text);
    }
    match payload.get("error") {
        Some(Value::String(text)) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Some(value) => text_field(value, &["message", "reason"]),
        None => None,
    }
}

fn plain_zcode_error(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    if raw.contains("未登录")
        || raw.contains("请登录")
        || raw.contains("请先登录")
        || lower.contains("not logged")
        || lower.contains("please log in")
        || lower.contains("authentication required")
        || lower.contains("unauthorized")
    {
        Some(ZCODE_NOT_READY.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events_for(line: &str) -> (Vec<PromptSessionEvent>, ZcodeTurn) {
        let mut events = Vec::new();
        let mut turn = ZcodeTurn::default();
        let mut plan = PlanBoard::default();
        let mut tools = ToolWatch::default();
        handle_zcode_stream_line(
            "ps-test",
            line,
            &mut turn,
            &mut plan,
            &mut tools,
            &mut |event| {
                events.push(event);
            },
        );
        (events, turn)
    }

    #[test]
    fn resume_id_keeps_zcode_sessions_only() {
        assert_eq!(
            zcode_resume_id(Some("sess_1fe74b61-77d7-466f-a6dd-f9246c751058")),
            Some("sess_1fe74b61-77d7-466f-a6dd-f9246c751058")
        );
        assert_eq!(
            zcode_resume_id(Some("1fe74b61-77d7-466f-a6dd-f9246c751058")),
            None
        );
        assert_eq!(zcode_resume_id(Some("  sess_ ")), None);
    }

    #[test]
    fn text_delta_becomes_a_reply_piece() {
        let (events, turn) = events_for(
            r#"{"type":"model.streaming","sessionId":"sess_1","payload":{"kind":"text_delta","delta":"你好"}}"#,
        );
        assert!(matches!(
            &events[0],
            PromptSessionEvent::Delta { text, .. } if text == "你好"
        ));
        assert_eq!(turn.session_id.as_deref(), Some("sess_1"));
        assert_eq!(turn.text, "你好");
    }

    #[test]
    fn tool_start_and_failure_are_visible() {
        let (events, _) = events_for(
            r#"{"type":"tool.updated","payload":{"kind":"started","toolCallId":"t1","toolName":"read_file","input":{"path":"a.ts"}}}"#,
        );
        assert!(events.iter().any(|event| matches!(
            event,
            PromptSessionEvent::Tool { step, .. } if step.id == "t1" && step.status == "running"
        )));
        let (events, turn) = events_for(
            r#"{"type":"turn.failed","sessionId":"sess_9","payload":{"message":"未登录"}}"#,
        );
        assert!(events.is_empty());
        assert_eq!(turn.error.as_deref(), Some("未登录"));
        assert_eq!(
            plain_zcode_error(turn.error.as_deref().unwrap_or("")).as_deref(),
            Some(ZCODE_NOT_READY)
        );
    }

    #[test]
    fn desktop_install_provider_config_sits_beside_resources() {
        let temp = tempfile::tempdir().expect("tempdir");
        let script = temp.path().join("Contents/Resources/glm/zcode.cjs");
        let builtin = temp
            .path()
            .join("Contents/Resources/config/provider/zcode-builtin.json");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::create_dir_all(builtin.parent().unwrap()).unwrap();
        std::fs::write(&script, b"").unwrap();
        std::fs::write(&builtin, b"{}").unwrap();
        let found = bundled_provider_config(&script).expect("builtin");
        assert_eq!(
            found.canonicalize().unwrap(),
            builtin.canonicalize().unwrap()
        );
    }
}
