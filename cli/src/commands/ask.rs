use agent_doctor_core::{
    run_prompt_session_with_cancel, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionStatus,
};
use anyhow::Result;
use std::io::{self, Write};
use std::path::PathBuf;

pub fn run(
    runtime: &str,
    prompt: &str,
    cwd: Option<&str>,
    timeout_sec: u64,
    skip_permissions: bool,
    full_auto: bool,
    json: bool,
) -> Result<()> {
    let cancel = PromptSessionCancel::new();
    {
        let cancel = cancel.clone();
        let _ = ctrlc::set_handler(move || {
            cancel.request();
        });
    }

    let options = PromptSessionOptions {
        runtime: runtime.to_string(),
        prompt: prompt.to_string(),
        cwd: cwd.map(PathBuf::from),
        timeout_sec,
        dangerously_skip_permissions: skip_permissions,
        full_auto,
        resume_thread_id: None,
        selected_mcps: Vec::new(),
    };

    let report = run_prompt_session_with_cancel(&options, cancel, None, |event| {
        if json {
            return;
        }
        match event {
            PromptSessionEvent::Started {
                runtime,
                cwd,
                command,
                ..
            } => {
                eprintln!("Agent Doctor — ask ({runtime})");
                eprintln!("Cwd: {cwd}");
                eprintln!("Cmd: {command}");
                eprintln!();
            }
            PromptSessionEvent::StdoutLine { line, .. } => {
                println!("{line}");
            }
            PromptSessionEvent::Delta { text, .. } => {
                print!("{text}");
            }
            PromptSessionEvent::StderrLine { line, .. } => {
                eprintln!("{line}");
            }
            PromptSessionEvent::Status { message, .. } => {
                eprintln!("… {message}");
            }
            PromptSessionEvent::PermissionRequest {
                tool_name,
                detail,
                ..
            } => {
                eprintln!("[permission] {tool_name}: {detail}");
                eprintln!("(inline allow/deny is available in the desktop chat window)");
            }
            PromptSessionEvent::PermissionResolved { .. }
            | PromptSessionEvent::Completed { .. } => {}
        }
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
    })?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        eprintln!();
        eprintln!(
            "— {} · exit {:?} · {}ms",
            status_label(&report.status),
            report.exit_code,
            report.duration_ms
        );
    }

    if matches!(
        report.status,
        PromptSessionStatus::Failed | PromptSessionStatus::TimedOut
    ) {
        std::process::exit(report.exit_code.unwrap_or(1));
    }
    if report.status == PromptSessionStatus::Cancelled {
        std::process::exit(130);
    }
    Ok(())
}

fn status_label(status: &PromptSessionStatus) -> &'static str {
    match status {
        PromptSessionStatus::Succeeded => "succeeded",
        PromptSessionStatus::Failed => "failed",
        PromptSessionStatus::Cancelled => "cancelled",
        PromptSessionStatus::TimedOut => "timed out",
    }
}
