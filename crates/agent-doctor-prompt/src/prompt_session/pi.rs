//! Pi ask backend (`pi --mode json`).
//!
//! JSON mode writes one JSON object per line on stdout and exits when the
//! prompt is done. Diagnostics stay on stderr. When Agent Doctor has already
//! written the `agent-doctor` provider, the turn names it. A Pi that only has
//! its own login is left on that login.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::Value;

use super::backend::AskBackend;
use super::control::PromptSessionControl;
use super::tool_step::{tool_step, tool_step_result};
use super::util::{combine_output, command_from_cli, format_tool_status, summarize};
use super::{
    next_session_id, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, PromptSessionStatus, MAX_TIMEOUT_SEC, MIN_TIMEOUT_SEC,
};
use agent_doctor_core::session_env::{apply_overlay_env, format_command_display};
use agent_doctor_core::session_launch::resolve_session_cwd;
use agent_doctor_core::setup::merge::{
    apply_pi_with_model, pi_saved_provider_ready, PI_PROVIDER_ID,
};
use agent_doctor_core::setup::{
    COMPANY_DEFAULT_MODEL, GATEWAY_URL_ENV, MODEL_ENV, PROTOCOL_ANTHROPIC, PROTOCOL_OPENAI,
    PROVIDER_PROTOCOL_ENV,
};

/// Shown when Pi exits without a model it can call. The raw notice names `/login` and a docs path.
const PI_NEEDS_PROVIDER: &str =
    "Pi 还没有可用的服务商。先在服务商里选一个已填好的，或在 Pi 里登录。";

pub struct PiAskBackend;

impl AskBackend for PiAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        _control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_pi(options, cancel, on_event)
    }
}

fn run_pi(
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<PromptSessionReport> {
    let session_id = next_session_id();
    let runtime = "pi".to_string();
    let prompt = options.prompt.trim();
    if prompt.is_empty() {
        anyhow::bail!("prompt must not be empty");
    }
    let cwd = resolve_session_cwd(options.cwd.as_deref(), options.workspace_name.as_deref());
    if !cwd.exists() {
        anyhow::bail!("session cwd does not exist: {}", cwd.display());
    }
    let timeout_sec = options.timeout_sec.clamp(MIN_TIMEOUT_SEC, MAX_TIMEOUT_SEC);
    let resume = options
        .resume_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let overlay = super::collect_overlay_env_for_options(options);
    let saved = pi_turn_target(&overlay, options.model.as_deref());
    let mut slot_ready = pi_saved_provider_ready();
    let mut model = options.model.clone();
    if let Some(target) = &saved {
        if apply_pi_with_model(
            &target.url,
            &target.key,
            &target.protocol,
            Some(&target.model),
        )
        .ok()
        .is_some_and(|result| result.applied)
        {
            slot_ready = true;
            model = Some(target.model.clone());
        }
    }
    let bin = std::env::var("AGENT_DOCTOR_PI_BIN").unwrap_or_else(|_| "pi".into());
    let mut cmd = build_pi_command(
        &bin,
        prompt,
        &cwd,
        resume,
        model.as_deref(),
        slot_ready,
        &options.image_paths,
    );
    apply_overlay_env(&mut cmd, &overlay);
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
        .context("Pi 没有启动。到工具页安装 Pi 后再试。")?;
    let mut pump = PiPump::default();
    let result = {
        let mut emit = |event| on_event(event);
        super::pump::pump_oneshot(
            &session_id,
            &mut child,
            timeout_sec,
            cancel.handle(),
            "failed waiting for pi ask",
            &mut pump,
            &mut emit,
        )?
    };
    let (mut status, exit_code, stdout, stderr, timeout) = result;
    let duration_ms = started.elapsed().as_millis() as u64;
    // Pi exits 0 when the model call fails. The reason is on the assistant
    // message (`stopReason: error`), not the process status.
    if pump.error.is_some() {
        status = PromptSessionStatus::Failed;
    }
    let failed = status != PromptSessionStatus::Succeeded;
    let reply = pump.text;
    let runtime_thread_id = pump
        .session_id
        .clone()
        .or_else(|| resume.map(str::to_string));
    let reported = pump.error.clone().or_else(|| {
        let stderr = stderr.trim();
        (!stderr.is_empty() && failed).then(|| stderr.to_string())
    });
    let plain = reported.as_deref().and_then(pi_plain_failure);
    let error = if let Some(sentence) = plain {
        Some((sentence.to_string(), true))
    } else {
        reported.map(|message| (message, false))
    };
    if let Some((message, plain)) = &error {
        on_event(PromptSessionEvent::TurnError {
            session_id: session_id.clone(),
            message: message.clone(),
            plain: *plain,
        });
    }
    let combined = match &error {
        Some((text, _)) => text.clone(),
        None if !reply.trim().is_empty() => reply.clone(),
        None if failed => combine_output(&stdout, &stderr),
        None => String::new(),
    };
    let summary = summarize(&combined, &status, &runtime);
    on_event(PromptSessionEvent::Completed {
        session_id: session_id.clone(),
        status: status.clone(),
        exit_code,
        summary: summary.clone(),
        timeout,
    });
    Ok(PromptSessionReport {
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
    })
}

pub(crate) fn build_pi_command(
    bin: &str,
    prompt: &str,
    cwd: &Path,
    resume: Option<&str>,
    model: Option<&str>,
    slot_ready: bool,
    images: &[std::path::PathBuf],
) -> Command {
    let mut cmd = command_from_cli(bin);
    cmd.current_dir(cwd)
        .arg("--mode")
        .arg("json")
        .arg("--approve");
    if let Some(id) = resume.map(str::trim).filter(|id| !id.is_empty()) {
        cmd.arg("--session").arg(id);
    }
    if slot_ready {
        cmd.arg("--provider").arg(PI_PROVIDER_ID);
        if let Some(model) = model
            .map(str::trim)
            .filter(|id| !id.is_empty() && !id.eq_ignore_ascii_case("auto"))
        {
            cmd.arg("--model").arg(model);
        }
    }
    for image in images {
        if image.is_file() {
            cmd.arg(format!("@{}", image.display()));
        }
    }
    cmd.arg(prompt)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

#[derive(Default)]
struct PiPump {
    text: String,
    session_id: Option<String>,
    error: Option<String>,
}

impl super::pump::OneShotHandler for PiPump {
    fn on_stdout(
        &mut self,
        session_id: &str,
        line: &str,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return;
        };
        apply_pi_event(self, session_id, &value, on_event);
    }
}

fn apply_pi_event(
    pump: &mut PiPump,
    session_id: &str,
    value: &Value,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) {
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "session" => {
            if let Some(id) = session_id_of(value) {
                pump.session_id = Some(id);
            }
        }
        "message_end" | "turn_end" => {
            note_pi_message(pump, session_id, value.get("message"), on_event)
        }
        "auto_retry_end" => {
            let success = value
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            if !success {
                if let Some(message) = text_field(value, &["finalError", "errorMessage"]) {
                    pump.error = Some(message);
                }
            }
        }
        "message_update" => {
            let event = value
                .get("assistantMessageEvent")
                .or_else(|| value.get("event"));
            let event_type = event
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let delta = event
                .and_then(|item| item.get("delta"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if delta.is_empty() {
                return;
            }
            match event_type {
                "text_delta" => {
                    pump.text.push_str(delta);
                    on_event(PromptSessionEvent::Delta {
                        session_id: session_id.to_string(),
                        text: delta.to_string(),
                    });
                }
                "thinking_delta" => on_event(PromptSessionEvent::Thinking {
                    session_id: session_id.to_string(),
                    text: delta.to_string(),
                }),
                _ => {}
            }
        }
        "tool_execution_start" | "toolcall_start" => {
            let id = text_field(value, &["toolCallId", "id"]).unwrap_or_else(|| "tool".into());
            let name = text_field(value, &["toolName", "name"]).unwrap_or_else(|| "tool".into());
            let args = value.get("args").cloned().unwrap_or(Value::Null);
            let step = tool_step(&id, &name, &args);
            on_event(PromptSessionEvent::Status {
                session_id: session_id.to_string(),
                phase: "tool".into(),
                message: format_tool_status(&name, ""),
            });
            on_event(PromptSessionEvent::Tool {
                session_id: session_id.to_string(),
                step: Box::new(step),
            });
        }
        "tool_execution_end" | "toolcall_end" => {
            let id = text_field(value, &["toolCallId", "id"]).unwrap_or_else(|| "tool".into());
            let failed = value
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let output = value.get("result").map(short_result).unwrap_or_default();
            on_event(PromptSessionEvent::Tool {
                session_id: session_id.to_string(),
                step: Box::new(tool_step_result(&id, &output, failed)),
            });
        }
        "agent_end" | "agent_settled" => {
            let aborted = value
                .get("aborted")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if aborted && pump.error.is_none() {
                pump.error = Some("这次没有答完。".into());
            }
            if let Some(message) = text_field(value, &["error", "message"]) {
                pump.error = Some(message);
            }
        }
        "error" => {
            if let Some(message) = text_field(value, &["message", "error"]) {
                pump.error = Some(message);
            }
        }
        _ => {}
    }
}

fn note_pi_message(
    pump: &mut PiPump,
    session_id: &str,
    message: Option<&Value>,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) {
    let Some(message) = message else {
        return;
    };
    let role = message.get("role").and_then(Value::as_str).unwrap_or("");
    if role != "assistant" {
        return;
    }
    if let Some(err) = message
        .get("errorMessage")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        pump.error = Some(err.to_string());
    } else if matches!(
        message.get("stopReason").and_then(Value::as_str),
        Some("error" | "aborted")
    ) && pump.error.is_none()
    {
        pump.error = Some("Pi 这一轮没有答完。".into());
    }
    if !pump.text.trim().is_empty() {
        return;
    }
    let text = assistant_text(message);
    if text.trim().is_empty() {
        return;
    }
    pump.text = text.clone();
    on_event(PromptSessionEvent::Delta {
        session_id: session_id.to_string(),
        text,
    });
}

struct PiTurnTarget {
    url: String,
    key: String,
    protocol: String,
    model: String,
}

/// Saved provider for this chat, when one is already filled in.
fn pi_turn_target(
    overlay: &std::collections::HashMap<String, String>,
    model: Option<&str>,
) -> Option<PiTurnTarget> {
    let anthropic = overlay
        .get(PROVIDER_PROTOCOL_ENV)
        .is_some_and(|protocol| protocol.eq_ignore_ascii_case(PROTOCOL_ANTHROPIC));
    let url = if anthropic {
        overlay
            .get("ANTHROPIC_BASE_URL")
            .or_else(|| overlay.get(GATEWAY_URL_ENV))
    } else {
        overlay
            .get("OPENAI_BASE_URL")
            .or_else(|| overlay.get(GATEWAY_URL_ENV))
    }?;
    let key = overlay
        .get("OPENAI_API_KEY")
        .or_else(|| overlay.get("ANTHROPIC_API_KEY"))
        .or_else(|| overlay.get("AGENT_DOCTOR_COMPANY_API_KEY"))?;
    let url = url.trim();
    let key = key.trim();
    if url.is_empty() || key.is_empty() {
        return None;
    }
    let model = model
        .map(str::trim)
        .filter(|id| !id.is_empty() && !id.eq_ignore_ascii_case("auto"))
        .map(str::to_string)
        .or_else(|| {
            overlay
                .get(MODEL_ENV)
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
        })
        .unwrap_or_else(|| COMPANY_DEFAULT_MODEL.to_string());
    Some(PiTurnTarget {
        url: url.to_string(),
        key: key.to_string(),
        protocol: if anthropic {
            PROTOCOL_ANTHROPIC
        } else {
            PROTOCOL_OPENAI
        }
        .to_string(),
        model,
    })
}

fn assistant_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Pi's own "no key / no model" notice. Other failures stay in the runtime's words
/// so the chat can classify a bad address or a rejected key.
fn pi_plain_failure(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("no api key")
        || lower.contains("no models available")
        || lower.contains("use /login")
    {
        Some(PI_NEEDS_PROVIDER)
    } else {
        None
    }
}

fn session_id_of(value: &Value) -> Option<String> {
    text_field(value, &["id", "sessionId", "session_id"]).or_else(|| {
        value
            .get("session")
            .and_then(|session| text_field(session, &["id", "sessionId"]))
    })
}

fn text_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

fn short_result(value: &Value) -> String {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Object(map) => ["output", "text", "message"]
            .iter()
            .find_map(|key| map.get(*key).and_then(Value::as_str))
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    };
    let trimmed = text.trim();
    if trimmed.chars().count() <= 500 {
        return trimmed.to_string();
    }
    trimmed.chars().take(500).collect()
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::super::pump::OneShotHandler;
    use super::{
        build_pi_command, pi_plain_failure, pi_turn_target, PiPump, PromptSessionEvent,
        PI_NEEDS_PROVIDER,
    };
    use agent_doctor_core::setup::{MODEL_ENV, PROTOCOL_OPENAI};

    fn argv(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn command_names_the_saved_provider_and_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = build_pi_command(
            "pi",
            "看一下这个项目",
            dir.path(),
            Some("sess-1"),
            Some("deepseek-v4-flash"),
            true,
            &[],
        );
        assert_eq!(
            argv(&cmd),
            vec![
                "--mode",
                "json",
                "--approve",
                "--session",
                "sess-1",
                "--provider",
                "agent-doctor",
                "--model",
                "deepseek-v4-flash",
                "看一下这个项目",
            ]
        );
    }

    #[test]
    fn own_login_is_not_overridden() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = build_pi_command("pi", "hi", dir.path(), None, Some("gpt-4o"), false, &[]);
        let args = argv(&cmd);
        assert!(!args
            .iter()
            .any(|arg| arg == "--provider" || arg == "--model"));
        assert_eq!(args.last().map(String::as_str), Some("hi"));
    }

    #[test]
    fn json_events_stream_text_tools_and_session() {
        let mut pump = PiPump::default();
        let mut events = Vec::new();
        let mut emit = |event| events.push(event);
        for line in [
            r#"{"type":"session","id":"abc"}"#,
            r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","delta":"想"}}"#,
            r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"你好"}}"#,
            r#"{"type":"tool_execution_start","toolCallId":"t1","toolName":"bash","args":{"command":"ls"}}"#,
            r#"{"type":"tool_execution_end","toolCallId":"t1","toolName":"bash","isError":false,"result":"ok"}"#,
        ] {
            OneShotHandler::on_stdout(&mut pump, "s", line, &mut emit);
        }
        assert_eq!(pump.session_id.as_deref(), Some("abc"));
        assert_eq!(pump.text, "你好");
        assert!(events.iter().any(|event| matches!(
            event,
            PromptSessionEvent::Thinking { text, .. } if text == "想"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            PromptSessionEvent::Delta { text, .. } if text == "你好"
        )));
        assert!(events
            .iter()
            .any(|event| matches!(event, PromptSessionEvent::Tool { .. })));
    }

    #[test]
    fn message_end_keeps_the_reply_and_the_model_error() {
        let mut pump = PiPump::default();
        let mut events = Vec::new();
        let mut emit = |event| events.push(event);
        OneShotHandler::on_stdout(
            &mut pump,
            "s",
            r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"你好"}],"stopReason":"stop"}}"#,
            &mut emit,
        );
        assert_eq!(pump.text, "你好");
        assert!(pump.error.is_none());

        let mut pump = PiPump::default();
        OneShotHandler::on_stdout(
            &mut pump,
            "s",
            r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"Connection error.: getaddrinfo ENOTFOUND example.invalid"}}"#,
            &mut emit,
        );
        assert_eq!(
            pump.error.as_deref(),
            Some("Connection error.: getaddrinfo ENOTFOUND example.invalid")
        );
        assert!(pi_plain_failure(pump.error.as_deref().unwrap_or("")).is_none());
    }

    #[test]
    fn missing_provider_is_a_plain_sentence() {
        let notice = "No API key found for the selected model.\nUse /login to log into a provider.";
        assert_eq!(pi_plain_failure(notice), Some(PI_NEEDS_PROVIDER));
    }

    #[test]
    fn saved_provider_becomes_the_pi_target() {
        let mut overlay = std::collections::HashMap::new();
        overlay.insert(
            "OPENAI_BASE_URL".into(),
            "https://gateway.example/v1".into(),
        );
        overlay.insert("OPENAI_API_KEY".into(), "sk-test".into());
        overlay.insert(MODEL_ENV.into(), "demo-model".into());
        let target = pi_turn_target(&overlay, None).unwrap();
        assert_eq!(target.url, "https://gateway.example/v1");
        assert_eq!(target.model, "demo-model");
        assert_eq!(target.protocol, PROTOCOL_OPENAI);
        assert!(pi_turn_target(&std::collections::HashMap::new(), None).is_none());
    }
}
