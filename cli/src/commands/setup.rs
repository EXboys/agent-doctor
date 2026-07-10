use agent_doctor_core::{execute_setup, SetupOptions, SetupReport};
use anyhow::Result;

pub fn run(url: &str, key: &str, provider: Option<&str>, json: bool) -> Result<()> {
    let report = execute_setup(&SetupOptions {
        gateway_url: url.to_string(),
        api_key: key.to_string(),
        hermes_provider: provider.unwrap_or("openai").to_string(),
    })?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    print_setup_report(&report);
    Ok(())
}

fn print_setup_report(report: &SetupReport) {
    println!("Agent Doctor — company setup\n");
    println!("Gateway: {}", report.gateway_url);
    println!("Evotown base: {}", report.evotown_base_url);
    println!("Profile: {}\n", report.profile_env_path);
    if let Some(path) = &report.evotown_agent_env_path {
        println!("Evotown agent env: {path}");
    }
    println!("Applied to runtimes:");
    for runtime in &report.runtimes {
        let status = if runtime.applied { "ok" } else { "skip" };
        println!(
            "  - {} [{}] {}",
            runtime.display_name, status, runtime.message
        );
        if let Some(path) = &runtime.config_path {
            println!("    config: {path}");
        }
        if let Some(backup) = &runtime.backup_path {
            println!("    backup: {backup}");
        }
    }
    println!(
        "\nLoad credentials in your shell:\n  set -a && source \"{}\" && set +a",
        report.profile_env_path
    );
    println!("\nVerify: agent-doctor doctor");
    println!("Sync skills: agent-doctor sync");
    println!("Pull policies: agent-doctor policy pull");
}
