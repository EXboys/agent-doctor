//! Cursor ask backend (`agent acp`).
//!
//! The desktop app stays where it is. This path only starts the official
//! Cursor command line, authenticates with that CLI's own login, and speaks ACP
//! on stdin/stdout.

use std::process::Stdio;
use std::time::Instant;

use anyhow::{bail, Result};

use super::acp::{pump_acp, AcpProfile};
use super::backend::AskBackend;
use super::control::PromptSessionControl;
use super::util::{command_from_cli, summarize, turn_error};
use super::vision::{encode_image, turn_images, EncodedImage};
use super::warm::LivePipes;
use super::{
    next_session_id, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, PromptSessionStatus, MAX_TIMEOUT_SEC, MIN_TIMEOUT_SEC,
};
use agent_doctor_core::adapters::cursor_cli_path;
use agent_doctor_core::session_env::{
    apply_cursor_env, apply_overlay_env, chat_uses_saved_service, format_command_display,
    overlay_uses_native_account, resolve_cursor_api_key,
};
use agent_doctor_core::session_launch::resolve_session_cwd;

const RUNTIME_ID: &str = "cursor";
const CURSOR_NEEDS_LOGIN: &str =
    "Cursor 要用它自己的账号，服务商里的密钥用不了。请点「打开原生」登录，或换用 Codex / Claude Code / Qoder。";

fn cursor_cli_missing() -> &'static str {
    agent_doctor_core::adapters::cursor_missing_install_part().unwrap_or("Cursor 命令行还没装。")
}

pub struct CursorAskBackend;

impl AskBackend for CursorAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_cursor(options, cancel, control, on_event)
    }
}

fn cursor_profile(logged_in: bool, has_api_key: bool) -> AcpProfile {
    // CURSOR_API_KEY already authenticates the process — only fall back to the
    // browser login if session/new still rejects. A saved Cursor login (or a
    // first-time Ask with neither) authenticates up front, like before.
    let preauthed = has_api_key && !logged_in;
    AcpProfile {
        incomplete_turn: "Cursor 这一轮没有完成".into(),
        resume_fallback: "上次对话接不上，正在新开一轮。".into(),
        no_session: "Cursor 没有返回这一轮的会话。".into(),
        refusal: "Cursor 拒绝继续这一轮。".into(),
        images_note: "这个 Cursor 收不了图片，这次只带上了图里的字。".into(),
        default_tool: "这一步操作".into(),
        auth_method: (!preauthed).then_some("cursor_login"),
        auth_on_demand: preauthed.then_some("cursor_login"),
        resume_method: "session/load",
    }
}

fn run_cursor(
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    control: Option<PromptSessionControl>,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<PromptSessionReport> {
    let session_id = next_session_id();
    let prompt = options.prompt.trim();
    if prompt.is_empty() {
        bail!("prompt must not be empty");
    }
    let cwd = resolve_session_cwd(options.cwd.as_deref(), options.workspace_name.as_deref());
    if !cwd.exists() {
        bail!("session cwd does not exist: {}", cwd.display());
    }
    let binary = std::env::var("AGENT_DOCTOR_CURSOR_BIN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| cursor_cli_path().map(|path| path.display().to_string()))
        .ok_or_else(|| anyhow::Error::msg(cursor_cli_missing()))?;
    let timeout_sec = options.timeout_sec.clamp(MIN_TIMEOUT_SEC, MAX_TIMEOUT_SEC);
    let resume_session_id = options
        .resume_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let auto_allow = options.dangerously_skip_permissions || options.full_auto;
    let overlay = super::collect_overlay_env_for_options(options);
    let logged_in = agent_doctor_core::is_runtime_logged_in(RUNTIME_ID);
    let has_cursor_key = resolve_cursor_api_key(&overlay).is_some();
    // Cursor's CLI only accepts a Cursor login or CURSOR_API_KEY — not the
    // OpenAI-compatible keys saved under 服务商 (unlike Codex / Qoder).
    // Only refuse when this chat explicitly pinned a saved service; ambient
    // active-provider overlay must not block a plain Cursor login Ask.
    if !logged_in
        && !has_cursor_key
        && chat_uses_saved_service(options.model.as_deref(), options.provider_id.as_deref())
    {
        bail!(CURSOR_NEEDS_LOGIN);
    }
    let images: Vec<EncodedImage> = turn_images(RUNTIME_ID, &options.image_paths, &overlay)
        .iter()
        .filter_map(|path| encode_image(path))
        .collect();

    let mut cmd = command_from_cli(&binary);
    let cursor_model = if overlay_uses_native_account(&overlay) {
        overlay
            .get(agent_doctor_core::setup::MODEL_ENV)
            .map(String::as_str)
    } else {
        options.model.as_deref()
    };
    if let Some(model) = cursor_model
        .map(str::trim)
        .filter(|m| !m.is_empty() && !m.eq_ignore_ascii_case("auto"))
    {
        cmd.arg("--model").arg(model);
    }
    cmd.arg("acp")
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_overlay_env(&mut cmd, &overlay);
    // Last: drop gateway keys overlay may have set, keep only Cursor credentials.
    apply_cursor_env(&mut cmd, &overlay);
    let command_display = format_command_display(&cmd);
    on_event(PromptSessionEvent::Started {
        session_id: session_id.clone(),
        runtime: RUNTIME_ID.to_string(),
        cwd: cwd.display().to_string(),
        command: command_display,
        client_run_id: String::new(),
    });

    let started = Instant::now();
    let writer = control.unwrap_or_default();
    let mut child = cmd
        .spawn()
        .map_err(|_| anyhow::Error::msg(cursor_cli_missing()))?;
    let mut pipes = match LivePipes::attach(&mut child) {
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
            None,
            resume_session_id.as_deref(),
            auto_allow,
            cursor_profile(logged_in, has_cursor_key),
            &mut emit,
        )
    };
    let duration_ms = started.elapsed().as_millis() as u64;
    let runtime = RUNTIME_ID.to_string();
    match result {
        Ok(turn) => {
            if display.trim().is_empty() && turn.status != PromptSessionStatus::Succeeded {
                if let Some(error) = turn.error.as_deref() {
                    on_event(turn_error(&session_id, error));
                }
            }
            let readable = if display.trim().is_empty() {
                turn.error.clone().unwrap_or(turn.stderr.clone())
            } else {
                display.clone()
            };
            let summary = summarize(&readable, &turn.status, &runtime);
            on_event(PromptSessionEvent::Completed {
                session_id: session_id.clone(),
                status: turn.status.clone(),
                exit_code: turn.exit_code,
                summary: summary.clone(),
                timeout: turn.timeout,
            });
            let _ = child.kill();
            let _ = child.wait();
            Ok(PromptSessionReport {
                usage: None,
                session_id,
                runtime,
                cwd: cwd.display().to_string(),
                status: turn.status,
                exit_code: turn.exit_code,
                summary,
                log_excerpt: readable,
                duration_ms,
                runtime_thread_id: turn.acp_session,
            })
        }
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let summary = summarize(&error.to_string(), &PromptSessionStatus::Failed, &runtime);
            on_event(turn_error(&session_id, &error.to_string()));
            on_event(PromptSessionEvent::Completed {
                session_id: session_id.clone(),
                status: PromptSessionStatus::Failed,
                exit_code: None,
                summary: summary.clone(),
                timeout: None,
            });
            Ok(PromptSessionReport {
                usage: None,
                session_id,
                runtime,
                cwd: cwd.display().to_string(),
                status: PromptSessionStatus::Failed,
                exit_code: None,
                summary,
                log_excerpt: error.to_string(),
                duration_ms,
                runtime_thread_id: None,
            })
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::prompt_session::util::TEST_ENV_LOCK;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    const FAKE_CURSOR_ACP: &str = r#"#!/usr/bin/env python3
import json, os, sys
log_path = os.environ["AD_CURSOR_LOG"]
def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()
def log(line):
    with open(log_path, "a") as handle:
        handle.write(line + "\n")
log("argv=" + " ".join(sys.argv[1:]))
authed = False
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
    elif method == "authenticate":
        authed = msg.get("params", {}).get("methodId") == "cursor_login"
        send({"jsonrpc": "2.0", "id": mid, "result": {}})
    elif method == "session/new":
        assert authed, "session/new before cursor_login"
        send({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": "cur-1"}})
    elif method == "session/load":
        assert authed, "session/load before cursor_login"
        send({"jsonrpc": "2.0", "id": mid, "result": {}})
    elif method == "session/prompt":
        sid = msg.get("params", {}).get("sessionId", "")
        send({"jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": sid,
            "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "cursor-ok"}}
        }})
        send({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "end_turn"}})
"#;

    #[test]
    fn cursor_authenticates_and_resumes_via_session_load() {
        let _guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("cursor.log");
        let bin = dir.path().join("fake-agent");
        fs::write(&bin, FAKE_CURSOR_ACP).unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();

        std::env::set_var("AGENT_DOCTOR_CURSOR_BIN", &bin);
        std::env::set_var("AD_CURSOR_LOG", &log_path);

        let make_opts = |resume: Option<&str>| PromptSessionOptions {
            runtime: "cursor".into(),
            prompt: "hi".into(),
            cwd: Some(dir.path().to_path_buf()),
            timeout_sec: 30,
            dangerously_skip_permissions: false,
            full_auto: false,
            resume_thread_id: resume.map(str::to_string),
            selected_mcps: Vec::new(),
            image_paths: Vec::new(),
            workspace_name: None,
            provider_id: None,
            model: None,
            browser_tools_enabled: None,
            computer_use_enabled: None,
        };
        let first = CursorAskBackend
            .run(
                &make_opts(None),
                PromptSessionCancel::new(),
                None,
                &mut |_| {},
            )
            .unwrap();
        let second = CursorAskBackend
            .run(
                &make_opts(first.runtime_thread_id.as_deref()),
                PromptSessionCancel::new(),
                None,
                &mut |_| {},
            )
            .unwrap();

        std::env::remove_var("AGENT_DOCTOR_CURSOR_BIN");
        std::env::remove_var("AD_CURSOR_LOG");

        assert_eq!(
            first.status,
            PromptSessionStatus::Succeeded,
            "{}",
            first.summary
        );
        assert_eq!(first.runtime_thread_id.as_deref(), Some("cur-1"));
        assert!(first.log_excerpt.contains("cursor-ok"));
        assert_eq!(
            second.status,
            PromptSessionStatus::Succeeded,
            "{}",
            second.summary
        );
        let calls = fs::read_to_string(log_path).unwrap();
        assert!(calls.contains("argv=acp"), "{calls}");
        assert!(calls.contains("authenticate\nsession/new"), "{calls}");
        assert!(calls.contains("authenticate\nsession/load"), "{calls}");
    }
}
