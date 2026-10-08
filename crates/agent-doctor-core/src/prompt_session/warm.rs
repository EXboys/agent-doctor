//! Keep a finished Claude / Codex process for the conversation's next message,
//! the way their own windows stay open between messages.
//!
//! Off unless the host calls [`enable_warm_sessions`]. A parked process is only
//! reused for the same runtime thread, launched the same way. Stopped, timed-out,
//! and failed turns are never parked.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use super::control::PromptSessionControl;
use super::util::{force_stop_child, join_reader, push_capped};

/// More than this and the oldest parked process is closed.
const MAX_WARM: usize = 3;
/// A parked process nobody used for this long is closed.
const IDLE_LIMIT: Duration = Duration::from_secs(10 * 60);
const REAP_EVERY: Duration = Duration::from_secs(30);
/// Time to leave on its own after stdin closes before it is stopped.
const EXIT_GRACE: Duration = Duration::from_secs(2);

static ENABLED: AtomicBool = AtomicBool::new(false);
static POOL: Mutex<Vec<WarmProcess>> = Mutex::new(Vec::new());
static REAPER: OnceLock<()> = OnceLock::new();

/// Reader threads for one child's stdout and stderr. They live as long as the
/// child, across every turn it serves.
pub(crate) struct LivePipes {
    queue: Arc<Mutex<Vec<(bool, String)>>>,
    stdout_acc: Arc<Mutex<String>>,
    stderr_acc: Arc<Mutex<String>>,
    handles: Vec<thread::JoinHandle<()>>,
}

impl LivePipes {
    pub(crate) fn attach(child: &mut Child) -> Result<Self> {
        let queue = Arc::new(Mutex::new(Vec::<(bool, String)>::new()));
        let stdout_acc = Arc::new(Mutex::new(String::new()));
        let stderr_acc = Arc::new(Mutex::new(String::new()));
        let stdout = child.stdout.take().context("missing stdout pipe")?;
        let stderr = child.stderr.take().context("missing stderr pipe")?;
        let handles = vec![
            spawn_reader(stdout, true, Arc::clone(&queue), Arc::clone(&stdout_acc)),
            spawn_reader(stderr, false, Arc::clone(&queue), Arc::clone(&stderr_acc)),
        ];
        Ok(Self {
            queue,
            stdout_acc,
            stderr_acc,
            handles,
        })
    }

    /// Lines read since the last call, in arrival order. `true` = stdout.
    pub(crate) fn drain(&self) -> Vec<(bool, String)> {
        let mut guard = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        guard.drain(..).collect()
    }

    /// Everything this turn printed (capped), then start the next turn empty.
    pub(crate) fn take_output(&self) -> (String, String) {
        let take = |acc: &Arc<Mutex<String>>| {
            acc.lock()
                .map(|mut guard| std::mem::take(&mut *guard))
                .unwrap_or_default()
        };
        (take(&self.stdout_acc), take(&self.stderr_acc))
    }

    pub(crate) fn join(&mut self, budget: Duration) {
        for handle in self.handles.drain(..) {
            join_reader(handle, budget);
        }
    }

    /// Drop what the previous turn left behind before a new turn reads.
    fn reset(&self) {
        let _ = self.drain();
        let _ = self.take_output();
    }
}

fn spawn_reader<R: std::io::Read + Send + 'static>(
    pipe: R,
    is_stdout: bool,
    queue: Arc<Mutex<Vec<(bool, String)>>>,
    acc: Arc<Mutex<String>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(pipe);
        for line in reader.lines().map_while(Result::ok) {
            push_capped(&acc, &line);
            if let Ok(mut guard) = queue.lock() {
                guard.push((is_stdout, line));
            }
        }
    })
}

pub(crate) struct WarmProcess {
    pub(crate) child: Child,
    pub(crate) stdin: ChildStdin,
    pub(crate) pipes: LivePipes,
    runtime: &'static str,
    thread_id: String,
    fingerprint: u64,
    parked_at: Instant,
}

/// Let finished Claude / Codex processes wait for the conversation's next message.
pub fn enable_warm_sessions() {
    ENABLED.store(true, Ordering::SeqCst);
    REAPER.get_or_init(|| {
        let _ = thread::Builder::new()
            .name("ad-warm-reaper".into())
            .spawn(|| loop {
                thread::sleep(REAP_EVERY);
                let expired = {
                    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
                    let (old, keep): (Vec<_>, Vec<_>) = pool
                        .drain(..)
                        .partition(|p| p.parked_at.elapsed() >= IDLE_LIMIT);
                    *pool = keep;
                    old
                };
                for process in expired {
                    retire(process.child, process.stdin, process.pipes);
                }
            });
    });
}

/// Close every parked process. Call when the app quits.
pub fn shutdown_warm_sessions() {
    ENABLED.store(false, Ordering::SeqCst);
    let all = {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *pool)
    };
    let waiters: Vec<_> = all
        .into_iter()
        .map(|p| thread::spawn(move || stop_now(p.child, p.stdin, p.pipes)))
        .collect();
    for waiter in waiters {
        let _ = waiter.join();
    }
}

pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// Same launch = same program, arguments, environment, folder, and config files.
pub(crate) fn fingerprint(cmd: &Command, files: &[PathBuf]) -> u64 {
    let mut hasher = DefaultHasher::new();
    cmd.get_program().hash(&mut hasher);
    for arg in cmd.get_args() {
        arg.hash(&mut hasher);
    }
    let mut envs: Vec<_> = cmd.get_envs().collect();
    envs.sort();
    envs.hash(&mut hasher);
    cmd.get_current_dir().hash(&mut hasher);
    for file in files {
        file.hash(&mut hasher);
        std::fs::read(file).ok().hash(&mut hasher);
    }
    hasher.finish()
}

/// The parked process for this thread, if it was launched the same way and is still running.
pub(crate) fn take(
    runtime: &'static str,
    thread_id: &str,
    fingerprint: u64,
) -> Option<WarmProcess> {
    if !enabled() {
        return None;
    }
    let found = {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        let index = pool
            .iter()
            .position(|p| p.runtime == runtime && p.thread_id == thread_id)?;
        pool.remove(index)
    };
    let mut process = found;
    let alive = matches!(process.child.try_wait(), Ok(None));
    if !alive || process.fingerprint != fingerprint {
        retire(process.child, process.stdin, process.pipes);
        return None;
    }
    process.pipes.reset();
    Some(process)
}

/// Keep this process for the thread's next message.
pub(crate) fn park(
    runtime: &'static str,
    thread_id: String,
    fingerprint: u64,
    child: Child,
    stdin: ChildStdin,
    pipes: LivePipes,
) {
    if !enabled() {
        retire(child, stdin, pipes);
        return;
    }
    let evicted = {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        let mut evicted = Vec::new();
        if let Some(index) = pool
            .iter()
            .position(|p| p.runtime == runtime && p.thread_id == thread_id)
        {
            evicted.push(pool.remove(index));
        }
        pool.push(WarmProcess {
            child,
            stdin,
            pipes,
            runtime,
            thread_id,
            fingerprint,
            parked_at: Instant::now(),
        });
        while pool.len() > MAX_WARM {
            let oldest = pool
                .iter()
                .enumerate()
                .min_by_key(|(_, p)| p.parked_at)
                .map(|(i, _)| i)
                .unwrap_or(0);
            evicted.push(pool.remove(oldest));
        }
        evicted
    };
    for process in evicted {
        retire(process.child, process.stdin, process.pipes);
    }
}

/// End of a turn: park the process for `park_as` (thread id, fingerprint) if it
/// is still running, otherwise close it.
pub(crate) fn keep_or_close(
    runtime: &'static str,
    park_as: Option<(String, u64)>,
    mut child: Child,
    pipes: LivePipes,
    control: &PromptSessionControl,
) {
    let alive = matches!(child.try_wait(), Ok(None));
    let stdin = park_as
        .as_ref()
        .filter(|_| alive)
        .and_then(|_| control.detach_stdin());
    match (park_as, stdin) {
        (Some((thread_id, fp)), Some(stdin)) => park(runtime, thread_id, fp, child, stdin, pipes),
        _ => {
            control.close();
            retire_without_stdin(child, pipes);
        }
    }
}

/// Close a process in the background: stdin first so it can leave cleanly.
pub(crate) fn retire(child: Child, stdin: ChildStdin, pipes: LivePipes) {
    let _ = thread::Builder::new()
        .name("ad-warm-retire".into())
        .spawn(move || stop_now(child, stdin, pipes));
}

/// Close a process whose stdin is already gone.
pub(crate) fn retire_without_stdin(mut child: Child, mut pipes: LivePipes) {
    let _ = thread::Builder::new()
        .name("ad-warm-retire".into())
        .spawn(move || {
            wait_or_stop(&mut child);
            pipes.join(Duration::from_millis(500));
        });
}

fn stop_now(mut child: Child, stdin: ChildStdin, mut pipes: LivePipes) {
    drop(stdin);
    wait_or_stop(&mut child);
    pipes.join(Duration::from_millis(500));
}

fn wait_or_stop(child: &mut Child) {
    let started = Instant::now();
    while started.elapsed() < EXIT_GRACE {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let pid = child.id();
    force_stop_child(child, pid);
}

#[cfg(test)]
pub(crate) fn parked_count() -> usize {
    POOL.lock().map(|pool| pool.len()).unwrap_or(0)
}

#[cfg(test)]
pub(crate) fn disable_for_test() {
    ENABLED.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_session::backend::AskBackend;
    use crate::prompt_session::util::TEST_ENV_LOCK;
    use crate::prompt_session::{
        ClaudeAskBackend, CodexAskBackend, PromptSessionCancel, PromptSessionEvent,
        PromptSessionOptions, PromptSessionReport, PromptSessionStatus,
    };

    #[cfg(unix)]
    fn write_fake_bin(dir: &std::path::Path, name: &str, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn ask(
        backend: &dyn AskBackend,
        runtime: &str,
        cwd: &std::path::Path,
        resume: Option<String>,
        cancel: PromptSessionCancel,
    ) -> (PromptSessionReport, String) {
        let mut reply = String::new();
        let report = backend
            .run(
                &PromptSessionOptions {
                    runtime: runtime.into(),
                    prompt: "hi".into(),
                    cwd: Some(cwd.to_path_buf()),
                    timeout_sec: 20,
                    dangerously_skip_permissions: true,
                    full_auto: true,
                    resume_thread_id: resume,
                    selected_mcps: Vec::new(),
                    image_paths: Vec::new(),
                },
                cancel,
                None,
                &mut |event| {
                    if let PromptSessionEvent::Delta { text, .. } = event {
                        reply.push_str(&text);
                    }
                },
            )
            .expect("session");
        (report, reply)
    }

    /// Replies `pid=<pid> turn=<n>` on each message and stays up until stdin closes.
    #[cfg(unix)]
    const FAKE_CODEX: &str = r##"#!/usr/bin/env python3
import json, os, sys
turns = 0
for line in sys.stdin:
    msg = json.loads(line)
    method = msg.get("method")
    if method == "initialize":
        print(json.dumps({"id": msg["id"], "result": {}}), flush=True)
    elif method in ("thread/start", "thread/resume"):
        print(json.dumps({"id": msg["id"], "result": {"thread": {"id": "thr_warm"}}}), flush=True)
    elif method == "turn/start":
        turns += 1
        print(json.dumps({"id": msg["id"], "result": {"turn": {"id": "t%d" % turns}}}), flush=True)
        text = "pid=%d turn=%d" % (os.getpid(), turns)
        print(json.dumps({"method": "turn/completed", "params": {"turn": {"id": "t%d" % turns, "items": [{"type": "agentMessage", "text": text}]}}}), flush=True)
"##;

    #[cfg(unix)]
    const FAKE_CLAUDE: &str = r##"#!/usr/bin/env python3
import json, os, sys
turns = 0
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("type") != "user":
        continue
    turns += 1
    text = "pid=%d turn=%d" % (os.getpid(), turns)
    print(json.dumps({"type": "stream_event", "session_id": "sid_warm", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": text}}}), flush=True)
    print(json.dumps({"type": "result", "session_id": "sid_warm", "is_error": False, "result": text}), flush=True)
"##;

    fn pid_of(reply: &str) -> String {
        reply
            .split_whitespace()
            .find_map(|part| part.strip_prefix("pid="))
            .unwrap_or_default()
            .to_string()
    }

    #[cfg(unix)]
    #[test]
    fn codex_next_message_reuses_the_running_server() {
        let _guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let bin = write_fake_bin(dir.path(), "fake-codex", FAKE_CODEX);
        std::env::set_var("AGENT_DOCTOR_CODEX_BIN", &bin);
        enable_warm_sessions();

        let (first, first_reply) = ask(
            &CodexAskBackend,
            "codex",
            dir.path(),
            None,
            PromptSessionCancel::new(),
        );
        assert_eq!(
            first.status,
            PromptSessionStatus::Succeeded,
            "{}",
            first.summary
        );
        assert_eq!(first.runtime_thread_id.as_deref(), Some("thr_warm"));
        assert!(first_reply.contains("turn=1"), "{first_reply}");

        let (second, second_reply) = ask(
            &CodexAskBackend,
            "codex",
            dir.path(),
            first.runtime_thread_id.clone(),
            PromptSessionCancel::new(),
        );
        assert_eq!(
            second.status,
            PromptSessionStatus::Succeeded,
            "{}",
            second.summary
        );
        assert!(second_reply.contains("turn=2"), "{second_reply}");
        assert_eq!(pid_of(&first_reply), pid_of(&second_reply));

        // A stopped turn is not kept: the next message starts a new server.
        let cancel = PromptSessionCancel::new();
        cancel.request();
        let (stopped, _) = ask(
            &CodexAskBackend,
            "codex",
            dir.path(),
            second.runtime_thread_id.clone(),
            cancel,
        );
        assert_eq!(stopped.status, PromptSessionStatus::Cancelled);
        let (fresh, fresh_reply) = ask(
            &CodexAskBackend,
            "codex",
            dir.path(),
            second.runtime_thread_id.clone(),
            PromptSessionCancel::new(),
        );
        assert_eq!(
            fresh.status,
            PromptSessionStatus::Succeeded,
            "{}",
            fresh.summary
        );
        assert!(fresh_reply.contains("turn=1"), "{fresh_reply}");
        assert_ne!(pid_of(&first_reply), pid_of(&fresh_reply));

        shutdown_warm_sessions();
        disable_for_test();
        assert_eq!(parked_count(), 0);
        std::env::remove_var("AGENT_DOCTOR_CODEX_BIN");
    }

    #[cfg(unix)]
    #[test]
    fn claude_next_message_reuses_the_process_until_config_changes() {
        let _guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let bin = write_fake_bin(dir.path(), "fake-claude", FAKE_CLAUDE);
        std::env::set_var("AGENT_DOCTOR_CLAUDE_BIN", &bin);
        enable_warm_sessions();

        let (first, first_reply) = ask(
            &ClaudeAskBackend,
            "claude-code",
            dir.path(),
            None,
            PromptSessionCancel::new(),
        );
        assert_eq!(
            first.status,
            PromptSessionStatus::Succeeded,
            "{}",
            first.summary
        );
        assert_eq!(first.runtime_thread_id.as_deref(), Some("sid_warm"));

        let (second, second_reply) = ask(
            &ClaudeAskBackend,
            "claude-code",
            dir.path(),
            first.runtime_thread_id.clone(),
            PromptSessionCancel::new(),
        );
        assert_eq!(
            second.status,
            PromptSessionStatus::Succeeded,
            "{}",
            second.summary
        );
        assert!(second_reply.contains("turn=2"), "{second_reply}");
        assert_eq!(pid_of(&first_reply), pid_of(&second_reply));

        // New project tools mean a new process, like reopening the window.
        std::fs::write(dir.path().join(".mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
        let (third, third_reply) = ask(
            &ClaudeAskBackend,
            "claude-code",
            dir.path(),
            second.runtime_thread_id.clone(),
            PromptSessionCancel::new(),
        );
        assert_eq!(
            third.status,
            PromptSessionStatus::Succeeded,
            "{}",
            third.summary
        );
        assert!(third_reply.contains("turn=1"), "{third_reply}");
        assert_ne!(pid_of(&first_reply), pid_of(&third_reply));

        shutdown_warm_sessions();
        disable_for_test();
        std::env::remove_var("AGENT_DOCTOR_CLAUDE_BIN");
    }

    #[test]
    fn fingerprint_changes_with_launch_and_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        std::fs::write(&config, "model = \"a\"").unwrap();
        let mut cmd = Command::new("codex");
        cmd.arg("app-server").env("OPENAI_BASE_URL", "https://a");
        let base = fingerprint(&cmd, std::slice::from_ref(&config));
        assert_eq!(base, fingerprint(&cmd, std::slice::from_ref(&config)));

        std::fs::write(&config, "model = \"b\"").unwrap();
        assert_ne!(base, fingerprint(&cmd, std::slice::from_ref(&config)));

        std::fs::write(&config, "model = \"a\"").unwrap();
        cmd.env("OPENAI_BASE_URL", "https://b");
        assert_ne!(base, fingerprint(&cmd, std::slice::from_ref(&config)));
    }
}
