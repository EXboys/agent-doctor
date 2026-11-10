//! DeepSeek Harness ask backend (`dsh --profile acp`).
//!
//! The headless profile answers one task and exits, and it cannot resume.
//! The shipped `acp` profile stays up and speaks Agent Client Protocol on
//! stdin/stdout: `session/new` or `session/resume`, then `session/prompt`.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{bail, Context, Result};

#[cfg(test)]
use serde_json::{json, Value};
#[cfg(test)]
use std::collections::HashMap;
#[cfg(any(test, windows))]
use std::path::PathBuf;

use super::acp::{pump_acp, AcpProfile};
use super::backend::AskBackend;
use super::control::PromptSessionControl;
use super::util::{command_from_cli, summarize, turn_error};
use super::vision::{encode_image, turn_images, EncodedImage};
use super::warm::{self, LivePipes};
use super::{
    next_session_id, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, PromptSessionStatus, MAX_TIMEOUT_SEC, MIN_TIMEOUT_SEC,
};
use agent_doctor_core::adapters::{DEEPSEEK_HARNESS_CLI, DEEPSEEK_HARNESS_RUNTIME_ID};
use agent_doctor_core::session_env::{
    apply_deepseek_harness_env, apply_overlay_env, format_command_display,
};
use agent_doctor_core::session_launch::resolve_session_cwd;

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
    #[cfg(windows)]
    reject_windows_drive_root(&cwd)?;
    let timeout_sec = options.timeout_sec.clamp(MIN_TIMEOUT_SEC, MAX_TIMEOUT_SEC);
    let resume_session_id = options
        .resume_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let auto_allow = options.dangerously_skip_permissions || options.full_auto;
    let overlay = super::collect_overlay_env_for_options(options);
    let hands = super::mcp_ensure::wire_hands(
        "deepseek-harness",
        options,
        &cwd,
        &overlay,
        |phase, message| {
            on_event(PromptSessionEvent::Status {
                session_id: session_id.clone(),
                phase: phase.into(),
                message,
            })
        },
    );
    let prompt = hands.prompt(prompt);
    let images: Vec<EncodedImage> = turn_images(RUNTIME_KEY, &options.image_paths, &overlay)
        .iter()
        .filter_map(|p| encode_image(p))
        .collect();

    ensure_chat_capable_dsh(&session_id, on_event)?;

    let mut cmd = build_command(&cwd, &overlay);
    let command_display = format_command_display(&cmd);
    let fingerprint = warm::enabled().then(|| {
        warm::fingerprint(
            &cmd,
            &[agent_doctor_core::DeepSeekHarnessAdapter::cordis_patch_path()],
        )
    });

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
            &prompt,
            images,
            already_open,
            resume_session_id.as_deref(),
            auto_allow,
            AcpProfile::deepseek(),
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
            if display.trim().is_empty() && turn.status != PromptSessionStatus::Succeeded {
                if let Some(error) = turn.error.as_deref() {
                    on_event(turn_error(&session_id, error));
                }
            }
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
    emit(turn_error(&session_id, error));
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

/// Old `dsh` builds have no `acp` profile and exit before the first message.
fn ensure_chat_capable_dsh(
    session_id: &str,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<()> {
    if std::env::var_os("AGENT_DOCTOR_DSH_BIN").is_some() {
        return Ok(());
    }
    let Some(installed) = agent_doctor_core::lifecycle::installed_deepseek_harness_version() else {
        return Ok(());
    };
    if !agent_doctor_core::lifecycle::deepseek_harness_is_outdated(&installed) {
        return Ok(());
    }
    on_event(PromptSessionEvent::Status {
        session_id: session_id.to_string(),
        phase: "setup".into(),
        message: "DeepSeek 助手版本太旧，正在更新，第一次要等一两分钟。".into(),
    });
    agent_doctor_core::lifecycle::run_deepseek_harness_lifecycle(
        agent_doctor_core::lifecycle::DeepSeekHarnessLifecycleAction::Update,
    )
    .map_err(|_| anyhow::anyhow!("DeepSeek 助手版本太旧，自动更新没有成功。请检查网络后再发一次。"))
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
    #[cfg(windows)]
    apply_safe_child_temp(&mut command);
    command
}

/// `C:`, `C:\`, `C:/` — the drive-root spellings Node's `fs.realpath` cannot
/// resolve (`EISDIR: illegal operation on a directory, realpath 'C:'`,
/// nodejs/node#6861, still open in libuv). Pure string shape, so it holds the
/// same meaning on every platform and stays testable off-Windows.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_windows_drive_root(path: &Path) -> bool {
    let Some(text) = path.to_str() else {
        return false;
    };
    let trimmed = text.trim().trim_matches(['\\', '/']);
    trimmed.len() == 2 && trimmed.as_bytes()[1] == b':'
}

/// dsh's Windows sandbox realpaths `os.tmpdir()` — straight from the child's
/// TEMP/TMP env — once per turn, without catching errors. A missing or
/// degenerate value fails every turn before the model can answer.
#[cfg(windows)]
fn apply_safe_child_temp(command: &mut Command) {
    for key in ["TEMP", "TMP"] {
        if let Some(temp) = swapped_temp(std::env::var_os(key).as_deref()) {
            command.env(key, temp);
        }
    }
}

/// The replacement for one broken TEMP/TMP value; `None` keeps the host value.
#[cfg(windows)]
fn swapped_temp(raw: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    if child_temp_is_usable(raw) {
        return None;
    }
    safe_windows_temp_dir()
}

#[cfg(windows)]
fn child_temp_is_usable(raw: Option<&std::ffi::OsStr>) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    let path = PathBuf::from(raw.to_string_lossy().trim());
    path.is_absolute() && path.is_dir() && !is_windows_drive_root(&path)
}

#[cfg(windows)]
fn safe_windows_temp_dir() -> Option<PathBuf> {
    let candidate = match std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        Some(local) if local.is_absolute() => local.join("Temp"),
        _ => {
            let home = std::env::var_os("USERPROFILE")?;
            PathBuf::from(home)
                .join("AppData")
                .join("Local")
                .join("Temp")
        }
    };
    if !candidate.is_dir() {
        std::fs::create_dir_all(&candidate).ok()?;
    }
    candidate.is_dir().then_some(candidate)
}

/// The chat turns fail with a Node `realpath` crash before anything else when
/// the workspace is a whole drive; dsh cannot run there, so say so instead.
#[cfg(windows)]
fn reject_windows_drive_root(cwd: &Path) -> anyhow::Result<()> {
    if is_windows_drive_root(cwd) {
        anyhow::bail!(
            "DeepSeek 不能把整个磁盘（{}）当项目目录，它启动时会检查这个路径并直接报错。\
             请在聊天里「添加项目」选一个具体的文件夹，把会话固定到它上面，再发一次。",
            cwd.display()
        );
    }
    Ok(())
}

#[cfg(test)]
pub(crate) use super::acp::{
    handle_update, prompt_blocks, strip_windows_verbatim, AcpState, Waiting,
};
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::prompt_session::util::TEST_ENV_LOCK;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use tempfile::tempdir;

    use crate::prompt_session::tool_step::ToolStep;

    fn empty_state() -> AcpState {
        AcpState {
            next_id: 1,
            waiting: HashMap::new(),
            acp_session: None,
            prompt_done: false,
            failed: None,
            resume_rejected: false,
            resume_target: None,
            tools: HashMap::new(),
            calls: HashMap::new(),
            images: Vec::new(),
            takes_images: false,
            profile: AcpProfile::deepseek(),
        }
    }

    fn replay(updates: impl IntoIterator<Item = Value>) -> Vec<PromptSessionEvent> {
        let mut state = empty_state();
        let mut events = Vec::new();
        for params in updates {
            handle_update("s1", &params, &mut state, &mut |ev| events.push(ev));
        }
        events
    }

    fn tool_rows(events: &[PromptSessionEvent]) -> usize {
        events
            .iter()
            .filter(|ev| matches!(ev, PromptSessionEvent::Status { phase, .. } if phase == "tool"))
            .count()
    }

    /// Every step sent for one call, merged the way the chat merges them by id.
    fn merged(events: &[PromptSessionEvent], id: &str) -> (ToolStep, usize) {
        let mut out = ToolStep::default();
        let mut finishes = 0;
        for ev in events {
            let PromptSessionEvent::Tool { step, .. } = ev else {
                continue;
            };
            if step.id != id {
                continue;
            }
            if step.status != "running" {
                finishes += 1;
                out.status = step.status.clone();
                out.output = step.output.clone();
                continue;
            }
            let output = std::mem::take(&mut out.output);
            out = (**step).clone();
            out.output = output;
        }
        (out, finishes)
    }

    #[test]
    fn real_dsh_tool_calls_become_one_folded_step_each() {
        let updates = include_str!("fixtures/dsh-acp-tools.jsonl")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap()["params"].clone());
        let events = replay(updates);
        assert_eq!(tool_rows(&events), 4, "one row per tool call");

        let ids: Vec<String> = events
            .iter()
            .filter_map(|ev| match ev {
                PromptSessionEvent::Tool { step, .. } => Some(step.id.clone()),
                _ => None,
            })
            .fold(Vec::new(), |mut ids, id| {
                if !ids.contains(&id) {
                    ids.push(id);
                }
                ids
            });
        assert_eq!(ids.len(), 4);

        let (bash, done) = merged(&events, &ids[0]);
        assert_eq!(bash.command, "ls -la");
        assert!(bash.output.contains("notes.txt"));
        assert_eq!(done, 1, "finished once");

        let (read, _) = merged(&events, &ids[1]);
        assert_eq!(read.kind, "read");
        assert_eq!(read.path, "/tmp/work/notes.txt");

        let (edit, _) = merged(&events, &ids[2]);
        assert_eq!(edit.kind, "edit");
        assert_eq!((edit.additions, edit.deletions), (1, 1));

        let (grep, _) = merged(&events, &ids[3]);
        assert_eq!(grep.kind, "search");
        assert_eq!(grep.query, "second");
        assert!(grep.output.contains("Line 2"));
    }

    #[test]
    fn a_spec_shaped_agent_is_read_by_kind_diff_and_location() {
        let update = |value: Value| json!({ "update": value });
        let events = replay([
            update(json!({"sessionUpdate": "tool_call", "toolCallId": "e1", "status": "pending"})),
            update(
                json!({"sessionUpdate": "tool_call_update", "toolCallId": "e1",
                          "title": "Run the tests", "kind": "execute", "rawInput": {"command": "cargo test"}}),
            ),
            update(
                json!({"sessionUpdate": "tool_call_update", "toolCallId": "e1", "status": "failed",
                          "rawOutput": {"stdout": "1 failed"}}),
            ),
            update(
                json!({"sessionUpdate": "tool_call", "toolCallId": "d1", "title": "Editing main.rs",
                          "kind": "edit", "status": "in_progress",
                          "locations": [{"path": "/w/main.rs", "line": 3}],
                          "content": [{"type": "diff", "path": "/w/main.rs", "oldText": "a\nb", "newText": "a\nc\nd"}]}),
            ),
            update(
                json!({"sessionUpdate": "tool_call_update", "toolCallId": "d1", "status": "completed"}),
            ),
        ]);
        assert_eq!(tool_rows(&events), 2);

        let (run, finishes) = merged(&events, "e1");
        assert_eq!(
            run.command, "cargo test",
            "a sentence title still reads as a command"
        );
        assert_eq!(run.title, "Run the tests");
        assert_eq!(run.status, "failed");
        assert_eq!(run.output, "1 failed");
        assert_eq!(finishes, 1);

        let (edit, _) = merged(&events, "d1");
        assert_eq!(edit.kind, "edit");
        assert_eq!(edit.path, "/w/main.rs");
        assert_eq!((edit.additions, edit.deletions), (3, 2));
    }

    #[test]
    fn a_completing_update_still_keeps_the_command() {
        let update = |value: Value| json!({ "update": value });
        let events = replay([
            update(json!({"sessionUpdate": "tool_call", "toolCallId": "e1", "status": "pending"})),
            update(
                json!({"sessionUpdate": "tool_call_update", "toolCallId": "e1",
                          "title": "Run the tests", "kind": "execute", "status": "completed",
                          "rawInput": {"command": "cargo test"},
                          "rawOutput": {"stdout": "ok"}}),
            ),
        ]);
        let (run, finishes) = merged(&events, "e1");
        assert_eq!(run.command, "cargo test");
        assert_eq!(run.title, "Run the tests");
        assert_eq!(run.status, "done");
        assert_eq!(run.output, "ok");
        assert_eq!(finishes, 1);
    }

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

    /// Every spelling Node's open `realpath` bug (nodejs/node#6861) crashes on,
    /// next to the shapes a normal project folder takes.
    #[test]
    fn drive_root_spellings_are_caught_and_normal_folders_pass() {
        for root in ["C:", r"C:\", "C:/", "c:", r"D:\ ", " C: "] {
            assert!(is_windows_drive_root(Path::new(root)), "{root}");
        }
        for real in [
            "/Users/airlu/project",
            r"C:\Users\airlu",
            r"C:\Users\airlu\AppData\Local\Temp",
            "d:/work",
        ] {
            assert!(!is_windows_drive_root(Path::new(real)), "{real}");
        }
        assert!(!is_windows_drive_root(Path::new("")));
    }

    #[test]
    fn acp_cwd_gives_dsh_plain_windows_spellings() {
        // fs::canonicalize answers verbatim on Windows; dsh must not see it.
        assert_eq!(
            strip_windows_verbatim(Path::new(r"\\?\C:\Users\airlu\project")),
            PathBuf::from(r"C:\Users\airlu\project")
        );
        assert_eq!(
            strip_windows_verbatim(Path::new(r"\\?\UNC\server\share\proj")),
            PathBuf::from(r"\\server\share\proj")
        );
        assert_eq!(
            strip_windows_verbatim(Path::new(r"C:\Users\airlu")),
            PathBuf::from(r"C:\Users\airlu")
        );
        assert_eq!(
            strip_windows_verbatim(Path::new("/tmp/work")),
            PathBuf::from("/tmp/work")
        );
    }

    /// A healthy TEMP keeps the host value; a missing, relative, nonexistent,
    /// or drive-root value is replaced. CI's rust-windows job runs this —
    /// that environment is exactly where the EISDIR incident lived.
    #[cfg(windows)]
    #[test]
    fn broken_child_temp_values_are_swapped_healthy_ones_kept() {
        let real = std::env::temp_dir();
        assert!(
            child_temp_is_usable(Some(real.as_os_str())),
            "the CI temp dir must count as usable: {}",
            real.display()
        );
        assert_eq!(
            swapped_temp(Some(real.as_os_str())),
            None,
            "a healthy value must not be touched"
        );
        for broken in [
            None,
            Some(r"C:"),
            Some("not-absolute"),
            Some(r"C:\definitely-missing"),
        ] {
            let raw = broken.map(std::ffi::OsStr::new);
            let swapped = swapped_temp(raw);
            assert!(swapped.is_some(), "{raw:?} must be swapped");
            assert!(swapped.unwrap().is_dir());
        }
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

    #[cfg(unix)]
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

    #[cfg(unix)]
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
            browser_tools_enabled: None,
            computer_use_enabled: None,
        }
    }

    #[test]
    fn a_resumed_session_replays_no_history_into_the_new_turn() {
        let mut state = empty_state();
        // session/load is in flight: the agent is replaying the saved
        // conversation as session/update traffic.
        state.waiting.insert(7, Waiting::Resume);
        let mut events = Vec::new();
        {
            let mut collect = |ev: PromptSessionEvent| events.push(ev);
            handle_update(
                "s1",
                &serde_json::json!({
                    "update": {"sessionUpdate": "tool_call", "toolCallId": "old-1",
                                "title": "Read file", "kind": "read", "status": "completed"}
                }),
                &mut state,
                &mut collect,
            );
            handle_update(
                "s1",
                &serde_json::json!({
                    "update": {"sessionUpdate": "agent_thought_chunk",
                                "content": {"type": "text", "text": "an old thought"}}
                }),
                &mut state,
                &mut collect,
            );
            handle_update(
                "s1",
                &serde_json::json!({
                    "update": {"sessionUpdate": "agent_message_chunk",
                                "content": {"type": "text", "text": "an old reply"}}
                }),
                &mut state,
                &mut collect,
            );
        }
        assert!(
            events.is_empty(),
            "replayed history stays out of the new turn"
        );

        // The load response lands, the gate lifts, live updates flow again.
        state.waiting.remove(&7);
        {
            let mut collect = |ev: PromptSessionEvent| events.push(ev);
            handle_update(
                "s1",
                &serde_json::json!({
                    "update": {"sessionUpdate": "tool_call", "toolCallId": "new-1",
                                "title": "Run tests", "kind": "execute", "status": "in_progress"}
                }),
                &mut state,
                &mut collect,
            );
        }
        assert_eq!(tool_rows(&events), 1, "only the live call surfaces");
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
