use agent_doctor_core::{
    build_explain_input, build_repair_preview_from_bundle, diff_probe_checks, execute_repair,
    execute_repair_loop, explain_runtime, list_runtime_backup_ids, probe_health_summary,
    probe_issue_score, probe_runtime, restore_runtime_backup, runtime_supports_playbook,
    suggest_runtime_repairs, ProbeStatus, RepairExecuteOptions, RepairLoopOptions, RepairRisk,
};
use anyhow::{bail, Result};

use crate::commands::print_explain_report;

#[allow(clippy::too_many_arguments)]
pub fn run(
    runtime: &str,
    apply: bool,
    rollback: bool,
    backup: Option<&str>,
    repair_loop: bool,
    plan: &str,
    explain: bool,
    json: bool,
) -> Result<()> {
    if explain && !apply && !rollback && !repair_loop {
        return run_explain(runtime, json);
    }
    if rollback {
        return run_rollback(runtime, backup, json);
    }
    if repair_loop {
        return run_loop(runtime, apply, plan, json);
    }
    if apply {
        return run_execute(runtime, json);
    }
    run_preview(runtime)
}

fn run_explain(runtime: &str, json: bool) -> Result<()> {
    let probe = probe_runtime(runtime)?;
    let input = build_explain_input(runtime, &probe, None);
    let report = explain_runtime(&input)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    print_explain_report(runtime, &report);
    Ok(())
}

fn run_preview(runtime: &str) -> Result<()> {
    let report = probe_runtime(runtime)?;
    let plan = build_repair_preview_from_bundle(report.to_diagnostic_bundle());

    println!("Agent Doctor — runtime probe and safe repair preview\n");
    println!("Runtime: {}", plan.runtime_id);
    println!("Summary: {}\n", plan.summary);

    if runtime_supports_playbook(runtime) {
        let backups = list_runtime_backup_ids(runtime)?;
        if !backups.is_empty() {
            println!("Recent backups (for --rollback):");
            for id in backups.iter().take(5) {
                println!("  - {id}");
            }
            println!();
        }
    }

    let suggested = suggest_runtime_repairs(runtime, &report);
    if !suggested.is_empty() {
        println!("Suggested fixes (use --apply to run auto-fixable items):");
        for item in &suggested {
            let mode = if item.auto_fixable {
                "auto-fixable"
            } else {
                "manual"
            };
            println!("  - {} [{mode}]", item.title);
            println!("    {}", item.description);
        }
        println!();
    }

    println!("Rule-based probe checks:");
    for check in &report.checks {
        println!(
            "  - {}: {} — {}",
            check.title,
            status_label(check.status),
            check.message
        );
        for detail in &check.details {
            println!("    detail: {detail}");
        }
    }

    println!("\nRedacted diagnostic facts:");
    for fact in &plan.redacted_facts {
        let marker = if fact.redacted { "redacted" } else { "visible" };
        println!("  - {}: {} ({marker})", fact.key, fact.value);
    }

    println!("\nPlanned repair phases:");
    for action in &plan.actions {
        let risk = match action.risk {
            RepairRisk::Low => "low",
            RepairRisk::Medium => "medium",
            RepairRisk::High => "high",
        };
        let confirmation = if action.requires_confirmation {
            "requires confirmation"
        } else {
            "automatic"
        };
        println!("  - {} [{} · {}]", action.title, risk, confirmation);
        println!("    {}", action.description);
        if !action.touches.is_empty() {
            println!("    touches: {}", action.touches.join(", "));
        }
    }

    println!(
        "\nNo files were modified. Run with --apply to execute backup, rule fixes, verification, and audit."
    );
    println!("Bounded loop: agent-doctor repair {runtime} --loop [--apply] [--plan ai]");
    println!("Rollback: agent-doctor repair {runtime} --rollback [--backup <id>]");
    Ok(())
}

fn run_loop(runtime: &str, apply: bool, plan: &str, json: bool) -> Result<()> {
    let use_ai_planner = match plan {
        "deterministic" => false,
        "ai" => true,
        other => bail!("unknown --plan '{other}' (use deterministic or ai)"),
    };

    let report = execute_repair_loop(
        runtime,
        &RepairLoopOptions {
            apply_confirmed_writes: apply,
            max_rounds: None,
            use_ai_planner,
            llm: None,
        },
    )?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!(
        "Agent Doctor — repair loop{}\n",
        if apply { " (apply)" } else { " (preview)" }
    );
    println!("Runtime: {}", report.runtime_id);
    println!(
        "Issue score: {} -> {}",
        probe_issue_score(&report.before_probe),
        probe_issue_score(&report.after_probe)
    );
    println!(
        "Health: {} -> {}",
        probe_health_summary(&report.before_probe),
        probe_health_summary(&report.after_probe)
    );

    for round in &report.rounds {
        println!(
            "\nRound {}: {} (score={})",
            round.round, round.probe_summary, round.issue_score
        );
        if !round.planned_action_ids.is_empty() {
            println!("  planned: {}", round.planned_action_ids.join(", "));
        }
        if !round.executed_action_ids.is_empty() {
            println!("  executed: {}", round.executed_action_ids.join(", "));
        }
        for item in &round.skipped_actions {
            println!("  skipped {}: {}", item.id, item.reason);
        }
        if !round.new_issues.is_empty() {
            let label = if round.rolled_back {
                "new problems (round undone)"
            } else {
                "new problems"
            };
            println!("  {label}:");
            for check in &round.new_issues {
                println!("    - {}: {}", check.title, check.message);
            }
        }
        if !round.tool_trace.is_empty() {
            println!("  agent tools: {} call(s)", round.tool_trace.len());
            for tool in &round.tool_trace {
                let status = if tool.success { "ok" } else { "fail" };
                println!(
                    "    - {:?} [{status}] applied={}{}",
                    tool.kind,
                    tool.applied,
                    tool.error
                        .as_ref()
                        .map(|reason| format!(" — {reason}"))
                        .unwrap_or_default()
                );
                if let Some(diff) = &tool.preview_diff {
                    println!("      diff:\n{diff}");
                }
            }
        }
    }

    if apply {
        let diff = diff_probe_checks(&report.before_probe, &report.after_probe);
        for (label, checks) in [("Fixed", &diff.fixed), ("New problems", &diff.new_issues)] {
            if !checks.is_empty() {
                println!("\n{label}:");
                for check in checks {
                    println!("  - {}", check.title);
                }
            }
        }
    }

    if !apply {
        println!("\nPreview only — pass --apply --loop to execute fixes.");
    } else {
        println!(
            "\nBackup: {} ({} file(s))",
            report.backup.root,
            report.backup.files.len()
        );
        if let Some(guide) = &report.guide_path {
            println!("API key guide: {guide}");
        }
        println!("Audit: {}", report.audit.id);
        println!(
            "Rollback: agent-doctor repair {runtime} --rollback --backup {}",
            report.backup.id
        );
    }
    Ok(())
}

fn run_execute(runtime: &str, json: bool) -> Result<()> {
    let report = execute_repair(
        runtime,
        &RepairExecuteOptions {
            apply_confirmed_writes: true,
        },
    )?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Agent Doctor — repair execute\n");
    println!("Runtime: {}", report.runtime_id);
    println!(
        "Backup: {} ({} file(s))",
        report.backup.root,
        report.backup.files.len()
    );
    for file in &report.backup.files {
        println!("  - {} -> {}", file.original_path, file.snapshot_path);
    }

    println!(
        "\nHealth: {} -> {}",
        probe_health_summary(&report.before_probe),
        probe_health_summary(&report.after_probe)
    );

    println!("\nExecuted actions:");
    if report.executed_action_ids.is_empty() {
        println!("  (none)");
    } else {
        for id in &report.executed_action_ids {
            println!("  - {id}");
        }
    }

    let fixes_ran = report
        .executed_action_ids
        .iter()
        .any(|id| id != "backup-runtime-configs");
    if !fixes_ran && report.skipped_actions.is_empty() {
        println!("\nNo rule fixes were required (config backup completed).");
    }
    if !report.skipped_actions.is_empty() {
        println!("\nSkipped actions:");
        for item in &report.skipped_actions {
            println!("  - {}: {}", item.id, item.reason);
        }
    }

    if let Some(guide) = &report.guide_path {
        println!("\nAPI key guide: {guide}");
        println!("  Open this file for setup steps. Secrets are not auto-filled.");
    }

    println!("\nAudit: {}", report.audit.id);
    println!("  verification: {}", report.audit.verification_summary);
    println!(
        "  rollback: agent-doctor repair {runtime} --rollback --backup {}",
        report.backup.id
    );
    Ok(())
}

fn run_rollback(runtime: &str, backup: Option<&str>, json: bool) -> Result<()> {
    let report = restore_runtime_backup(runtime, backup)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Agent Doctor — restore from backup\n");
    println!("Runtime: {}", report.runtime_id);
    println!("Backup: {} ({})", report.backup_id, report.backup_root);
    println!("\nRestored files:");
    for path in &report.restored_files {
        println!("  - {path}");
    }
    Ok(())
}

fn status_label(status: ProbeStatus) -> &'static str {
    match status {
        ProbeStatus::Pass => "pass",
        ProbeStatus::Warn => "warn",
        ProbeStatus::Fail => "fail",
        ProbeStatus::NotApplicable => "n/a",
        ProbeStatus::NotChecked => "not checked",
    }
}
