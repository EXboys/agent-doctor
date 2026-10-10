//! Codex ask backend via `codex app-server` (JSON-RPC over stdio).

use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::backend::AskBackend;
use super::control::PromptSessionControl;
use super::env::{
    apply_codex_env, apply_overlay_env, codex_provider_config_args,
    collect_overlay_env_for_options, format_command_display, prepare_codex_home,
    resolve_codex_overlay,
};
use super::mcp_ensure::{ensure_browser_mcp_for_ask, wants_browser_mcp};
use super::util::{
    combine_output, command_from_cli, force_stop_child, humanize_runtime_error,
    is_runtime_stderr_noise, strip_runtime_stderr_noise, summarize, SessionClock, ThinkingLine,
    ToolWatch,
};
use super::warm::{self, LivePipes};
use super::{
    next_session_id, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, PromptSessionStatus, MAX_TIMEOUT_SEC, MIN_TIMEOUT_SEC,
};
use crate::session_launch::resolve_session_cwd;

mod protocol;
pub(crate) use protocol::*;

type CodexPumpOutcome = (super::PumpResult, Option<String>);

static RPC_SEQ: AtomicU64 = AtomicU64::new(1);

pub struct CodexAskBackend;

impl AskBackend for CodexAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_codex_app_server(options, cancel, control, on_event)
    }
}

fn next_rpc_id() -> u64 {
    RPC_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Interactive Ask approval policy.
///
/// Use `on-request` (not `untrusted` / UnlessTrusted): the latter rejects
/// `require_escalated` and breaks agent turns. Trusted projects may still
/// auto-approve safe commands; sandbox escapes / explicit approvals still hit UI.
pub(crate) fn interactive_approval_policy() -> Value {
    json!("on-request")
}

/// No Allow button is on screen. Codex then rejects forced deletes instead of asking.
pub(crate) fn elevated_approval_policy() -> Value {
    json!("never")
}

/// Desktop chat can show Allow. Always ask, including when auto-approve is on.
///
/// `never` does not mean “allow everything”. Codex rejects `rm -f` / `rm -rf`
/// outright, so a video edit that clears a temp folder dies before the button
/// exists. Auto-approve is the chat ticking Allow, not this policy.
pub(crate) fn approval_policy_for_turn(has_ui: bool) -> Value {
    if has_ui {
        interactive_approval_policy()
    } else {
        elevated_approval_policy()
    }
}

/// `thread/start` SandboxMode — kebab-case enum.
pub(crate) fn thread_sandbox_mode() -> &'static str {
    "workspace-write"
}

/// `turn/start` SandboxPolicy — camelCase `type`.
///
/// Always include isolated `CODEX_HOME` (and the app data dir). Seatbelt
/// `workspace-write` only sees the project folder; a nested `codex` then cannot
/// read its own config and dies with `Operation not permitted (os error 1)`.
pub(crate) fn turn_sandbox_policy(cwd: &str, extra_roots: &[String]) -> Value {
    json!({
        "type": "workspaceWrite",
        "writableRoots": sandbox_writable_roots(cwd, extra_roots),
        "networkAccess": true
    })
}

fn sandbox_writable_roots(cwd: &str, extra_roots: &[String]) -> Vec<String> {
    let mut roots = Vec::new();
    let mut push = |raw: &str| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return;
        }
        if roots.iter().any(|existing| existing == trimmed) {
            return;
        }
        roots.push(trimmed.to_string());
    };
    push(cwd);
    for root in extra_roots {
        push(root);
    }
    roots
}

fn ask_sandbox_roots(overlay: &std::collections::HashMap<String, String>) -> Vec<String> {
    let mut roots = Vec::new();
    if let Some(home) = overlay
        .get("CODEX_HOME")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        roots.push(home.to_string());
    }
    if let Some(dir) = crate::adapters::util::config_dir() {
        roots.push(dir.join("agent-doctor").display().to_string());
    }
    // Videos and other files usually sit on the Desktop, in Downloads, or in
    // Movies — not inside the project folder. Temp clips sit in /tmp.
    if let Some(home) = dirs::home_dir() {
        roots.push(home.display().to_string());
    }
    let temp = std::env::temp_dir();
    if !temp.as_os_str().is_empty() {
        roots.push(temp.display().to_string());
    }
    #[cfg(unix)]
    {
        roots.push("/tmp".to_string());
    }
    roots
}

/// How long to wait for `thread/start` or `thread/resume` before giving up.
/// A stuck resume used to sit on “正在恢复 Codex 会话…” until the whole turn timed out.
const THREAD_OPEN_BUDGET: Duration = Duration::from_secs(15);

fn thread_open_request(
    resume_thread_id: Option<&str>,
    cwd: &str,
    approval_policy: &Value,
    developer_instructions: Option<&str>,
) -> (&'static str, Value) {
    let mut params = json!({
        "cwd": cwd,
        "approvalPolicy": approval_policy,
        "sandbox": thread_sandbox_mode(),
        "approvalsReviewer": "user",
    });
    if let Some(instructions) = developer_instructions {
        params["developerInstructions"] = Value::String(instructions.to_string());
    }
    if let Some(thread_id) = resume_thread_id {
        params["threadId"] = json!(thread_id);
        // Rebuilding every past turn into the response can block the reply.
        // Ask only needs the thread id; the next turn appends to it.
        params["excludeTurns"] = json!(true);
        ("thread/resume", params)
    } else {
        params["serviceName"] = json!("agent_doctor_ask");
        ("thread/start", params)
    }
}

fn is_thread_open_failure(err: &anyhow::Error) -> bool {
    format!("{err:#}").contains("codex thread did not open")
}

fn codex_ask_developer_instructions(browser_mcp: bool) -> String {
    let mut text = String::from(
        "Use ordinary shell and file tools, including when the task edits a video or other files outside the project folder, or removes temporary files. \
         If an ordinary command is blocked, call it again with require_escalated so the app can show Allow, then continue the task. \
         Do not stop, and do not tell the user to open a terminal, change settings, or avoid a command. \
         Create and edit files with the file tools already listed for you. Do not call apply_patch: this Codex rejects that call. \
         If no file tool is listed, write the file with an ordinary shell command. \
         When request_user_input is available and you need a secret, token, password, or a decision only the user can make, call that tool. \
         Do not ask the user to paste a secret into the chat.",
    );
    if browser_mcp {
        text.push_str("\n\n");
        text.push_str(super::mcp_ensure::browser_mcp_tool_instructions());
    }
    text
}

fn run_codex_app_server(
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    control: Option<PromptSessionControl>,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<PromptSessionReport> {
    let session_id = next_session_id();
    let runtime = "codex".to_string();

    let prompt = options.prompt.trim();
    if prompt.is_empty() {
        bail!("prompt must not be empty");
    }

    let cwd = resolve_session_cwd(options.cwd.as_deref(), options.workspace_name.as_deref());
    if !cwd.exists() {
        bail!("session cwd does not exist: {}", cwd.display());
    }

    let timeout_sec = options.timeout_sec.clamp(MIN_TIMEOUT_SEC, MAX_TIMEOUT_SEC);
    let overlay = collect_overlay_env_for_options(options);
    prepare_codex_home(&overlay);
    // Project-local Codex config is loaded with the message's folder, separate
    // from ~/.codex. A missing tool there fails the same way.
    let _ = crate::setup::merge::drop_unreachable_codex_mcp_servers(
        &cwd.join(".codex").join("config.toml"),
    );
    let browser_mcp = wants_browser_mcp(options);
    if browser_mcp {
        if let Some(note) = ensure_browser_mcp_for_ask("codex", &cwd, &overlay) {
            on_event(PromptSessionEvent::Status {
                session_id: session_id.clone(),
                phase: "mcp".into(),
                message: note,
            });
        }
    }

    let cmd = build_app_server_command(&cwd, &overlay)?;
    let command_display = format_command_display(&cmd);

    on_event(PromptSessionEvent::Started {
        session_id: session_id.clone(),
        runtime: runtime.clone(),
        cwd: cwd.display().to_string(),
        command: command_display,
        client_run_id: String::new(),
    });

    let started = Instant::now();

    let has_ui_control = control.is_some();
    let control = control.unwrap_or_default();

    // Auto-approve still asks Codex, then the chat accepts. `never` would
    // reject forced deletes before that button exists.
    let interactive = has_ui_control;
    let approval_policy = approval_policy_for_turn(has_ui_control);
    let resume_thread_id = options
        .resume_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let developer_instructions = if browser_mcp || interactive {
        Some(codex_ask_developer_instructions(browser_mcp))
    } else {
        None
    };

    let display_text = Arc::new(Mutex::new(String::new()));
    let display_for_cb = Arc::clone(&display_text);
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
            _ => {}
        }
        on_event(event);
    };

    let agent_prompt = if browser_mcp {
        format!(
            "{}\n\n{}",
            super::mcp_ensure::browser_mcp_tool_instructions(),
            prompt
        )
    } else {
        prompt.to_string()
    };
    let prepared = super::vision::prepare_turn_images("codex", &options.image_paths, &overlay);
    let images: Vec<String> = prepared
        .paths
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    // Temp files stay until this function returns (Codex reads them during the turn).
    let _keep_images = prepared.keep;
    let new_state = |thread_id: Option<String>, waiting_thread: Option<u64>| PumpState {
        session_id: session_id.clone(),
        waiting_thread,
        waiting_turn: None,
        thread_id,
        thread_open_error: None,
        thread_wait_started: Instant::now(),
        turn_done: false,
        interactive,
        cwd: cwd.display().to_string(),
        sandbox_roots: ask_sandbox_roots(&overlay),
        prompt: agent_prompt.clone(),
        images: images.clone(),
        approval_policy: approval_policy.clone(),
        saw_agent_delta: false,
        tools: ToolWatch::default(),
        thinking: ThinkingLine::default(),
    };

    let fingerprint =
        warm::enabled().then(|| warm::fingerprint(&cmd, &codex_config_files(&cwd, &overlay)));
    let keep_alive = fingerprint.is_some();

    let mut reused: Option<(Result<CodexPumpOutcome>, Child, LivePipes)> = None;
    if let (Some(fp), Some(thread_id)) = (fingerprint, resume_thread_id.as_deref()) {
        if let Some(parked) = warm::take("codex", thread_id, fp) {
            let warm::WarmProcess {
                mut child,
                stdin,
                mut pipes,
                ..
            } = parked;
            control.close();
            control.attach_stdin(stdin);
            let mut state = new_state(Some(thread_id.to_string()), None);
            if start_turn(&control, &mut state, &mut emit).is_ok() {
                let outcome = pump_app_server(
                    &mut child,
                    &mut pipes,
                    timeout_sec,
                    cancel.handle(),
                    &control,
                    &mut state,
                    keep_alive,
                    &mut emit,
                )
                .map(|outcome| (outcome, state.thread_id.clone()));
                reused = Some((outcome, child, pipes));
            } else {
                control.close();
                warm::retire_without_stdin(child, pipes);
            }
        }
    }

    let mut dropped_resume = false;
    let (result, child, pipes) = match reused {
        Some(done) => done,
        None => {
            let mut resume_attempt = resume_thread_id.clone();
            let mut allow_fresh_start = resume_thread_id.is_some();
            loop {
                control.close();
                let mut attempt_cmd = build_app_server_command(&cwd, &overlay)?;
                let mut child = attempt_cmd
                    .spawn()
                    .context("failed to spawn codex app-server")?;
                if let Some(stdin) = child.stdin.take() {
                    control.attach_stdin(stdin);
                } else {
                    let _ = child.kill();
                    bail!("codex app-server missing stdin pipe");
                }
                // Read stdout before any request. On Windows a full pipe blocks
                // the server, and a blocked server never reads the next line.
                let mut pipes = match LivePipes::attach(&mut child) {
                    Ok(pipes) => pipes,
                    Err(err) => {
                        control.close();
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(err);
                    }
                };

                let resume_for_attempt = resume_attempt.clone();
                let attempt = (|| -> Result<CodexPumpOutcome> {
                    let init_id = next_rpc_id();
                    control.write_line(
                        &json!({
                            "method": "initialize",
                            "id": init_id,
                            "params": {
                                "clientInfo": {
                                    "name": "agent_doctor",
                                    "title": "Agent Doctor",
                                    "version": env!("CARGO_PKG_VERSION")
                                }
                            }
                        })
                        .to_string(),
                    )?;
                    control.write_line(
                        &json!({ "method": "initialized", "params": {} }).to_string(),
                    )?;

                    let thread_id_rpc = next_rpc_id();
                    let (thread_method, thread_params) = thread_open_request(
                        resume_for_attempt.as_deref(),
                        &cwd.display().to_string(),
                        &approval_policy,
                        developer_instructions.as_deref(),
                    );
                    emit(PromptSessionEvent::Status {
                        session_id: session_id.clone(),
                        phase: "starting".into(),
                        message: if resume_for_attempt.is_some() {
                            "正在恢复 Codex 会话…".into()
                        } else if dropped_resume {
                            "上次会话接不上，正在新开一轮…".into()
                        } else {
                            "正在启动 Codex 会话…".into()
                        },
                    });
                    control.write_line(
                        &json!({
                            "method": thread_method,
                            "id": thread_id_rpc,
                            "params": thread_params
                        })
                        .to_string(),
                    )?;

                    let mut state = new_state(resume_for_attempt.clone(), Some(thread_id_rpc));
                    let outcome = pump_app_server(
                        &mut child,
                        &mut pipes,
                        timeout_sec,
                        cancel.handle(),
                        &control,
                        &mut state,
                        keep_alive,
                        &mut emit,
                    )?;
                    Ok((outcome, state.thread_id))
                })();

                match attempt {
                    Err(err) if allow_fresh_start && is_thread_open_failure(&err) => {
                        control.close();
                        let _ = child.kill();
                        let _ = child.wait();
                        pipes.join(Duration::from_millis(500));
                        allow_fresh_start = false;
                        dropped_resume = true;
                        resume_attempt = None;
                        continue;
                    }
                    other => break (other, child, pipes),
                }
            }
        }
    };

    let duration_ms = started.elapsed().as_millis() as u64;
    let park_as = match (&result, fingerprint) {
        (Ok(((PromptSessionStatus::Succeeded, ..), Some(thread_id))), Some(fp)) => {
            Some((thread_id.clone(), fp))
        }
        _ => None,
    };
    warm::keep_or_close("codex", park_as, child, pipes, &control);

    let report = match result {
        Ok(((status, exit_code, stdout, stderr, timeout), runtime_thread_id)) => {
            let display = display_text.lock().map(|g| g.clone()).unwrap_or_default();
            let combined = if display.trim().is_empty() {
                combine_output(&stdout, &stderr)
            } else {
                display
            };
            let summary = summarize(&combined, &status, &runtime);
            emit(PromptSessionEvent::Completed {
                session_id: session_id.clone(),
                status: status.clone(),
                exit_code,
                summary: summary.clone(),
                timeout,
            });
            PromptSessionReport {
                usage: None,
                session_id,
                runtime,
                cwd: cwd.display().to_string(),
                status,
                exit_code,
                summary: summary.clone(),
                log_excerpt: combined,
                duration_ms,
                runtime_thread_id,
            }
        }
        Err(err) => {
            let summary = humanize_runtime_error(&format!("{err:#}"));
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
                cwd: cwd.display().to_string(),
                status: PromptSessionStatus::Failed,
                exit_code: None,
                summary: summary.clone(),
                log_excerpt: summary,
                duration_ms,
                runtime_thread_id: if dropped_resume {
                    None
                } else {
                    resume_thread_id
                },
            }
        }
    };
    Ok(report)
}

fn build_app_server_command(
    cwd: &std::path::Path,
    overlay: &std::collections::HashMap<String, String>,
) -> Result<Command> {
    let bin = std::env::var("AGENT_DOCTOR_CODEX_BIN").unwrap_or_else(|_| "codex".into());
    let mut cmd = command_from_cli(&bin);
    cmd.arg("app-server")
        .arg("-c")
        .arg("features.default_mode_request_user_input=true");
    for arg in codex_provider_config_args(resolve_codex_overlay(overlay).as_ref()) {
        cmd.arg(arg);
    }
    cmd.current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_overlay_env(&mut cmd, overlay);
    apply_codex_env(&mut cmd, overlay);
    Ok(cmd)
}

/// How long to keep the stdout pipe open after stdin EOF.
///
/// `codex app-server` stdio mode leaves on stdin EOF (same contract as LSP and
/// MCP). Its own watchdog can then sit for up to 45s, so Ask reaps the process
/// after this grace. The stdout reader stays up the whole time: dropping it
/// first is what makes the server log `Failed to write to stdout: Broken pipe`.
const APP_SERVER_SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

pub(crate) struct PumpState {
    session_id: String,
    waiting_thread: Option<u64>,
    waiting_turn: Option<u64>,
    thread_id: Option<String>,
    thread_open_error: Option<String>,
    thread_wait_started: Instant,
    turn_done: bool,
    interactive: bool,
    cwd: String,
    sandbox_roots: Vec<String>,
    prompt: String,
    images: Vec<String>,
    approval_policy: Value,
    saw_agent_delta: bool,
    tools: ToolWatch,
    thinking: ThinkingLine,
}

fn codex_config_files(
    cwd: &std::path::Path,
    overlay: &std::collections::HashMap<String, String>,
) -> Vec<std::path::PathBuf> {
    let mut files = vec![
        crate::adapters::util::home_join(".codex").join("config.toml"),
        cwd.join(".codex").join("config.toml"),
    ];
    let homes = [
        overlay.get("CODEX_HOME").cloned(),
        std::env::var("CODEX_HOME").ok(),
    ];
    for home in homes.into_iter().flatten() {
        files.push(std::path::PathBuf::from(home).join("config.toml"));
    }
    files
}

/// Read one turn. With `keep_alive` the server stays up after the turn so it
/// can be parked; otherwise stdin closes and the process is reaped.
#[allow(clippy::too_many_arguments)]
fn pump_app_server<F>(
    child: &mut Child,
    pipes: &mut LivePipes,
    timeout_sec: u64,
    cancel: Arc<AtomicBool>,
    control: &PromptSessionControl,
    state: &mut PumpState,
    keep_alive: bool,
    on_event: &mut F,
) -> Result<super::PumpResult>
where
    F: FnMut(PromptSessionEvent),
{
    let pid = child.id();
    let mut clock = SessionClock::new(timeout_sec);
    let status;
    let exit_code;
    let mut timeout_note = None;
    let mut shutdown_started: Option<Instant> = None;
    let mut still_running = false;

    loop {
        let drained = pipes.drain();
        if !drained.is_empty() || control.has_pending() {
            clock.touch();
        }
        for (is_stdout, line) in drained {
            if is_stdout {
                handle_rpc_line(line, control, state, on_event)?;
            } else if !is_runtime_stderr_noise(&line) {
                on_event(PromptSessionEvent::StderrLine {
                    session_id: state.session_id.clone(),
                    line: humanize_runtime_error(&line),
                });
            }
        }
        clock.set_tool_running(state.tools.running());

        if let Some(message) = state.thread_open_error.clone() {
            force_stop_child(child, pid);
            pipes.join(Duration::from_millis(500));
            bail!("codex thread did not open: {message}");
        }
        if state.waiting_thread.is_some()
            && state.thread_wait_started.elapsed() >= THREAD_OPEN_BUDGET
        {
            force_stop_child(child, pid);
            pipes.join(Duration::from_millis(500));
            bail!("codex thread did not open: timed out");
        }

        if cancel.load(Ordering::SeqCst) {
            force_stop_child(child, pid);
            status = PromptSessionStatus::Cancelled;
            exit_code = None;
            break;
        }
        if clock.expired() {
            timeout_note = Some(clock.timeout_note(state.tools.last_label()));
            force_stop_child(child, pid);
            status = PromptSessionStatus::TimedOut;
            exit_code = None;
            break;
        }

        if state.turn_done && keep_alive && matches!(child.try_wait(), Ok(None)) {
            status = PromptSessionStatus::Succeeded;
            exit_code = None;
            still_running = true;
            break;
        }
        // One-shot Ask: stdin EOF is the server's shutdown signal. Keep reading
        // stdout until the process leaves or the grace expires, then SIGKILL.
        if state.turn_done && shutdown_started.is_none() {
            control.close();
            shutdown_started = Some(Instant::now());
        }

        match child.try_wait() {
            Ok(Some(wait_status)) => {
                exit_code = wait_status.code();
                // A finished turn stays successful even if the server's shutdown
                // exit code is non-zero. Leaving before the turn is a failure.
                status = if state.turn_done {
                    PromptSessionStatus::Succeeded
                } else {
                    PromptSessionStatus::Failed
                };
                break;
            }
            Ok(None) => {
                if shutdown_started
                    .is_some_and(|started| started.elapsed() >= APP_SERVER_SHUTDOWN_GRACE)
                {
                    force_stop_child(child, pid);
                    status = PromptSessionStatus::Succeeded;
                    exit_code = None;
                    break;
                }
                thread::sleep(Duration::from_millis(40));
            }
            Err(error) => return Err(error).context("failed waiting for codex app-server"),
        }
    }

    if !still_running {
        pipes.join(Duration::from_millis(500));
    }
    // Final drain
    for (is_stdout, line) in pipes.drain() {
        if is_stdout {
            let _ = handle_rpc_line(line, control, state, on_event);
        } else if !is_runtime_stderr_noise(&line) {
            on_event(PromptSessionEvent::StderrLine {
                session_id: state.session_id.clone(),
                line: humanize_runtime_error(&line),
            });
        }
    }

    let (stdout, stderr) = pipes.take_output();
    let stderr = strip_runtime_stderr_noise(&stderr);
    Ok((status, exit_code, stdout, stderr, timeout_note))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_session::control::CodexReplyKind;

    #[test]
    fn missing_program_startup_is_not_a_message_failure() {
        assert!(mcp_startup_is_missing_program(
            "MCP startup failed: No such file or directory (os error 2)"
        ));
        assert!(mcp_startup_is_missing_program(
            "系统找不到指定的路径。 (os error 3)"
        ));
        assert!(mcp_startup_is_missing_program(
            "MCP startup failed: program not found"
        ));
        assert!(!mcp_startup_is_missing_program("connection refused"));
    }

    #[test]
    fn shortens_zsh_lc_wrappers() {
        assert_eq!(shorten_tool_label("/bin/zsh -lc pwd"), "pwd");
        assert_eq!(shorten_tool_label("/bin/zsh -lc 'ls -la'"), "ls -la");
        assert_eq!(shorten_tool_label("echo hi"), "echo hi");
    }

    #[test]
    fn resume_asks_for_metadata_only_and_start_does_not() {
        let (_method, resume) =
            thread_open_request(Some("thr_1"), "/tmp", &json!("on-request"), None);
        assert_eq!(resume["threadId"], "thr_1");
        assert_eq!(resume["excludeTurns"], true);
        assert!(resume.get("developerInstructions").is_none());
        assert!(resume.get("serviceName").is_none());

        let (method, start) =
            thread_open_request(None, "/tmp", &json!("on-request"), Some("be brief"));
        assert_eq!(method, "thread/start");
        assert!(start.get("excludeTurns").is_none());
        assert!(start.get("threadId").is_none());
        assert_eq!(start["developerInstructions"], "be brief");
        assert_eq!(start["serviceName"], "agent_doctor_ask");
    }

    #[test]
    fn turn_input_adds_pictures_after_text() {
        let input = protocol::turn_input("看图", &["/tmp/a.png".into()]);
        assert_eq!(input[0], json!({"type": "text", "text": "看图"}));
        assert_eq!(
            input[1],
            json!({"type": "localImage", "path": "/tmp/a.png"})
        );
        assert_eq!(protocol::turn_input("hi", &[]).as_array().unwrap().len(), 1);
    }

    #[test]
    fn thread_open_error_does_not_pretend_the_turn_finished() {
        let mut state = PumpState {
            session_id: "s1".into(),
            waiting_thread: Some(7),
            waiting_turn: None,
            thread_id: Some("thr_old".into()),
            thread_open_error: None,
            thread_wait_started: Instant::now(),
            turn_done: false,
            interactive: true,
            cwd: "/tmp".into(),
            sandbox_roots: Vec::new(),
            prompt: "hi".into(),
            images: Vec::new(),
            approval_policy: json!("on-request"),
            saw_agent_delta: false,
            tools: ToolWatch::default(),
            thinking: ThinkingLine::default(),
        };
        let control = PromptSessionControl::new();
        handle_rpc_response(
            &json!({"id": 7, "error": {"message": "no rollout found for thread id"}}),
            &json!(7),
            &control,
            &mut state,
            &mut |_| {},
        )
        .expect("response");
        assert!(!state.turn_done);
        assert!(state.waiting_thread.is_none());
        assert!(state
            .thread_open_error
            .as_deref()
            .unwrap_or("")
            .contains("no rollout"));
    }

    #[test]
    fn older_and_newer_codex_events_still_surface() {
        let mut state = PumpState {
            session_id: "s1".into(),
            waiting_thread: None,
            waiting_turn: None,
            thread_id: None,
            thread_open_error: None,
            thread_wait_started: Instant::now(),
            turn_done: false,
            interactive: false,
            cwd: "/tmp".into(),
            sandbox_roots: Vec::new(),
            prompt: "hi".into(),
            images: Vec::new(),
            approval_policy: json!("on-request"),
            saw_agent_delta: false,
            tools: ToolWatch::default(),
            thinking: ThinkingLine::default(),
        };
        let mut events = Vec::new();
        handle_notification(
            "item/agent_message/delta",
            Some(&json!({"delta": "旧版回复"})),
            &mut state,
            &mut |event| events.push(event),
        );
        handle_notification(
            "error",
            Some(&json!({"error": {"message": "配额用完了"}})),
            &mut state,
            &mut |event| events.push(event),
        );
        handle_notification(
            "item/started",
            Some(&json!({"item": {"type": "webSearch", "query": "登录按钮"}})),
            &mut state,
            &mut |event| events.push(event),
        );
        handle_notification(
            "thread/tokenUsage/updated",
            Some(&json!({"tokenUsage": {
                "total": {"inputTokens": 9000, "cachedInputTokens": 0, "outputTokens": 900},
                "last": {"inputTokens": 1200, "cachedInputTokens": 1000, "outputTokens": 40}
            }})),
            &mut state,
            &mut |event| events.push(event),
        );
        handle_notification(
            "turn/completed",
            Some(&json!({"turn": {"status": "failed", "error": {"message": "这一轮失败了"}}})),
            &mut state,
            &mut |event| events.push(event),
        );
        assert!(events.iter().any(|event| matches!(
            event,
            PromptSessionEvent::Usage { usage, .. }
                if usage.input == 200 && usage.cache_read == 1000 && usage.output == 40
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            PromptSessionEvent::Delta { text, .. } if text == "旧版回复"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            PromptSessionEvent::StderrLine { line, .. } if line.contains("配额用完了")
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            PromptSessionEvent::Status { message, .. } if message.contains("登录按钮")
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            PromptSessionEvent::StderrLine { line, .. } if line.contains("这一轮失败了")
        )));
        assert!(state.turn_done);
    }

    #[test]
    fn reads_camel_case_agent_message_without_deltas() {
        assert!(is_agent_message_type("agentMessage"));
        assert!(is_agent_message_type("agent_message"));
        assert_eq!(
            agent_item_text(&json!({"type": "agentMessage", "text": "你好"})).as_deref(),
            Some("你好")
        );
        assert_eq!(
            json_delta_text(&json!({"delta": {"text": "hi"}})).as_deref(),
            Some("hi")
        );
        assert_eq!(
            turn_completed_agent_text(&json!({
                "turn": {
                    "status": "completed",
                    "items": [
                        {"type": "agentMessage", "text": "hello from turn"}
                    ]
                }
            }))
            .as_deref(),
            Some("hello from turn")
        );
        assert!(matches!(
            codex_reply_kind(
                "item/tool/requestUserInput",
                &json!({"questions": [{"id": "token", "question": "Paste token"}]})
            ),
            Some(CodexReplyKind::UserLine {
                elicitation: false,
                ..
            })
        ));
        assert_eq!(
            permission_input_mode(
                "item/tool/requestUserInput",
                &json!({"questions": [{"id": "token", "question": "Paste token"}]})
            ),
            "secret"
        );
        assert_eq!(
            permission_input_mode(
                "item/tool/requestUserInput",
                &json!({"questions": [{"id": "go", "question": "Continue? (y/n)"}]})
            ),
            "line"
        );
        assert!(matches!(
            codex_reply_kind("mcpServer/elicitation/request", &json!({})),
            Some(CodexReplyKind::Elicitation)
        ));
        let approval = json!({
            "serverName": "browser",
            "message": "Allow the browser MCP server to run tool \"browser_navigate\"?",
            "requestedSchema": { "type": "object", "properties": {} }
        });
        assert!(matches!(
            codex_reply_kind("mcpServer/elicitation/request", &approval),
            Some(CodexReplyKind::Elicitation)
        ));
        assert_eq!(
            permission_input_mode("mcpServer/elicitation/request", &approval),
            "choice"
        );
        let (tool, detail, _) = permission_from_codex("mcpServer/elicitation/request", &approval);
        assert_eq!(tool, "MCP browser");
        assert!(detail.contains("browser_navigate"));
        let form = json!({
            "serverName": "browser",
            "message": "Which page should be opened?",
            "requestedSchema": {
                "type": "object",
                "properties": { "url": { "type": "string" } }
            }
        });
        assert_eq!(
            permission_input_mode("mcpServer/elicitation/request", &form),
            "line"
        );
    }

    #[test]
    fn temp_folder_deletes_continue_without_a_prompt() {
        assert!(protocol::temp_only_delete(&json!({
            "command": ["/bin/zsh", "-c", "rm -rf /tmp/clips && echo done"]
        })));
        assert!(!protocol::temp_only_delete(&json!({
            "command": "rm -rf /tmp/clips ~/Movies/cut.mp4"
        })));
        assert!(!protocol::temp_only_delete(&json!({
            "command": "rm -rf /Users/me/Desktop/video.mp4"
        })));
        assert!(!protocol::temp_only_delete(&json!({
            "command": "ffmpeg -i /Users/me/Desktop/a.mp4 /Users/me/Desktop/b.mp4"
        })));
    }

    #[test]
    fn permission_from_command_approval() {
        let params = json!({
            "command": ["curl", "-s", "https://example.com"],
            "reason": "network",
            "cwd": "/tmp"
        });
        let (tool, detail, _) =
            permission_from_codex("item/commandExecution/requestApproval", &params);
        assert_eq!(tool, "Bash");
        assert_eq!(detail, "network");
    }

    #[test]
    fn interactive_policy_is_on_request_not_unless_trusted() {
        assert_eq!(interactive_approval_policy(), json!("on-request"));
        assert_eq!(elevated_approval_policy(), json!("never"));
        assert_eq!(approval_policy_for_turn(true), json!("on-request"));
        assert_eq!(approval_policy_for_turn(false), json!("never"));
        assert_eq!(thread_sandbox_mode(), "workspace-write");
        let policy =
            turn_sandbox_policy("/tmp/proj", &["/tmp/codex-home".into(), "/tmp/proj".into()]);
        assert_eq!(policy["type"], "workspaceWrite");
        assert_eq!(
            policy["writableRoots"],
            json!(["/tmp/proj", "/tmp/codex-home"])
        );
        let roots = ask_sandbox_roots(&std::collections::HashMap::new());
        assert!(roots
            .iter()
            .any(|root| root == "/tmp" || root.contains("tmp")));
        if let Some(home) = dirs::home_dir() {
            let home = home.display().to_string();
            assert!(roots.iter().any(|root| root == &home));
        }
    }

    #[cfg(unix)]
    #[test]
    fn app_server_permission_allow_via_control() {
        use crate::prompt_session::util::TEST_ENV_LOCK;
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::sync::Mutex as StdMutex;
        use tempfile::tempdir;

        let _guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        fn write_fake_bin(dir: &Path, name: &str, script: &str) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join(name);
            fs::write(&path, script).expect("write fake bin");
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).unwrap();
            path
        }

        let dir = tempdir().unwrap();
        let bin = write_fake_bin(
            dir.path(),
            "fake-codex",
            r##"#!/usr/bin/env python3
import json, sys

def read():
    line = sys.stdin.readline()
    if not line:
        return None
    return json.loads(line)

# initialize
msg = read()
assert msg["method"] == "initialize"
print(json.dumps({"id": msg["id"], "result": {"userAgent": "fake"}}), flush=True)
msg = read()
assert msg["method"] == "initialized"

# thread/start
msg = read()
assert msg["method"] == "thread/start"
assert msg["params"]["approvalPolicy"] == "on-request"
assert msg["params"]["sandbox"] == "workspace-write"
print(json.dumps({"id": msg["id"], "result": {"thread": {"id": "thr_1"}}}), flush=True)

# turn/start
msg = read()
assert msg["method"] == "turn/start"
assert msg["params"]["sandboxPolicy"]["type"] == "workspaceWrite"
print(json.dumps({"id": msg["id"], "result": {"turn": {"id": "turn_1", "status": "inProgress"}}}), flush=True)
print(json.dumps({"method": "turn/started", "params": {"turn": {"id": "turn_1"}}}), flush=True)
print(json.dumps({
  "id": 99,
  "method": "item/commandExecution/requestApproval",
  "params": {"command": ["echo", "hi"], "reason": "run echo"}
}), flush=True)

# wait for decision
msg = read()
assert msg["id"] == 99
assert msg["result"]["decision"] == "accept"

print(json.dumps({
  "method": "item/completed",
  "params": {"item": {"id": "i1", "type": "agent_message", "text": "codex-ok"}}
}), flush=True)
print(json.dumps({"method": "turn/completed", "params": {"turn": {"id": "turn_1"}}}), flush=True)
import time
time.sleep(5)
"##,
        );
        std::env::set_var("AGENT_DOCTOR_CODEX_BIN", &bin);

        let control = PromptSessionControl::new();
        let events = StdMutex::new(Vec::new());
        let control_for_reply = control.clone();
        thread::spawn(move || {
            for _ in 0..200 {
                thread::sleep(Duration::from_millis(20));
                if control_for_reply.respond_permission("99", true).is_ok() {
                    return;
                }
            }
        });

        let report = CodexAskBackend
            .run(
                &PromptSessionOptions {
                    runtime: "codex".into(),
                    prompt: "run echo".into(),
                    cwd: Some(dir.path().to_path_buf()),
                    timeout_sec: 10,
                    dangerously_skip_permissions: false,
                    full_auto: false,
                    resume_thread_id: None,
                    selected_mcps: Vec::new(),
                    image_paths: Vec::new(),
                    workspace_name: None,
                    provider_id: None,
                    model: None,
                },
                PromptSessionCancel::new(),
                Some(control),
                &mut |ev| events.lock().unwrap().push(ev),
            )
            .expect("session");
        std::env::remove_var("AGENT_DOCTOR_CODEX_BIN");
        assert_eq!(
            report.status,
            PromptSessionStatus::Succeeded,
            "summary={} events={:?}",
            report.summary,
            events
                .lock()
                .unwrap()
                .iter()
                .map(|e| match e {
                    PromptSessionEvent::Started { .. } => "started".into(),
                    PromptSessionEvent::Status { message, .. } => format!("status:{message}"),
                    PromptSessionEvent::Delta { text, .. } => format!("delta:{text}"),
                    PromptSessionEvent::PermissionRequest { tool_name, .. } => {
                        format!("perm:{tool_name}")
                    }
                    PromptSessionEvent::StderrLine { line, .. } => format!("err:{line}"),
                    PromptSessionEvent::Completed { status, .. } => format!("done:{status:?}"),
                    _ => "other".into(),
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(report.runtime_thread_id.as_deref(), Some("thr_1"));
        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(
            e,
            PromptSessionEvent::PermissionRequest { tool_name, .. } if tool_name == "Bash"
        )));
        assert!(evs.iter().any(|e| matches!(
            e,
            PromptSessionEvent::Delta { text, .. } if text.contains("codex-ok")
        )));
    }

    #[cfg(unix)]
    #[test]
    fn app_server_shutdown_hides_broken_pipe() {
        use crate::prompt_session::util::TEST_ENV_LOCK;
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::sync::Mutex as StdMutex;
        use tempfile::tempdir;

        let _guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        fn write_fake_bin(dir: &Path, name: &str, script: &str) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join(name);
            fs::write(&path, script).expect("write fake bin");
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).unwrap();
            path
        }

        let dir = tempdir().unwrap();
        let bin = write_fake_bin(
            dir.path(),
            "fake-codex",
            r##"#!/usr/bin/env python3
import json, sys

def read():
    line = sys.stdin.readline()
    if not line:
        return None
    return json.loads(line)

msg = read()
print(json.dumps({"id": msg["id"], "result": {"userAgent": "fake"}}), flush=True)
read()  # initialized
msg = read()
print(json.dumps({"id": msg["id"], "result": {"thread": {"id": "thr_bye"}}}), flush=True)
msg = read()
print(json.dumps({"id": msg["id"], "result": {"turn": {"id": "turn_1"}}}), flush=True)
print(json.dumps({
  "method": "turn/completed",
  "params": {"turn": {"id": "turn_1", "items": [{"type": "agentMessage", "text": "done-ok"}]}}
}), flush=True)
print("provider rejected the key", file=sys.stderr, flush=True)
print(
    "2026-09-28T09:19:04.710092Z ERROR codex_app_server_transport::transport::stdio: Failed to write to stdout: Broken pipe (os error 32)",
    file=sys.stderr,
    flush=True,
)
# Stdio servers exit when the client closes stdin. Don't block the test if
# the parent is still flushing.
sys.exit(0)
"##,
        );
        std::env::set_var("AGENT_DOCTOR_CODEX_BIN", &bin);
        let events = StdMutex::new(Vec::new());
        let report = CodexAskBackend
            .run(
                &PromptSessionOptions {
                    runtime: "codex".into(),
                    prompt: "hi".into(),
                    cwd: Some(dir.path().to_path_buf()),
                    timeout_sec: 10,
                    dangerously_skip_permissions: false,
                    full_auto: true,
                    resume_thread_id: None,
                    selected_mcps: Vec::new(),
                    image_paths: Vec::new(),
                    workspace_name: None,
                    provider_id: None,
                    model: None,
                },
                PromptSessionCancel::new(),
                Some(PromptSessionControl::new()),
                &mut |ev| events.lock().unwrap().push(ev),
            )
            .expect("session");
        std::env::remove_var("AGENT_DOCTOR_CODEX_BIN");
        assert_eq!(
            report.status,
            PromptSessionStatus::Succeeded,
            "{}",
            report.summary
        );
        assert!(!report.summary.to_ascii_lowercase().contains("broken pipe"));
        assert!(!report
            .log_excerpt
            .to_ascii_lowercase()
            .contains("broken pipe"));
        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(
            e,
            PromptSessionEvent::Delta { text, .. } if text.contains("done-ok")
        )));
        assert!(evs.iter().any(|e| matches!(
            e,
            PromptSessionEvent::StderrLine { line, .. } if line.contains("provider rejected")
        )));
        assert!(!evs.iter().any(|e| matches!(
            e,
            PromptSessionEvent::StderrLine { line, .. } if line.to_ascii_lowercase().contains("broken pipe")
        )));
    }
}
