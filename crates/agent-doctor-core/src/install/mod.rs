use std::path::PathBuf;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::lifecycle::{
    claude_code_install_shell_command, codex_install_shell_command,
    deepseek_harness_install_shell_command, hermes_install_shell_command,
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
    /// Re-run the installer even when the binary already exists on PATH.
    pub force: bool,
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

    // GUI apps inherit a minimal PATH; seed Homebrew/npm dirs before installers run.
    crate::adapters::util::refresh_managed_runtime_path();

    let emit = |phase: &str, message: &str, percent: u8| InstallProgressEvent {
        runtime_id: runtime_id.to_string(),
        phase: phase.to_string(),
        message: message.to_string(),
        percent,
    };

    on_progress(emit(
        "probing",
        "Checking whether the runtime binary is installed…",
        5,
    ));

    let has_rule_install = runtime_supports_lifecycle(runtime_id);
    let before_probe = probe_runtime(runtime_id)?;
    let binary_missing = needs_binary_install(&before_probe);
    let install_needed = binary_missing || options.force;
    let mut skipped_actions = Vec::new();
    let mut install_log_path = None;
    let mut install_attempts = 0u8;
    let mut install_succeeded = !install_needed;

    if !install_needed {
        on_progress(emit(
            "done",
            "Already installed — no install action required.",
            100,
        ));
    } else if has_rule_install {
        if options.force && !binary_missing {
            on_progress(emit(
                "installing",
                "Force reinstall requested — re-running installer…",
                12,
            ));
        }
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
                    // Installers update the user PATH in the OS; refresh this process
                    // so the post-install probe can see the new binary.
                    crate::adapters::util::refresh_managed_runtime_path();
                    on_progress(emit(
                        "verifying",
                        "Installer finished — verifying binary…",
                        85,
                    ));
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
    crate::adapters::util::refresh_managed_runtime_path();
    let mut after_probe = probe_runtime(runtime_id)?;
    if install_needed && !install_succeeded && !needs_binary_install(&after_probe) {
        install_succeeded = true;
    }
    // npm/brew may finish linking a moment after the installer exits.
    if install_needed && needs_binary_install(&after_probe) {
        std::thread::sleep(std::time::Duration::from_millis(750));
        crate::adapters::util::refresh_managed_runtime_path();
        after_probe = probe_runtime(runtime_id)?;
        if !needs_binary_install(&after_probe) {
            install_succeeded = true;
            skipped_actions.retain(|item| !is_install_failure_action(&item.id));
        }
    }
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
            on_progress(emit(
                "installing",
                "Running repair loop for remaining issues…",
                92,
            ));
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

    if matches!(runtime_id, "claude-code" | "codex" | "deepseek-harness") {
        on_progress(InstallProgressEvent {
            runtime_id: runtime_id.to_string(),
            phase: "installing".to_string(),
            message: "Checking Node.js / npm…".to_string(),
            percent: 12,
        });
        if let Err(error) = crate::lifecycle::nodejs::ensure_npm_with_progress(|line, node_pct| {
            let percent = (10 + u16::from(node_pct) * 40 / 100) as u8;
            on_progress(InstallProgressEvent {
                runtime_id: runtime_id.to_string(),
                phase: "output".to_string(),
                message: line.to_string(),
                percent,
            });
        }) {
            return Err(InstallRunError {
                reason: format!("Node.js / npm auto-install failed: {error:#}"),
                log_path: None,
            });
        }
    }

    on_progress(InstallProgressEvent {
        runtime_id: runtime_id.to_string(),
        phase: "installing".to_string(),
        message: format!("$ {command}"),
        percent: 52,
    });

    let mut line_count = 0u32;
    let mut saw_deps = false;
    let capture = run_shell_command_streaming(&command, |line| {
        if plain_progress_line(line).is_empty() {
            return;
        }
        line_count = line_count.saturating_add(1);
        let (message, percent) = install_output_progress(line, line_count, &mut saw_deps);
        on_progress(InstallProgressEvent {
            runtime_id: runtime_id.to_string(),
            phase: "output".to_string(),
            message,
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

fn install_output_progress(line: &str, line_count: u32, saw_deps: &mut bool) -> (String, u8) {
    if let Some(pct) = labeled_percent(line, "Receiving objects:") {
        let percent = 52 + u16::from(pct) * 28 / 100;
        return (format!("正在下载代码… {pct}%"), percent as u8);
    }
    if let Some(pct) = labeled_percent(line, "Resolving deltas:") {
        let percent = 80 + u16::from(pct) * 8 / 100;
        return (format!("正在下载代码… {pct}%"), percent as u8);
    }
    if let Some(pct) = curl_bar_percent(line) {
        return (format!("正在下载，已完成 {pct}%。"), pct.clamp(1, 99));
    }
    if let Some(progress) = dependency_progress(line) {
        *saw_deps = true;
        return progress;
    }
    if *saw_deps {
        return ("正在安装依赖…".to_string(), 92);
    }
    let percent = (52 + (line_count.min(30) as u8)).min(85);
    let shown = plain_progress_line(line);
    if shown.is_empty() {
        return ("正在安装…".to_string(), percent);
    }
    (shown, percent)
}

fn dependency_progress(line: &str) -> Option<(String, u8)> {
    let line = plain_progress_line(line);
    if line.is_empty() {
        return None;
    }
    if line.contains("node-v") && line.contains("Downloading") {
        return Some(("正在下载 Node.js…".to_string(), 70));
    }
    if let Some(total) = leading_count(&line, "Resolved ", " package") {
        return Some((format!("正在确认要装哪些依赖，一共 {total} 个。"), 86));
    }
    if let Some(total) = leading_count(&line, "Installed ", " package") {
        return Some((format!("依赖装好了，一共 {total} 个。"), 98));
    }
    if let Some((done, total)) = done_of_total(&line) {
        let percent = 88 + done * 10 / total.max(1);
        return Some((
            format!("正在安装依赖，已完成 {done} 个，一共 {total} 个。"),
            (percent as u8).min(98),
        ));
    }
    if line.contains("Resolving dependencies") {
        return Some(("正在确认要装哪些依赖。".to_string(), 86));
    }
    if line.contains("Installing dependencies")
        || line.contains("Preparing packages")
        || line.contains("Installing wheels")
        || line.contains("Downloading")
        || line.contains("uv sync")
    {
        return Some(("正在安装依赖…".to_string(), 88));
    }
    None
}

fn plain_progress_line(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if ch == '\u{8}' {
            out.pop();
            continue;
        }
        if ch.is_control() {
            continue;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

fn leading_count(line: &str, prefix: &str, suffix: &str) -> Option<u32> {
    let rest = line.trim().strip_prefix(prefix)?;
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() || !rest[digits.len()..].starts_with(suffix) {
        return None;
    }
    digits.parse().ok()
}

fn done_of_total(line: &str) -> Option<(u16, u16)> {
    let (left, right) = line.split_once('/')?;
    let done: String = left
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let total: String = right.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if done.is_empty() || total.is_empty() {
        return None;
    }
    let done: u16 = done.parse().ok()?;
    let total: u16 = total.parse().ok()?;
    if total == 0 || done > total {
        return None;
    }
    Some((done, total))
}

fn curl_bar_percent(line: &str) -> Option<u8> {
    let line = plain_progress_line(line);
    if !line.contains('#') || !line.contains('%') {
        return None;
    }
    let before = line.rsplit_once('%')?.0;
    let number: String = before
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let value: f32 = number.parse().ok()?;
    if !(0.0..=100.0).contains(&value) {
        return None;
    }
    Some(value.round() as u8)
}

fn labeled_percent(line: &str, label: &str) -> Option<u8> {
    let rest = line.split_once(label)?.1;
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    let pct: u8 = digits.parse().ok()?;
    Some(pct.min(100))
}

fn install_shell_command(runtime_id: &str) -> Option<String> {
    match runtime_id {
        "hermes" => Some(hermes_install_shell_command()),
        "deepseek-harness" => Some(deepseek_harness_install_shell_command()),
        "openclaw" => Some(openclaw_install_shell_command()),
        "claude-code" => Some(claude_code_install_shell_command()),
        "codex" => Some(codex_install_shell_command()),
        _ => None,
    }
}

fn install_action_id(runtime_id: &str) -> &'static str {
    match runtime_id {
        "hermes" => "fix-hermes-install",
        "deepseek-harness" => "fix-deepseek-harness-install",
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
            "deepseek-harness" => {
                steps.push("Retry: agent-doctor install deepseek-harness".to_string());
                steps.push("Manual: npm install --global @deepseek-ai/dsh@0.1.0-rc.6".to_string());
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
                steps.push(
                    "If npm is missing, Agent Doctor installs a user-local Node.js LTS automatically (needs network)."
                        .to_string(),
                );
                steps.push("Manual: npm install -g @anthropic-ai/claude-code".to_string());
            }
            "codex" => {
                steps.push("Retry: agent-doctor install codex".to_string());
                steps.push(
                    "If npm is missing, Agent Doctor installs a user-local Node.js LTS automatically (needs network)."
                        .to_string(),
                );
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
    fn install_progress_follows_git_and_dependency_stages() {
        let mut saw_deps = false;
        let (message, percent) = install_output_progress(
            "Receiving objects:  40% (10/25), 1.00 MiB | 100.00 KiB/s",
            30,
            &mut saw_deps,
        );
        assert_eq!(message, "正在下载代码… 40%");
        assert_eq!(percent, 63);
        let (message, percent) =
            install_output_progress("Installing dependencies...", 31, &mut saw_deps);
        assert_eq!(message, "正在安装依赖…");
        assert!(percent >= 88, "{percent}");
        let (message, _) =
            install_output_progress("Downloading cryptography (12/259)", 32, &mut saw_deps);
        assert_eq!(message, "正在安装依赖，已完成 12 个，一共 259 个。");
        let (message, percent) =
            install_output_progress("Installed 259 packages in 3s", 33, &mut saw_deps);
        assert_eq!(message, "依赖装好了，一共 259 个。");
        assert_eq!(percent, 98);
        let bar = format!("##{}3.5%", " ".repeat(70));
        let (message, percent) = install_output_progress(&bar, 34, &mut saw_deps);
        assert_eq!(message, "正在下载，已完成 4%。");
        assert_eq!(percent, 4);
        let (message, _) = install_output_progress(
            "Downloading node-v26.9.0-darwin-arm64.tar.xz...",
            35,
            &mut saw_deps,
        );
        assert_eq!(message, "正在下载 Node.js…");
    }

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
