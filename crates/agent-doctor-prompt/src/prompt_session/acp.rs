//! Generic Agent Client Protocol (ACP) client shared by every ACP-speaking
//! ask backend.
//!
//! Owns the wire state machine — `initialize` → `session/new`/`session/resume`
//! → `session/prompt`, `session/update` translation, tool-call folding and
//! `session/request_permission` — so a new ACP agent only describes its launch
//! (`AcpProfile` + command) instead of re-implementing the protocol.
//! Agent-specific behavior (version self-update, vendor env, warm pools)
//! stays in the backend that wraps this module.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use super::control::PromptSessionControl;
use super::tool_step::{tool_result_text, tool_step, tool_step_result};
use super::util::{
    force_stop_child, format_tool_status, is_runtime_stderr_noise, tool_activity_detail,
    SessionClock,
};
use super::vision::EncodedImage;
use super::warm::LivePipes;
use super::{PromptSessionEvent, PromptSessionStatus, TimeoutNote};

/// Copy shown to the user when this agent speaks. Every string the protocol
/// layer emits comes from here, so agents keep their own voice.
pub(crate) struct AcpProfile {
    pub incomplete_turn: String,
    pub resume_fallback: String,
    pub no_session: String,
    pub refusal: String,
    pub images_note: String,
    /// Fallback tool title when the request and the call carry none.
    pub default_tool: String,
}

impl AcpProfile {
    pub(crate) fn deepseek() -> Self {
        Self {
            incomplete_turn: "DeepSeek 这一轮没有完成".into(),
            resume_fallback: "上次对话接不上，正在新开一轮。".into(),
            no_session: "DeepSeek 没有返回这一轮的会话。".into(),
            refusal: "DeepSeek 拒绝继续这一轮。".into(),
            images_note: "这个 DeepSeek 助手收不了图片，这次只带上了图里的字。".into(),
            default_tool: "这一步操作".into(),
        }
    }

    pub(crate) fn generic() -> Self {
        Self {
            incomplete_turn: "ACP 这一轮没有完成".into(),
            resume_fallback: "上次对话接不上，正在新开一轮。".into(),
            no_session: "ACP 助手没有返回这一轮的会话。".into(),
            refusal: "ACP 助手拒绝继续这一轮。".into(),
            images_note: "这个 ACP 助手收不了图片，这次只带上了图里的字。".into(),
            default_tool: "这一步操作".into(),
        }
    }
}

pub(crate) struct TurnOutcome {
    pub status: PromptSessionStatus,
    pub exit_code: Option<i32>,
    pub stderr: String,
    pub error: Option<String>,
    pub timeout: Option<TimeoutNote>,
    pub acp_session: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Waiting {
    Initialize,
    Resume,
    NewSession,
    Prompt,
}

pub(crate) struct AcpState {
    pub next_id: i64,
    pub waiting: HashMap<i64, Waiting>,
    pub acp_session: Option<String>,
    pub prompt_done: bool,
    pub failed: Option<String>,
    pub resume_rejected: bool,
    /// Id we asked to resume. Some agents omit `sessionId` on a successful resume.
    pub resume_target: Option<String>,
    /// toolCallId → (name, raw arguments). The permission request itself has no title.
    pub tools: HashMap<String, (String, String)>,
    /// toolCallId → what the agent has said about that call so far.
    pub calls: HashMap<String, AcpCall>,
    pub images: Vec<EncodedImage>,
    /// `initialize` said `promptCapabilities.image`.
    pub takes_images: bool,
    pub profile: AcpProfile,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn pump_acp<F>(
    session_id: &str,
    cwd: &Path,
    child: &mut Child,
    pipes: &mut LivePipes,
    timeout_sec: u64,
    cancel: Arc<AtomicBool>,
    control: &PromptSessionControl,
    prompt: &str,
    images: Vec<EncodedImage>,
    already_open: Option<String>,
    resume_id: Option<&str>,
    auto_allow: bool,
    profile: AcpProfile,
    on_event: &mut F,
) -> Result<TurnOutcome>
where
    F: FnMut(PromptSessionEvent),
{
    let pid = child.id();
    let mut state = AcpState {
        next_id: 1,
        waiting: HashMap::new(),
        acp_session: already_open.clone(),
        prompt_done: false,
        failed: None,
        resume_rejected: false,
        resume_target: None,
        tools: HashMap::new(),
        calls: HashMap::new(),
        images,
        takes_images: false,
        profile,
    };
    let mut clock = SessionClock::new(timeout_sec);
    let mut timeout_note = None;
    let mut stderr_text = String::new();

    if let Some(sid) = already_open {
        queue_prompt(control, &mut state, &sid, prompt)?;
    } else {
        queue_rpc(
            control,
            &mut state,
            Waiting::Initialize,
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientInfo": {
                    "name": "agent_doctor",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        )?;
    }

    let status = loop {
        if cancel.load(Ordering::SeqCst) {
            if let Some(sid) = state.acp_session.clone() {
                let _ = control.write_line(
                    &json!({
                        "jsonrpc": "2.0",
                        "method": "session/cancel",
                        "params": { "sessionId": sid }
                    })
                    .to_string(),
                );
            }
            force_stop_child(child, pid);
            break PromptSessionStatus::Cancelled;
        }
        if clock.expired() {
            timeout_note = Some(clock.timeout_note(""));
            force_stop_child(child, pid);
            break PromptSessionStatus::TimedOut;
        }

        let drained = pipes.drain();
        if !drained.is_empty() {
            clock.touch();
        }
        for (is_stdout, line) in drained {
            if !is_stdout {
                if !line.trim().is_empty() && !is_runtime_stderr_noise(&line) {
                    if !stderr_text.is_empty() {
                        stderr_text.push('\n');
                    }
                    stderr_text.push_str(&line);
                    on_event(PromptSessionEvent::StderrLine {
                        session_id: session_id.to_string(),
                        line,
                    });
                }
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let messages = if let Some(list) = value.as_array() {
                list.clone()
            } else {
                vec![value]
            };
            for message in messages {
                handle_acp_message(
                    session_id, &message, control, auto_allow, &mut state, on_event,
                );
            }
        }

        if state.prompt_done {
            break PromptSessionStatus::Succeeded;
        }
        advance_handshake(cwd, control, prompt, resume_id, &mut state)?;
        if state.failed.is_some() && !state.waiting.values().any(|kind| *kind == Waiting::Prompt) {
            force_stop_child(child, pid);
            break PromptSessionStatus::Failed;
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                break if state.prompt_done {
                    PromptSessionStatus::Succeeded
                } else if cancel.load(Ordering::SeqCst) {
                    PromptSessionStatus::Cancelled
                } else {
                    PromptSessionStatus::Failed
                };
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(error) => return Err(error).context("failed waiting for the ACP agent"),
        }
    };

    Ok(TurnOutcome {
        status,
        exit_code: child
            .try_wait()
            .ok()
            .and_then(|status| status.and_then(|s| s.code())),
        stderr: stderr_text,
        error: state.failed,
        timeout: timeout_note,
        acp_session: state.acp_session,
    })
}

fn advance_handshake(
    cwd: &Path,
    control: &PromptSessionControl,
    prompt: &str,
    resume_id: Option<&str>,
    state: &mut AcpState,
) -> Result<()> {
    if state
        .waiting
        .values()
        .any(|kind| *kind == Waiting::Initialize)
        || state.failed.is_some()
    {
        return Ok(());
    }
    if state.acp_session.is_none()
        && !state
            .waiting
            .values()
            .any(|kind| matches!(kind, Waiting::Resume | Waiting::NewSession))
    {
        if let Some(sid) = resume_id.filter(|_| !state.resume_rejected) {
            state.resume_target = Some(sid.to_string());
            queue_rpc(
                control,
                state,
                Waiting::Resume,
                "session/resume",
                json!({
                    "sessionId": sid,
                    "cwd": acp_cwd(cwd),
                    "mcpServers": [],
                }),
            )?;
        } else {
            queue_rpc(
                control,
                state,
                Waiting::NewSession,
                "session/new",
                json!({
                    "cwd": acp_cwd(cwd),
                    "mcpServers": [],
                }),
            )?;
        }
        return Ok(());
    }
    if state.acp_session.is_some()
        && !state.waiting.values().any(|kind| *kind == Waiting::Prompt)
        && !state.prompt_done
        && !state.waiting.values().any(|kind| {
            matches!(
                kind,
                Waiting::Resume | Waiting::NewSession | Waiting::Initialize
            )
        })
    {
        let sid = state.acp_session.clone().unwrap_or_default();
        queue_prompt(control, state, &sid, prompt)?;
    }
    Ok(())
}

fn queue_prompt(
    control: &PromptSessionControl,
    state: &mut AcpState,
    sid: &str,
    prompt: &str,
) -> Result<()> {
    let blocks = prompt_blocks(prompt, &state.images, state.takes_images);
    queue_rpc(
        control,
        state,
        Waiting::Prompt,
        "session/prompt",
        json!({
            "sessionId": sid,
            "prompt": blocks,
        }),
    )
}

/// ACP content blocks: text, then pictures when the agent accepts them.
pub(crate) fn prompt_blocks(prompt: &str, images: &[EncodedImage], takes_images: bool) -> Value {
    let mut blocks = vec![json!({ "type": "text", "text": prompt })];
    if takes_images {
        blocks.extend(images.iter().map(|image| {
            json!({
                "type": "image",
                "mimeType": image.media_type,
                "data": image.base64,
            })
        }));
    }
    Value::Array(blocks)
}

fn queue_rpc(
    control: &PromptSessionControl,
    state: &mut AcpState,
    kind: Waiting,
    method: &str,
    params: Value,
) -> Result<()> {
    let id = state.next_id;
    state.next_id += 1;
    state.waiting.insert(id, kind);
    control.write_line(
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        })
        .to_string(),
    )?;
    Ok(())
}

fn handle_acp_message<F>(
    session_id: &str,
    message: &Value,
    control: &PromptSessionControl,
    auto_allow: bool,
    state: &mut AcpState,
    on_event: &mut F,
) where
    F: FnMut(PromptSessionEvent),
{
    if let Some(method) = message.get("method").and_then(|v| v.as_str()) {
        if message.get("id").is_some() {
            handle_server_request(
                session_id, method, message, control, auto_allow, state, on_event,
            );
        } else if method == "session/update" {
            handle_update(
                session_id,
                message.get("params").unwrap_or(&Value::Null),
                state,
                on_event,
            );
        }
        return;
    }

    let Some(id) = message.get("id").and_then(|v| v.as_i64()) else {
        return;
    };
    let Some(kind) = state.waiting.remove(&id) else {
        return;
    };
    if let Some(error) = message.get("error") {
        let text = error
            .get("message")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| state.profile.incomplete_turn.clone());
        if kind == Waiting::Resume {
            state.resume_rejected = true;
            on_event(PromptSessionEvent::Status {
                session_id: session_id.to_string(),
                phase: "session".into(),
                message: state.profile.resume_fallback.clone(),
            });
            return;
        }
        state.failed = Some(text);
        return;
    }
    let result = message.get("result").cloned().unwrap_or(Value::Null);
    match kind {
        Waiting::Initialize => {
            state.takes_images = result
                .pointer("/agentCapabilities/promptCapabilities/image")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !state.images.is_empty() && !state.takes_images {
                on_event(PromptSessionEvent::Status {
                    session_id: session_id.to_string(),
                    phase: "info".into(),
                    message: state.profile.images_note.clone(),
                });
            }
        }
        Waiting::Resume | Waiting::NewSession => {
            let sid = result
                .get("sessionId")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .filter(|id| !id.is_empty())
                .or_else(|| {
                    if kind == Waiting::Resume {
                        state.resume_target.clone()
                    } else {
                        None
                    }
                });
            if let Some(sid) = sid {
                state.acp_session = Some(sid);
            } else {
                state.failed = Some(state.profile.no_session.clone());
            }
        }
        Waiting::Prompt => {
            let stop = result
                .get("stopReason")
                .and_then(|v| v.as_str())
                .unwrap_or("end_turn");
            if stop == "refusal" {
                state.failed = Some(state.profile.refusal.clone());
            } else {
                state.prompt_done = true;
            }
            if let Some(usage) = agent_doctor_core::usage::find_usage(&result) {
                on_event(PromptSessionEvent::Usage {
                    session_id: session_id.to_string(),
                    usage,
                    model: None,
                });
            }
        }
    }
}

fn handle_server_request<F>(
    session_id: &str,
    method: &str,
    message: &Value,
    control: &PromptSessionControl,
    auto_allow: bool,
    state: &AcpState,
    on_event: &mut F,
) where
    F: FnMut(PromptSessionEvent),
{
    let Some(id) = message.get("id") else {
        return;
    };
    let request_id = id.to_string().trim_matches('"').to_string();
    if method != "session/request_permission" {
        let _ = control.write_line(
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not supported" }
            })
            .to_string(),
        );
        return;
    }
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let (allow_id, deny_id) = permission_option_ids(&params);
    if auto_allow {
        let _ = control.write_line(&PromptSessionControl::acp_permission_line(
            &request_id,
            &allow_id,
        ));
        return;
    }
    let call_id = params
        .pointer("/toolCall/toolCallId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let remembered = state.tools.get(call_id);
    let tool_name = params
        .pointer("/toolCall/title")
        .and_then(|v| v.as_str())
        .filter(|text| !text.is_empty())
        .or_else(|| {
            remembered
                .map(|(name, _)| name.as_str())
                .filter(|text| !text.is_empty())
        })
        .unwrap_or(&state.profile.default_tool)
        .to_string();
    let detail = remembered
        .map(|(_, raw)| raw.clone())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| tool_name.clone());
    control.remember_acp_permission(&request_id, allow_id, deny_id);
    on_event(PromptSessionEvent::PermissionRequest {
        session_id: session_id.to_string(),
        request_id,
        tool_name,
        detail,
        input_json: String::new(),
        input_mode: "choice".into(),
    });
}

/// The option ids the desktop's allow/deny answer maps to: the first
/// `allow*`-kind option and the first `deny`/`reject`-kind one. Agents that
/// omit options (DeepSeek Harness) get the spellings its protocol expects.
fn permission_option_ids(params: &Value) -> (String, String) {
    const FALLBACK_ALLOW: &str = "allow-once";
    const FALLBACK_DENY: &str = "reject-once";
    let options = params.get("options").and_then(Value::as_array);
    let Some(options) = options else {
        return (FALLBACK_ALLOW.into(), FALLBACK_DENY.into());
    };
    let kind = |option: &Value| {
        option
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let allow = options
        .iter()
        .find(|option| kind(option).starts_with("allow"))
        .or_else(|| options.first());
    let deny = options
        .iter()
        .find(|option| {
            let kind = kind(option);
            kind.starts_with("deny") || kind.starts_with("reject")
        })
        .or_else(|| options.get(1));
    let id_of = |option: Option<&Value>| {
        option
            .and_then(|option| option.get("optionId"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    };
    (
        id_of(allow).unwrap_or_else(|| FALLBACK_ALLOW.into()),
        id_of(deny).unwrap_or_else(|| FALLBACK_DENY.into()),
    )
}

pub(crate) fn handle_update<F>(
    session_id: &str,
    params: &Value,
    state: &mut AcpState,
    on_event: &mut F,
) where
    F: FnMut(PromptSessionEvent),
{
    // A resumed session replays its saved history as session/update traffic
    // before session/load answers. Those calls and thoughts already live in
    // the transcript from their own turn; until the load response lifts this
    // gate, everything here is replay, not the new prompt's turn.
    if state
        .waiting
        .values()
        .any(|phase| *phase == Waiting::Resume)
    {
        return;
    }
    let update = params.get("update").unwrap_or(params);
    let kind = update
        .get("sessionUpdate")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match kind {
        "agent_message_chunk" => {
            if let Some(text) = chunk_text(update) {
                on_event(PromptSessionEvent::Delta {
                    session_id: session_id.to_string(),
                    text,
                });
            }
        }
        "agent_thought_chunk" => {
            if let Some(text) = chunk_text(update) {
                on_event(PromptSessionEvent::Thinking {
                    session_id: session_id.to_string(),
                    text,
                });
            }
        }
        "tool_call" | "tool_call_update" => handle_tool_update(session_id, update, state, on_event),
        _ => {}
    }
}

/// Everything known so far about one ACP tool call. Updates only carry the fields that changed.
#[derive(Default)]
pub(crate) struct AcpCall {
    title: String,
    kind: String,
    input: Value,
    path: String,
    /// `content` block `{type:"diff", path, oldText, newText}`.
    diff: Option<(String, String, String)>,
    shown: bool,
    finished: bool,
}

/// One step per `toolCallId`: announced once, refreshed while any field changes, finished once.
fn handle_tool_update<F>(session_id: &str, update: &Value, state: &mut AcpState, on_event: &mut F)
where
    F: FnMut(PromptSessionEvent),
{
    let Some(id) = update
        .get("toolCallId")
        .and_then(|v| v.as_str())
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    let call = state.calls.entry(id.to_string()).or_default();
    let changed = merge_acp_call(call, update);
    let remembered = state.tools.entry(id.to_string()).or_default();
    remembered.0 = call.title.clone();
    if !call.input.is_null() {
        remembered.1 = tool_arguments(&call.input);
    }

    let status = update.get("status").and_then(|v| v.as_str()).unwrap_or("");
    let finished = matches!(status, "completed" | "failed");
    if !call.shown {
        call.shown = true;
        let name = acp_tool_name(call);
        on_event(PromptSessionEvent::Status {
            session_id: session_id.to_string(),
            phase: "tool".into(),
            message: format_tool_status(name, &tool_activity_detail(name, &call.input)),
        });
        on_event(PromptSessionEvent::Tool {
            session_id: session_id.to_string(),
            step: Box::new(acp_step(id, call)),
        });
    } else if changed && !call.finished {
        on_event(PromptSessionEvent::Tool {
            session_id: session_id.to_string(),
            step: Box::new(acp_step(id, call)),
        });
    }
    if finished && !call.finished {
        call.finished = true;
        let output = update
            .get("rawOutput")
            .map(tool_result_text)
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| acp_content_text(update.get("content")));
        on_event(PromptSessionEvent::Tool {
            session_id: session_id.to_string(),
            step: Box::new(tool_step_result(id, &output, status == "failed")),
        });
    }
}

/// Copy the fields this update carries. True when something that shapes the step changed.
fn merge_acp_call(call: &mut AcpCall, update: &Value) -> bool {
    let mut changed = false;
    let text = |key: &str| {
        update
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    if let Some(title) = text("title").filter(|t| *t != call.title) {
        call.title = title.to_string();
        changed = true;
    }
    if let Some(kind) = text("kind").filter(|k| *k != call.kind) {
        call.kind = kind.to_string();
        changed = true;
    }
    if let Some(raw) = update.get("rawInput").filter(|raw| !raw.is_null()) {
        if *raw != call.input {
            call.input = raw.clone();
            changed = true;
        }
    }
    if let Some(path) = update
        .pointer("/locations/0/path")
        .and_then(|v| v.as_str())
        .filter(|p| !p.is_empty() && *p != call.path)
    {
        call.path = path.to_string();
        changed = true;
    }
    let diff = update
        .get("content")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("type").and_then(Value::as_str) == Some("diff"))
        })
        .map(|item| {
            let field = |key: &str| {
                item.get(key)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            };
            (field("path"), field("oldText"), field("newText"))
        });
    if diff.is_some() && diff != call.diff {
        call.diff = diff;
        changed = true;
    }
    changed
}

/// The tool name the steps know. A specific ACP `kind` wins; agents that leave it as
/// `other` put the tool's own name in `title`.
fn acp_tool_name(call: &AcpCall) -> &str {
    match call.kind.as_str() {
        "execute" => "bash",
        "read" => "read",
        "edit" => "edit",
        "search" => "search",
        "fetch" => "webfetch",
        _ if !call.title.is_empty() => &call.title,
        _ => "tool",
    }
}

fn acp_step(id: &str, call: &AcpCall) -> super::tool_step::ToolStep {
    let name = acp_tool_name(call);
    let empty_input = call.input.as_object().map_or(true, |obj| obj.is_empty());
    let mut step = match &call.diff {
        Some((path, old, new)) if empty_input => tool_step(
            id,
            "edit",
            &json!({"file_path": path, "old_string": old, "new_string": new}),
        ),
        _ => tool_step(id, name, &call.input),
    };
    if step.path.is_empty() && step.command.is_empty() {
        step.path = call.path.clone();
    }
    if step.title.is_empty() && name != call.title {
        step.title = call.title.clone();
    }
    step
}

/// Text blocks of an ACP tool result: `[{type:"content", content:{type:"text", text}}]`.
fn acp_content_text(content: Option<&Value>) -> String {
    content
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.pointer("/content/text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Hand agents the plain Win32 spelling: `fs::canonicalize` on Windows answers
/// with a verbatim `\\?\C:\…` prefix, which Node-based agents mishandle and
/// store forever.
pub(crate) fn acp_cwd(cwd: &Path) -> String {
    let canonical = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    strip_windows_verbatim(&canonical).display().to_string()
}

/// Pure text work on the verbatim spellings `fs::canonicalize` emits, kept
/// off-`cfg` so the drive-root class stays testable on every platform.
pub(crate) fn strip_windows_verbatim(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest.to_string());
    }
    path.to_path_buf()
}

fn tool_arguments(raw: &Value) -> String {
    match raw {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn chunk_text(update: &Value) -> Option<String> {
    let content = update.get("content")?;
    let text = content.get("text").and_then(|v| v.as_str())?;
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}
