use std::collections::HashMap;
use std::io::Write;
use std::process::ChildStdin;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use serde_json::Value;

/// How a pending permission request should be answered on stdin.
#[derive(Debug, Clone)]
enum PendingReply {
    /// Claude Code `control_response` with updated tool input.
    ClaudeTool { input: Value },
    /// Codex app-server JSON-RPC result (shape depends on the request method).
    CodexRpc { kind: CodexReplyKind },
}

#[derive(Debug, Clone)]
pub(crate) enum CodexReplyKind {
    /// `item/*/requestApproval` → `{ decision: accept|decline }`.
    Decision,
    /// `mcpServer/elicitation/request` → `{ action, content }`.
    Elicitation,
    /// `item/permissions/requestApproval` → `{ permissions, scope }`.
    Permissions { requested: Value },
}

/// Bidirectional control channel for Ask permission prompts (Claude or Codex).
#[derive(Clone, Default)]
pub struct PromptSessionControl {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pending: Arc<Mutex<HashMap<String, PendingReply>>>,
}

impl PromptSessionControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn attach_stdin(&self, stdin: ChildStdin) {
        if let Ok(mut guard) = self.stdin.lock() {
            *guard = Some(stdin);
        }
    }

    pub(crate) fn remember_claude_tool_input(&self, request_id: &str, input: Value) {
        if let Ok(mut guard) = self.pending.lock() {
            guard.insert(request_id.to_string(), PendingReply::ClaudeTool { input });
        }
    }

    pub(crate) fn remember_codex_reply(&self, request_id: &str, kind: CodexReplyKind) {
        if let Ok(mut guard) = self.pending.lock() {
            guard.insert(request_id.to_string(), PendingReply::CodexRpc { kind });
        }
    }

    fn take_pending(&self, request_id: &str) -> Option<PendingReply> {
        self.pending
            .lock()
            .ok()
            .and_then(|mut guard| guard.remove(request_id))
    }

    pub(crate) fn write_line(&self, line: &str) -> Result<()> {
        let mut guard = self
            .stdin
            .lock()
            .map_err(|_| anyhow::anyhow!("permission control lock poisoned"))?;
        let stdin = guard
            .as_mut()
            .context("ask session has no open stdin for permission replies")?;
        writeln!(stdin, "{line}").context("write permission reply")?;
        stdin.flush().context("flush permission reply")?;
        Ok(())
    }

    fn claude_payload(request_id: &str, allow: bool, input: Value) -> Value {
        if allow {
            serde_json::json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": request_id,
                    "response": {
                        "behavior": "allow",
                        "updatedInput": input
                    }
                }
            })
        } else {
            serde_json::json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": request_id,
                    "response": {
                        "behavior": "deny",
                        "message": "User denied this action"
                    }
                }
            })
        }
    }

    fn codex_payload(request_id: &str, allow: bool, kind: &CodexReplyKind) -> Value {
        let id: Value = request_id
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(request_id.to_string()));
        let result = match kind {
            CodexReplyKind::Decision => {
                let decision = if allow { "accept" } else { "decline" };
                serde_json::json!({ "decision": decision })
            }
            CodexReplyKind::Elicitation => {
                if allow {
                    serde_json::json!({ "action": "accept", "content": {} })
                } else {
                    serde_json::json!({ "action": "decline", "content": null })
                }
            }
            CodexReplyKind::Permissions { requested } => {
                if allow {
                    serde_json::json!({
                        "decision": "accept",
                        "permissions": requested,
                        "scope": "turn"
                    })
                } else {
                    serde_json::json!({
                        "decision": "decline",
                        "permissions": {},
                        "scope": "turn"
                    })
                }
            }
        };
        serde_json::json!({ "id": id, "result": result })
    }

    /// Allow or deny a pending permission request (Claude control or Codex RPC).
    pub fn respond_permission(&self, request_id: &str, allow: bool) -> Result<()> {
        let request_id = request_id.trim();
        if request_id.is_empty() {
            bail!("request_id is required");
        }

        let payload = match self.take_pending(request_id) {
            Some(PendingReply::CodexRpc { kind }) => Self::codex_payload(request_id, allow, &kind),
            Some(PendingReply::ClaudeTool { input }) => {
                Self::claude_payload(request_id, allow, input)
            }
            None => {
                // No remembered pending yet (race / stale id). Prefer Claude shape for
                // non-numeric ids; for numeric ids wait until remember_codex_reply.
                if request_id.chars().all(|c| c.is_ascii_digit()) {
                    bail!("no pending Codex approval for request_id={request_id}");
                }
                Self::claude_payload(request_id, allow, serde_json::json!({}))
            }
        };
        self.write_line(&payload.to_string())
    }

    pub(crate) fn auto_ack_claude_control(&self, request_id: &str) -> Result<()> {
        let payload = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id
            }
        });
        self.write_line(&payload.to_string())
    }

    /// Decline an unexpected Codex server request without UI.
    pub(crate) fn decline_codex_rpc(&self, request_id: &Value) -> Result<()> {
        let payload = serde_json::json!({
            "id": request_id,
            "result": { "decision": "decline" }
        });
        self.write_line(&payload.to_string())
    }

    pub(crate) fn close(&self) {
        if let Ok(mut guard) = self.stdin.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.pending.lock() {
            guard.clear();
        }
    }
}
