//! DeepSeek Harness ask backend (`dsh --profile acp`).
//!
//! The headless profile answers one task and exits, and it cannot resume.
//! The shipped `acp` profile stays up and speaks Agent Client Protocol on
//! stdin/stdout: `session/new` or `session/resume`, then `session/prompt`.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::backend::AskBackend;
use super::control::PromptSessionControl;
use super::env::{
    apply_deepseek_harness_env, apply_overlay_env, collect_overlay_env_for_options,
    format_command_display,
};
use super::util::{
    command_from_cli, force_stop_child, is_runtime_stderr_noise, summarize, SessionClock,
};
use super::vision::{encode_image, turn_images, EncodedImage};
use super::warm::{self, LivePipes};
use super::{
    next_session_id, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, PromptSessionStatus, MAX_TIMEOUT_SEC, MIN_TIMEOUT_SEC,
};
use crate::adapters::{DEEPSEEK_HARNESS_CLI, DEEPSEEK_HARNESS_RUNTIME_ID};
use crate::session_launch::resolve_session_cwd;

const RUNTIME_KEY: &str = "deepseek-harness";

pub struct DeepSeekHarnessAskBackend;

impl AskBackend for DeepSeekHarnessAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_deepseek_harness(options, cancel, control, on_event)
    }
}

fn run_deepseek_harness(
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    control: Option<PromptSessionControl>,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<PromptSessionReport> {
    let session_id = next_session_id();
    let runtime = DEEPSEEK_HARNESS_RUNTIME_ID.to_string();
    let prompt = options.prompt.trim();
    if prompt.is_empty() {
        bail!("prompt must not be empty");
    }
    let cwd = resolve_session_cwd(options.cwd.as_deref(), options.workspace_name.as_deref());
    if !cwd.exists() {
        bail!("session cwd does not exist: {}", cwd.display());
    }
    let timeout_sec = options.timeout_sec.clamp(MIN_TIMEOUT_SEC, MAX_TIMEOUT_SEC);
    let resume_session_id = options
        .resume_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let auto_allow = options.dangerously_skip_permissions || options.full_auto;
    let overlay = collect_overlay_env_for_options(options);
    let images: Vec<EncodedImage> = turn_images(RUNTIME_KEY, &options.image_paths, &overlay)
        .iter()
        .filter_map(|p| encode_image(p))
        .collect();

    let mut cmd = build_command(&cwd, &overlay);
    let command_display = format_command_display(&cmd);
    let fingerprint = warm::enabled().then(|| warm::fingerprint(&cmd, &[]));

    on_event(PromptSessionEvent::Started {
        session_id: session_id.clone(),
        runtime: runtime.clone(),
        cwd: cwd.display().to_string(),
        command: command_display,
        client_run_id: String::new(),
    });

    let started = Instant::now();
    let writer = control.unwrap_or_default();
    let mut reused = None;
    // A parked process skipped `initialize`, so it cannot say whether it takes pictures.
    if let (Some(fp), Some(sid), true) =
        (fingerprint, resume_session_id.as_deref(), images.is_empty())
    {
        if let Some(parked) = warm::take(RUNTIME_KEY, sid, fp) {
            let warm::WarmProcess {
                child,
                stdin,
                pipes,
                ..
            } = parked;
            writer.close();
            writer.attach_stdin(stdin);
            reused = Some((child, pipes, sid.to_string()));
        }
    }

    let (mut child, mut pipes, already_open) = match reused {
        Some((child, pipes, sid)) => (child, pipes, Some(sid)),
        None => {
            let mut child = cmd.spawn().context("failed to spawn dsh")?;
            let pipes = match LivePipes::attach(&mut child) {
                Ok(pipes) => pipes,
                Err(err) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(err);
                }
            };
            if let Some(stdin) = child.stdin.take() {
                writer.close();
                writer.attach_stdin(stdin);
            }
            (child, pipes, None)
        }
    };

    let mut display = String::new();
    let result = {
        let mut emit = |event: PromptSessionEvent| {
            if let PromptSessionEvent::Delta { text, .. } = &event {
                display.push_str(text);
            }
            on_event(event);
        };
        pump_acp(
            &session_id,
            &cwd,
            &mut child,
            &mut pipes,
            timeout_sec,
            cancel.handle(),
            &writer,
            prompt,
            images,
            already_open,
            resume_session_id.as_deref(),
            auto_allow,
            &mut emit,
        )
    };

    let acp_session = result
        .as_ref()
        .ok()
        .and_then(|turn| turn.acp_session.clone());
    let park_as = match (&result, fingerprint) {
        (Ok(turn), Some(fp)) if turn.status == PromptSessionStatus::Succeeded => {
            turn.acp_session.clone().map(|sid| (sid, fp))
        }
        _ => None,
    };
    warm::keep_or_close(RUNTIME_KEY, park_as, child, pipes, &writer);

    let duration_ms = started.elapsed().as_millis() as u64;
    let report = match result {
        Ok(turn) => {
            let readable = if display.trim().is_empty() {
                turn.error.unwrap_or(turn.stderr)
            } else {
                display
            };
            let summary = summarize(&readable, &turn.status, &runtime);
            on_event(PromptSessionEvent::Completed {
                session_id: session_id.clone(),
                status: turn.status.clone(),
                exit_code: turn.exit_code,
                summary: summary.clone(),
                timeout: turn.timeout,
            });
            PromptSessionReport {
                usage: None,
                session_id,
                runtime,
                cwd: cwd.display().to_string(),
                status: turn.status,
                exit_code: turn.exit_code,
                summary,
                log_excerpt: readable,
                duration_ms,
                runtime_thread_id: acp_session,
            }
        }
        Err(error) => finish_failed(
            session_id,
            runtime,
            cwd.display().to_string(),
            duration_ms,
            acp_session,
            &error.to_string(),
            on_event,
        ),
    };
    Ok(report)
}

fn finish_failed(
    session_id: String,
    runtime: String,
    cwd: String,
    duration_ms: u64,
    acp_session: Option<String>,
    error: &str,
    emit: &mut dyn FnMut(PromptSessionEvent),
) -> PromptSessionReport {
    let summary = summarize(error, &PromptSessionStatus::Failed, &runtime);
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
        cwd,
        status: PromptSessionStatus::Failed,
        exit_code: None,
        summary,
        log_excerpt: error.to_string(),
        duration_ms,
        runtime_thread_id: acp_session,
    }
}

fn build_command(cwd: &Path, overlay: &std::collections::HashMap<String, String>) -> Command {
    let binary =
        std::env::var("AGENT_DOCTOR_DSH_BIN").unwrap_or_else(|_| DEEPSEEK_HARNESS_CLI.into());
    let mut command = command_from_cli(&binary);
    command
        .arg("--profile")
        .arg("acp")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_overlay_env(&mut command, overlay);
    apply_deepseek_harness_env(&mut command, overlay);
    command
}

struct TurnOutcome {
    status: PromptSessionStatus,
    exit_code: Option<i32>,
    stderr: String,
    error: Option<String>,
    timeout: Option<super::TimeoutNote>,
    acp_session: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Waiting {
    Initialize,
    Resume,
    NewSession,
    Prompt,
}

struct AcpState {
    next_id: i64,
    waiting: HashMap<i64, Waiting>,
    acp_session: Option<String>,
    prompt_done: bool,
    failed: Option<String>,
    resume_rejected: bool,
    /// Id we asked to resume. Real `dsh` omits `sessionId` on a successful resume.
    resume_target: Option<String>,
    /// toolCallId → (name, raw arguments). The permission request itself has no title.
    tools: HashMap<String, (String, String)>,
    images: Vec<EncodedImage>,
    /// `initialize` said `promptCapabilities.image`.
    takes_images: bool,
}

#[allow(clippy::too_many_arguments)]
fn pump_acp<F>(
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
        images,
        takes_images: false,
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
            Err(error) => return Err(error).context("failed waiting for dsh"),
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
fn prompt_blocks(prompt: &str, images: &[EncodedImage], takes_images: bool) -> Value {
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
            .unwrap_or("DeepSeek 这一轮没有完成")
            .to_string();
        if kind == Waiting::Resume {
            state.resume_rejected = true;
            on_event(PromptSessionEvent::Status {
                session_id: session_id.to_string(),
                phase: "session".into(),
                message: "上次对话接不上，正在新开一轮。".into(),
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
                    message: "这个 DeepSeek 助手收不了图片，这次只带上了图里的字。".into(),
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
                state.failed = Some("DeepSeek 没有返回这一轮的会话。".into());
            }
        }
        Waiting::Prompt => {
            let stop = result
                .get("stopReason")
                .and_then(|v| v.as_str())
                .unwrap_or("end_turn");
            if stop == "refusal" {
                state.failed = Some("DeepSeek 拒绝继续这一轮。".into());
            } else {
                state.prompt_done = true;
            }
            if let Some(usage) = crate::usage::find_usage(&result) {
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
    if auto_allow {
        let _ = control.write_line(&PromptSessionControl::dsh_permission_line(
            &request_id,
            true,
        ));
        return;
    }
    let params = message.get("params").cloned().unwrap_or(Value::Null);
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
        .unwrap_or("这一步操作");
    let detail = remembered
        .map(|(_, raw)| raw.clone())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| tool_name.to_string());
    control.remember_dsh_permission(&request_id);
    on_event(PromptSessionEvent::PermissionRequest {
        session_id: session_id.to_string(),
        request_id,
        tool_name: tool_name.to_string(),
        detail,
        input_json: String::new(),
        input_mode: "choice".into(),
    });
}

fn handle_update<F>(session_id: &str, params: &Value, state: &mut AcpState, on_event: &mut F)
where
    F: FnMut(PromptSessionEvent),
{
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
        "tool_call" | "tool_call_update" => {
            if let Some(id) = update.get("toolCallId").and_then(|v| v.as_str()) {
                let entry = state
                    .tools
                    .entry(id.to_string())
                    .or_insert_with(|| (String::new(), String::new()));
                if let Some(title) = update
                    .get("title")
                    .and_then(|v| v.as_str())
                    .filter(|text| !text.is_empty())
                {
                    entry.0 = title.to_string();
                }
                if let Some(raw) = update.get("rawInput") {
                    entry.1 = tool_arguments(raw);
                }
            }
            let title = update
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|text| !text.is_empty())
                .unwrap_or("正在使用工具");
            on_event(PromptSessionEvent::Status {
                session_id: session_id.to_string(),
                phase: "tool".into(),
                message: title.to_string(),
            });
        }
        _ => {}
    }
}

fn acp_cwd(cwd: &Path) -> String {
    std::fs::canonicalize(cwd)
        .unwrap_or_else(|_| cwd.to_path_buf())
        .display()
        .to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_session::util::TEST_ENV_LOCK;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn prompt_blocks_add_pictures_only_when_agent_takes_them() {
        let images = vec![EncodedImage {
            media_type: "image/webp",
            base64: "QUJD".into(),
        }];
        let with = prompt_blocks("图", &images, true);
        assert_eq!(with[0], json!({"type": "text", "text": "图"}));
        assert_eq!(
            with[1],
            json!({"type": "image", "mimeType": "image/webp", "data": "QUJD"})
        );
        assert_eq!(
            prompt_blocks("图", &images, false)
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[cfg(unix)]
    fn write_fake(dir: &Path, name: &str, script: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, script).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    const FAKE_ACP: &str = r#"#!/usr/bin/env python3
import json, os, sys
log_path = os.environ["AD_DSH_LOG"]
def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()
def log(line):
    with open(log_path, "a") as handle:
        handle.write(line + "\n")
log("pid=%d" % os.getpid())
log("env=%s|%s" % (("set" if os.environ.get("DEEPSEEK_API_KEY") else ""), os.environ.get("DEEPSEEK_BASE_URL", "")))
pending_prompt = None
for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    method = msg.get("method", "")
    mid = msg.get("id")
    log(method)
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {"protocolVersion": 1}})
    elif method == "session/new":
        send({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": "sess-1"}})
    elif method == "session/resume":
        sid = msg.get("params", {}).get("sessionId", "")
        if sid == "sess-1":
            send({"jsonrpc": "2.0", "id": mid, "result": {}})
        else:
            send({"jsonrpc": "2.0", "id": mid, "error": {"code": -32602, "message": "unknown session"}})
    elif method == "session/prompt":
        blocks = msg.get("params", {}).get("prompt") or []
        text = blocks[0].get("text", "") if blocks else ""
        if text == "need-approval":
            pending_prompt = mid
            send({"jsonrpc": "2.0", "id": 9, "method": "session/request_permission", "params": {"toolCall": {"title": "改文件"}}})
        else:
            sid = msg.get("params", {}).get("sessionId", "")
            send({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": sid,
                "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "final-answer"}}
            }})
            send({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "end_turn"}})
    elif pending_prompt is not None and isinstance(msg.get("result"), dict):
        option = msg.get("result", {}).get("outcome", {}).get("optionId", "")
        log("perm=%s" % option)
        send({"jsonrpc": "2.0", "id": pending_prompt, "result": {"stopReason": "end_turn"}})
        pending_prompt = None
"#;

    fn options(dir: &Path, prompt: &str, resume: Option<&str>) -> PromptSessionOptions {
        PromptSessionOptions {
            runtime: DEEPSEEK_HARNESS_RUNTIME_ID.into(),
            prompt: prompt.into(),
            cwd: Some(dir.to_path_buf()),
            timeout_sec: 30,
            dangerously_skip_permissions: false,
            full_auto: false,
            resume_thread_id: resume.map(str::to_string),
            selected_mcps: Vec::new(),
            image_paths: Vec::new(),
            workspace_name: None,
            provider_id: None,
            model: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn acp_resumes_the_saved_session_and_keeps_the_key() {
        let _guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        warm::disable_for_test();
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("calls.txt");
        let binary = write_fake(dir.path(), "fake-dsh", FAKE_ACP);
        std::env::set_var("AGENT_DOCTOR_DSH_BIN", &binary);
        std::env::set_var("AD_DSH_LOG", &log_path);
        std::env::set_var("DEEPSEEK_API_KEY", "test-secret");
        std::env::set_var("DEEPSEEK_BASE_URL", "https://api.deepseek.com/v1");

        let first = DeepSeekHarnessAskBackend
            .run(
                &options(dir.path(), "hello", Some("unsupported-old-id")),
                PromptSessionCancel::new(),
                None,
                &mut |_| {},
            )
            .unwrap();
        let second = DeepSeekHarnessAskBackend
            .run(
                &options(dir.path(), "again", first.runtime_thread_id.as_deref()),
                PromptSessionCancel::new(),
                None,
                &mut |_| {},
            )
            .unwrap();

        std::env::remove_var("AGENT_DOCTOR_DSH_BIN");
        std::env::remove_var("AD_DSH_LOG");
        std::env::remove_var("DEEPSEEK_API_KEY");
        std::env::remove_var("DEEPSEEK_BASE_URL");

        assert_eq!(
            first.status,
            PromptSessionStatus::Succeeded,
            "{}",
            first.summary
        );
        assert_eq!(first.runtime_thread_id.as_deref(), Some("sess-1"));
        assert!(
            first.log_excerpt.contains("final-answer"),
            "{}",
            first.log_excerpt
        );
        assert_eq!(
            second.status,
            PromptSessionStatus::Succeeded,
            "{}",
            second.summary
        );
        let calls = fs::read_to_string(log_path).unwrap();
        assert!(calls.contains("session/resume"), "{calls}");
        assert!(calls.contains("session/new"), "{calls}");
        assert!(calls.contains("session/prompt"), "{calls}");
    }

    #[cfg(unix)]
    #[test]
    fn warm_process_accepts_the_next_prompt_without_restarting() {
        let _guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("calls.txt");
        let binary = write_fake(dir.path(), "fake-dsh", FAKE_ACP);
        std::env::set_var("AGENT_DOCTOR_DSH_BIN", &binary);
        std::env::set_var("AD_DSH_LOG", &log_path);
        warm::enable_warm_sessions();

        let first = DeepSeekHarnessAskBackend
            .run(
                &options(dir.path(), "hello", None),
                PromptSessionCancel::new(),
                None,
                &mut |_| {},
            )
            .unwrap();
        let second = DeepSeekHarnessAskBackend
            .run(
                &options(dir.path(), "again", first.runtime_thread_id.as_deref()),
                PromptSessionCancel::new(),
                None,
                &mut |_| {},
            )
            .unwrap();

        warm::shutdown_warm_sessions();
        warm::disable_for_test();
        std::env::remove_var("AGENT_DOCTOR_DSH_BIN");
        std::env::remove_var("AD_DSH_LOG");

        assert_eq!(
            first.status,
            PromptSessionStatus::Succeeded,
            "{}",
            first.summary
        );
        assert_eq!(
            second.status,
            PromptSessionStatus::Succeeded,
            "{}",
            second.summary
        );
        let calls = fs::read_to_string(log_path).unwrap();
        let pids: Vec<_> = calls
            .lines()
            .filter_map(|line| line.strip_prefix("pid="))
            .collect();
        assert_eq!(pids.len(), 1, "{calls}");
        assert_eq!(calls.matches("session/prompt").count(), 2, "{calls}");
        assert_eq!(calls.matches("initialize").count(), 1, "{calls}");
    }

    #[cfg(unix)]
    #[test]
    fn auto_allow_answers_a_tool_without_asking() {
        let _guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        warm::disable_for_test();
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("calls.txt");
        let binary = write_fake(dir.path(), "fake-dsh", FAKE_ACP);
        std::env::set_var("AGENT_DOCTOR_DSH_BIN", &binary);
        std::env::set_var("AD_DSH_LOG", &log_path);
        let mut opts = options(dir.path(), "need-approval", None);
        opts.dangerously_skip_permissions = true;
        let mut asked = false;
        let report = DeepSeekHarnessAskBackend
            .run(&opts, PromptSessionCancel::new(), None, &mut |event| {
                if matches!(event, PromptSessionEvent::PermissionRequest { .. }) {
                    asked = true;
                }
            })
            .unwrap();
        std::env::remove_var("AGENT_DOCTOR_DSH_BIN");
        std::env::remove_var("AD_DSH_LOG");
        assert_eq!(
            report.status,
            PromptSessionStatus::Succeeded,
            "{}",
            report.summary
        );
        assert!(!asked);
        let calls = fs::read_to_string(log_path).unwrap();
        assert!(calls.contains("perm=allow-once"), "{calls}");
    }
}
