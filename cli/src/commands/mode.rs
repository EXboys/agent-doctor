use agent_doctor_core::{
    load_mode_status, switch_to_personal_mode, switch_to_team_mode, ModeStatus, ModeSwitchReport,
    MODE_PERSONAL, MODE_TEAM, MODE_UNSET,
};
use agent_doctor_mcp::{wire_browser_mcp_defaults, BrowserMcpWireReport};
use anyhow::Result;

pub fn show(json: bool) -> Result<()> {
    let status = load_mode_status()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }
    print_mode_status(&status);
    Ok(())
}

pub fn switch_personal(provider_id: Option<&str>, with_browser_mcp: bool, json: bool) -> Result<()> {
    let report = switch_to_personal_mode(provider_id)?;
    finish_switch(report, with_browser_mcp, json)
}

pub fn switch_team(with_browser_mcp: bool, json: bool) -> Result<()> {
    let report = switch_to_team_mode()?;
    finish_switch(report, with_browser_mcp, json)
}

fn finish_switch(report: ModeSwitchReport, with_browser_mcp: bool, json: bool) -> Result<()> {
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

    print_mode_switch_report(&report);
    if let Some(mcp) = browser_mcp {
        print_browser_mcp_report(&mcp);
    } else {
        println!("\nOptional: re-run with --with-browser-mcp to write Browser MCP into Codex/Claude.");
    }
    Ok(())
}

pub fn wire_browser_mcp_after_setup() -> Result<BrowserMcpWireReport> {
    let binary = agent_doctor_core::resolve_agent_doctor_binary().unwrap_or_else(|_| {
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("agent-doctor"))
    });
    wire_browser_mcp_defaults(&binary)
}

fn print_mode_status(status: &ModeStatus) {
    println!("Agent Doctor — LLM mode\n");
    println!("Mode: {}", status.mode);
    match status.mode.as_str() {
        MODE_PERSONAL => {
            println!(
                "Active: {} · {}",
                status.active_label.as_deref().unwrap_or("Personal"),
                status.active_gateway_url.as_deref().unwrap_or("—")
            );
        }
        MODE_TEAM => {
            println!(
                "Active: Evotown · {}",
                status.active_gateway_url.as_deref().unwrap_or("—")
            );
        }
        MODE_UNSET => println!("No active gateway overlay yet."),
        other => println!("Mode: {other}"),
    }
    println!(
        "Personal ready: {} · Team ready: {}",
        status.personal_ready, status.team_ready
    );
    println!("\nSwitch:");
    println!("  agent-doctor mode personal [--provider-id <id>] [--with-browser-mcp]");
    println!("  agent-doctor mode team [--with-browser-mcp]");
}

fn print_mode_switch_report(report: &ModeSwitchReport) {
    println!("Agent Doctor — mode switch\n");
    println!("{}", report.message);
    if let Some(url) = &report.active_gateway_url {
        println!(
            "Gateway: {} ({})",
            url,
            report.active_label.as_deref().unwrap_or("—")
        );
    }
    println!("\nRuntimes:");
    for runtime in &report.runtimes {
        let status = if runtime.applied { "ok" } else { "skip" };
        println!(
            "  - {} [{}] {}",
            runtime.display_name, status, runtime.message
        );
        if let Some(path) = &runtime.config_path {
            println!("    config: {path}");
        }
    }
    if !report.warnings.is_empty() {
        println!("\nWarnings:");
        for warning in &report.warnings {
            println!("  - {warning}");
        }
    }
    println!("\nVerify: agent-doctor doctor");
}

pub fn print_browser_mcp_report(report: &BrowserMcpWireReport) {
    println!("\nBrowser MCP:");
    for item in &report.results {
        let status = if item.ok { "ok" } else { "fail" };
        println!("  - {} [{}] {}", item.runtime, status, item.message);
        if let Some(path) = &item.config_path {
            println!("    config: {path}");
        }
    }
}
