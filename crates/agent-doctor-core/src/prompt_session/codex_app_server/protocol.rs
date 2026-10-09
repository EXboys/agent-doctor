use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::prompt_session::control::{CodexReplyKind, PromptSessionControl};
use crate::prompt_session::plan::{plan_from_item, plan_from_tool};
use crate::prompt_session::util::{humanize_runtime_error, THINKING_AFTER_TOOL};
use crate::prompt_session::PromptSessionEvent;

use super::*;

pub(crate) fn handle_rpc_line<F>(
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
    if let (Some(id), Some(method)) = (
        value.get("id"),
        value.get("method").and_then(|m| m.as_str()),
    ) {
        return handle_server_request(method, id, value.get("params"), control, state, on_event);
    }

    // Notification
    if let Some(method) = value.get("method").and_then(|m| m.as_str()) {
        handle_notification(method, value.get("params"), state, on_event);
    }

    Ok(())
}

pub(crate) fn handle_rpc_response<F>(
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
        // Opening the thread failed. Leave the turn unfinished so the caller
        // can start a fresh thread instead of ending with an empty reply.
        if state.waiting_thread == id_num {
            state.waiting_thread = None;
            state.thread_open_error = Some(msg.to_string());
            return Ok(());
        }
        on_event(PromptSessionEvent::StderrLine {
            session_id: state.session_id.clone(),
            line: humanize_runtime_error(msg),
        });
        if state.waiting_turn == id_num {
            state.turn_done = true;
        }
        return Ok(());
    }

    if state.waiting_thread == id_num {
        state.waiting_thread = None;
        let thread_id = value
            .pointer("/result/thread/id")
            .or_else(|| value.pointer("/result/thread/sessionId"))
            .or_else(|| value.pointer("/result/threadId"))
            .or_else(|| value.pointer("/result/id"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .context("thread/start missing thread id")?;
        state.thread_id = Some(thread_id);
        return start_turn(control, state, on_event);
    }

    if state.waiting_turn == id_num {
        state.waiting_turn = None;
        // Turn accepted; completion arrives via turn/completed notification.
    }

    Ok(())
}

/// Text first, then each picture as `localImage` (Codex reads and encodes the file).
pub(crate) fn turn_input(prompt: &str, images: &[String]) -> Value {
    let mut input = vec![json!({ "type": "text", "text": prompt })];
    input.extend(
        images
            .iter()
            .map(|path| json!({ "type": "localImage", "path": path })),
    );
    Value::Array(input)
}

/// Send this message as a new turn on the open thread.
pub(crate) fn start_turn<F>(
    control: &PromptSessionControl,
    state: &mut PumpState,
    on_event: &mut F,
) -> Result<()>
where
    F: FnMut(PromptSessionEvent),
{
    let thread_id = state.thread_id.clone().context("no open Codex thread")?;
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
                "input": turn_input(&state.prompt, &state.images),
                "cwd": state.cwd,
                "approvalPolicy": state.approval_policy,
                // SandboxPolicy.type uses camelCase (unlike thread `sandbox` SandboxMode kebab-case).
                "sandboxPolicy": turn_sandbox_policy(&state.cwd, &state.sandbox_roots)
            }
        })
        .to_string(),
    )
}

pub(crate) fn handle_server_request<F>(
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

    let params = params.cloned().unwrap_or(Value::Null);
    if let Some(kind) = codex_reply_kind(method, &params) {
        if !state.interactive {
            control.remember_codex_reply(&request_id, kind);
            let _ = control.respond_permission(&request_id, false);
            return Ok(());
        }
        // Clearing a temp folder is how a video edit throws away scraps.
        // Ask only when the delete can reach the user's own files.
        if temp_only_delete(&params) {
            control.remember_codex_reply(&request_id, kind);
            let _ = control.respond_permission(&request_id, true);
            return Ok(());
        }
        let (tool_name, detail, input_json) = permission_from_codex(method, &params);
        control.remember_codex_reply(&request_id, kind);
        let input_mode = permission_input_mode(method, &params);
        let waiting = if input_mode == "choice" {
            format!("等待确认：{tool_name}")
        } else {
            "需要你输入".to_string()
        };
        on_event(PromptSessionEvent::Status {
            session_id: state.session_id.clone(),
            phase: "permission".into(),
            message: waiting,
        });
        on_event(PromptSessionEvent::PermissionRequest {
            session_id: state.session_id.clone(),
            request_id,
            tool_name,
            detail,
            input_json,
            input_mode,
        });
        return Ok(());
    }

    on_event(PromptSessionEvent::Status {
        session_id: state.session_id.clone(),
        phase: "info".into(),
        message: format!("auto-decline unsupported request: {method}"),
    });
    let _ = control.decline_codex_rpc(id);
    Ok(())
}

pub(crate) fn handle_notification<F>(
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
            let status = params
                .pointer("/turn/status")
                .or_else(|| params.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if status == "failed" {
                let msg =
                    codex_error_message(&params).unwrap_or_else(|| "Codex turn failed".into());
                on_event(PromptSessionEvent::Status {
                    session_id: state.session_id.clone(),
                    phase: "error".into(),
                    message: msg.clone(),
                });
                on_event(PromptSessionEvent::StderrLine {
                    session_id: state.session_id.clone(),
                    line: humanize_runtime_error(&msg),
                });
            } else {
                let fallback = if state.saw_agent_delta {
                    None
                } else {
                    turn_completed_agent_text(&params)
                };
                if let Some(text) = fallback {
                    state.saw_agent_delta = true;
                    on_event(PromptSessionEvent::Delta {
                        session_id: state.session_id.clone(),
                        text,
                    });
                }
                on_event(PromptSessionEvent::Status {
                    session_id: state.session_id.clone(),
                    phase: "writing".into(),
                    message: "本轮完成".into(),
                });
            }
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
        // Current app-server uses camelCase. Older builds used snake_case.
        "item/agentMessage/delta" | "item/agent_message/delta" => {
            if let Some(text) = json_delta_text(&params) {
                // Do NOT emit Status on every delta — the chat UI seals the
                // assistant bubble on phase changes, which otherwise stores
                // one localStorage message per token.
                state.saw_agent_delta = true;
                on_event(PromptSessionEvent::Delta {
                    session_id: state.session_id.clone(),
                    text,
                });
            }
        }
        "item/reasoning/summaryTextDelta" | "item/reasoning/summaryPartAdded" => {
            let delta = if method == "item/reasoning/summaryPartAdded" {
                "\n\n".to_string()
            } else {
                json_delta_text(&params).unwrap_or_default()
            };
            if !delta.is_empty() {
                on_event(PromptSessionEvent::Thinking {
                    session_id: state.session_id.clone(),
                    text: delta.clone(),
                });
            }
            if let Some(message) = state.thinking.push(&delta) {
                on_event(PromptSessionEvent::Status {
                    session_id: state.session_id.clone(),
                    phase: "thinking".into(),
                    message,
                });
            }
        }
        "mcpServer/startupStatus/updated" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("mcp");
            let status = params
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("starting");
            let error = params.get("error").and_then(|v| v.as_str()).unwrap_or("");
            // A missing program is cleaned before launch. If one still fails
            // that way, the reply can continue — don't show it as this message failing.
            if status == "failed" && mcp_startup_is_missing_program(error) {
                eprintln!("[ask] ignored missing tool {name}: {error}");
                return;
            }
            let message = if error.is_empty() {
                format!("MCP {name}: {status}")
            } else {
                format!("MCP {name}: {status} ({error})")
            };
            // Tool startup is not the reply failing. Keep it out of the red error chip.
            on_event(PromptSessionEvent::Status {
                session_id: state.session_id.clone(),
                phase: if status == "failed" { "warn" } else { "mcp" }.into(),
                message,
            });
        }
        "turn/plan/updated" => {
            if let Some(items) = plan_from_tool("update_plan", &params) {
                on_event(PromptSessionEvent::Plan {
                    session_id: state.session_id.clone(),
                    items,
                });
            }
        }
        "item/completed" | "item/started" => {
            let item = params.get("item").cloned().unwrap_or(Value::Null);
            if let Some(items) = plan_from_item(&item) {
                on_event(PromptSessionEvent::Plan {
                    session_id: state.session_id.clone(),
                    items,
                });
                return;
            }
            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match item_type {
                // Prefer streamed deltas; only fall back to the completed payload
                // when no delta arrived (some turns emit text only on completed).
                // Official app-server uses camelCase `agentMessage`; older builds used snake_case.
                t if is_agent_message_type(t) && method == "item/completed" => {
                    if !state.saw_agent_delta {
                        if let Some(text) = agent_item_text(&item) {
                            state.saw_agent_delta = true;
                            on_event(PromptSessionEvent::Delta {
                                session_id: state.session_id.clone(),
                                text,
                            });
                        }
                    }
                }
                t if is_agent_message_type(t) => {}
                t if is_tool_activity(t) => {
                    let item_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if method == "item/started" {
                        state.tools.start(item_id);
                        state
                            .tools
                            .remember(&shorten_tool_label(&tool_activity_label(&item, item_type)));
                    } else {
                        state.tools.finish(item_id);
                        if !state.tools.running() {
                            on_event(PromptSessionEvent::Status {
                                session_id: state.session_id.clone(),
                                phase: "thinking".into(),
                                message: THINKING_AFTER_TOOL.into(),
                            });
                        }
                    }
                    // Emit once on start — completed would duplicate the same chip in UI.
                    if method == "item/started" {
                        let label = shorten_tool_label(&tool_activity_label(&item, item_type));
                        on_event(PromptSessionEvent::Status {
                            session_id: state.session_id.clone(),
                            phase: "tool".into(),
                            message: label,
                        });
                    }
                }
                "reasoning" => {
                    if method == "item/started" {
                        state.thinking.reset();
                        on_event(PromptSessionEvent::Status {
                            session_id: state.session_id.clone(),
                            phase: "thinking".into(),
                            message: "正在思考…".into(),
                        });
                    }
                }
                "contextCompaction" | "context_compaction" | "compacted" => {
                    if method == "item/started" {
                        on_event(PromptSessionEvent::Status {
                            session_id: state.session_id.clone(),
                            phase: "session".into(),
                            message: "正在整理这段对话…".into(),
                        });
                    }
                }
                "enteredReviewMode" | "entered_review_mode" => {
                    if method == "item/started" {
                        on_event(PromptSessionEvent::Status {
                            session_id: state.session_id.clone(),
                            phase: "tool".into(),
                            message: "正在检查改动…".into(),
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
            if let Some(msg) = codex_error_message(&params) {
                on_event(PromptSessionEvent::StderrLine {
                    session_id: state.session_id.clone(),
                    line: humanize_runtime_error(&msg),
                });
            }
        }
        _ => {}
    }
}

pub(crate) fn mcp_startup_is_missing_program(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("os error 2")
        || lower.contains("no such file")
        || lower.contains("program not found")
        || lower.contains("系统找不到")
}

pub(crate) fn is_agent_message_type(item_type: &str) -> bool {
    item_type == "agentMessage" || item_type == "agent_message"
}

fn codex_key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn is_tool_activity(item_type: &str) -> bool {
    matches!(
        codex_key(item_type).as_str(),
        "commandexecution"
            | "filechange"
            | "mcptoolcall"
            | "collabtoolcall"
            | "dynamictoolcall"
            | "websearch"
            | "imageview"
    )
}

fn tool_activity_label(item: &Value, item_type: &str) -> String {
    item.get("command")
        .and_then(|v| {
            if let Some(text) = v.as_str() {
                Some(text.to_string())
            } else if let Some(arr) = v.as_array() {
                Some(
                    arr.iter()
                        .filter_map(|part| part.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            } else {
                None
            }
        })
        .or_else(|| {
            item.get("query")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            let server = item.get("server").and_then(|v| v.as_str());
            let tool = item.get("tool").and_then(|v| v.as_str());
            match (server, tool) {
                (Some(server), Some(tool)) => Some(format!("{server}/{tool}")),
                _ => tool.map(str::to_string),
            }
        })
        .or_else(|| {
            item.get("path")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| item_type.to_string())
}

/// Official failures use `{ error: { message } }`. Some builds put `message` on the params.
pub(crate) fn codex_error_message(params: &Value) -> Option<String> {
    params
        .pointer("/error/message")
        .or_else(|| params.pointer("/turn/error/message"))
        .or_else(|| params.get("message"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .or_else(|| {
            params
                .get("error")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        })
}

pub(crate) fn json_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Object(map) => map
            .get("text")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        Value::Array(items) => {
            let joined = items
                .iter()
                .filter_map(json_text)
                .collect::<Vec<_>>()
                .join("");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        }
        _ => None,
    }
}

pub(crate) fn json_delta_text(params: &Value) -> Option<String> {
    params
        .get("delta")
        .and_then(json_text)
        .or_else(|| params.pointer("/item/text").and_then(json_text))
}

pub(crate) fn agent_item_text(item: &Value) -> Option<String> {
    json_text(&item.get("text").cloned().unwrap_or(Value::Null))
        .or_else(|| json_text(&item.get("content").cloned().unwrap_or(Value::Null)))
        .or_else(|| json_text(&item.get("message").cloned().unwrap_or(Value::Null)))
}

pub(crate) fn turn_completed_agent_text(params: &Value) -> Option<String> {
    if let Some(text) = params
        .pointer("/turn/lastAgentMessage")
        .or_else(|| params.pointer("/lastAgentMessage"))
        .or_else(|| params.pointer("/turn/last_agent_message"))
        .and_then(json_text)
    {
        return Some(text);
    }
    let items = params
        .pointer("/turn/items")
        .or_else(|| params.get("items"))
        .and_then(|v| v.as_array())?;
    for item in items.iter().rev() {
        let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if is_agent_message_type(item_type) {
            if let Some(text) = agent_item_text(item) {
                return Some(text);
            }
        }
    }
    None
}

pub(crate) fn codex_reply_kind(method: &str, params: &Value) -> Option<CodexReplyKind> {
    let lower = method.to_ascii_lowercase();
    if lower.contains("elicitation") {
        if elicitation_wants_text(params) {
            return Some(CodexReplyKind::UserLine {
                question_id: first_question_id(params),
                elicitation: true,
                questions: params
                    .get("questions")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new())),
            });
        }
        return Some(CodexReplyKind::Elicitation);
    }
    if lower.contains("requestuserinput") {
        return Some(CodexReplyKind::UserLine {
            question_id: first_question_id(params),
            elicitation: false,
            questions: params
                .get("questions")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        });
    }
    if lower.contains("permissions") && lower.contains("requestapproval") {
        let requested = params
            .get("permissions")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));
        return Some(CodexReplyKind::Permissions { requested });
    }
    if lower.contains("requestapproval") {
        return Some(CodexReplyKind::Decision);
    }
    None
}

pub(crate) fn permission_input_mode(method: &str, params: &Value) -> String {
    match codex_reply_kind(method, params) {
        Some(CodexReplyKind::UserLine { .. }) if request_has_options(params) => "options".into(),
        Some(CodexReplyKind::UserLine { .. }) if prompt_looks_secret(params) => "secret".into(),
        Some(CodexReplyKind::UserLine { .. }) => "line".into(),
        _ => "choice".into(),
    }
}

fn request_has_options(params: &Value) -> bool {
    params
        .get("questions")
        .and_then(|v| v.as_array())
        .is_some_and(|questions| {
            questions.iter().any(|question| {
                question
                    .get("options")
                    .and_then(|v| v.as_array())
                    .is_some_and(|options| !options.is_empty())
            })
        })
}

fn first_question_id(params: &Value) -> String {
    params
        .pointer("/questions/0/id")
        .or_else(|| params.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("value")
        .to_string()
}

fn elicitation_wants_text(params: &Value) -> bool {
    if mcp_tool_approval(params) {
        return false;
    }
    if params.get("questions").is_some() || prompt_looks_secret(params) {
        return true;
    }
    schema_needs_typed_answer(params.get("requestedSchema"))
}

/// Codex asks "Allow the … MCP server to run tool …?" as an elicitation.
/// An empty schema still means yes/no, not a sentence to type.
fn mcp_tool_approval(params: &Value) -> bool {
    let meta = params.get("meta").or_else(|| params.get("_meta"));
    if meta
        .and_then(|value| value.get("codex_approval_kind"))
        .and_then(|value| value.as_str())
        == Some("mcp_tool_call")
    {
        return true;
    }
    params
        .get("message")
        .and_then(|value| value.as_str())
        .is_some_and(|message| message.contains("MCP server to run tool"))
}

fn schema_needs_typed_answer(schema: Option<&Value>) -> bool {
    let Some(schema) = schema.filter(|value| !value.is_null()) else {
        return false;
    };
    let Some(properties) = schema.get("properties").and_then(|value| value.as_object()) else {
        return false;
    };
    properties.values().any(|property| {
        let kind = property.get("type").and_then(|value| value.as_str());
        let enumerated = property
            .get("enum")
            .and_then(|value| value.as_array())
            .is_some_and(|items| !items.is_empty());
        kind == Some("string") && !enumerated
    })
}

fn prompt_looks_secret(params: &Value) -> bool {
    let blob = params.to_string().to_ascii_lowercase();
    [
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
}

fn command_line(params: &Value) -> String {
    let Some(command) = params.get("command") else {
        return String::new();
    };
    if let Some(text) = command.as_str() {
        return text.to_string();
    }
    command
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

fn is_temp_path(path: &str) -> bool {
    let path = path.trim().trim_matches('"').trim_matches('\'');
    if path.is_empty() || path.starts_with('-') || path.contains('*') || path.contains('$') {
        return false;
    }
    let mut roots = vec![
        "/tmp".to_string(),
        "/private/tmp".to_string(),
        std::env::temp_dir().display().to_string(),
    ];
    roots.retain(|root| !root.is_empty());
    roots.iter().any(|root| {
        let root = root.trim_end_matches('/');
        path == root || path.starts_with(&format!("{root}/"))
    })
}

/// Forced delete whose every path stays in a temp folder.
pub(crate) fn temp_only_delete(params: &Value) -> bool {
    let line = command_line(params);
    let lower = line.to_ascii_lowercase();
    let forced = lower.contains("rm -rf")
        || lower.contains("rm -fr")
        || lower.contains("rm -f")
        || lower.contains("rm --force");
    if !forced || line.contains('$') || line.contains('~') {
        return false;
    }
    let paths: Vec<&str> = line
        .split_whitespace()
        .map(|token| token.trim_matches(|c| c == '"' || c == '\'' || c == ';'))
        .filter(|token| token.starts_with('/'))
        .filter(|token| {
            !matches!(
                token.rsplit('/').next(),
                Some("zsh" | "bash" | "sh" | "rm" | "ffmpeg")
            )
        })
        .collect();
    !paths.is_empty() && paths.iter().all(|path| is_temp_path(path))
}

pub(crate) fn permission_from_codex(method: &str, params: &Value) -> (String, String, String) {
    let tool_name = if method.contains("fileChange") || method.contains("file_change") {
        "FileChange".to_string()
    } else if method.contains("permissions") {
        "Permissions".to_string()
    } else if method.to_ascii_lowercase().contains("elicitation") {
        params
            .get("serverName")
            .or_else(|| params.get("server"))
            .and_then(|v| v.as_str())
            .map(|s| format!("MCP {s}"))
            .unwrap_or_else(|| "MCP".to_string())
    } else if method.to_ascii_lowercase().contains("requestuserinput") {
        params
            .get("tool")
            .or_else(|| params.get("toolName"))
            .and_then(|v| v.as_str())
            .unwrap_or("Tool")
            .to_string()
    } else {
        "Bash".to_string()
    };

    let command_line = command_line(params);
    let command_lower = command_line.to_ascii_lowercase();
    let show_command = command_lower.contains("rm -f")
        || command_lower.contains("rm -rf")
        || command_lower.contains("rm -fr")
        || command_lower.contains("ffmpeg")
        || command_lower.contains(".mp4")
        || command_lower.contains(".mov")
        || command_lower.contains(".mkv");
    let detail = if show_command {
        command_line
    } else if method.to_ascii_lowercase().contains("requestuserinput") {
        params
            .pointer("/questions/0/question")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "需要你回答".to_string())
    } else {
        params
            .get("reason")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                params
                    .get("message")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
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
            .unwrap_or_else(|| params.to_string())
    };

    (tool_name, detail, params.to_string())
}

/// Collapse shell wrappers like `/bin/zsh -lc 'pwd'` → `pwd` for compact UI chips.
pub(crate) fn shorten_tool_label(raw: &str) -> String {
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
