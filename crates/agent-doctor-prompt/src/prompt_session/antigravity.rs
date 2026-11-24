//! Antigravity ask backend (`agy acp`).
//!
//! The installed CLI speaks Agent Client Protocol on stdin/stdout. A turn
//! uses the Google account already signed in on this computer. A saved
//! Gemini key is used only when this chat picked a Gemini address.

use std::collections::HashMap;
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
use agent_doctor_core::adapters::{
    agy_cli_path, disable_gemini_key_provider, enable_gemini_key_provider,
};
use agent_doctor_core::profile::{COMPANY_API_KEY_ENV, GATEWAY_URL_ENV};
use agent_doctor_core::session_env::{
    apply_overlay_env, format_command_display, overlay_uses_native_account,
};
use agent_doctor_core::session_launch::resolve_session_cwd;

const RUNTIME_ID: &str = "antigravity";
const AGY_MISSING: &str = "这台电脑还没装 Antigravity。先在助手列表里点安装。";
const AGY_NEEDS_LOGIN: &str = "Antigravity 还没登录。先用自己的 Google 账号登录，再回到这里问。";

pub struct AntigravityAskBackend;

impl AskBackend for AntigravityAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_antigravity(options, cancel, control, on_event)
    }
}

fn antigravity_profile() -> AcpProfile {
    AcpProfile {
        incomplete_turn: "Antigravity 这一轮没有完成".into(),
        resume_fallback: "上次对话接不上，正在新开一轮。".into(),
        no_session: "Antigravity 没有返回这一轮的会话。".into(),
        refusal: "Antigravity 拒绝继续这一轮。".into(),
        images_note: "这次图片没带上，只问了文字。".into(),
        default_tool: "这一步操作".into(),
        auth_method: None,
        auth_on_demand: None,
        resume_method: "session/resume",
    }
}

fn run_antigravity(
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
    let binary = std::env::var("AGENT_DOCTOR_AGY_BIN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| agy_cli_path().map(|path| path.display().to_string()))
        .ok_or_else(|| anyhow::Error::msg(AGY_MISSING))?;
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
    cmd.arg("acp");
    if auto_allow {
        cmd.arg("--dangerously-skip-permissions");
    }
    cmd.current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_overlay_env(&mut cmd, &overlay);
    apply_saved_gemini_key(&mut cmd, &overlay);
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
    let mut child = cmd.spawn().map_err(|_| anyhow::Error::msg(AGY_MISSING))?;
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
            antigravity_profile(),
            &mut emit,
        )
    };
    let duration_ms = started.elapsed().as_millis() as u64;
    let runtime = RUNTIME_ID.to_string();
    match result {
        Ok(turn) => {
            let error = turn.error.as_deref().map(plain_agy_error);
            if display.trim().is_empty() && turn.status != PromptSessionStatus::Succeeded {
                if let Some(error) = error.as_deref() {
                    on_event(turn_error(&session_id, error));
                }
            }
            let readable = if display.trim().is_empty() {
                error.unwrap_or(turn.stderr.clone())
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
            let message = plain_agy_error(&error.to_string());
            let summary = summarize(&message, &PromptSessionStatus::Failed, &runtime);
            on_event(turn_error(&session_id, &message));
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
                log_excerpt: message,
                duration_ms,
                runtime_thread_id: None,
            })
        }
    }
}

fn plain_agy_error(raw: &str) -> String {
    let lowered = raw.to_ascii_lowercase();
    if lowered.contains("authentication required")
        || lowered.contains("not logged in")
        || lowered.contains("login required")
    {
        AGY_NEEDS_LOGIN.to_string()
    } else {
        raw.to_string()
    }
}

fn apply_saved_gemini_key(cmd: &mut std::process::Command, overlay: &HashMap<String, String>) {
    if overlay_uses_native_account(overlay) {
        for key in [
            "GEMINI_API_KEY",
            "GOOGLE_GEMINI_BASE_URL",
            "GOOGLE_GENAI_USE_VERTEXAI",
            "GOOGLE_CLOUD_PROJECT",
            "GOOGLE_CLOUD_LOCATION",
        ] {
            cmd.env_remove(key);
        }
        disable_gemini_key_provider();
        return;
    }
    let url = overlay
        .get(GATEWAY_URL_ENV)
        .map(String::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let gemini = url.contains("generativelanguage.googleapis.com");
    let vertex = url.contains("aiplatform.googleapis.com");
    if !gemini && !vertex {
        return;
    }
    let key = overlay
        .get(COMPANY_API_KEY_ENV)
        .or_else(|| overlay.get("GEMINI_API_KEY"))
        .or_else(|| overlay.get("OPENAI_API_KEY"))
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    let Some(key) = key else {
        return;
    };
    cmd.env("GEMINI_API_KEY", key);
    if gemini {
        cmd.env(
            "GOOGLE_GEMINI_BASE_URL",
            "https://generativelanguage.googleapis.com/v1beta",
        );
        cmd.env_remove("GOOGLE_GENAI_USE_VERTEXAI");
        enable_gemini_key_provider();
    } else {
        cmd.env("GOOGLE_GENAI_USE_VERTEXAI", "true");
        cmd.env_remove("GOOGLE_GEMINI_BASE_URL");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::prompt_session::util::TEST_ENV_LOCK;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    const FAKE_AGY_ACP: &str = r#"#!/usr/bin/env python3
import json, os, sys
log_path = os.environ["AD_AGY_LOG"]
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
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": "agy-1"}}) + "\n")
        sys.stdout.flush()
    elif method == "session/prompt":
        sid = msg.get("params", {}).get("sessionId", "")
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": sid,
            "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "agy-ok"}}
        }}) + "\n")
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "end_turn"}}) + "\n")
        sys.stdout.flush()
"#;

    #[test]
    fn antigravity_chat_speaks_acp() {
        let _guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("agy.log");
        let bin = dir.path().join("fake-agy");
        fs::write(&bin, FAKE_AGY_ACP).unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();

        std::env::set_var("AGENT_DOCTOR_AGY_BIN", &bin);
        std::env::set_var("AD_AGY_LOG", &log_path);

        let opts = PromptSessionOptions {
            runtime: "antigravity".into(),
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
        let report = AntigravityAskBackend
            .run(&opts, PromptSessionCancel::new(), None, &mut |_| {})
            .unwrap();

        std::env::remove_var("AGENT_DOCTOR_AGY_BIN");
        std::env::remove_var("AD_AGY_LOG");

        assert_eq!(
            report.status,
            PromptSessionStatus::Succeeded,
            "{}",
            report.summary
        );
        assert_eq!(report.runtime_thread_id.as_deref(), Some("agy-1"));
        assert!(report.log_excerpt.contains("agy-ok"));
        let calls = fs::read_to_string(log_path).unwrap();
        assert!(calls.contains("acp"), "{calls}");
    }

    #[test]
    fn login_failure_is_a_plain_sentence() {
        assert_eq!(plain_agy_error("Authentication required"), AGY_NEEDS_LOGIN);
    }
}
