use agent_doctor_core::{
    execute_install, needs_binary_install, probe_health_summary, probe_issue_score, ExplainReport,
    InstallOptions, InstallReport,
};
use agent_doctor_mcp::wiring_next_steps_for_runtime;
use anyhow::Result;

pub fn run(
    runtime: &str,
    explain: bool,
    plan_ai: bool,
    repair_after: bool,
    retry: u8,
    json: bool,
) -> Result<()> {
    let report = execute_install(
        runtime,
        &InstallOptions {
            explain,
            plan_ai_repair: plan_ai,
            repair_after,
            retry_count: retry,
        },
    )?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    print_install_report(&report);
    Ok(())
}

pub fn print_install_report(report: &InstallReport) {
    println!("Agent Doctor — install {}\n", report.runtime_id);

    if !report.install_needed {
        println!("Already installed — no install action required.");
    } else {
        let used_rule_install = report.install_attempts > 0;
        if !used_rule_install {
            println!("No rule installer — starting AI install.");
        }
        println!(
            "Rule install: {} ({} attempt(s))",
            if used_rule_install {
                if report.install_succeeded {
                    "succeeded"
                } else {
                    "failed"
                }
            } else {
                "skipped"
            },
            report.install_attempts
        );
        if report.repair_loop.is_some() {
            println!("AI repair loop ran for install / remaining issues.");
        }
    }

    println!(
        "Health: {} -> {} (score {} -> {})",
        probe_health_summary(&report.before_probe),
        probe_health_summary(&report.after_probe),
        probe_issue_score(&report.before_probe),
        probe_issue_score(&report.after_probe)
    );

    if let Some(path) = &report.install_log_path {
        println!("\nInstall log: {path}");
    }

    for item in &report.skipped_actions {
        println!("Skipped {}: {}", item.id, item.reason);
    }

    if !report.manual_fallback.is_empty() {
        println!("\nNext steps:");
        for step in &report.manual_fallback {
            println!("  - {step}");
        }
    }

    if let Some(explain) = &report.explain {
        println!();
        print_explain_report(&report.runtime_id, explain);
    }

    if let Some(loop_report) = &report.repair_loop {
        println!(
            "\nRepair loop: {} -> {}",
            probe_health_summary(&loop_report.before_probe),
            probe_health_summary(&loop_report.after_probe)
        );
        if !loop_report.executed_action_ids.is_empty() {
            println!("Executed: {}", loop_report.executed_action_ids.join(", "));
        }
    }

    if !needs_binary_install(&report.after_probe) {
        if let Some(steps) = wiring_next_steps_for_runtime(&report.runtime_id) {
            println!("\nWire runtime (install does not rewrite gateway/MCP configs):");
            for step in steps {
                println!("  - {step}");
            }
        }
    }
}

pub fn print_explain_report(runtime: &str, report: &ExplainReport) {
    println!("AI diagnosis — {runtime}\n");
    println!("{}\n", report.summary);
    if !report.likely_causes.is_empty() {
        println!("Likely causes:");
        for cause in &report.likely_causes {
            println!("  - {cause}");
        }
        println!();
    }
    if !report.recommended_action_ids.is_empty() {
        println!(
            "Recommended actions: {}",
            report.recommended_action_ids.join(", ")
        );
    }
    if !report.user_next_steps.is_empty() {
        println!("\nSuggested next steps:");
        for step in &report.user_next_steps {
            println!("  - {step}");
        }
    }
}
