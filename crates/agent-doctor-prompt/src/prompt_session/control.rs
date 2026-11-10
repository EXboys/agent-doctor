use std::collections::HashMap;
use std::io::Write;
use std::process::ChildStdin;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};
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
    /// ACP `session/request_permission`: answer with the exact optionId the
    /// request offered, mapped from the desktop's allow/deny answer.
    AcpPermission { allow_id: String, deny_id: String },
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

/// Why a control send failed. `stdin-closed` and `dead-pipe` are final — the
/// reply can never reach the backend process, so no retry helps. `transient`
/// may succeed on another try. The class travels inside the error text as
/// `[ad-send=<class>]` because the error crosses to the UI as a plain string.
pub fn send_error_class(error: &anyhow::Error) -> &'static str {
    let chain = format!("{error:#}");
    if chain.contains("ad-send=stdin-closed") {
        "stdin-closed"
    } else if chain.contains("ad-send=dead-pipe") {
        "dead-pipe"
    } else if chain.contains("no active ask session") {
        "session-gone"
    } else {
        "transient"
    }
}

fn send_io_error(what: &'static str, err: std::io::Error) -> anyhow::Error {
    anyhow::anyhow!("{what}: {err} [ad-send={}]", send_io_class(&err))
}

fn send_io_class(err: &std::io::Error) -> &'static str {
    match err.kind() {
        std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::UnexpectedEof
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted => "dead-pipe",
        _ => match err.raw_os_error() {
            // Windows reports a closed pipe as ERROR_BROKEN_PIPE (109) or
            // ERROR_NO_DATA (232), often without a mapped ErrorKind.
            Some(109) | Some(232) => "dead-pipe",
            _ => "transient",
        },
    }
}

/// Bidirectional control channel for Ask permission prompts (Claude or Codex,
/// or the built-in doctor agent's in-process confirmations).
#[derive(Clone, Default)]
pub struct PromptSessionControl {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pending: Arc<Mutex<HashMap<String, PendingReply>>>,
    /// Built-in doctor agent: request_id → one-shot allow/deny sender. The
    /// loop blocks on the receiver; `respond_permission` delivers the verdict.
    doctor: Arc<Mutex<HashMap<String, std::sync::mpsc::SyncSender<bool>>>>,
}

impl PromptSessionControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm one built-in doctor confirmation. The loop blocks on the returned
    /// receiver until `respond_permission` delivers the user's verdict.
    pub fn register_doctor_confirmation(
        &self,
        request_id: &str,
    ) -> std::sync::mpsc::Receiver<bool> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        if let Ok(mut guard) = self.doctor.lock() {
            guard.insert(request_id.to_string(), sender);
        }
        receiver
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

    /// Remember which ACP option ids this request maps to for allow/deny.
    pub(crate) fn remember_acp_permission(
        &self,
        request_id: &str,
        allow_id: String,
        deny_id: String,
    ) {
        if let Ok(mut guard) = self.pending.lock() {
            guard.insert(
                request_id.to_string(),
                PendingReply::AcpPermission { allow_id, deny_id },
            );
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
        let stdin = guard.as_mut().ok_or_else(|| {
            anyhow::anyhow!(
                "ask session has no open stdin for permission replies [ad-send=stdin-closed]"
            )
        })?;
        if let Err(err) = writeln!(stdin, "{line}") {
            return Err(send_io_error("write permission reply", err));
        }
        if let Err(err) = stdin.flush() {
            return Err(send_io_error("flush permission reply", err));
        }
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

    /// ACP reply line carrying the exact `optionId` the request offered.
    pub fn acp_permission_line(request_id: &str, option_id: &str) -> String {
        Self::acp_permission_payload(request_id, option_id).to_string()
    }

    fn acp_permission_payload(request_id: &str, option_id: &str) -> Value {
        let id: Value = request_id
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(request_id.to_string()));
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

        if let Ok(mut guard) = self.doctor.lock() {
            if let Some(sender) = guard.remove(request_id) {
                let _ = sender.send(allow);
                return Ok(());
            }
        }

        let (payload, consumed) = self.compose_permission(request_id, allow)?;
        if let Err(err) = self.write_line(&payload.to_string()) {
            // The ask goes back, so a retry builds the same reply with the real
            // input instead of an empty `updatedInput`.
            if let Some(reply) = consumed {
                self.restore_pending(request_id, reply);
            }
            return Err(err);
        }
        Ok(())
    }

    /// Build the reply line for one pending ask, consuming its entry. Returns
    /// the entry so a failed write can put it back (`None` for an unknown id —
    /// the Claude fallback has nothing to restore).
    fn compose_permission(
        &self,
        request_id: &str,
        allow: bool,
    ) -> Result<(Value, Option<PendingReply>)> {
        let Some(reply) = self.take_pending(request_id) else {
            // No remembered pending yet (race / stale id). Prefer Claude shape for
            // non-numeric ids; for numeric ids wait until remember_codex_reply.
            if request_id.chars().all(|c| c.is_ascii_digit()) {
                bail!("no pending Codex approval for request_id={request_id}");
            }
            return Ok((
                Self::claude_payload(request_id, allow, serde_json::json!({})),
                None,
            ));
        };
        let payload = match &reply {
            PendingReply::CodexRpc { kind } => Self::codex_payload(request_id, allow, kind),
            PendingReply::ClaudeTool { input } => {
                Self::claude_payload(request_id, allow, input.clone())
            }
            PendingReply::ClaudeAsk { .. } => Self::claude_payload(request_id, false, Value::Null),
            PendingReply::AcpPermission { allow_id, deny_id } => {
                let option_id = if allow { allow_id } else { deny_id };
                Self::acp_permission_payload(request_id, option_id)
            }
        };
        Ok((payload, Some(reply)))
    }

    /// Put one consumed pending entry back so a retry builds the same reply.
    /// `or_insert`: a freshly re-armed ask with the same id must not be lost.
    fn restore_pending(&self, request_id: &str, reply: PendingReply) {
        if let Ok(mut guard) = self.pending.lock() {
            guard.entry(request_id.to_string()).or_insert(reply);
        }
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
        let Some(reply) = self.take_pending(request_id) else {
            bail!("no pending text prompt for request_id={request_id}");
        };
        let is_text_prompt = matches!(
            &reply,
            PendingReply::ClaudeAsk { .. }
                | PendingReply::CodexRpc {
                    kind: CodexReplyKind::UserLine { .. }
                }
        );
        if !is_text_prompt {
            // Wrong kind for a typed reply — put the ask back so it can still
            // wait for its allow instead of silently dying here.
            self.restore_pending(request_id, reply);
            bail!("no pending text prompt for request_id={request_id}");
        }
        let payload = match &reply {
            PendingReply::CodexRpc {
                kind:
                    CodexReplyKind::UserLine {
                        question_id,
                        elicitation,
                        questions,
                    },
            } => {
                let id: Value = request_id
                    .parse::<i64>()
                    .map(Value::from)
                    .unwrap_or_else(|_| Value::String(request_id.to_string()));
                let result = if *elicitation {
                    serde_json::json!({
                        "action": "accept",
                        "content": { question_id: text }
                    })
                } else {
                    codex_user_input_result(questions, question_id, text)
                };
                serde_json::json!({ "id": id, "result": result })
            }
            PendingReply::ClaudeAsk { input } => {
                let updated = claude_ask_updated_input(input, text);
                Self::claude_payload(request_id, true, updated)
            }
            _ => unreachable!("is_text_prompt filtered every other kind"),
        };
        if let Err(err) = self.write_line(&payload.to_string()) {
            self.restore_pending(request_id, reply);
            return Err(err);
        }
        Ok(())
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
    use super::{claude_ask_updated_input, send_error_class, send_io_class, PromptSessionControl};
    use serde_json::json;

    #[test]
    fn send_error_class_reads_the_marker_from_the_chain() {
        let dead = anyhow::anyhow!(
            "write permission reply: Broken pipe (os error 32) [ad-send=dead-pipe]"
        );
        assert_eq!(send_error_class(&dead), "dead-pipe");
        let gone = anyhow::anyhow!("no active ask session for permission reply");
        assert_eq!(send_error_class(&gone), "session-gone");
        assert_eq!(send_error_class(&anyhow::anyhow!("boom")), "transient");
    }

    #[test]
    fn a_control_without_stdin_reports_it_as_closed() {
        let control = super::PromptSessionControl::new();
        let err = control.write_line("{}").expect_err("no stdin attached");
        assert_eq!(send_error_class(&err), "stdin-closed");
    }

    #[test]
    fn broken_pipe_codes_classify_dead() {
        assert_eq!(
            send_io_class(&std::io::Error::from_raw_os_error(109)),
            "dead-pipe"
        );
        assert_eq!(
            send_io_class(&std::io::Error::from_raw_os_error(232)),
            "dead-pipe"
        );
        let kind = std::io::Error::from(std::io::ErrorKind::BrokenPipe);
        assert_eq!(send_io_class(&kind), "dead-pipe");
        let other = std::io::Error::other("slow disk");
        assert_eq!(send_io_class(&other), "transient");
    }

    #[test]
    fn a_failed_allow_keeps_the_ask_and_retries_with_the_real_input() {
        let control = PromptSessionControl::new();
        let input = json!({"command": "echo hi"});
        control.remember_claude_tool_input("req-1", input.clone());

        // No stdin attached: the send fails, but the ask must survive it.
        let err = control
            .respond_permission("req-1", true)
            .expect_err("no stdin");
        assert_eq!(send_error_class(&err), "stdin-closed");
        assert!(control.has_pending(), "failed send must put the ask back");

        // The retry builds the very same payload, carrying the original input.
        let (line, consumed) = control.compose_permission("req-1", true).unwrap();
        assert_eq!(line["response"]["response"]["behavior"], "allow");
        assert_eq!(line["response"]["response"]["updatedInput"], input);
        assert!(consumed.is_some());
        // And putting it back again keeps the round-trip stable.
        control.restore_pending("req-1", consumed.unwrap());
        let (again, _) = control.compose_permission("req-1", true).unwrap();
        assert_eq!(line, again);
    }

    #[test]
    fn a_failed_line_reply_restores_the_question() {
        let control = PromptSessionControl::new();
        control.remember_claude_question(
            "req-2",
            json!({"questions": [{"question": "用哪种登录？"}]}),
        );

        let err = control
            .respond_line("req-2", "浏览器")
            .expect_err("no stdin");
        assert_eq!(send_error_class(&err), "stdin-closed");
        assert!(
            control.has_pending(),
            "failed reply must put the question back"
        );

        let updated = claude_ask_updated_input(
            &json!({"questions": [{"question": "用哪种登录？"}]}),
            "浏览器",
        );
        assert_eq!(updated["answers"]["用哪种登录？"], "浏览器");
    }

    #[test]
    fn a_line_reply_to_a_tool_ask_leaves_the_ask_intact() {
        let control = PromptSessionControl::new();
        control.remember_claude_tool_input("req-3", json!({"command": "ls"}));

        let err = control
            .respond_line("req-3", "hello")
            .expect_err("tool ask is not a text prompt");
        assert!(err.to_string().contains("no pending text prompt"));
        assert!(control.has_pending(), "the tool ask must keep waiting");
    }

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
