use agent_doctor_core::{execute_sync, load_evotown_config, SyncOptions};
use anyhow::Result;

pub fn run(
    dry_run: bool,
    only: &[String],
    runtime: Option<&str>,
    bundle: Option<&str>,
    json: bool,
) -> Result<()> {
    let config = load_evotown_config()?;
    let report = execute_sync(
        &config,
        &SyncOptions {
            dry_run,
            only_skills: only.to_vec(),
            runtime_target: runtime.map(str::to_string),
            bundle_id: bundle.map(str::to_string),
        },
    )?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    print_sync_report(&report, dry_run);
    if report.failed > 0 {
        anyhow::bail!("sync completed with {} failure(s)", report.failed);
    }
    Ok(())
}

fn print_sync_report(report: &agent_doctor_core::SyncReport, dry_run: bool) {
    println!(
        "Agent Doctor — Evotown skill sync{}\n",
        if dry_run { " (dry run)" } else { "" }
    );
    println!("Evotown: {}", report.base_url);
    println!("Bundle: {} ({})", report.bundle_id, report.runtime_target);
    println!("Skills dir: {}", report.skills_dir);
    println!("Lock file: {}\n", report.lock_path);

    for outcome in &report.outcomes {
        let mark = match outcome.outcome.as_str() {
            "installed" => "↓",
            "skipped" => "·",
            _ => "!",
        };
        println!(
            "  {mark} {}@{} — {}{}",
            outcome.skill_id,
            outcome.version,
            outcome.outcome,
            outcome
                .detail
                .as_ref()
                .map(|detail| format!(" ({detail})"))
                .unwrap_or_default()
        );
    }

    println!(
        "\nSync done — installed/updated: {}, skipped: {}, failed: {}",
        report.installed, report.skipped, report.failed
    );
}
