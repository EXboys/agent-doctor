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
    /// Claude `AskUserQuestion`: the answer is written into `updatedInput.answers`.
    ClaudeAsk { input: Value },
    /// Codex app-server JSON-RPC result (shape depends on the request method).
    CodexRpc { kind: CodexReplyKind },
    /// DeepSeek ACP `session/request_permission`: allow-once or reject-once.
    DshPermission,
}

#[derive(Debug, Clone)]
pub(crate) enum CodexReplyKind {
    /// `item/*/requestApproval` → `{ decision: accept|decline }`.
    Decision,
    /// `mcpServer/elicitation/request` → `{ action, content }`.
    Elicitation,
    /// `item/permissions/requestApproval` → `{ permissions, scope }`.
    Permissions { requested: Value },
    /// One line of text the user typed. Not an allow/deny choice.
    UserLine {
        question_id: String,
        elicitation: bool,
        questions: Value,
    },
}

/// Codex `request_user_input` answers are `{ id: { answers: [string] } }`, not a bare string.
/// A chosen option is the label. Typed text is prefixed `user_note:`, matching the Codex client.
pub(crate) fn codex_user_input_result(questions: &Value, fallback_id: &str, text: &str) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(map) = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|parsed| parsed.get("answers").and_then(|v| v.as_object()).cloned())
    {
        for (key, value) in map {
            let answer = value.as_str().unwrap_or("").trim();
            if answer.is_empty() {
                continue;
            }
            let id = resolve_question_id(questions, &key).unwrap_or(key);
            out.insert(id.clone(), answer_entry(questions, &id, answer));
        }
    }
    if out.is_empty() {
        let id = fallback_id.to_string();
        out.insert(id.clone(), answer_entry(questions, &id, text));
    }
    serde_json::json!({ "answers": out })
}

fn answer_entry(questions: &Value, id: &str, text: &str) -> Value {
    let value = if matches_option_label(questions, id, text) || text.starts_with("user_note:") {
        text.to_string()
    } else {
        format!("user_note: {text}")
    };
    serde_json::json!({ "answers": [value] })
}

fn resolve_question_id(questions: &Value, key: &str) -> Option<String> {
    let list = questions.as_array()?;
    if list
        .iter()
        .any(|q| q.get("id").and_then(|v| v.as_str()) == Some(key))
    {
        return Some(key.to_string());
    }
    list.iter().find_map(|q| {
        let question = q.get("question").and_then(|v| v.as_str())?;
        if question == key {
            q.get("id").and_then(|v| v.as_str()).map(str::to_string)
        } else {
            None
        }
    })
}

fn matches_option_label(questions: &Value, id: &str, text: &str) -> bool {
    questions
        .as_array()
        .and_then(|list| {
            list.iter()
                .find(|q| q.get("id").and_then(|v| v.as_str()) == Some(id))
        })
        .and_then(|q| q.get("options").and_then(|v| v.as_array()))
        .is_some_and(|options| {
            options
                .iter()
                .any(|option| option.get("label").and_then(|v| v.as_str()) == Some(text))
        })
}

/// Build the `updatedInput` Claude expects for `AskUserQuestion`.
/// `text` is either one answer, or JSON `{"answers": { question: answer }}`.
pub(crate) fn claude_ask_updated_input(input: &Value, text: &str) -> Value {
    let questions = input
        .get("questions")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    if let Ok(parsed) = serde_json::from_str::<Value>(text) {
        if let Some(answers) = parsed.get("answers").filter(|v| v.is_object()) {
            return serde_json::json!({ "questions": questions, "answers": answers });
        }
    }
    let question = input
        .pointer("/questions/0/question")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("value");
    serde_json::json!({
        "questions": questions,
        "answers": { question: text }
    })
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

    pub(crate) fn remember_claude_question(&self, request_id: &str, input: Value) {
        if let Ok(mut guard) = self.pending.lock() {
            guard.insert(request_id.to_string(), PendingReply::ClaudeAsk { input });
        }
    }

    pub(crate) fn remember_claude_tool_input(&self, request_id: &str, input: Value) {
        if let Ok(mut guard) = self.pending.lock() {
            guard.insert(request_id.to_string(), PendingReply::ClaudeTool { input });
        }
    }

    pub(crate) fn remember_dsh_permission(&self, request_id: &str) {
        if let Ok(mut guard) = self.pending.lock() {
            guard.insert(request_id.to_string(), PendingReply::DshPermission);
        }
    }

    pub(crate) fn remember_codex_reply(&self, request_id: &str, kind: CodexReplyKind) {
        if let Ok(mut guard) = self.pending.lock() {
            guard.insert(request_id.to_string(), PendingReply::CodexRpc { kind });
        }
    }

    pub(crate) fn has_pending(&self) -> bool {
        self.pending
            .lock()
            .map(|guard| !guard.is_empty())
            .unwrap_or(false)
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
            CodexReplyKind::UserLine { .. } => {
                serde_json::json!({ "action": "decline", "content": null })
            }
        };
        serde_json::json!({ "id": id, "result": result })
    }

    pub(crate) fn dsh_permission_line(request_id: &str, allow: bool) -> String {
        Self::dsh_permission_payload(request_id, allow).to_string()
    }

    fn dsh_permission_payload(request_id: &str, allow: bool) -> Value {
        let id: Value = request_id
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(request_id.to_string()));
        let option_id = if allow { "allow-once" } else { "reject-once" };
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "outcome": { "outcome": "selected", "optionId": option_id }
            }
        })
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
            Some(PendingReply::ClaudeAsk { .. }) => {
                Self::claude_payload(request_id, false, Value::Null)
            }
            Some(PendingReply::DshPermission) => Self::dsh_permission_payload(request_id, allow),
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

    /// Send one typed line to a prompt that is waiting for text.
    /// The caller must not store `text` in the chat transcript.
    pub fn respond_line(&self, request_id: &str, text: &str) -> Result<()> {
        let request_id = request_id.trim();
        let text = text.trim();
        if request_id.is_empty() {
            bail!("request_id is required");
        }
        if text.is_empty() {
            bail!("reply text is required");
        }
        match self.take_pending(request_id) {
            Some(PendingReply::CodexRpc {
                kind:
                    CodexReplyKind::UserLine {
                        question_id,
                        elicitation,
                        questions,
                    },
            }) => {
                let id: Value = request_id
                    .parse::<i64>()
                    .map(Value::from)
                    .unwrap_or_else(|_| Value::String(request_id.to_string()));
                let result = if elicitation {
                    serde_json::json!({
                        "action": "accept",
                        "content": { question_id: text }
                    })
                } else {
                    codex_user_input_result(&questions, &question_id, text)
                };
                let payload = serde_json::json!({ "id": id, "result": result });
                self.write_line(&payload.to_string())
            }
            Some(PendingReply::ClaudeAsk { input }) => {
                let updated = claude_ask_updated_input(&input, text);
                self.write_line(&Self::claude_payload(request_id, true, updated).to_string())
            }
            _ => bail!("no pending text prompt for request_id={request_id}"),
        }
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

    /// Hand stdin back without closing it, so the process can serve another turn.
    pub(crate) fn detach_stdin(&self) -> Option<ChildStdin> {
        if let Ok(mut guard) = self.pending.lock() {
            guard.clear();
        }
        self.stdin.lock().ok().and_then(|mut guard| guard.take())
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

#[cfg(test)]
mod tests {
    use super::claude_ask_updated_input;
    use serde_json::json;

    #[test]
    fn claude_ask_answer_uses_the_question_text_as_key() {
        let input =
            json!({"questions":[{"question":"用哪种登录？","options":[{"label":"浏览器"}]}]});
        let updated = claude_ask_updated_input(&input, "浏览器");
        assert_eq!(updated["answers"]["用哪种登录？"], "浏览器");
    }

    #[test]
    fn claude_ask_json_answers_pass_through() {
        let input = json!({"questions":[{"question":"甲"},{"question":"乙"}]});
        let updated = claude_ask_updated_input(&input, r#"{"answers":{"甲":"一","乙":"二"}}"#);
        assert_eq!(updated["answers"]["甲"], "一");
        assert_eq!(updated["answers"]["乙"], "二");
    }

    #[test]
    fn codex_typed_secret_is_an_answer_object() {
        let questions = json!([{
            "id": "api_key",
            "question": "请提供密钥",
            "isOther": true,
            "options": [{"label": "我在下方填写"}, {"label": "改用环境变量"}]
        }]);
        let result = super::codex_user_input_result(&questions, "api_key", "typed-secret");
        assert_eq!(
            result["answers"]["api_key"]["answers"][0],
            "user_note: typed-secret"
        );
    }

    #[test]
    fn codex_option_label_is_not_prefixed() {
        let questions = json!([{
            "id": "api_key",
            "question": "请提供密钥",
            "options": [{"label": "改用环境变量"}]
        }]);
        let result = super::codex_user_input_result(
            &questions,
            "api_key",
            r#"{"answers":{"请提供密钥":"改用环境变量"}}"#,
        );
        assert_eq!(result["answers"]["api_key"]["answers"][0], "改用环境变量");
    }
}
