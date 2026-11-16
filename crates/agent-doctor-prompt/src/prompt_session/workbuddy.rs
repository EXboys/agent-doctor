//! WorkBuddy ask backend (`codebuddy --acp`).
//!
//! The desktop app stays where it is. This path only starts the official
//! `codebuddy` command line, which speaks ACP on stdin/stdout.

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
use agent_doctor_core::adapters::workbuddy_cli_path;
use agent_doctor_core::session_env::{
    apply_overlay_env, apply_workbuddy_env, format_command_display,
};
use agent_doctor_core::session_launch::resolve_session_cwd;

const RUNTIME_ID: &str = "workbuddy";
const WORKBUDDY_CLI_MISSING: &str =
    "桌面里的 WorkBuddy 留在那边。这里要用它的命令行，这台电脑还没装。";

pub struct WorkbuddyAskBackend;

impl AskBackend for WorkbuddyAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_workbuddy(options, cancel, control, on_event)
    }
}

fn workbuddy_profile() -> AcpProfile {
    AcpProfile {
        incomplete_turn: "WorkBuddy 这一轮没有完成".into(),
        resume_fallback: "上次对话接不上，正在新开一轮。".into(),
        no_session: "WorkBuddy 没有返回这一轮的会话。".into(),
        refusal: "WorkBuddy 拒绝继续这一轮。".into(),
        images_note: "这个 WorkBuddy 收不了图片，这次只带上了图里的字。".into(),
        default_tool: "这一步操作".into(),
        auth_method: None,
        // `internal` is CodeBuddy's Chinese Site / WeChat browser login; only
        // invoked when `session/new` says the CLI is not signed in yet.
        auth_on_demand: Some("internal"),
        resume_method: "session/resume",
    }
}

fn run_workbuddy(
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
    let binary = std::env::var("AGENT_DOCTOR_WORKBUDDY_BIN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| workbuddy_cli_path().map(|path| path.display().to_string()))
        .ok_or_else(|| anyhow::Error::msg(WORKBUDDY_CLI_MISSING))?;
    let timeout_sec = options.timeout_sec.clamp(MIN_TIMEOUT_SEC, MAX_TIMEOUT_SEC);
    let resume_session_id = options
        .resume_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let auto_allow = options.dangerously_skip_permissions || options.full_auto;
    let overlay = super::collect_overlay_env_for_options(options);
    let images: Vec<EncodedImage> = turn_images(RUNTIME_ID, &options.image_paths, &overlay)
        .iter()
        .filter_map(|path| encode_image(path))
        .collect();

    let mut cmd = command_from_cli(&binary);
    cmd.arg("--acp")
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_overlay_env(&mut cmd, &overlay);
    apply_workbuddy_env(&mut cmd, &overlay);
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
        .map_err(|_| anyhow::Error::msg(WORKBUDDY_CLI_MISSING))?;
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
            workbuddy_profile(),
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

    const FAKE_CODEBUDDY_ACP: &str = r#"#!/usr/bin/env python3
import json, os, sys
log_path = os.environ["AD_WB_LOG"]
with open(log_path, "a") as handle:
    handle.write("argv=" + " ".join(sys.argv[1:]) + "\n")
for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    method = msg.get("method", "")
    mid = msg.get("id")
    if method == "initialize":
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": {"protocolVersion": 1}}) + "\n")
        sys.stdout.flush()
    elif method == "session/new":
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": "wb-1"}}) + "\n")
        sys.stdout.flush()
    elif method == "session/prompt":
        sid = msg.get("params", {}).get("sessionId", "")
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": sid,
            "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "wb-ok"}}
        }}) + "\n")
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "end_turn"}}) + "\n")
        sys.stdout.flush()
"#;

    #[test]
    fn workbuddy_speaks_acp_on_its_cli() {
        let _guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("wb.log");
        let bin = dir.path().join("fake-codebuddy");
        fs::write(&bin, FAKE_CODEBUDDY_ACP).unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();

        std::env::set_var("AGENT_DOCTOR_WORKBUDDY_BIN", &bin);
        std::env::set_var("AD_WB_LOG", &log_path);

        let opts = PromptSessionOptions {
            runtime: "workbuddy".into(),
            prompt: "hi".into(),
            cwd: Some(dir.path().to_path_buf()),
            timeout_sec: 30,
            dangerously_skip_permissions: false,
            full_auto: false,
            resume_thread_id: None,
            selected_mcps: Vec::new(),
            image_paths: Vec::new(),
            workspace_name: None,
            provider_id: None,
            model: None,
            browser_tools_enabled: None,
            computer_use_enabled: None,
        };
        let report = WorkbuddyAskBackend
            .run(&opts, PromptSessionCancel::new(), None, &mut |_| {})
            .unwrap();

        std::env::remove_var("AGENT_DOCTOR_WORKBUDDY_BIN");
        std::env::remove_var("AD_WB_LOG");

        assert_eq!(
            report.status,
            PromptSessionStatus::Succeeded,
            "{}",
            report.summary
        );
        assert_eq!(report.runtime_thread_id.as_deref(), Some("wb-1"));
        assert!(report.log_excerpt.contains("wb-ok"));
        let calls = fs::read_to_string(log_path).unwrap();
        assert!(calls.contains("argv=--acp"), "{calls}");
    }

    const FAKE_CODEBUDDY_AUTH_ON_DEMAND: &str = r#"#!/usr/bin/env python3
import json, os, sys
log_path = os.environ["AD_WB_LOG"]
authed = False
for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    method = msg.get("method", "")
    mid = msg.get("id")
    with open(log_path, "a") as handle:
        handle.write(method + "\n")
    if method == "initialize":
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": {"protocolVersion": 1}}) + "\n")
        sys.stdout.flush()
    elif method == "session/new":
        if not authed:
            sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "error": {"code": -32000, "message": "Authentication required", "data": {"category": "auth"}}}) + "\n")
        else:
            sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": "wb-auth-1"}}) + "\n")
        sys.stdout.flush()
    elif method == "authenticate":
        assert msg.get("params", {}).get("methodId") == "internal"
        authed = True
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": {}}) + "\n")
        sys.stdout.flush()
    elif method == "session/prompt":
        sid = msg.get("params", {}).get("sessionId", "")
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": sid,
            "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "wb-after-login"}}
        }}) + "\n")
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "end_turn"}}) + "\n")
        sys.stdout.flush()
"#;

    #[test]
    fn workbuddy_authenticates_on_demand_when_not_signed_in() {
        let _guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("wb-auth.log");
        let bin = dir.path().join("fake-codebuddy-auth");
        fs::write(&bin, FAKE_CODEBUDDY_AUTH_ON_DEMAND).unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();

        std::env::set_var("AGENT_DOCTOR_WORKBUDDY_BIN", &bin);
        std::env::set_var("AD_WB_LOG", &log_path);

        let opts = PromptSessionOptions {
            runtime: "workbuddy".into(),
            prompt: "你好".into(),
            cwd: Some(dir.path().to_path_buf()),
            timeout_sec: 30,
            dangerously_skip_permissions: false,
            full_auto: false,
            resume_thread_id: None,
            selected_mcps: Vec::new(),
            image_paths: Vec::new(),
            workspace_name: None,
            provider_id: None,
            model: None,
            browser_tools_enabled: None,
            computer_use_enabled: None,
        };
        let report = WorkbuddyAskBackend
            .run(&opts, PromptSessionCancel::new(), None, &mut |_| {})
            .unwrap();

        std::env::remove_var("AGENT_DOCTOR_WORKBUDDY_BIN");
        std::env::remove_var("AD_WB_LOG");

        assert_eq!(
            report.status,
            PromptSessionStatus::Succeeded,
            "{}",
            report.summary
        );
        assert_eq!(report.runtime_thread_id.as_deref(), Some("wb-auth-1"));
        assert!(report.log_excerpt.contains("wb-after-login"));
        let calls = fs::read_to_string(log_path).unwrap();
        assert!(
            calls.contains("session/new\nauthenticate\nsession/new\nsession/prompt"),
            "{calls}"
        );
    }
}
