use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::prompt_session::control::{CodexReplyKind, PromptSessionControl};
use crate::prompt_session::util::humanize_runtime_error;
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
            .or_else(|| value.pointer("/result/threadId"))
            .or_else(|| value.pointer("/result/id"))
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
        let (tool_name, detail, input_json) = permission_from_codex(method, &params);
        control.remember_codex_reply(&request_id, kind);
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
            if !state.saw_agent_delta {
                if let Some(text) = turn_completed_agent_text(&params) {
                    state.saw_agent_delta = true;
                    on_event(PromptSessionEvent::Delta {
                        session_id: state.session_id.clone(),
                        text,
                    });
                }
            }
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
            on_event(PromptSessionEvent::Status {
                session_id: state.session_id.clone(),
                phase: if status == "failed" { "error" } else { "mcp" }.into(),
                message,
            });
        }
        "item/completed" | "item/started" => {
            let item = params.get("item").cloned().unwrap_or(Value::Null);
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
                "commandExecution" | "command_execution" | "fileChange" | "file_change"
                | "mcpToolCall" | "mcp_tool_call" => {
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
                            .or_else(|| {
                                let server = item.get("server").and_then(|v| v.as_str());
                                let tool = item.get("tool").and_then(|v| v.as_str());
                                match (server, tool) {
                                    (Some(s), Some(t)) => Some(format!("{s}/{t}")),
                                    _ => None,
                                }
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

pub(crate) fn mcp_startup_is_missing_program(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("os error 2") || lower.contains("no such file") || lower.contains("系统找不到")
}

pub(crate) fn is_agent_message_type(item_type: &str) -> bool {
    item_type == "agentMessage" || item_type == "agent_message"
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
        return Some(CodexReplyKind::Elicitation);
    }
    if lower.contains("permissions") && lower.contains("requestapproval") {
        let requested = params
            .get("permissions")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));
        return Some(CodexReplyKind::Permissions { requested });
    }
    if lower.contains("requestapproval")
        || lower.ends_with("requestuserinput")
        || lower.contains("requestuserinput")
    {
        return Some(CodexReplyKind::Decision);
    }
    None
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

    let detail = params
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
        .unwrap_or_else(|| params.to_string());

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
