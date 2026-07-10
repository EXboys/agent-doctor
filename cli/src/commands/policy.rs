use agent_doctor_core::{execute_policy_pull, load_evotown_config};
use anyhow::Result;

pub fn pull(json: bool) -> Result<()> {
    let config = load_evotown_config()?;
    let report = execute_policy_pull(&config)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Agent Doctor — Evotown policy pull\n");
    println!("Evotown: {}", report.base_url);
    println!(
        "Cached {} policies to {}",
        report.policy_count, report.cache_path
    );
    println!("Fetched at: {}", report.fetched_at);
    Ok(())
}
