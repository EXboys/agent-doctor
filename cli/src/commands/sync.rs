use agent_doctor_core::{execute_skills_sync, SkillsSourceKind, SkillsSyncOptions, SyncReport};
use anyhow::Result;

pub fn run(
    dry_run: bool,
    only: &[String],
    runtime: Option<&str>,
    bundle: Option<&str>,
    source: Option<&str>,
    json: bool,
) -> Result<()> {
    let source_override = source
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .and_then(SkillsSourceKind::parse);

    let report = execute_skills_sync(&SkillsSyncOptions {
        dry_run,
        only_skills: only.to_vec(),
        runtime_target: runtime.map(str::to_string),
        pack_or_bundle_id: bundle.map(str::to_string),
        source_override,
    })?;

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

fn print_sync_report(report: &SyncReport, dry_run: bool) {
    println!(
        "Agent Doctor — skill sync{}\n",
        if dry_run { " (dry run)" } else { "" }
    );
    println!("Source: {}", report.base_url);
    println!("Bundle/pack: {} ({})", report.bundle_id, report.runtime_target);
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
