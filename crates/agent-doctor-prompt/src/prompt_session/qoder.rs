//! Qoder ask backend (`qodercli -p --output-format stream-json`).
//!
//! `qodercli --acp` unconditionally enforces Qoder browser OAuth (`requireAuth`)
//! and ignores custom providers unless the user has signed into Qoder first.
//! Headless stream-json (`qodercli -p --output-format stream-json`) honors
//! `providers.agent-doctor` + `security.auth.useExternal = true`, so Ask works
//! directly with the user's custom or TeamUps official provider — matching
//! Claude Code, Codex, and WorkBuddy — while still supporting Qoder's own
//! subscription account when the user opted in and signed in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{bail, Context, Result};

use super::backend::AskBackend;
use super::claude::{
    ask_private_dir, extract_claude_result_text, extract_claude_session_id, is_claude_session_id,
    project_mcp_launch, pump_claude, stream_json_result_failed, without_stream_json,
    CLAUDE_ASK_INSTRUCTIONS,
};
use super::control::PromptSessionControl;
use super::mcp_ensure::wire_hands;
use super::util::{combine_output, command_from_cli, summarize, turn_error};
use super::vision::prepare_turn_images;
use super::warm::LivePipes;
use super::{
    next_session_id, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, PromptSessionStatus, TimeoutNote, MAX_TIMEOUT_SEC, MIN_TIMEOUT_SEC,
};
use agent_doctor_core::adapters::qoder_cli_path;
use agent_doctor_core::session_env::{
    apply_overlay_env, apply_qoder_env, format_command_display, resolve_qoder_overlay,
};
use agent_doctor_core::session_launch::resolve_session_cwd;
use agent_doctor_core::setup::merge::qoder_provider_settings;
use agent_doctor_core::setup::MODEL_ENV;

const RUNTIME_ID: &str = "qoder";
const QODER_CLI_MISSING: &str = "桌面里的 Qoder 留在那边。这里要用它的命令行，这台电脑还没装。";
const MAX_QODER_ATTACHMENTS: usize = 5;

pub struct QoderAskBackend;

impl AskBackend for QoderAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_qoder(options, cancel, control, on_event)
    }
}

struct QoderLaunch<'a> {
    binary: &'a str,
    cwd: &'a Path,
    skip_permissions: bool,
    interactive: bool,
    computer_mcp: bool,
    attachments: &'a [PathBuf],
    overlay: &'a HashMap<String, String>,
}

struct QoderAttemptOutcome {
    status: PromptSessionStatus,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    display: String,
    timeout: Option<TimeoutNote>,
}

fn run_qoder(
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
    let binary = std::env::var("AGENT_DOCTOR_QODER_BIN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| qoder_cli_path().map(|path| path.display().to_string()))
        .ok_or_else(|| anyhow::Error::msg(QODER_CLI_MISSING))?;
    let timeout_sec = options.timeout_sec.clamp(MIN_TIMEOUT_SEC, MAX_TIMEOUT_SEC);
    let resume_session_id = options
        .resume_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|id| is_claude_session_id(id))
        .map(str::to_string);
    let skip_permissions = options.dangerously_skip_permissions || options.full_auto;
    let interactive = !skip_permissions && control.is_some();

    let overlay = super::collect_overlay_env_for_options(options);
    let hands = wire_hands(RUNTIME_ID, options, &cwd, &overlay, |phase, message| {
        on_event(PromptSessionEvent::Status {
            session_id: session_id.clone(),
            phase: phase.into(),
            message,
        })
    });
    let effective_prompt = hands.prompt(prompt);
    let _ = agent_doctor_core::forget_project_computer_mcp(&cwd);
    let prepared_images = prepare_turn_images(RUNTIME_ID, &options.image_paths, &overlay);

    let launch = QoderLaunch {
        binary: &binary,
        cwd: &cwd,
        skip_permissions,
        interactive,
        computer_mcp: hands.computer,
        attachments: &prepared_images.paths,
        overlay: &overlay,
    };

    let started = Instant::now();
    let writer = control.unwrap_or_default();
    let mut announced = false;
    let mut active_resume = resume_session_id.as_deref();

    let outcome = loop {
        let mut cmd = build_qoder_command(&effective_prompt, &launch, active_resume)?;
        if !announced {
            announced = true;
            on_event(PromptSessionEvent::Started {
                session_id: session_id.clone(),
                runtime: RUNTIME_ID.to_string(),
                cwd: cwd.display().to_string(),
                command: format_command_display(&cmd),
                client_run_id: String::new(),
            });
        }
        let can_retry_resume = active_resume.is_some();
        let attempt = run_qoder_attempt(
            &session_id,
            &mut cmd,
            timeout_sec,
            &cancel,
            &writer,
            interactive,
            can_retry_resume,
            on_event,
        );
        match attempt {
            Ok(turn)
                if can_retry_resume
                    && turn.status == PromptSessionStatus::Failed
                    && turn.display.trim().is_empty()
                    && is_missing_qoder_resume(&combine_output(&turn.stdout, &turn.stderr)) =>
            {
                active_resume = None;
                on_event(PromptSessionEvent::Status {
                    session_id: session_id.clone(),
                    phase: "session".into(),
                    message: "上次对话接不上，正在新开一轮。".into(),
                });
                continue;
            }
            other => break other,
        }
    };

    // Keep temp shrunk attachments alive until the process finishes reading them.
    drop(prepared_images);

    let duration_ms = started.elapsed().as_millis() as u64;
    let runtime = RUNTIME_ID.to_string();
    match outcome {
        Ok(mut turn) => {
            if turn.display.trim().is_empty() && turn.status == PromptSessionStatus::Succeeded {
                if let Some(text) = extract_claude_result_text(&turn.stdout) {
                    turn.display = text.clone();
                    on_event(PromptSessionEvent::Delta {
                        session_id: session_id.clone(),
                        text,
                    });
                }
            }
            let result_err = stream_json_result_failed(&turn.stdout);
            if result_err.is_some() && turn.status == PromptSessionStatus::Succeeded {
                turn.status = PromptSessionStatus::Failed;
            }
            let combined = if !turn.display.trim().is_empty() {
                turn.display.clone()
            } else if let Some(err) = result_err.as_deref() {
                err.to_string()
            } else {
                combine_output(&turn.stdout, &turn.stderr)
            };
            let readable = if !turn.display.trim().is_empty() {
                turn.display
            } else if let Some(err) = result_err {
                err
            } else {
                combine_output(&without_stream_json(&turn.stdout), &turn.stderr)
            };
            if turn.status != PromptSessionStatus::Succeeded && !readable.trim().is_empty() {
                on_event(turn_error(&session_id, readable.trim()));
            }
            let summary = summarize(&readable, &turn.status, &runtime);
            let runtime_thread_id = extract_claude_session_id(&turn.stdout)
                .or_else(|| active_resume.map(str::to_string));
            on_event(PromptSessionEvent::Completed {
                session_id: session_id.clone(),
                status: turn.status.clone(),
                exit_code: turn.exit_code,
                summary: summary.clone(),
                timeout: turn.timeout,
            });
            Ok(PromptSessionReport {
                usage: None,
                session_id,
                runtime,
                cwd: cwd.display().to_string(),
                status: turn.status,
                exit_code: turn.exit_code,
                summary,
                log_excerpt: combined,
                duration_ms,
                runtime_thread_id,
            })
        }
        Err(error) => {
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

#[allow(clippy::too_many_arguments)]
fn run_qoder_attempt(
    session_id: &str,
    cmd: &mut Command,
    timeout_sec: u64,
    cancel: &PromptSessionCancel,
    writer: &PromptSessionControl,
    interactive: bool,
    suppress_errors_for_retry: bool,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<QoderAttemptOutcome> {
    let mut child = cmd
        .spawn()
        .map_err(|_| anyhow::Error::msg(QODER_CLI_MISSING))?;
    let mut pipes = match LivePipes::attach(&mut child) {
        Ok(pipes) => pipes,
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err);
        }
    };
    writer.close();
    if interactive {
        if let Some(stdin) = child.stdin.take() {
            writer.attach_stdin(stdin);
        }
    }

    let display_text = Arc::new(Mutex::new(String::new()));
    let display_for_cb = Arc::clone(&display_text);
    let held_errors: Arc<Mutex<Vec<PromptSessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let held_for_cb = Arc::clone(&held_errors);

    let pump = {
        let mut emit = |event: PromptSessionEvent| {
            match &event {
                PromptSessionEvent::Delta { text, .. } => {
                    if let Ok(mut guard) = display_for_cb.lock() {
                        guard.push_str(text);
                    }
                }
                PromptSessionEvent::StdoutLine { line, .. } => {
                    if let Ok(mut guard) = display_for_cb.lock() {
                        if !guard.is_empty() {
                            guard.push('\n');
                        }
                        guard.push_str(line);
                    }
                }
                PromptSessionEvent::TurnError { .. } | PromptSessionEvent::StderrLine { .. }
                    if suppress_errors_for_retry =>
                {
                    if let Ok(mut guard) = held_for_cb.lock() {
                        guard.push(event);
                    }
                    return;
                }
                _ => {}
            }
            on_event(event);
        };
        pump_claude(
            session_id,
            &mut child,
            &mut pipes,
            timeout_sec,
            cancel.handle(),
            interactive.then_some(writer),
            false,
            &mut emit,
        )
    };
    writer.close();
    let _ = child.kill();
    let _ = child.wait();

    let ((status, exit_code, stdout, stderr, timeout), _) = pump?;
    let display = display_text.lock().map(|g| g.clone()).unwrap_or_default();
    let is_resume_miss = suppress_errors_for_retry
        && status == PromptSessionStatus::Failed
        && display.trim().is_empty()
        && is_missing_qoder_resume(&combine_output(&stdout, &stderr));
    if !is_resume_miss {
        if let Ok(mut guard) = held_errors.lock() {
            for event in guard.drain(..) {
                on_event(event);
            }
        }
    }

    Ok(QoderAttemptOutcome {
        status,
        exit_code,
        stdout,
        stderr,
        display,
        timeout,
    })
}

fn build_qoder_command(
    prompt: &str,
    launch: &QoderLaunch<'_>,
    resume_session_id: Option<&str>,
) -> Result<Command> {
    let mut cmd = command_from_cli(launch.binary);
    cmd.arg("-p")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--include-partial-messages")
        .current_dir(launch.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    match project_mcp_launch(launch.cwd, dirs::home_dir().as_deref()) {
        Ok(Some(mcp)) => {
            if mcp.strict {
                cmd.arg("--strict-mcp-config");
            }
            cmd.arg(mcp.arg);
        }
        Ok(None) => {}
        Err(error) => {
            cmd.arg("--strict-mcp-config");
            eprintln!("! Qoder project tools were left out this turn: {error:#}");
        }
    }
    if launch.computer_mcp {
        if let Some(path) = agent_doctor_core::claude_computer_mcp_config_path() {
            if path.is_file() {
                cmd.arg(format!("--mcp-config={}", path.display()));
            }
        }
    }
    if let Some(sid) = resume_session_id.filter(|sid| is_claude_session_id(sid)) {
        cmd.arg("--resume").arg(sid);
    }
    for path in launch.attachments.iter().take(MAX_QODER_ATTACHMENTS) {
        cmd.arg("--attachment").arg(path);
    }

    let mut ask_settings = serde_json::Map::new();
    if let Some((url, key, chosen, proto)) = resolve_qoder_overlay(launch.overlay) {
        if let Some(serde_json::Value::Object(provider_map)) =
            qoder_provider_settings(&url, &key, proto, Some(&chosen))
        {
            for (k, v) in provider_map {
                ask_settings.insert(k, v);
            }
        }
    } else if let Some(account_model) = launch
        .overlay
        .get(MODEL_ENV)
        .map(|m| m.trim())
        .filter(|m| !m.is_empty())
    {
        cmd.arg("-m").arg(account_model);
    }

    if launch.skip_permissions {
        cmd.arg("--dangerously-skip-permissions");
        cmd.stdin(Stdio::null());
    } else if launch.interactive {
        let permissions = serde_json::json!({
            "ask": [
                "Bash",
                "Edit",
                "Write",
                "NotebookEdit",
                "WebFetch",
                "WebSearch",
                "AskUserQuestion",
                "mcp__browser__*",
                "mcp__computer__*"
            ]
        });
        ask_settings.insert("permissions".into(), permissions);
        cmd.arg("--permission-prompt-tool")
            .arg("stdio")
            .arg("--append-system-prompt")
            .arg(CLAUDE_ASK_INSTRUCTIONS)
            .stdin(Stdio::piped());
    } else {
        // Without `--permission-prompt-tool stdio`, `qodercli -p` reads non-TTY
        // stdin until EOF before starting the turn; `Stdio::null()` avoids blocking.
        cmd.stdin(Stdio::null());
    }

    if !ask_settings.is_empty() {
        cmd.arg("--settings")
            .arg(write_qoder_ask_settings(&serde_json::Value::Object(
                ask_settings,
            ))?);
    }
    cmd.arg("--").arg(prompt);
    apply_overlay_env(&mut cmd, launch.overlay);
    apply_qoder_env(&mut cmd, launch.overlay);
    Ok(cmd)
}

fn write_qoder_ask_settings(settings: &serde_json::Value) -> Result<PathBuf> {
    use std::hash::{Hash, Hasher};
    let body = settings.to_string();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut hasher);
    let dir = ask_private_dir()?;
    let path = dir.join(format!("qoder-settings-{:016x}.json", hasher.finish()));
    if std::fs::read_to_string(&path).ok().as_deref() != Some(body.as_str()) {
        std::fs::write(&path, &body).with_context(|| format!("write {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
    Ok(path)
}

fn is_missing_qoder_resume(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("error resuming session")
        || lower.contains("invalid session identifier")
        || lower.contains("no conversation found")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::prompt_session::util::TEST_ENV_LOCK;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    struct EnvRestore(Vec<(&'static str, Option<String>)>);

    impl EnvRestore {
        fn new(keys: &[&'static str]) -> Self {
            Self(keys.iter().map(|&k| (k, std::env::var(k).ok())).collect())
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            for (key, prev) in &self.0 {
                match prev {
                    Some(val) => std::env::set_var(key, val),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    const FAKE_QODER_STREAM_JSON: &str = r#"#!/usr/bin/env python3
import json, os, sys
log_path = os.environ["AD_QODER_LOG"]
argv = sys.argv[1:]
settings_payload = {}
if "--settings" in argv:
    idx = argv.index("--settings")
    with open(argv[idx + 1], "r", encoding="utf-8") as fh:
        settings_payload = json.load(fh)
with open(log_path, "a", encoding="utf-8") as handle:
    handle.write(json.dumps({
        "argv": argv,
        "qoder_model": os.environ.get("QODER_MODEL"),
        "settings": settings_payload,
    }) + "\n")
if "--resume" in argv:
    ridx = argv.index("--resume")
    sid = argv[ridx + 1]
    if sid == "00000000-0000-0000-0000-000000000000":
        sys.stderr.write(f'Error resuming session: Invalid session identifier "{sid}".\n')
        sys.exit(1)
sid = "358d9b8e-f820-4489-95d3-03b9c93177d6"
sys.stdout.write(json.dumps({"type": "system", "subtype": "init", "session_id": sid}) + "\n")
sys.stdout.write(json.dumps({
    "type": "stream_event",
    "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "qoder-provider-ok"}},
    "session_id": sid,
}) + "\n")
sys.stdout.write(json.dumps({
    "type": "result",
    "subtype": "success",
    "is_error": False,
    "result": "qoder-provider-ok",
    "session_id": sid,
}) + "\n")
sys.stdout.flush()
"#;

    #[test]
    fn qoder_uses_configured_provider_without_requiring_qoder_login() {
        let _guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _restore = EnvRestore::new(&[
            "AGENT_DOCTOR_QODER_BIN",
            "AD_QODER_LOG",
            "OPENAI_BASE_URL",
            "OPENAI_API_KEY",
            "AGENT_DOCTOR_MODEL",
            "HOME",
        ]);
        let dir = tempdir().unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let log_path = dir.path().join("qoder.log");
        let bin = dir.path().join("fake-qodercli");
        fs::write(&bin, FAKE_QODER_STREAM_JSON).unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();

        std::env::set_var("HOME", &home);
        std::env::set_var("AGENT_DOCTOR_QODER_BIN", &bin);
        std::env::set_var("AD_QODER_LOG", &log_path);
        std::env::set_var("OPENAI_BASE_URL", "https://agentdoctor.cn/api/v1/official");
        std::env::set_var("OPENAI_API_KEY", "lic-official-test");
        std::env::set_var("AGENT_DOCTOR_MODEL", "deepseek-flash");

        let opts = PromptSessionOptions {
            runtime: "qoder".into(),
            prompt: "你好".into(),
            cwd: Some(dir.path().to_path_buf()),
            timeout_sec: 30,
            dangerously_skip_permissions: false,
            full_auto: false,
            resume_thread_id: Some("00000000-0000-0000-0000-000000000000".into()),
            selected_mcps: Vec::new(),
            image_paths: Vec::new(),
            workspace_name: None,
            provider_id: None,
            model: None,
            browser_tools_enabled: Some(false),
            computer_use_enabled: Some(false),
        };
        let report = QoderAskBackend
            .run(&opts, PromptSessionCancel::new(), None, &mut |_| {})
            .unwrap();

        assert_eq!(
            report.status,
            PromptSessionStatus::Succeeded,
            "{}",
            report.summary
        );
        assert_eq!(
            report.runtime_thread_id.as_deref(),
            Some("358d9b8e-f820-4489-95d3-03b9c93177d6")
        );
        assert!(report.log_excerpt.contains("qoder-provider-ok"));

        let lines: Vec<serde_json::Value> = fs::read_to_string(log_path)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2, "stale resume retries once cleanly");
        let second = &lines[1];
        let argv = second["argv"].as_array().unwrap();
        assert!(argv.iter().any(|a| a == "-p"));
        assert!(!argv.iter().any(|a| a == "--acp"));
        assert!(!argv.iter().any(|a| a == "--resume"));
        assert_eq!(second["qoder_model"], "agent-doctor/deepseek-flash");
        assert_eq!(
            second["settings"]["security"]["auth"]["useExternal"],
            serde_json::json!(true)
        );
        assert_eq!(
            second["settings"]["providers"]["agent-doctor"]["baseUrl"],
            "https://agentdoctor.cn/api/v1/official"
        );
        assert_eq!(
            second["settings"]["providers"]["agent-doctor"]["apiKey"],
            "lic-official-test"
        );
    }
}
