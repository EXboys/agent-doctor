//! Light Ask sessions for Claude Code, Codex, Hermes, and OpenClaw.
//!
//! UI consumes a shared [`PromptSessionEvent`] stream. Each runtime is an
//! [`AskBackend`] adapter (Claude: `control_request`; Codex: `app-server`;
//! Hermes/OpenClaw: headless CLI).

mod backend;
mod claude;
mod codex_app_server;
mod control;
mod env;
mod hermes;
mod mcp_ensure;
mod openclaw;
mod util;

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::evotown::normalize_runtime;

pub use backend::AskBackend;
pub use control::PromptSessionControl;

use claude::ClaudeAskBackend;
use codex_app_server::CodexAskBackend;
use hermes::HermesAskBackend;
use openclaw::OpenClawAskBackend;

static SESSION_SEQ: AtomicU64 = AtomicU64::new(1);

pub(crate) const DEFAULT_TIMEOUT_SEC: u64 = 600;
pub(crate) const MAX_TIMEOUT_SEC: u64 = 3600;
pub(crate) const MIN_TIMEOUT_SEC: u64 = 1;
/// Cap retained combined output for the final report (UI still streams live).
pub(crate) const MAX_CAPTURE_CHARS: usize = 200_000;
pub(crate) const SUMMARY_CHARS: usize = 1_500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSessionOptions {
    pub runtime: String,
    pub prompt: String,
    pub cwd: Option<std::path::PathBuf>,
    #[serde(default = "default_timeout_sec")]
    pub timeout_sec: u64,
    /// Claude Code / Hermes. Default false — UI must confirm before enabling.
    #[serde(default)]
    pub dangerously_skip_permissions: bool,
    /// Codex / OpenClaw elevated mode. Default false.
    #[serde(default)]
    pub full_auto: bool,
    /// Resume an existing thread/session id when set (Codex/Claude/Hermes/OpenClaw).
    #[serde(default)]
    pub resume_thread_id: Option<String>,
    /// MCP server names selected in Ask (e.g. `browser`). Wired into Claude/Codex config before spawn.
    #[serde(default)]
    pub selected_mcps: Vec<String>,
}

fn default_timeout_sec() -> u64 {
    DEFAULT_TIMEOUT_SEC
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptSessionStatus {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PromptSessionEvent {
    Started {
        session_id: String,
        runtime: String,
        cwd: String,
        command: String,
    },
    /// Live status for the ask UI (requesting / thinking / writing / tool).
    Status {
        session_id: String,
        phase: String,
        message: String,
    },
    /// Incremental assistant text (may be partial; not necessarily a full line).
    Delta {
        session_id: String,
        text: String,
    },
    StdoutLine {
        session_id: String,
        line: String,
    },
    StderrLine {
        session_id: String,
        line: String,
    },
    /// Tool / command permission prompt (Claude control_request or Codex app-server).
    PermissionRequest {
        session_id: String,
        request_id: String,
        tool_name: String,
        detail: String,
        input_json: String,
    },
    /// User (or host) resolved a permission prompt.
    PermissionResolved {
        session_id: String,
        request_id: String,
        allowed: bool,
    },
    Completed {
        session_id: String,
        status: PromptSessionStatus,
        exit_code: Option<i32>,
        summary: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSessionReport {
    pub session_id: String,
    pub runtime: String,
    pub cwd: String,
    pub status: PromptSessionStatus,
    pub exit_code: Option<i32>,
    pub summary: String,
    pub log_excerpt: String,
    pub duration_ms: u64,
    /// Codex thread id or Claude session id for the next turn's resume.
    #[serde(default)]
    pub runtime_thread_id: Option<String>,
}

/// Cooperative cancel flag shared with a running session.
#[derive(Debug, Clone, Default)]
pub struct PromptSessionCancel {
    inner: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl PromptSessionCancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(&self) {
        self.inner.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_requested(&self) -> bool {
        self.inner.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn handle(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.inner)
    }
}

pub fn next_session_id() -> String {
    let n = SESSION_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("ps-{n}")
}

/// Run a one-shot prompt session, streaming lines via `on_event`.
pub fn run_prompt_session<F>(
    options: &PromptSessionOptions,
    on_event: F,
) -> Result<PromptSessionReport>
where
    F: FnMut(PromptSessionEvent),
{
    run_prompt_session_with_cancel(options, PromptSessionCancel::new(), None, on_event)
}

pub fn run_prompt_session_with_cancel<F>(
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    control: Option<PromptSessionControl>,
    mut on_event: F,
) -> Result<PromptSessionReport>
where
    F: FnMut(PromptSessionEvent),
{
    let runtime = normalize_runtime(&options.runtime);
    match runtime.as_str() {
        "claude-code" => ClaudeAskBackend.run(options, cancel, control, &mut on_event),
        "codex" => CodexAskBackend.run(options, cancel, control, &mut on_event),
        "hermes" => HermesAskBackend.run(options, cancel, control, &mut on_event),
        "openclaw" => OpenClawAskBackend.run(options, cancel, control, &mut on_event),
        other => bail!("ask supports claude-code, codex, hermes, or openclaw (got '{other}')"),
    }
}

#[cfg(test)]
mod tests {
    use super::util::TEST_ENV_LOCK;
    use super::*;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[cfg(unix)]
    fn write_fake_bin(dir: &std::path::Path, name: &str, script: &str) -> std::path::PathBuf {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, script).expect("write fake bin");
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[test]
    fn rejects_empty_prompt_and_unknown_runtime() {
        let err = run_prompt_session(
            &PromptSessionOptions {
                runtime: "claude-code".into(),
                prompt: "   ".into(),
                cwd: None,
                timeout_sec: 30,
                dangerously_skip_permissions: false,
                full_auto: false,
                resume_thread_id: None,
                selected_mcps: Vec::new(),
            },
            |_| {},
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("empty"));

        let err = run_prompt_session(
            &PromptSessionOptions {
                runtime: "not-a-runtime".into(),
                prompt: "hi".into(),
                cwd: None,
                timeout_sec: 30,
                dangerously_skip_permissions: false,
                full_auto: false,
                resume_thread_id: None,
                selected_mcps: Vec::new(),
            },
            |_| {},
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("claude-code, codex, hermes, or openclaw"));
    }

    #[test]
    fn filters_codex_banner_stderr() {
        assert!(util::is_runtime_stderr_noise("OpenAI Codex v0.145.0"));
        assert!(util::is_runtime_stderr_noise(
            "Reading additional input from stdin..."
        ));
        assert!(!util::is_runtime_stderr_noise(
            "ERROR: Missing environment variable"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn cancels_long_running_process() {
        let _guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        // Codex path now uses app-server; fake script answers handshake then sleeps.
        let bin = write_fake_bin(
            dir.path(),
            "fake-codex",
            r##"#!/usr/bin/env python3
import json, sys, time
def read():
    line=sys.stdin.readline()
    return json.loads(line) if line else None
msg=read(); print(json.dumps({"id":msg["id"],"result":{}}), flush=True)
msg=read()  # initialized
msg=read(); print(json.dumps({"id":msg["id"],"result":{"thread":{"id":"t1"}}}), flush=True)
msg=read(); print(json.dumps({"id":msg["id"],"result":{"turn":{"id":"u1"}}}), flush=True)
time.sleep(30)
"##,
        );
        std::env::set_var("AGENT_DOCTOR_CODEX_BIN", &bin);

        let cancel = PromptSessionCancel::new();
        let cancel_bg = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            cancel_bg.request();
        });

        let report = run_prompt_session_with_cancel(
            &PromptSessionOptions {
                runtime: "codex".into(),
                prompt: "hello".into(),
                cwd: Some(dir.path().to_path_buf()),
                timeout_sec: 60,
                dangerously_skip_permissions: false,
                full_auto: true,
                resume_thread_id: None,
                selected_mcps: Vec::new(),
            },
            cancel,
            None,
            |_| {},
        )
        .expect("session");

        std::env::remove_var("AGENT_DOCTOR_CODEX_BIN");
        assert_eq!(report.status, PromptSessionStatus::Cancelled);
        assert!(report.duration_ms < 10_000);
    }
}
