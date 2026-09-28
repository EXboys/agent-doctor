use serde::{Deserialize, Serialize};

use super::execute::{
    build_audit_report, create_runtime_backup_snapshot, probe_health_summary, probe_issue_score,
};
use super::llm::LlmConfig;
use super::planner::{
    build_masked_repair_context, AiRepairPlanner, DeterministicPlanner, MaskedRepairContext,
    PlannerOptions, RepairPlanner,
};
use super::tools::RepairToolResult;
use super::{AuditReport, BackupSnapshot, RepairPlan, SkippedRepairAction};
use crate::probe::{probe_runtime, RuntimeProbeReport};
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
            });
            break;
        }

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
            });
            break;
        }

        executed_action_ids.extend(round_executed.clone());
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
            });
            break;
        }

        current_probe = probe_runtime(runtime_id)?;
        let new_score = probe_issue_score(&current_probe);
        rounds.push(RepairLoopRound {
            round,
            probe_summary: probe_health_summary(&current_probe),
            issue_score: new_score,
            planned_action_ids,
            executed_action_ids: round_executed,
            skipped_actions: round_skipped,
            tool_trace,
            masked_context: context,
        });

        if new_score >= previous_score {
            break;
        }
        previous_score = new_score;
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

fn tool_applied_ids(trace: &[RepairToolResult]) -> Vec<String> {
    trace
        .iter()
        .filter(|item| item.applied && item.success)
        .map(|item| format!("tool:{:?}", item.kind).to_ascii_lowercase())
        .collect()
}
