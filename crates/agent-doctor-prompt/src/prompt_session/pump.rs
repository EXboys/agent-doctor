//! The shared one-shot subprocess pump.
//!
//! Hermes, OpenClaw and ZCode asks run the same loop: reader threads push
//! `(is_stdout, line)` into a shared queue, the loop drains that queue into
//! per-runtime handling, and the turn ends on cancel, the session clock, both
//! pipes closing, or the child exiting. What differs per runtime — how one
//! stdout line becomes events, which stderr lines are already explained —
//! lives behind [`OneShotHandler`], so a fix to the queue or the EOF grace
//! lands here once, not three times. (Claude and Codex run their own loops in
//! [`super::claude`] and [`super::codex_app_server`], over the shared
//! [`super::warm::LivePipes`], whose pipes live across turns.)

use std::cell::Cell;
use std::io::Read;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use super::util::{
    event_is_model_reply, finish_oneshot_after_pipes_closed, force_stop_child,
    is_runtime_stderr_noise, join_reader, push_capped, SessionClock,
};
use super::{PromptSessionEvent, PromptSessionStatus, PumpResult};

/// Where one pipe's lines go: the shared arrival queue, the capped capture
/// for the final parse, and the flag that flips once the pipe is exhausted.
struct LineSink {
    is_stdout: bool,
    queue: Arc<Mutex<Vec<(bool, String)>>>,
    capture: Arc<Mutex<String>>,
    eof: Arc<AtomicBool>,
}

impl LineSink {
    fn new(
        is_stdout: bool,
        queue: Arc<Mutex<Vec<(bool, String)>>>,
        capture: Arc<Mutex<String>>,
        eof: Arc<AtomicBool>,
    ) -> Self {
        Self {
            is_stdout,
            queue,
            capture,
            eof,
        }
    }
}

/// Read one pipe line by line: every line joins the shared queue in arrival
/// order and the capped capture; `eof` flips when the pipe closes. Runs on
/// its own thread — join it with [`join_reader`].
fn pump_lines<R>(reader: R, sink: LineSink) -> thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(reader);
        for line in reader.lines().map_while(Result::ok) {
            push_capped(&sink.capture, &line);
            if let Ok(mut guard) = sink.queue.lock() {
                guard.push((sink.is_stdout, line));
            }
        }
        sink.eof.store(true, Ordering::SeqCst);
    })
}

/// The per-runtime half of a one-shot ask: how drained lines become events,
/// and when quiet time means the turn is over.
pub(crate) trait OneShotHandler {
    /// One stdout line. Emit live deltas, status and tool chips here.
    fn on_stdout(
        &mut self,
        session_id: &str,
        line: &str,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    );

    /// A stderr line this runtime already explained, on top of the shared
    /// [`is_runtime_stderr_noise`]. May be stateful (OpenClaw's tool block).
    fn hides_stderr(&mut self, line: &str) -> bool {
        let _ = line;
        false
    }

    /// A stderr line that passed the filters. The default shows it as-is;
    /// ZCode promotes plain runtime failures to turn errors.
    fn on_stderr(
        &mut self,
        session_id: &str,
        line: &str,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) {
        on_event(PromptSessionEvent::StderrLine {
            session_id: session_id.to_string(),
            line: line.to_string(),
        });
    }

    /// Work between drains — OpenClaw polls its session trace here. `true`
    /// counts as activity and keeps the clock alive.
    fn poll(&mut self, session_id: &str, on_event: &mut dyn FnMut(PromptSessionEvent)) -> bool {
        let _ = (session_id, on_event);
        false
    }

    /// A tool step is open, so quiet time is not a stall.
    fn tool_running(&self) -> bool {
        false
    }

    /// Also end the turn when it goes quiet with no step open, before the
    /// hard limit. Runtimes that report open steps turn this on.
    fn stall_guard(&self) -> bool {
        false
    }

    /// The open or last tool for the timeout note.
    fn timeout_label(&self) -> String {
        String::new()
    }

    /// After the readers joined and the queue drained: last chance to emit.
    fn finish(&mut self, session_id: &str, on_event: &mut dyn FnMut(PromptSessionEvent)) {
        let _ = (session_id, on_event);
    }
}

/// Drive one one-shot child: reader threads, the drain/wait loop, and the
/// final collection. Returns status, exit code, capped stdout and stderr, and
/// the timeout note when the pump stopped the turn.
pub(crate) fn pump_oneshot<F, H>(
    session_id: &str,
    child: &mut Child,
    timeout_sec: u64,
    cancel: Arc<AtomicBool>,
    waiting_context: &'static str,
    handler: &mut H,
    on_event: &mut F,
) -> Result<PumpResult>
where
    F: FnMut(PromptSessionEvent),
    H: OneShotHandler,
{
    let pid = child.id();
    let queue = Arc::new(Mutex::new(Vec::<(bool, String)>::new()));
    let stdout_acc = Arc::new(Mutex::new(String::new()));
    let stderr_acc = Arc::new(Mutex::new(String::new()));
    let stdout_eof = Arc::new(AtomicBool::new(false));
    let stderr_eof = Arc::new(AtomicBool::new(false));

    let stdout = child.stdout.take().context("missing stdout pipe")?;
    let stderr = child.stderr.take().context("missing stderr pipe")?;
    let stdout_handle = pump_lines(
        stdout,
        LineSink::new(
            true,
            Arc::clone(&queue),
            Arc::clone(&stdout_acc),
            Arc::clone(&stdout_eof),
        ),
    );
    let stderr_handle = pump_lines(
        stderr,
        LineSink::new(
            false,
            Arc::clone(&queue),
            Arc::clone(&stderr_acc),
            Arc::clone(&stderr_eof),
        ),
    );

    let heard_reply = Cell::new(false);
    let drain = |handler: &mut H, on_event: &mut F| -> bool {
        let drained = {
            let mut guard = queue.lock().unwrap_or_else(|e| e.into_inner());
            guard.drain(..).collect::<Vec<_>>()
        };
        let saw = !drained.is_empty();
        for (is_stdout, line) in drained {
            if is_stdout {
                handler.on_stdout(session_id, &line, &mut |event| {
                    if event_is_model_reply(&event) {
                        heard_reply.set(true);
                    }
                    on_event(event);
                });
                continue;
            }
            if handler.hides_stderr(&line) || is_runtime_stderr_noise(&line) {
                continue;
            }
            handler.on_stderr(session_id, &line, on_event);
        }
        saw
    };

    let mut clock = SessionClock::new(timeout_sec);
    let mut pipes_closed_at: Option<Instant> = None;
    let mut timeout_note = None;
    let (status, exit_code) = loop {
        let saw_output = drain(handler, on_event);
        // Poll even after output: OpenClaw's trace must keep ticking.
        let saw_poll = handler.poll(session_id, &mut |event| {
            if event_is_model_reply(&event) {
                heard_reply.set(true);
            }
            on_event(event);
        });
        if saw_output || saw_poll {
            clock.touch();
        }
        clock.set_tool_running(handler.tool_running());
        if cancel.load(Ordering::SeqCst) {
            force_stop_child(child, pid);
            break (PromptSessionStatus::Cancelled, None);
        }
        if clock.expired()
            || (handler.stall_guard() && clock.stalled())
            || clock.awaiting_first_reply(heard_reply.get())
        {
            timeout_note = Some(clock.timeout_note(&handler.timeout_label()));
            force_stop_child(child, pid);
            break (PromptSessionStatus::TimedOut, None);
        }
        if let Some(done) = finish_oneshot_after_pipes_closed(
            child,
            pid,
            &stdout_eof,
            &stderr_eof,
            &mut pipes_closed_at,
        ) {
            break done;
        }
        match child.try_wait() {
            Ok(Some(wait_status)) => {
                let code = wait_status.code();
                let status = if wait_status.success() {
                    PromptSessionStatus::Succeeded
                } else {
                    PromptSessionStatus::Failed
                };
                break (status, code);
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(error) => return Err(error).context(waiting_context),
        }
    };

    join_reader(stdout_handle, Duration::from_millis(500));
    join_reader(stderr_handle, Duration::from_millis(500));
    drain(handler, on_event);
    handler.finish(session_id, on_event);

    let stdout = stdout_acc.lock().map(|g| g.clone()).unwrap_or_default();
    let stderr = stderr_acc.lock().map(|g| g.clone()).unwrap_or_default();
    Ok((status, exit_code, stdout, stderr, timeout_note))
}
