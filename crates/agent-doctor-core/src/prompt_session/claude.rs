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
use super::plan::{tool_carries_plan, PlanBoard};
use super::util::{
    combine_output, command_from_cli, force_stop_child, format_tool_status,
    is_runtime_stderr_noise, summarize, tool_chip, tool_input_detail, SessionClock, ThinkingLine,
    ToolWatch, THINKING_AFTER_TOOL,
};
use super::warm::{self, LivePipes};
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

    // A process that reads messages on stdin can take the next one too.
    let stream_input = interactive || warm::enabled();
    let launch = ClaudeLaunch {
        cwd: &cwd,
        skip_permissions: options.dangerously_skip_permissions,
        interactive,
        stream_input,
        overlay: &overlay,
    };
    let mut cmd = build_claude_command(&effective_prompt, &launch, resume_session_id)?;
    let command_display = format_command_display(&cmd);
    let fingerprint = if warm::enabled() {
        build_claude_command("", &launch, None)
            .ok()
            .map(|base| warm::fingerprint(&base, &claude_config_files(&cwd)))
    } else {
        None
    };

    on_event(PromptSessionEvent::Started {
        session_id: session_id.clone(),
        runtime: runtime.clone(),
        cwd: cwd.display().to_string(),
        command: command_display,
    });

    let started = Instant::now();
    let writer = control.clone().unwrap_or_default();
    let user_msg = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{ "type": "text", "text": effective_prompt }]
        }
    })
    .to_string();

    let mut reused = None;
    if let (Some(fp), Some(sid)) = (fingerprint, resume_session_id) {
        if let Some(parked) = warm::take("claude-code", sid, fp) {
            let warm::WarmProcess {
                child,
                stdin,
                pipes,
                ..
            } = parked;
            writer.close();
            writer.attach_stdin(stdin);
            if writer.write_line(&user_msg).is_ok() {
                reused = Some((child, pipes));
            } else {
                writer.close();
                warm::retire_without_stdin(child, pipes);
            }
        }
    }
    let (mut child, mut pipes) = match reused {
        Some(live) => live,
        None => {
            let mut child = cmd.spawn().context("failed to spawn claude-code")?;
            let pipes = match LivePipes::attach(&mut child) {
                Ok(pipes) => pipes,
                Err(err) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(err);
                }
            };
            if stream_input {
                if let Some(stdin) = child.stdin.take() {
                    writer.close();
                    writer.attach_stdin(stdin);
                    if let Err(err) = writer.write_line(&user_msg) {
                        writer.close();
                        warm::retire_without_stdin(child, pipes);
                        return Err(err).context("failed to send Claude ask prompt on stdin");
                    }
                }
            }
            (child, pipes)
        }
    };

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
        &mut pipes,
        timeout_sec,
        cancel.handle(),
        stream_input.then_some(&writer),
        fingerprint.is_some(),
        &mut emit,
    );

    let park_as = match (&result, fingerprint) {
        (Ok((PromptSessionStatus::Succeeded, _, stdout, _, _)), Some(fp)) => {
            extract_claude_session_id(stdout)
                .or_else(|| resume_session_id.map(str::to_string))
                .map(|sid| (sid, fp))
        }
        _ => None,
    };
    warm::keep_or_close("claude-code", park_as, child, pipes, &writer);

    let duration_ms = started.elapsed().as_millis() as u64;
    let report = match result {
        Ok((status, exit_code, stdout, stderr, timeout)) => {
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
                display.clone()
            };
            let readable = if display.trim().is_empty() {
                combine_output(&without_stream_json(&stdout), &stderr)
            } else {
                display
            };
            let summary = summarize(&readable, &status, &runtime);
            let runtime_thread_id = extract_claude_session_id(&stdout)
                .or_else(|| resume_session_id.map(str::to_string));
            emit(PromptSessionEvent::Completed {
                session_id: session_id.clone(),
                status: status.clone(),
                exit_code,
                summary: summary.clone(),
                timeout,
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
                timeout: None,
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

struct ClaudeLaunch<'a> {
    cwd: &'a Path,
    skip_permissions: bool,
    interactive: bool,
    /// Messages go in on stdin as stream-json instead of as an argument.
    stream_input: bool,
    overlay: &'a std::collections::HashMap<String, String>,
}

fn claude_config_files(cwd: &Path) -> Vec<std::path::PathBuf> {
    let home = crate::adapters::util::home_join(".claude");
    vec![
        home.join("settings.json"),
        cwd.join(".mcp.json"),
        cwd.join(".claude").join("settings.json"),
        cwd.join(".claude").join("settings.local.json"),
    ]
}

fn build_claude_command(
    prompt: &str,
    launch: &ClaudeLaunch<'_>,
    resume_session_id: Option<&str>,
) -> Result<Command> {
    let ClaudeLaunch {
        cwd,
        skip_permissions,
        interactive: interactive_permissions,
        stream_input,
        overlay,
    } = *launch;
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
    // `--mcp-config` is variadic. A separate path argument makes Claude treat
    // every later positional (the user prompt) as another config file, then
    // `open()` that text. `ENAMETOOLONG` is that failure when the prompt is long.
    // `=` keeps the path attached to the flag.
    let mcp_config = cwd.join(".mcp.json");
    if mcp_config.is_file() {
        cmd.arg(format!("--mcp-config={}", mcp_config.display()));
    }
    if let Some(sid) = resume_session_id {
        cmd.arg("--resume").arg(sid);
    }
    if skip_permissions {
        cmd.arg("--dangerously-skip-permissions");
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
                    "AskUserQuestion",
                    "mcp__browser__*"
                ]
            }
        });
        cmd.arg("--permission-prompt-tool")
            .arg("stdio")
            .arg("--settings")
            .arg(ask_settings.to_string())
            .arg("--append-system-prompt")
            .arg(CLAUDE_ASK_INSTRUCTIONS);
    }
    if stream_input || interactive_permissions {
        cmd.arg("--input-format").arg("stream-json");
        cmd.stdin(Stdio::piped());
    } else {
        cmd.arg("--").arg(prompt).stdin(Stdio::null());
    }
    apply_overlay_env(&mut cmd, overlay);
    apply_claude_env(&mut cmd, overlay);
    Ok(cmd)
}

/// Read one turn. With `keep_alive` Claude stays up after `result` so it can be
/// parked; otherwise stdin closes and the process is reaped.
#[allow(clippy::too_many_arguments)]
fn pump_claude<F>(
    session_id: &str,
    child: &mut Child,
    pipes: &mut LivePipes,
    timeout_sec: u64,
    cancel: Arc<AtomicBool>,
    control: Option<&PromptSessionControl>,
    keep_alive: bool,
    on_event: &mut F,
) -> Result<super::PumpResult>
where
    F: FnMut(PromptSessionEvent),
{
    let pid = child.id();
    let turn_done = Arc::new(AtomicBool::new(false));
    let mut plan_board = PlanBoard::default();
    let mut tools = ToolWatch::default();
    let mut thinking = ThinkingLine::default();
    let mut drain = |on_event: &mut F, tools: &mut ToolWatch, pipes: &LivePipes| -> bool {
        let drained = pipes.drain();
        let saw = !drained.is_empty();
        for (is_stdout, line) in drained {
            if is_stdout {
                let value = serde_json::from_str::<serde_json::Value>(&line).ok();
                if let Some(text) = value.as_ref().and_then(claude_thinking_delta) {
                    on_event(PromptSessionEvent::Thinking {
                        session_id: session_id.to_string(),
                        text: text.to_string(),
                    });
                }
                let status = value
                    .as_ref()
                    .and_then(|value| claude_thinking_status(value, tools, &mut thinking));
                if let Some(message) = status {
                    on_event(PromptSessionEvent::Status {
                        session_id: session_id.to_string(),
                        phase: "thinking".into(),
                        message,
                    });
                }
                for event in parse_claude_stream_line(
                    session_id,
                    &line,
                    control,
                    Some(turn_done.as_ref()),
                    &mut plan_board,
                ) {
                    on_event(event);
                }
            } else if !is_runtime_stderr_noise(&line) {
                on_event(PromptSessionEvent::StderrLine {
                    session_id: session_id.to_string(),
                    line,
                });
            }
        }
        saw
    };

    let mut clock = SessionClock::new(timeout_sec);
    let mut closed_after_result = false;
    let mut timeout_note = None;
    let mut still_running = false;
    let (status, exit_code) = loop {
        let saw_output = drain(on_event, &mut tools, pipes);
        if saw_output || control.is_some_and(PromptSessionControl::has_pending) {
            clock.touch();
        }
        clock.set_tool_running(tools.running());
        if cancel.load(Ordering::SeqCst) {
            force_stop_child(child, pid);
            break (PromptSessionStatus::Cancelled, None);
        }
        if clock.expired() {
            timeout_note = Some(clock.timeout_note(tools.last_label()));
            force_stop_child(child, pid);
            break (PromptSessionStatus::TimedOut, None);
        }
        if turn_done.load(Ordering::SeqCst) && keep_alive && matches!(child.try_wait(), Ok(None)) {
            still_running = true;
            break (PromptSessionStatus::Succeeded, None);
        }
        // stream-json keeps Claude alive for another stdin turn after `result`.
        // Without keep_alive, close stdin so the process can exit and the UI unblocks.
        if turn_done.load(Ordering::SeqCst) && !closed_after_result {
            closed_after_result = true;
            if let Some(control) = control {
                control.close();
            }
        }
        match child.try_wait() {
            Ok(Some(wait_status)) => {
                let code = wait_status.code();
                let status = if wait_status.success() || turn_done.load(Ordering::SeqCst) {
                    PromptSessionStatus::Succeeded
                } else {
                    PromptSessionStatus::Failed
                };
                break (status, code);
            }
            Ok(None) => {
                if closed_after_result {
                    // Give Claude a moment to exit after stdin EOF, then stop it.
                    thread::sleep(Duration::from_millis(200));
                    if child.try_wait().ok().flatten().is_none() {
                        force_stop_child(child, pid);
                        break (PromptSessionStatus::Succeeded, None);
                    }
                    continue;
                }
                thread::sleep(Duration::from_millis(40));
            }
            Err(error) => return Err(error).context("failed waiting for prompt session"),
        }
    };

    if !still_running {
        pipes.join(Duration::from_millis(500));
    }
    drain(on_event, &mut tools, pipes);

    let (stdout, stderr) = pipes.take_output();
    Ok((status, exit_code, stdout, stderr, timeout_note))
}

fn claude_thinking_delta(value: &serde_json::Value) -> Option<&str> {
    if value.get("type").and_then(|v| v.as_str()) != Some("stream_event") {
        return None;
    }
    value
        .pointer("/event/delta/thinking")
        .and_then(|v| v.as_str())
        .filter(|text| !text.is_empty())
}

/// Live row text while Claude thinks, or once every open tool has returned.
fn claude_thinking_status(
    value: &serde_json::Value,
    tools: &mut ToolWatch,
    thinking: &mut ThinkingLine,
) -> Option<String> {
    if value.get("type").and_then(|v| v.as_str()) == Some("stream_event") {
        let event = value.get("event")?;
        match event.get("type").and_then(|v| v.as_str()) {
            Some("content_block_start")
                if event
                    .pointer("/content_block/type")
                    .and_then(|v| v.as_str())
                    == Some("thinking") =>
            {
                thinking.reset();
                None
            }
            Some("content_block_delta") => thinking.push(claude_thinking_delta(value)?),
            _ => None,
        }
    } else {
        let was_running = tools.running();
        observe_claude_tools(value, tools);
        (was_running && !tools.running()).then(|| THINKING_AFTER_TOOL.to_string())
    }
}

/// `tool_use` arrives on an assistant message; its `tool_result` comes back on
/// the next user message with the same id.
fn observe_claude_tools(value: &serde_json::Value, tools: &mut ToolWatch) {
    let (wanted, id_key) = match value.get("type").and_then(|v| v.as_str()) {
        Some("assistant") => ("tool_use", "id"),
        Some("user") => ("tool_result", "tool_use_id"),
        Some("result") => {
            tools.clear();
            return;
        }
        _ => return,
    };
    let Some(blocks) = value.pointer("/message/content").and_then(|v| v.as_array()) else {
        return;
    };
    for block in blocks {
        if block.get("type").and_then(|v| v.as_str()) != Some(wanted) {
            continue;
        }
        let id = block.get(id_key).and_then(|v| v.as_str()).unwrap_or("");
        if wanted == "tool_use" {
            let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
            let detail = block
                .get("input")
                .map(tool_input_detail)
                .unwrap_or_default();
            tools.start(id);
            tools.remember(&tool_chip(name, &detail));
        } else {
            tools.finish(id);
        }
    }
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
        .filter(|s| !s.is_empty())
        .map(str::to_string);

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
    let command = if lower.contains("bash") || lower == "shell" {
        pick_str(&["command", "cmd"])
    } else {
        None
    }
    .or_else(|| pick_str(&["command", "cmd"]));

    // Keep description + command together so the Ask UI can show one module.
    if description.is_some() || command.is_some() {
        let mut map = serde_json::Map::new();
        if let Some(desc) = description {
            map.insert("description".into(), serde_json::Value::String(desc));
        }
        if let Some(cmd) = command {
            map.insert("command".into(), serde_json::Value::String(cmd));
        } else if let Some(path) = pick_str(&["file_path", "path", "filePath", "url", "query"]) {
            map.insert("command".into(), serde_json::Value::String(path));
        }
        if !map.is_empty() {
            return serde_json::Value::Object(map).to_string();
        }
    }

    if let Some(path) = pick_str(&["file_path", "path", "filePath", "url", "query"]) {
        return path;
    }

    let compact = input.to_string();
    if compact.len() <= 280 {
        compact
    } else {
        format!("{}…", &compact[..277])
    }
}

fn is_ask_user_question(name: &str) -> bool {
    name.eq_ignore_ascii_case("AskUserQuestion")
}

fn claude_question_detail(input: &serde_json::Value) -> String {
    input
        .pointer("/questions/0/question")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("需要你回答")
        .to_string()
}

/// Open questions (`kind: text|number`) and choice questions with fewer than
/// two options never reach the user: Claude rejects them before the prompt.
/// Tell the model to use an open question for a secret or free-text reply.
const CLAUDE_ASK_INSTRUCTIONS: &str = "When you need a secret, token, password, or any value only the user knows, call AskUserQuestion with one question, \"kind\": \"text\", and no options. A choice question with fewer than 2 options is rejected before the user can answer. Do not add a filler option. Do not ask the user to paste the secret into the chat, export an environment variable, or write it to a file. Wait for the tool result.";

fn question_is_open(question: &serde_json::Value) -> bool {
    question
        .get("kind")
        .and_then(|v| v.as_str())
        .is_some_and(|kind| {
            kind.eq_ignore_ascii_case("text") || kind.eq_ignore_ascii_case("number")
        })
}

fn question_has_choice(question: &serde_json::Value) -> bool {
    if question_is_open(question) {
        return false;
    }
    question
        .get("options")
        .and_then(|v| v.as_array())
        .is_some_and(|options| options.len() >= 2)
}

fn claude_question_mode(input: &serde_json::Value) -> String {
    let has_choice = input
        .get("questions")
        .and_then(|v| v.as_array())
        .is_some_and(|questions| questions.iter().any(question_has_choice));
    if has_choice {
        return "options".to_string();
    }
    let blob = input.to_string().to_ascii_lowercase();
    if [
        "password",
        "passphrase",
        "token",
        "api key",
        "apikey",
        "secret",
        "密钥",
        "口令",
    ]
    .iter()
    .any(|needle| blob.contains(needle))
    {
        return "secret".to_string();
    }
    "line".to_string()
}

fn parse_claude_stream_line(
    session_id: &str,
    line: &str,
    control: Option<&PromptSessionControl>,
    turn_done: Option<&AtomicBool>,
    plan_board: &mut PlanBoard,
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
                let asking = is_ask_user_question(&tool_name);
                let detail = if asking {
                    claude_question_detail(&input)
                } else {
                    permission_detail(&tool_name, &request, &input)
                };
                let input_mode = if asking {
                    claude_question_mode(&input)
                } else {
                    "choice".to_string()
                };
                if let Some(control) = control {
                    if asking {
                        control.remember_claude_question(&request_id, input.clone());
                    } else {
                        control.remember_claude_tool_input(&request_id, input.clone());
                    }
                }
                let status = if asking {
                    "需要你回答".to_string()
                } else {
                    format!("等待确认：{tool_name}")
                };
                vec![
                    PromptSessionEvent::Status {
                        session_id: session_id.to_string(),
                        phase: "permission".into(),
                        message: status,
                    },
                    PromptSessionEvent::PermissionRequest {
                        session_id: session_id.to_string(),
                        request_id,
                        tool_name,
                        detail,
                        input_json: input.to_string(),
                        input_mode,
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
                        // With `--include-partial-messages`, text already arrived via
                        // `stream_event` deltas. Re-emitting the full assistant text
                        // here doubles the bubble and looks like the reply is stuck
                        // mid-stream.
                        if block
                            .get("text")
                            .and_then(|v| v.as_str())
                            .is_some_and(|text| !text.is_empty())
                        {
                            out.push(PromptSessionEvent::Status {
                                session_id: session_id.to_string(),
                                phase: "writing".into(),
                                message: "正在生成回复…".into(),
                            });
                        }
                    }
                    "tool_use" => {
                        let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                        let tool_use_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let input = block
                            .get("input")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        if let Some(items) = plan_board.observe_tool(name, tool_use_id, &input) {
                            out.push(PromptSessionEvent::Plan {
                                session_id: session_id.to_string(),
                                items,
                            });
                        } else if !tool_carries_plan(name) {
                            let detail = tool_input_detail(&input);
                            out.push(PromptSessionEvent::Status {
                                session_id: session_id.to_string(),
                                phase: "tool".into(),
                                message: format_tool_status(name, &detail),
                            });
                        }
                    }
                    _ => {}
                }
            }
            out
        }
        "stream_event" => {
            let mut out = Vec::new();
            // Only stream assistant *text*. `partial_json` is tool-input streaming and
            // must not land in the chat bubble (it duplicated Bash JSON next to the
            // permission card).
            let delta = value
                .pointer("/event/delta/text")
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
                    // The full checklist arrives on the assistant message. Starting
                    // the block here would record a plan tool twice.
                    if !tool_carries_plan(name) {
                        let detail = value
                            .pointer("/event/content_block/input")
                            .map(tool_input_detail)
                            .unwrap_or_default();
                        out.push(PromptSessionEvent::Status {
                            session_id: session_id.to_string(),
                            phase: "tool".into(),
                            message: format_tool_status(name, &detail),
                        });
                    }
                }
            }
            out
        }
        "user" => plan_board.observe_user(&value).map_or(Vec::new(), |items| {
            vec![PromptSessionEvent::Plan {
                session_id: session_id.to_string(),
                items,
            }]
        }),
        // Final envelope — surface errors immediately; successful `result` text is
        // applied after pump when live deltas were missing (avoids duplicating
        // assistant text that already streamed).
        "result" => {
            if let Some(flag) = turn_done {
                flag.store(true, Ordering::SeqCst);
            }
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
                    phase: "done".into(),
                    message: "回复已完成".into(),
                }]
            }
        }
        _ => Vec::new(),
    }
}

/// Stream-json event lines are protocol, not something to show the user.
fn without_stream_json(stdout: &str) -> String {
    stdout
        .lines()
        .filter(|line| {
            // The capped capture can cut an event line in half, so it no longer parses.
            if line.trim_start().starts_with("{\"type\"") {
                return false;
            }
            serde_json::from_str::<serde_json::Value>(line)
                .map(|value| !value.is_object())
                .unwrap_or(true)
        })
        .collect::<Vec<_>>()
        .join("\n")
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

    #[test]
    fn readable_output_drops_stream_json_lines() {
        let stdout = "{\"type\":\"system\",\"subtype\":\"init\",\"cwd\":\"/tmp\"}\n\
                      {\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"te\n\
                      plain note";
        assert_eq!(without_stream_json(stdout), "plain note");
    }

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
    fn assistant_text_does_not_reemit_streamed_deltas() {
        let events = parse_claude_stream_line(
            "s1",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello once"}]}}"#,
            None,
            None,
            &mut PlanBoard::default(),
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, PromptSessionEvent::Delta { .. })),
            "full assistant text must not duplicate stream_event deltas: {events:?}"
        );
        assert!(events.iter().any(|e| matches!(
            e,
            PromptSessionEvent::Status { phase, .. } if phase == "writing"
        )));
    }

    #[test]
    fn a_tool_runs_until_its_result_comes_back() {
        let mut tools = ToolWatch::default();
        let mut thinking = ThinkingLine::default();
        let line = |raw: &str| serde_json::from_str::<serde_json::Value>(raw).unwrap();
        let started = claude_thinking_status(
            &line(
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"npm test"}}]}}"#,
            ),
            &mut tools,
            &mut thinking,
        );
        assert_eq!(started, None);
        assert!(tools.running());
        assert_eq!(tools.last_label(), "Bash: npm test");
        let returned = claude_thinking_status(
            &line(
                r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#,
            ),
            &mut tools,
            &mut thinking,
        );
        assert!(!tools.running());
        assert_eq!(returned.as_deref(), Some(THINKING_AFTER_TOOL));
    }

    #[test]
    fn thinking_deltas_become_a_live_line() {
        let mut tools = ToolWatch::default();
        let mut thinking = ThinkingLine::default();
        let line = |raw: &str| serde_json::from_str::<serde_json::Value>(raw).unwrap();
        claude_thinking_status(
            &line(
                r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"thinking"}}}"#,
            ),
            &mut tools,
            &mut thinking,
        );
        let shown = claude_thinking_status(
            &line(
                r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"看一下笔记目录\n然后"}}}"#,
            ),
            &mut tools,
            &mut thinking,
        );
        assert_eq!(shown.as_deref(), Some("正在思考：看一下笔记目录"));
        assert_eq!(
            claude_thinking_delta(&line(
                r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"然后"}}}"#
            )),
            Some("然后")
        );
        assert_eq!(
            claude_thinking_delta(&line(
                r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}}"#
            )),
            None
        );
    }

    #[test]
    fn result_marks_turn_done() {
        let flag = AtomicBool::new(false);
        let _ = parse_claude_stream_line(
            "s1",
            r#"{"type":"result","is_error":false,"result":"ok"}"#,
            None,
            Some(&flag),
            &mut PlanBoard::default(),
        );
        assert!(flag.load(Ordering::SeqCst));
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
            None,
            &mut PlanBoard::default(),
        );
        assert!(events.iter().any(|e| matches!(
            e,
            PromptSessionEvent::StderrLine { line, .. } if line == "boom"
        )));
    }

    #[test]
    fn todo_write_emits_a_plan_instead_of_a_tool_chip() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TodoWrite","input":{"todos":[{"content":"改按钮","status":"in_progress"},{"content":"再试一次","status":"pending"}]}}]}}"#;
        let events = parse_claude_stream_line("s1", line, None, None, &mut PlanBoard::default());
        assert!(events.iter().any(|event| matches!(
            event,
            PromptSessionEvent::Plan { items, .. }
                if items.len() == 2
                    && items[0].text == "改按钮"
                    && items[0].state == crate::PlanStepState::Doing
        )));
        assert!(!events.iter().any(
            |event| matches!(event, PromptSessionEvent::Status { phase, .. } if phase == "tool")
        ));
    }

    #[test]
    fn parse_control_request_emits_permission() {
        let control = PromptSessionControl::new();
        let line = r#"{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls -la"}}}"#;
        let events =
            parse_claude_stream_line("s1", line, Some(&control), None, &mut PlanBoard::default());
        assert!(events.iter().any(|e| matches!(
            e,
            PromptSessionEvent::PermissionRequest {
                request_id,
                tool_name,
                detail,
                input_mode,
                ..
            } if request_id == "req-1"
                && tool_name == "Bash"
                && detail.contains("ls -la")
                && input_mode == "choice"
        )));
    }

    #[test]
    fn open_secret_question_is_a_hidden_field() {
        let line = r#"{"type":"control_request","request_id":"req-s","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"把密钥贴在这里","kind":"text"}]}}}"#;
        let events = parse_claude_stream_line("s1", line, None, None, &mut PlanBoard::default());
        assert!(events.iter().any(|e| matches!(
            e,
            PromptSessionEvent::PermissionRequest {
                detail,
                input_mode,
                ..
            } if detail == "把密钥贴在这里" && input_mode == "secret"
        )));
    }

    #[test]
    fn interactive_claude_routes_open_questions() {
        let dir = tempdir().unwrap();
        let overlay = std::collections::HashMap::new();
        let launch = ClaudeLaunch {
            cwd: dir.path(),
            skip_permissions: false,
            interactive: true,
            stream_input: true,
            overlay: &overlay,
        };
        let cmd = build_claude_command("hi", &launch, None).unwrap();
        let args: Vec<String> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let joined = args.join("\n");
        assert!(joined.contains("\"AskUserQuestion\""));
        assert!(joined.contains("kind"));
        assert!(joined.contains("fewer than 2 options"));
    }

    #[test]
    fn ask_user_question_is_an_option_prompt() {
        let line = r#"{"type":"control_request","request_id":"req-q","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"用哪种登录？","options":[{"label":"浏览器"},{"label":"密钥"}]}]}}}"#;
        let events = parse_claude_stream_line("s1", line, None, None, &mut PlanBoard::default());
        assert!(events.iter().any(|e| matches!(
            e,
            PromptSessionEvent::PermissionRequest {
                tool_name,
                detail,
                input_mode,
                ..
            } if tool_name == "AskUserQuestion"
                && detail == "用哪种登录？"
                && input_mode == "options"
        )));
    }

    #[test]
    fn mcp_config_flag_does_not_swallow_prompt() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
        let prompt = "Response style: answer the user directly and concisely.";
        let overlay = std::collections::HashMap::new();
        let launch = ClaudeLaunch {
            cwd: dir.path(),
            skip_permissions: true,
            interactive: false,
            stream_input: false,
            overlay: &overlay,
        };
        let cmd = build_claude_command(prompt, &launch, None).unwrap();
        let args: Vec<String> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.iter().any(|arg| arg.starts_with("--mcp-config=")),
            "mcp config must be one argv: {args:?}"
        );
        assert!(
            !args.iter().any(|arg| arg == "--mcp-config"),
            "bare --mcp-config slurps the prompt: {args:?}"
        );
        let dash = args
            .iter()
            .position(|arg| arg == "--")
            .expect("prompt separator");
        assert_eq!(args.get(dash + 1).map(String::as_str), Some(prompt));
        assert!(args[..dash]
            .iter()
            .any(|arg| arg == "--dangerously-skip-permissions"));
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
