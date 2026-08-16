use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use super::backend::AskBackend;
use super::control::PromptSessionControl;
use super::env::{
    apply_deepseek_harness_env, apply_overlay_env, collect_overlay_env, format_command_display,
};
use super::util::{force_stop_child, join_reader, push_capped, summarize};
use super::{
    next_session_id, PromptSessionCancel, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, PromptSessionStatus, MAX_TIMEOUT_SEC, MIN_TIMEOUT_SEC,
};
use crate::adapters::{DEEPSEEK_HARNESS_CLI, DEEPSEEK_HARNESS_RUNTIME_ID};
use crate::session_launch::resolve_session_cwd;

pub struct DeepSeekHarnessAskBackend;

impl AskBackend for DeepSeekHarnessAskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        _control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(PromptSessionEvent),
    ) -> Result<PromptSessionReport> {
        run_deepseek_harness(options, cancel, on_event)
    }
}

fn run_deepseek_harness(
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    on_event: &mut dyn FnMut(PromptSessionEvent),
) -> Result<PromptSessionReport> {
    let session_id = next_session_id();
    let runtime = DEEPSEEK_HARNESS_RUNTIME_ID.to_string();
    let prompt = options.prompt.trim();
    if prompt.is_empty() {
        bail!("prompt must not be empty");
    }
    let cwd = resolve_session_cwd(options.cwd.as_deref());
    if !cwd.exists() {
        bail!("session cwd does not exist: {}", cwd.display());
    }
    if options
        .resume_thread_id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
    {
        on_event(PromptSessionEvent::Status {
            session_id: session_id.clone(),
            phase: "session".into(),
            message: "dsh headless does not expose resume; started a new session.".into(),
        });
    }

    let mut command = build_command(prompt, &cwd);
    on_event(PromptSessionEvent::Started {
        session_id: session_id.clone(),
        runtime: runtime.clone(),
        cwd: cwd.display().to_string(),
        command: format_command_display(&command),
    });

    let started = Instant::now();
    let mut child = command.spawn().context("failed to spawn dsh")?;
    let timeout = options.timeout_sec.clamp(MIN_TIMEOUT_SEC, MAX_TIMEOUT_SEC);
    let (status, exit_code, stdout, stderr) =
        collect_final_output(&mut child, timeout, cancel.handle())?;
    let duration_ms = started.elapsed().as_millis() as u64;
    let final_stdout = stdout.trim().to_string();
    if !final_stdout.is_empty() {
        on_event(PromptSessionEvent::Delta {
            session_id: session_id.clone(),
            text: final_stdout.clone(),
        });
    } else if status != PromptSessionStatus::Succeeded && !stderr.trim().is_empty() {
        on_event(PromptSessionEvent::Delta {
            session_id: session_id.clone(),
            text: stderr.trim().to_string(),
        });
    }
    let summary_input = if final_stdout.is_empty() {
        stderr.trim().to_string()
    } else {
        final_stdout.clone()
    };
    let summary = summarize(&summary_input, &status, &runtime);
    on_event(PromptSessionEvent::Completed {
        session_id: session_id.clone(),
        status: status.clone(),
        exit_code,
        summary: summary.clone(),
    });
    Ok(PromptSessionReport {
        session_id,
        runtime,
        cwd: cwd.display().to_string(),
        status,
        exit_code,
        summary,
        log_excerpt: summary_input,
        duration_ms,
        // The official one-shot command has no documented resume identifier.
        runtime_thread_id: None,
    })
}

fn build_command(prompt: &str, cwd: &Path) -> Command {
    let binary =
        std::env::var("AGENT_DOCTOR_DSH_BIN").unwrap_or_else(|_| DEEPSEEK_HARNESS_CLI.into());
    let overlay = collect_overlay_env();
    let mut command = Command::new(binary);
    command
        .arg("--profile")
        .arg("headless")
        .arg(prompt)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_overlay_env(&mut command, &overlay);
    apply_deepseek_harness_env(&mut command, &overlay);
    command
}

fn collect_final_output(
    child: &mut Child,
    timeout_sec: u64,
    cancel: Arc<AtomicBool>,
) -> Result<(PromptSessionStatus, Option<i32>, String, String)> {
    let pid = child.id();
    let stdout_acc = Arc::new(Mutex::new(String::new()));
    let stderr_acc = Arc::new(Mutex::new(String::new()));
    let stdout = child.stdout.take().context("missing stdout pipe")?;
    let stderr = child.stderr.take().context("missing stderr pipe")?;
    let stdout_target = Arc::clone(&stdout_acc);
    let stderr_target = Arc::clone(&stderr_acc);
    let stdout_reader = thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            push_capped(&stdout_target, &line);
        }
    });
    let stderr_reader = thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            push_capped(&stderr_target, &line);
        }
    });

    let deadline = Instant::now() + Duration::from_secs(timeout_sec);
    let (status, exit_code) = loop {
        if cancel.load(Ordering::SeqCst) {
            force_stop_child(child, pid);
            break (PromptSessionStatus::Cancelled, None);
        }
        if Instant::now() >= deadline {
            force_stop_child(child, pid);
            break (PromptSessionStatus::TimedOut, None);
        }
        match child.try_wait() {
            Ok(Some(wait_status)) => {
                break (
                    if wait_status.success() {
                        PromptSessionStatus::Succeeded
                    } else {
                        PromptSessionStatus::Failed
                    },
                    wait_status.code(),
                );
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(error) => return Err(error).context("failed waiting for dsh headless"),
        }
    };
    join_reader(stdout_reader, Duration::from_millis(500));
    join_reader(stderr_reader, Duration::from_millis(500));
    let stdout = stdout_acc
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    let stderr = stderr_acc
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    Ok((status, exit_code, stdout, stderr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_session::util::TEST_ENV_LOCK;
    use std::fs;
    use tempfile::tempdir;

    #[cfg(unix)]
    #[test]
    fn uses_official_headless_shape_and_does_not_fake_resume() {
        use std::os::unix::fs::PermissionsExt;
        let _guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = tempdir().unwrap();
        let args_path = dir.path().join("args.txt");
        let env_path = dir.path().join("env.txt");
        let binary = dir.path().join("fake-dsh");
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf '%s|%s\\n' \"${{DEEPSEEK_API_KEY:+set}}\" \"$DEEPSEEK_BASE_URL\" > '{}'\necho final-answer\n",
                args_path.display(),
                env_path.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("AGENT_DOCTOR_DSH_BIN", &binary);
        std::env::set_var("OPENAI_API_KEY", "test-secret");
        std::env::set_var("OPENAI_BASE_URL", "https://api.deepseek.com/v1");
        let report = DeepSeekHarnessAskBackend
            .run(
                &PromptSessionOptions {
                    runtime: DEEPSEEK_HARNESS_RUNTIME_ID.into(),
                    prompt: "hello".into(),
                    cwd: Some(dir.path().to_path_buf()),
                    timeout_sec: 30,
                    dangerously_skip_permissions: false,
                    full_auto: false,
                    resume_thread_id: Some("unsupported-old-id".into()),
                    selected_mcps: Vec::new(),
                },
                PromptSessionCancel::new(),
                None,
                &mut |_| {},
            )
            .unwrap();
        std::env::remove_var("AGENT_DOCTOR_DSH_BIN");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("OPENAI_BASE_URL");
        assert_eq!(report.status, PromptSessionStatus::Succeeded);
        assert_eq!(report.runtime_thread_id, None);
        assert_eq!(
            fs::read_to_string(args_path).unwrap(),
            "--profile\nheadless\nhello\n"
        );
        assert_eq!(
            fs::read_to_string(env_path).unwrap(),
            "set|https://api.deepseek.com/v1\n"
        );
    }
}
