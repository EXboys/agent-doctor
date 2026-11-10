//! Generic ACP ask backend: wraps any agent that speaks Agent Client
//! Protocol on stdin/stdout, launched by a user-configured command.
//!
//! Launch sources, both read at spawn time:
//! - `AGENT_DOCTOR_ACP_BIN` — the agent executable (required)
//! - `AGENT_DOCTOR_ACP_ARGS` — optional extra arguments, shell-split
//!   (quotes honored), inserted before the binary runs
//!
//! Keep the launch surface intentionally tiny: everything the protocol needs
//! lives in [`super::acp`], and session resume rides the agent's own
//! `sessionId` exactly like every other runtime.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use super::acp::{self, AcpProfile};
use super::backend::AskBackend;
use super::control::PromptSessionControl;
use super::util::{command_from_cli, summarize, turn_error};
use super::vision::{encode_image, turn_images, EncodedImage};
use super::{
    next_session_id, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, PromptSessionStatus, MAX_TIMEOUT_SEC, MIN_TIMEOUT_SEC,
};
use agent_doctor_core::session_env::{apply_overlay_env, format_command_display};
use agent_doctor_core::session_launch::resolve_session_cwd;

const RUNTIME_ID: &str = "acp";

pub struct GenericAcpAskBackend;

impl AskBackend for GenericAcpAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_generic_acp(options, cancel, control, on_event)
    }
}

/// Split a shell-ish argument string honoring single and double quotes.
fn split_args(raw: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in raw.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

fn run_generic_acp(
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    control: Option<PromptSessionControl>,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<PromptSessionReport> {
    let session_id = next_session_id();
    let runtime = RUNTIME_ID.to_string();
    let prompt = options.prompt.trim();
    if prompt.is_empty() {
        bail!("prompt must not be empty");
    }
    let binary = match std::env::var("AGENT_DOCTOR_ACP_BIN") {
        Ok(bin) if !bin.trim().is_empty() => bin.trim().to_string(),
        _ => bail!(
            "ACP 助手还没有配置启动命令。设置环境变量 AGENT_DOCTOR_ACP_BIN（可连同 AGENT_DOCTOR_ACP_ARGS）后再发一次。"
        ),
    };
    let args = split_args(&std::env::var("AGENT_DOCTOR_ACP_ARGS").unwrap_or_default());
    let cwd = resolve_session_cwd(options.cwd.as_deref(), options.workspace_name.as_deref());
    if !cwd.exists() {
        bail!("session cwd does not exist: {}", cwd.display());
    }
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
        .filter_map(|p| encode_image(p))
        .collect();

    let mut cmd = build_command(&cwd, &overlay, &binary, &args);
    let command_display = format_command_display(&cmd);

    on_event(PromptSessionEvent::Started {
        session_id: session_id.clone(),
        runtime: runtime.clone(),
        cwd: cwd.display().to_string(),
        command: command_display,
        client_run_id: String::new(),
    });

    let started = std::time::Instant::now();
    let writer = control.unwrap_or_default();
    let mut child = cmd.spawn().context("failed to spawn the ACP agent")?;
    let mut pipes = match super::warm::LivePipes::attach(&mut child) {
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
        acp::pump_acp(
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
            AcpProfile::generic(),
            &mut emit,
        )
    };
    let acp_session = result
        .as_ref()
        .ok()
        .and_then(|turn| turn.acp_session.clone());

    let _ = child.kill();
    let _ = child.wait();

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
            PromptSessionReport {
                usage: None,
                session_id,
                runtime,
                cwd: cwd.display().to_string(),
                status: PromptSessionStatus::Failed,
                exit_code: None,
                summary,
                log_excerpt: error.to_string(),
                duration_ms,
                runtime_thread_id: acp_session,
            }
        }
    };
    Ok(report)
}

fn build_command(
    cwd: &Path,
    overlay: &HashMap<String, String>,
    binary: &str,
    args: &[String],
) -> Command {
    let mut command = command_from_cli(binary);
    for arg in args {
        command.arg(arg);
    }
    command
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_overlay_env(&mut command, overlay);
    command
}

#[cfg(test)]
mod tests {
    use super::split_args;

    #[test]
    fn quoted_args_split_without_losing_content() {
        assert_eq!(
            split_args(r#"--model "gpt 5" --profile 'acp v2' --flag"#),
            vec!["--model", "gpt 5", "--profile", "acp v2", "--flag"]
        );
        assert!(split_args("   ").is_empty());
    }
}
