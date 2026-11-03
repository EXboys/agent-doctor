//! Built-in "doctor" ask backend: the official Agent Doctor agent loop,
//! running in-process (no external CLI). Backs the desktop chat when no other
//! agent runtime is installed — and stays available next to them for
//! diagnosing and repairing the ones that broke.

use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use agent_doctor_core::session_launch::resolve_session_cwd;
use agent_doctor_repair::official_agent::{
    run_official_agent_turn, OfficialAgentEvent, OfficialAgentOptions, PermissionGate,
};
use agent_doctor_repair::official_llm_config;
use agent_doctor_repair::OFFICIAL_AGENT_RUNTIME_ID;

use super::plan::{PlanStep, PlanStepState};
use super::tool_step::ToolStep;
use super::{
    next_session_id, AskBackend, PromptSessionCancel, PromptSessionControl, PromptSessionEvent,
    PromptSessionOptions, PromptSessionReport, PromptSessionStatus,
};

/// One approval may stay unanswered this long before the loop treats it as a
/// decline (the chat can also auto-approve or forward it to the phone).
const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(600);

/// Error-code prefix the desktop maps to provider-setup guidance.
pub const DOCTOR_NO_PROVIDER: &str = "doctor_no_provider";
pub const DOCTOR_CANCELLED: &str = "doctor_cancelled";

pub struct DoctorAskBackend;

impl AskBackend for DoctorAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_doctor(options, cancel, control, on_event)
    }
}

/// Blocks the official loop on the chat permission card until the user (or the
/// auto-approve default) answers through `PromptSessionControl`. A turn cancel
/// wakes the wait within a poll tick so the run slot frees up immediately.
struct ControlGate {
    control: PromptSessionControl,
    cancel: PromptSessionCancel,
}

/// How often a pending confirmation re-checks the cancel flag.
const CONFIRM_POLL: Duration = Duration::from_millis(250);

impl PermissionGate for ControlGate {
    fn confirm(&mut self, request_id: &str, _tool: &str, _detail: &str) -> bool {
        let receiver = self.control.register_doctor_confirmation(request_id);
        let deadline = Instant::now() + CONFIRMATION_TIMEOUT;
        loop {
            if self.cancel.is_requested() {
                return false;
            }
            if Instant::now() >= deadline {
                return false;
            }
            match receiver.recv_timeout(CONFIRM_POLL) {
                Ok(allow) => return allow,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return false,
            }
        }
    }
}

/// `buildPromptWithHistory` appends "Reply in Simplified Chinese" to the style
/// block only for Chinese turns; use it as the locale signal.
fn locale_is_zh(prompt: &str) -> bool {
    prompt.contains("Reply in Simplified Chinese")
}

fn run_doctor(
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    control: Option<PromptSessionControl>,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<PromptSessionReport> {
    let session_id = next_session_id();
    let runtime = OFFICIAL_AGENT_RUNTIME_ID.to_string();
    let prompt = options.prompt.trim().to_string();
    if prompt.is_empty() {
        bail!("prompt must not be empty");
    }
    let cwd = resolve_session_cwd(options.cwd.as_deref(), options.workspace_name.as_deref());
    if !cwd.exists() {
        bail!("session cwd does not exist: {}", cwd.display());
    }

    on_event(PromptSessionEvent::Started {
        session_id: session_id.clone(),
        runtime: runtime.clone(),
        cwd: cwd.display().to_string(),
        command: "agent-doctor built-in".to_string(),
        client_run_id: String::new(),
    });

    let started = Instant::now();
    let Some(control) = control else {
        return Ok(fail_run(
            &session_id,
            &runtime,
            &cwd,
            started,
            "doctor agent has no permission control",
            on_event,
        ));
    };

    let locale = if locale_is_zh(&prompt) {
        None
    } else {
        Some("en".to_string())
    };
    let agent_options = OfficialAgentOptions::for_chat(prompt, locale, Some(cwd.clone()));

    let mut step_seq = 0usize;
    let mut _streamed_text = false;
    let mut gate = ControlGate {
        control,
        cancel: cancel.clone(),
    };
    let config = match official_llm_config() {
        Ok(config) => config,
        Err(error) => {
            return Ok(fail_run(
                &session_id,
                &runtime,
                &cwd,
                started,
                &format!("{DOCTOR_NO_PROVIDER}: {error:#}"),
                on_event,
            ));
        }
    };
    let result = run_official_agent_turn(
        &agent_options,
        &config,
        &cancel.handle(),
        |event| match event {
            OfficialAgentEvent::Thinking { round } => on_event(PromptSessionEvent::Status {
                session_id: session_id.clone(),
                phase: "thinking".to_string(),
                message: format!("第 {round} 轮…"),
            }),
            OfficialAgentEvent::Tool { tool, target } => {
                step_seq += 1;
                on_event(PromptSessionEvent::Tool {
                    session_id: session_id.clone(),
                    step: Box::new(doctor_step(
                        &session_id,
                        step_seq,
                        &tool,
                        target.as_deref(),
                        "running",
                    )),
                });
                on_event(PromptSessionEvent::Status {
                    session_id: session_id.clone(),
                    phase: "tool".to_string(),
                    message: tool.clone(),
                });
            }
            OfficialAgentEvent::Reasoning { text } => {
                on_event(PromptSessionEvent::Thinking {
                    session_id: session_id.clone(),
                    text,
                });
            }
            OfficialAgentEvent::Delta { text } => {
                _streamed_text = true;
                on_event(PromptSessionEvent::Delta {
                    session_id: session_id.clone(),
                    text,
                });
            }
            OfficialAgentEvent::ToolResult { tool, success } => {
                step_seq += 1;
                let status = if success { "done" } else { "failed" };
                on_event(PromptSessionEvent::Tool {
                    session_id: session_id.clone(),
                    step: Box::new(doctor_step(&session_id, step_seq, &tool, None, status)),
                });
            }
            OfficialAgentEvent::Plan { steps } => {
                on_event(PromptSessionEvent::Plan {
                    session_id: session_id.clone(),
                    items: steps
                        .into_iter()
                        .map(|step| PlanStep {
                            text: step.title,
                            state: if step.completed {
                                PlanStepState::Done
                            } else {
                                PlanStepState::Pending
                            },
                        })
                        .collect(),
                });
            }
            OfficialAgentEvent::Confirmation {
                request_id,
                tool,
                detail,
            } => {
                on_event(PromptSessionEvent::PermissionRequest {
                    session_id: session_id.clone(),
                    request_id,
                    tool_name: tool,
                    detail,
                    input_json: String::new(),
                    input_mode: "choice".to_string(),
                });
            }
        },
        &mut gate,
    );

    let duration_ms = started.elapsed().as_millis() as u64;
    match result {
        Ok(report) => {
            let answer = report.answer;
            if !answer.is_empty() && !report.streamed {
                _streamed_text = true;
                on_event(PromptSessionEvent::Delta {
                    session_id: session_id.clone(),
                    text: answer.clone(),
                });
            }
            let summary = summarize_answer(&answer);
            on_event(PromptSessionEvent::Completed {
                session_id: session_id.clone(),
                status: PromptSessionStatus::Succeeded,
                exit_code: None,
                summary: summary.clone(),
                timeout: None,
            });
            Ok(PromptSessionReport {
                usage: None,
                session_id,
                runtime,
                cwd: cwd.display().to_string(),
                status: PromptSessionStatus::Succeeded,
                exit_code: None,
                summary,
                log_excerpt: String::new(),
                duration_ms,
                runtime_thread_id: None,
            })
        }
        Err(error) => {
            let message = format!("{error:#}");
            let friendly = if message.starts_with("cancelled") {
                format!("{DOCTOR_CANCELLED}: stopped by user")
            } else {
                message.clone()
            };
            fail_run(&session_id, &runtime, &cwd, started, &friendly, on_event);
            Ok(PromptSessionReport {
                usage: None,
                session_id,
                runtime,
                cwd: cwd.display().to_string(),
                status: if message.starts_with("cancelled") {
                    PromptSessionStatus::Cancelled
                } else {
                    PromptSessionStatus::Failed
                },
                exit_code: None,
                summary: friendly,
                log_excerpt: message,
                duration_ms,
                runtime_thread_id: None,
            })
        }
    }
}

fn doctor_step(
    session_id: &str,
    seq: usize,
    tool: &str,
    target: Option<&str>,
    status: &str,
) -> ToolStep {
    let (kind, command) = if tool == "bash" {
        ("terminal", target.unwrap_or_default().to_string())
    } else {
        ("other", String::new())
    };
    ToolStep {
        id: format!("{session_id}-doctor-{seq}"),
        kind: kind.to_string(),
        name: tool.to_string(),
        status: status.to_string(),
        title: String::new(),
        path: if tool == "bash" {
            String::new()
        } else {
            target.unwrap_or_default().to_string()
        },
        command,
        query: String::new(),
        line_start: None,
        line_end: None,
        additions: 0,
        deletions: 0,
        added_lines: Vec::new(),
        deleted_lines: Vec::new(),
        output: String::new(),
        exit_code: None,
    }
}

/// Emit the failure pair (turn error + completed) other backends send, so the
/// chat shows plain guidance instead of a hanging spinner.
fn fail_run(
    session_id: &str,
    _runtime: &str,
    _cwd: &std::path::Path,
    started: Instant,
    message: &str,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> PromptSessionReport {
    on_event(PromptSessionEvent::TurnError {
        session_id: session_id.to_string(),
        message: message.to_string(),
        plain: true,
    });
    on_event(PromptSessionEvent::Completed {
        session_id: session_id.to_string(),
        status: PromptSessionStatus::Failed,
        exit_code: None,
        summary: message.to_string(),
        timeout: None,
    });
    PromptSessionReport {
        usage: None,
        session_id: session_id.to_string(),
        runtime: OFFICIAL_AGENT_RUNTIME_ID.to_string(),
        cwd: String::new(),
        status: PromptSessionStatus::Failed,
        exit_code: None,
        summary: message.to_string(),
        log_excerpt: String::new(),
        duration_ms: started.elapsed().as_millis() as u64,
        runtime_thread_id: None,
    }
}

fn summarize_answer(answer: &str) -> String {
    let mut summary: String = answer.chars().take(400).collect();
    if summary.chars().count() < answer.chars().count() {
        summary.push('…');
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_signal_comes_from_the_chat_style_block() {
        assert!(locale_is_zh(
            "Response style: … Reply in Simplified Chinese."
        ));
        assert!(!locale_is_zh("Response style: answer the user directly."));
    }

    #[test]
    fn summarize_keeps_answer_short() {
        let long = "x".repeat(1_000);
        let summary = summarize_answer(&long);
        assert!(summary.chars().count() <= 401);
        assert!(summary.ends_with('…'));
        assert_eq!(summarize_answer("short"), "short");
    }
}
