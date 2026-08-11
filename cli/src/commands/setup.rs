use agent_doctor_core::{execute_setup, SetupOptions, SetupReport};
use anyhow::Result;

use super::mode::{print_browser_mcp_report, wire_browser_mcp_after_setup};

pub fn run(
    url: &str,
    key: &str,
    provider: Option<&str>,
    with_browser_mcp: bool,
    json: bool,
) -> Result<()> {
    let report = execute_setup(&SetupOptions {
        gateway_url: url.to_string(),
        api_key: key.to_string(),
        hermes_provider: provider.unwrap_or("openai").to_string(),
    })?;

    let browser_mcp = if with_browser_mcp {
        Some(wire_browser_mcp_after_setup()?)
    } else {
        None
    };

    if json {
        let mut value = serde_json::to_value(&report)?;
        if let Some(mcp) = &browser_mcp {
            if let Some(obj) = value.as_object_mut() {
                obj.insert("browser_mcp".into(), serde_json::to_value(mcp)?);
            }
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    print_setup_report(&report);
    if let Some(mcp) = browser_mcp {
        print_browser_mcp_report(&mcp);
    } else {
        println!(
            "\nOptional: re-run with --with-browser-mcp to write Browser MCP into Codex/Claude."
        );
    }
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
