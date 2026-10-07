use serde::{Deserialize, Serialize};

use super::execute::{
    build_audit_report, create_runtime_backup_snapshot, probe_health_summary, probe_issue_score,
};
use super::llm::LlmConfig;
use super::planner::{
    build_masked_repair_context, AiRepairPlanner, DeterministicPlanner, MaskedRepairContext,
    PlannerOptions, RepairPlanner,
};
use super::restore::restore_backup_snapshot;
use super::tools::RepairToolResult;
use super::{AuditReport, BackupSnapshot, RepairPlan, SkippedRepairAction};
use crate::probe::{probe_runtime, ProbeStatus, RuntimeProbeReport};
use crate::runtime::{
    apply_runtime_playbook_filtered, runtime_supports_playbook, suggest_runtime_repairs,
};

const DEFAULT_MAX_ROUNDS: u32 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairLoopRound {
    pub round: u32,
    pub probe_summary: String,
    pub issue_score: u32,
    pub planned_action_ids: Vec<String>,
    pub executed_action_ids: Vec<String>,
    pub skipped_actions: Vec<SkippedRepairAction>,
    pub tool_trace: Vec<RepairToolResult>,
    pub masked_context: MaskedRepairContext,
    /// Checks this round made worse; non-empty with `rolled_back` means its edits were undone.
    #[serde(default)]
    pub new_issues: Vec<CheckChange>,
    #[serde(default)]
    pub rolled_back: bool,
}

/// One check whose state differs between two probes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckChange {
    pub id: String,
    pub title: String,
    pub status: ProbeStatus,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckDiff {
    /// Was warn/fail before, now passes (or no longer reported).
    pub fixed: Vec<CheckChange>,
    /// New warn/fail, or a check that got worse.
    pub new_issues: Vec<CheckChange>,
}

fn status_rank(status: ProbeStatus) -> u8 {
    match status {
        ProbeStatus::Pass | ProbeStatus::NotApplicable => 0,
        ProbeStatus::NotChecked => 1,
        ProbeStatus::Warn => 2,
        ProbeStatus::Fail => 3,
    }
}

fn is_problem(status: ProbeStatus) -> bool {
    status_rank(status) >= 2
}

pub fn diff_probe_checks(before: &RuntimeProbeReport, after: &RuntimeProbeReport) -> CheckDiff {
    let mut diff = CheckDiff::default();
    for old in before
        .checks
        .iter()
        .filter(|check| is_problem(check.status))
    {
        let still_bad = after
            .checks
            .iter()
            .any(|check| check.id == old.id && is_problem(check.status));
        if !still_bad {
            diff.fixed.push(CheckChange {
                id: old.id.clone(),
                title: old.title.clone(),
                status: ProbeStatus::Pass,
                message: old.message.clone(),
            });
        }
    }
    for new in after.checks.iter().filter(|check| is_problem(check.status)) {
        let before_rank = before
            .checks
            .iter()
            .find(|check| check.id == new.id)
            .map(|check| status_rank(check.status))
            .unwrap_or(0);
        if status_rank(new.status) > before_rank {
            diff.new_issues.push(CheckChange {
                id: new.id.clone(),
                title: new.title.clone(),
                status: new.status,
                message: new.message.clone(),
            });
        }
    }
    diff
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepairLoopOptions {
    pub apply_confirmed_writes: bool,
    pub max_rounds: Option<u32>,
    pub use_ai_planner: bool,
    /// Model for the AI planner; `None` reads it from the environment.
    #[serde(skip)]
    pub llm: Option<LlmConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairLoopReport {
    pub runtime_id: String,
    pub plan: RepairPlan,
    pub backup: BackupSnapshot,
    pub before_probe: RuntimeProbeReport,
    pub after_probe: RuntimeProbeReport,
    pub rounds: Vec<RepairLoopRound>,
    pub executed_action_ids: Vec<String>,
    pub skipped_actions: Vec<SkippedRepairAction>,
    pub audit: AuditReport,
    pub guide_path: Option<String>,
}

/// Generic bounded repair loop: probe → mask → plan (optional agent tools) → apply → verify.
pub fn execute_repair_loop(
    runtime_id: &str,
    options: &RepairLoopOptions,
) -> anyhow::Result<RepairLoopReport> {
    let max_rounds = options.max_rounds.unwrap_or(DEFAULT_MAX_ROUNDS);
    let before_probe = probe_runtime(runtime_id)?;
    let plan = super::build_repair_preview_from_bundle(before_probe.to_diagnostic_bundle());
    let backup = create_runtime_backup_snapshot(runtime_id)?;

    let mut rounds = Vec::new();
    let mut executed_action_ids = vec!["backup-runtime-configs".to_string()];
    let mut skipped_actions = Vec::new();
    let mut guide_path = None;

    let mut current_probe = before_probe.clone();
    let mut previous_score = probe_issue_score(&current_probe);

    let planner_options = PlannerOptions {
        apply_tool_writes: options.apply_confirmed_writes,
    };

    for round in 1..=max_rounds {
        let suggested = suggest_runtime_repairs(runtime_id, &current_probe);
        let auto_fixable_count = suggested.iter().filter(|item| item.auto_fixable).count();
        if auto_fixable_count == 0 && !options.use_ai_planner {
            let context = build_masked_repair_context(runtime_id, &current_probe, suggested);
            rounds.push(RepairLoopRound {
                round,
                probe_summary: probe_health_summary(&current_probe),
                issue_score: previous_score,
                planned_action_ids: Vec::new(),
                executed_action_ids: Vec::new(),
                skipped_actions: vec![SkippedRepairAction {
                    id: "repair-loop".to_string(),
                    reason: "no auto-fixable issues remain".to_string(),
                }],
                tool_trace: Vec::new(),
                masked_context: context,
                new_issues: Vec::new(),
                rolled_back: false,
            });
            break;
        }

        // The AI planner writes while planning, so snapshot before it runs.
        let round_backup = if options.apply_confirmed_writes && round > 1 {
            create_runtime_backup_snapshot(runtime_id)?
        } else {
            backup.clone()
        };

        let mut context = build_masked_repair_context(runtime_id, &current_probe, suggested);
        let plan_result = if options.use_ai_planner {
            AiRepairPlanner {
                config: options.llm.clone(),
            }
            .plan(&mut context, &planner_options)?
        } else {
            DeterministicPlanner.plan(&mut context, &planner_options)?
        };
        let planned_action_ids = plan_result.action_ids;
        let tool_trace = plan_result.tool_trace;

        let mut round_executed = tool_applied_ids(&tool_trace);
        let mut round_skipped = Vec::new();

        if !options.apply_confirmed_writes {
            round_skipped.push(SkippedRepairAction {
                id: "repair-loop".to_string(),
                reason: "pass --apply to execute planned fixes".to_string(),
            });
            rounds.push(RepairLoopRound {
                round,
                probe_summary: probe_health_summary(&current_probe),
                issue_score: previous_score,
                planned_action_ids,
                executed_action_ids: round_executed,
                skipped_actions: round_skipped,
                tool_trace,
                masked_context: context,
                new_issues: Vec::new(),
                rolled_back: false,
            });
            break;
        }

        if runtime_supports_playbook(runtime_id) && !planned_action_ids.is_empty() {
            let filter = if planned_action_ids.is_empty() {
                None
            } else {
                Some(planned_action_ids.as_slice())
            };
            let playbook = apply_runtime_playbook_filtered(runtime_id, &current_probe, filter)?;
            if let Some(path) = playbook.guide_path {
                guide_path = Some(path.display().to_string());
            }
            round_executed.extend(playbook.executed);
            round_skipped.extend(playbook.skipped);
        } else if !runtime_supports_playbook(runtime_id) && round_executed.is_empty() {
            round_skipped.push(SkippedRepairAction {
                id: "repair-loop".to_string(),
                reason: format!("runtime '{runtime_id}' has no repair playbook yet"),
            });
            rounds.push(RepairLoopRound {
                round,
                probe_summary: probe_health_summary(&current_probe),
                issue_score: previous_score,
                planned_action_ids,
                executed_action_ids: round_executed,
                skipped_actions: round_skipped,
                tool_trace,
                masked_context: context,
                new_issues: Vec::new(),
                rolled_back: false,
            });
            break;
        }

        skipped_actions.extend(round_skipped.clone());

        if round_executed.is_empty() {
            rounds.push(RepairLoopRound {
                round,
                probe_summary: probe_health_summary(&current_probe),
                issue_score: previous_score,
                planned_action_ids,
                executed_action_ids: round_executed,
                skipped_actions: round_skipped,
                tool_trace,
                masked_context: context,
                new_issues: Vec::new(),
                rolled_back: false,
            });
            break;
        }

        let round_probe = probe_runtime(runtime_id)?;
        let new_score = probe_issue_score(&round_probe);
        let new_issues = diff_probe_checks(&current_probe, &round_probe).new_issues;
        let rolled_back = should_roll_back(previous_score, new_score, &new_issues);
        if rolled_back {
            restore_backup_snapshot(&round_backup)?;
            current_probe = probe_runtime(runtime_id)?;
            skipped_actions.push(SkippedRepairAction {
                id: format!("repair-loop-round-{round}"),
                reason: "changes made new problems, so they were undone".to_string(),
            });
        } else {
            executed_action_ids.extend(round_executed.clone());
            current_probe = round_probe;
        }
        let round_score = probe_issue_score(&current_probe);
        rounds.push(RepairLoopRound {
            round,
            probe_summary: probe_health_summary(&current_probe),
            issue_score: round_score,
            planned_action_ids,
            executed_action_ids: round_executed,
            skipped_actions: round_skipped,
            tool_trace,
            masked_context: context,
            new_issues,
            rolled_back,
        });

        if rolled_back || round_score >= previous_score {
            break;
        }
        previous_score = round_score;
    }

    let after_probe = if options.apply_confirmed_writes {
        probe_runtime(runtime_id)?
    } else {
        current_probe
    };

    let audit = build_audit_report(
        runtime_id,
        &plan,
        &backup,
        &before_probe,
        &after_probe,
        &executed_action_ids,
    );

    Ok(RepairLoopReport {
        runtime_id: runtime_id.to_string(),
        plan,
        backup,
        before_probe,
        after_probe,
        rounds,
        executed_action_ids,
        skipped_actions,
        audit,
        guide_path,
    })
}

/// Keep a round that lowers the score even if it adds a smaller problem;
/// undo one that trades a fix for a new problem or makes things worse.
fn should_roll_back(previous_score: u32, new_score: u32, new_issues: &[CheckChange]) -> bool {
    new_score > previous_score || (new_score == previous_score && !new_issues.is_empty())
}

fn tool_applied_ids(trace: &[RepairToolResult]) -> Vec<String> {
    trace
        .iter()
        .filter(|item| item.applied && item.success)
        .map(|item| format!("tool:{:?}", item.kind).to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{ProbeCheck, ProbeSeverity};
    use crate::repair::SensitivityLevel;

    fn probe(checks: &[(&str, ProbeStatus)]) -> RuntimeProbeReport {
        RuntimeProbeReport {
            runtime_id: "openclaw".to_string(),
            display_name: "OpenClaw".to_string(),
            binary_name: "openclaw".to_string(),
            checks: checks
                .iter()
                .map(|(id, status)| {
                    ProbeCheck::new(
                        *id,
                        *id,
                        *status,
                        ProbeSeverity::Warning,
                        "msg",
                        SensitivityLevel::Public,
                    )
                })
                .collect(),
            facts: vec![],
        }
    }

    fn ids(changes: &[CheckChange]) -> Vec<&str> {
        changes.iter().map(|change| change.id.as_str()).collect()
    }

    #[test]
    fn diff_reports_swap_of_one_problem_for_another() {
        let before = probe(&[
            ("gateway.connectivity", ProbeStatus::Warn),
            ("openclaw.schema.legacy_gateway_url", ProbeStatus::Pass),
            ("binary.upstream_version", ProbeStatus::Warn),
        ]);
        let after = probe(&[
            ("gateway.connectivity", ProbeStatus::Pass),
            ("openclaw.schema.legacy_gateway_url", ProbeStatus::Warn),
            ("binary.upstream_version", ProbeStatus::Warn),
        ]);
        let diff = diff_probe_checks(&before, &after);
        assert_eq!(ids(&diff.fixed), vec!["gateway.connectivity"]);
        assert_eq!(
            ids(&diff.new_issues),
            vec!["openclaw.schema.legacy_gateway_url"]
        );
    }

    #[test]
    fn diff_counts_vanished_problem_as_fixed_and_new_check_as_issue() {
        let before = probe(&[("gateway.profile", ProbeStatus::Warn)]);
        let after = probe(&[("mcp.browser.configured", ProbeStatus::Fail)]);
        let diff = diff_probe_checks(&before, &after);
        assert_eq!(ids(&diff.fixed), vec!["gateway.profile"]);
        assert_eq!(ids(&diff.new_issues), vec!["mcp.browser.configured"]);
    }

    #[test]
    fn diff_counts_warn_to_fail_as_new_issue_not_fixed() {
        let before = probe(&[("config.schema", ProbeStatus::Warn)]);
        let after = probe(&[("config.schema", ProbeStatus::Fail)]);
        let diff = diff_probe_checks(&before, &after);
        assert!(diff.fixed.is_empty());
        assert_eq!(ids(&diff.new_issues), vec!["config.schema"]);
    }

    #[test]
    fn roll_back_only_when_round_does_not_pay_off() {
        let issue = vec![CheckChange {
            id: "x".to_string(),
            title: "x".to_string(),
            status: ProbeStatus::Warn,
            message: String::new(),
        }];
        // Fixed a warn, added a warn: same score, undo.
        assert!(should_roll_back(20, 20, &issue));
        // Worse overall: undo.
        assert!(should_roll_back(20, 30, &[]));
        // Fixed a fail, added a warn: clearly better, keep.
        assert!(!should_roll_back(130, 40, &issue));
        // Nothing changed: nothing to undo.
        assert!(!should_roll_back(20, 20, &[]));
    }
}
