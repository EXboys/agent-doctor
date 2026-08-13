use agent_doctor_core::{execute_register, RegisterOptions};
use anyhow::Result;

pub fn run(options: RegisterOptions, json: bool) -> Result<()> {
    let report = execute_register(&options)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Agent Doctor — Evotown engine register\n");
    println!("Evotown: {}", report.base_url);
    println!("Engine:  {} ({})", report.engine_id, report.engine_type);
    if report.rotated {
        println!("Rotate:  yes");
    }
    if let Some(path) = &report.saved_to {
        println!("Saved:   {path}");
    }
    if let Some(token) = &report.ingest_token {
        println!();
        println!("Per-engine ingest token (keep on this machine only):");
        println!("{token}");
        if report.saved_to.is_none() {
            println!();
            println!("Tip: re-run without --no-save-token to write EVOTOWN_ENGINE_INGEST_TOKEN.");
        }
    }
    println!();
    println!("{}", report.detail);
    if report.ingest_token_issued || report.saved_to.is_some() {
        println!("Next: agent-doctor connect");
    }
    Ok(())
}
