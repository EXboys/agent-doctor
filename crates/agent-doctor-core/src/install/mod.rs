use std::path::PathBuf;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::lifecycle::{
    claude_code_install_shell_command, codex_install_shell_command, hermes_install_shell_command,
    openclaw_install_shell_command, run_shell_command_streaming, write_install_log,
};
use crate::probe::{probe_runtime, ProbeStatus, RuntimeProbeReport};
use crate::repair::{
    execute_repair_loop, explain_runtime, probe_health_summary, probe_issue_score, ExplainReport,
    RepairLoopOptions, RepairLoopReport, SkippedRepairAction,
};
use crate::runtime::{descriptor_by_id, runtime_supports_lifecycle, suggest_runtime_repairs};

fn is_install_failure_action(id: &str) -> bool {
    id.ends_with("-install")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallOptions {
    /// After rule-based install, run AI repair loop for remaining issues.
    pub plan_ai_repair: bool,
    /// After install, run deterministic repair loop when issues remain.
    pub repair_after: bool,
    /// Call LLM explain (diagnosis / failure interpretation).
    pub explain: bool,
    /// Retry rule-based install up to N extra times on failure.
    pub retry_count: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallReport {
    pub runtime_id: String,
    pub install_needed: bool,
    pub install_succeeded: bool,
    pub install_attempts: u8,
    pub before_probe: RuntimeProbeReport,
    pub after_probe: RuntimeProbeReport,
    pub skipped_actions: Vec<SkippedRepairAction>,
    pub install_log_path: Option<String>,
    pub manual_fallback: Vec<String>,
    pub explain: Option<ExplainReport>,
    pub repair_loop: Option<RepairLoopReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallProgressEvent {
    pub runtime_id: String,
    /// probing | installing | output | verifying | done
    pub phase: String,
    pub message: String,
    pub percent: u8,
}

pub fn execute_install(runtime_id: &str, options: &InstallOptions) -> Result<InstallReport> {
    execute_install_with_progress(runtime_id, options, |_| {})
}

pub fn execute_install_with_progress<F>(
    runtime_id: &str,
    options: &InstallOptions,
    mut on_progress: F,
) -> Result<InstallReport>
where
    F: FnMut(InstallProgressEvent),
{
    if descriptor_by_id(runtime_id).is_none() {
        bail!("unknown runtime '{runtime_id}'");
    }

    let emit = |phase: &str, message: &str, percent: u8| InstallProgressEvent {
        runtime_id: runtime_id.to_string(),
        phase: phase.to_string(),
        message: message.to_string(),
        percent,
    };

    on_progress(emit("probing", "Checking whether the runtime binary is installed…", 5));

    let has_rule_install = runtime_supports_lifecycle(runtime_id);
    let before_probe = probe_runtime(runtime_id)?;
    let install_needed = needs_binary_install(&before_probe);
    let mut skipped_actions = Vec::new();
    let mut install_log_path = None;
    let mut install_attempts = 0u8;
    let mut install_succeeded = !install_needed;

    if !install_needed {
        on_progress(emit("done", "Already installed — no install action required.", 100));
    } else if has_rule_install {
        let max_attempts = 1 + options.retry_count;
        while install_attempts < max_attempts {
            install_attempts += 1;
            on_progress(emit(
                "installing",
                &format!("Running installer (attempt {install_attempts}/{max_attempts})…"),
                15,
            ));
            match run_rule_install(runtime_id, &mut on_progress) {
                Ok(path) => {
                    install_log_path = Some(path.display().to_string());
                    on_progress(emit("verifying", "Installer finished — verifying binary…", 85));
                    let after_attempt = probe_runtime(runtime_id)?;
                    if !needs_binary_install(&after_attempt) {
                        install_succeeded = true;
                        break;
                    }
                    skipped_actions.push(SkippedRepairAction {
                        id: install_action_id(runtime_id).to_string(),
                        reason: "install script finished but binary still not found on PATH"
                            .to_string(),
                    });
                }
                Err(error) => {
                    if let Some(path) = error.log_path {
                        install_log_path = Some(path.display().to_string());
                    }
                    skipped_actions.push(SkippedRepairAction {
                        id: install_action_id(runtime_id).to_string(),
                        reason: error.reason,
                    });
                }
            }
        }
    } else {
        on_progress(emit(
            "installing",
            "No rule installer — falling back to AI / allowlisted install…",
            20,
        ));
    }

    on_progress(emit("verifying", "Re-probing runtime health…", 90));
    let mut after_probe = probe_runtime(runtime_id)?;
    if install_needed && !install_succeeded {
        install_succeeded = !needs_binary_install(&after_probe);
    }

    let explain = if options.explain {
        Some(build_and_explain(
            runtime_id,
            &after_probe,
            install_log_path.as_deref(),
            skipped_actions.last(),
        )?)
    } else {
        None
    };

    let repair_loop = match repair_loop_after_install(
        options,
        install_needed,
        install_succeeded,
        has_rule_install,
        &after_probe,
    ) {
        Some(use_ai_planner) => {
            on_progress(emit("installing", "Running repair loop for remaining issues…", 92));
            let loop_report = execute_repair_loop(
                runtime_id,
                &RepairLoopOptions {
                    apply_confirmed_writes: true,
                    max_rounds: None,
                    use_ai_planner,
                },
            )?;
            after_probe = loop_report.after_probe.clone();
            install_succeeded = !install_needed || !needs_binary_install(&after_probe);
            Some(loop_report)
        }
        None => None,
    };

    on_progress(emit(
        "done",
        if install_succeeded {
            "Install finished."
        } else {
            "Install finished with issues."
        },
        100,
    ));

    Ok(InstallReport {
        runtime_id: runtime_id.to_string(),
        install_needed,
        install_succeeded,
        install_attempts,
        before_probe,
        after_probe,
        skipped_actions,
        install_log_path,
        manual_fallback: manual_fallback_steps(runtime_id, install_needed, install_succeeded),
        explain,
        repair_loop,
    })
}

struct InstallRunError {
    reason: String,
    log_path: Option<PathBuf>,
}

fn run_rule_install<F>(
    runtime_id: &str,
    on_progress: &mut F,
) -> std::result::Result<PathBuf, InstallRunError>
where
    F: FnMut(InstallProgressEvent),
{
    let command = install_shell_command(runtime_id).ok_or_else(|| InstallRunError {
        reason: format!("no install command for runtime '{runtime_id}'"),
        log_path: None,
    })?;

    on_progress(InstallProgressEvent {
        runtime_id: runtime_id.to_string(),
        phase: "installing".to_string(),
        message: format!("$ {command}"),
        percent: 20,
    });

    let mut line_count = 0u32;
    let capture = run_shell_command_streaming(&command, |line| {
        line_count = line_count.saturating_add(1);
        let percent = (20 + (line_count.min(60) as u8)).min(80);
        on_progress(InstallProgressEvent {
            runtime_id: runtime_id.to_string(),
            phase: "output".to_string(),
            message: line.to_string(),
            percent,
        });
    })
    .map_err(|error| InstallRunError {
        reason: error.to_string(),
        log_path: None,
    })?;

    let log_path = write_install_log(runtime_id, &capture).ok();

    if capture.success {
        log_path.ok_or_else(|| InstallRunError {
            reason: "failed to save install log".to_string(),
            log_path: None,
        })
    } else {
        let tail = capture.combined_output();
        let tail = tail
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        Err(InstallRunError {
            reason: if tail.is_empty() {
                format!("installer exited with status {:?}", capture.exit_code)
            } else {
                tail
            },
            log_path,
        })
    }
}

fn install_shell_command(runtime_id: &str) -> Option<String> {
    match runtime_id {
        "hermes" => Some(hermes_install_shell_command()),
        "openclaw" => Some(openclaw_install_shell_command()),
        "claude-code" => Some(claude_code_install_shell_command()),
        "codex" => Some(codex_install_shell_command()),
        _ => None,
    }
}

fn install_action_id(runtime_id: &str) -> &'static str {
    match runtime_id {
        "hermes" => "fix-hermes-install",
        "openclaw" => "fix-openclaw-install",
        "claude-code" => "fix-claude-code-install",
        "codex" => "fix-codex-install",
        _ => "fix-install",
    }
}

pub fn needs_binary_install(probe: &RuntimeProbeReport) -> bool {
    probe
        .checks
        .iter()
        .any(|check| check.id == "binary.exists" && check.status == ProbeStatus::Fail)
}

fn has_remaining_work(probe: &RuntimeProbeReport) -> bool {
    suggest_runtime_repairs(&probe.runtime_id, probe)
        .iter()
        .any(|item| item.auto_fixable)
}

/// No rule installer → AI install. Rule failed → AI repair. Success → optional `--plan ai` / `--repair`.
fn repair_loop_after_install(
    options: &InstallOptions,
    install_needed: bool,
    install_succeeded: bool,
    has_rule_install: bool,
    probe: &RuntimeProbeReport,
) -> Option<bool> {
    if !has_remaining_work(probe) {
        return None;
    }

    let binary_missing = needs_binary_install(probe);
    if install_needed && binary_missing {
        if !has_rule_install {
            return Some(true);
        }
        if !install_succeeded {
            return Some(true);
        }
    }

    if options.plan_ai_repair || options.repair_after {
        return Some(options.plan_ai_repair);
    }

    None
}

fn build_and_explain(
    runtime_id: &str,
    probe: &RuntimeProbeReport,
    log_path: Option<&str>,
    skipped: Option<&SkippedRepairAction>,
) -> Result<ExplainReport> {
    let install_failure = skipped
        .filter(|item| is_install_failure_action(&item.id))
        .map(|item| {
            let log_tail = log_path
                .and_then(|path| std::fs::read_to_string(path).ok())
                .map(|content| {
                    content
                        .lines()
                        .rev()
                        .take(12)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            crate::repair::ExplainInstallFailure {
                action_id: item.id.clone(),
                reason: item.reason.clone(),
                log_path: log_path.map(str::to_string),
                log_tail,
            }
        });

    let input = build_explain_input(runtime_id, probe, install_failure);
    explain_runtime(&input)
}

fn manual_fallback_steps(
    runtime_id: &str,
    install_needed: bool,
    install_succeeded: bool,
) -> Vec<String> {
    let mut steps = Vec::new();
    if install_needed && !install_succeeded {
        match runtime_id {
            "hermes" => {
                steps.push("Retry: agent-doctor install hermes".to_string());
                steps.push(
                    "Manual: see https://github.com/NousResearch/hermes-agent install docs"
                        .to_string(),
                );
            }
            "openclaw" => {
                steps.push("Retry: agent-doctor install openclaw".to_string());
                steps.push(
                    "Manual: curl -fsSL https://openclaw.ai/install.sh | bash -s -- --no-onboard"
                        .to_string(),
                );
                steps.push("Then: openclaw onboard --install-daemon".to_string());
            }
            "claude-code" => {
                steps.push("Retry: agent-doctor install claude-code".to_string());
                steps.push("Manual: npm install -g @anthropic-ai/claude-code".to_string());
            }
            "codex" => {
                steps.push("Retry: agent-doctor install codex".to_string());
                steps.push("Manual: npm install -g @openai/codex".to_string());
            }
            _ => steps.push(format!("Retry: agent-doctor install {runtime_id}")),
        }
    } else if install_succeeded && runtime_id == "openclaw" {
        steps.push("Run: openclaw onboard --install-daemon (if not done yet)".to_string());
    }
    steps
}

pub fn build_explain_input(
    runtime_id: &str,
    probe: &RuntimeProbeReport,
    install_failure: Option<crate::repair::ExplainInstallFailure>,
) -> crate::repair::ExplainInput {
    use crate::repair::{ExplainCheck, ExplainInput, ExplainSuggestion};

    let suggested = suggest_runtime_repairs(runtime_id, probe);
    ExplainInput {
        runtime_id: runtime_id.to_string(),
        probe_summary: probe_health_summary(probe),
        issue_score: probe_issue_score(probe),
        checks: probe
            .checks
            .iter()
            .map(|check| ExplainCheck {
                title: check.title.clone(),
                status: format!("{:?}", check.status).to_ascii_lowercase(),
                message: check.message.clone(),
            })
            .collect(),
        suggested_repairs: suggested
            .iter()
            .map(|item| ExplainSuggestion {
                id: item.id.clone(),
                title: item.title.clone(),
                auto_fixable: item.auto_fixable,
            })
            .collect(),
        install_failure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{ProbeCheck, ProbeSeverity};

    #[test]
    fn detects_missing_binary() {
        let probe = RuntimeProbeReport {
            runtime_id: "openclaw".to_string(),
            display_name: "OpenClaw".to_string(),
            binary_name: "openclaw".to_string(),
            checks: vec![ProbeCheck::new(
                "binary.exists",
                "Binary on PATH",
                ProbeStatus::Fail,
                ProbeSeverity::Error,
                "missing",
                crate::repair::SensitivityLevel::Public,
            )],
            facts: Vec::new(),
        };
        assert!(needs_binary_install(&probe));
    }

    #[test]
    fn repair_loop_after_rule_install_failure_uses_ai() {
        let probe = RuntimeProbeReport {
            runtime_id: "openclaw".to_string(),
            display_name: "OpenClaw".to_string(),
            binary_name: "openclaw".to_string(),
            checks: vec![ProbeCheck::new(
                "binary.exists",
                "Binary on PATH",
                ProbeStatus::Fail,
                ProbeSeverity::Error,
                "missing",
                crate::repair::SensitivityLevel::Public,
            )],
            facts: Vec::new(),
        };
        assert_eq!(
            repair_loop_after_install(&InstallOptions::default(), true, false, true, &probe),
            Some(true)
        );
    }

    #[test]
    fn repair_loop_without_rules_uses_ai_directly() {
        let probe = RuntimeProbeReport {
            runtime_id: "claude-code".to_string(),
            display_name: "Claude Code".to_string(),
            binary_name: "claude".to_string(),
            checks: vec![ProbeCheck::new(
                "binary.exists",
                "Binary on PATH",
                ProbeStatus::Fail,
                ProbeSeverity::Error,
                "missing",
                crate::repair::SensitivityLevel::Public,
            )],
            facts: Vec::new(),
        };
        assert_eq!(
            repair_loop_after_install(&InstallOptions::default(), true, false, false, &probe),
            Some(true)
        );
    }

    #[test]
    fn repair_loop_after_success_requires_flag() {
        let probe = RuntimeProbeReport {
            runtime_id: "openclaw".to_string(),
            display_name: "OpenClaw".to_string(),
            binary_name: "openclaw".to_string(),
            checks: vec![ProbeCheck::new(
                "binary.exists",
                "Binary on PATH",
                ProbeStatus::Fail,
                ProbeSeverity::Error,
                "missing",
                crate::repair::SensitivityLevel::Public,
            )],
            facts: Vec::new(),
        };
        assert_eq!(
            repair_loop_after_install(
                &InstallOptions {
                    plan_ai_repair: true,
                    ..Default::default()
                },
                true,
                true,
                true,
                &probe
            ),
            Some(true)
        );
        assert_eq!(
            repair_loop_after_install(&InstallOptions::default(), true, true, true, &probe),
            None
        );
    }
}
