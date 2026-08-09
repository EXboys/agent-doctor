use std::sync::Mutex;

use agent_doctor_mcp::{
    browser_mcp_status, configure_for, discover_chrome, LazyBrowser, McpConfigureOptions,
};
use anyhow::Result;

pub fn run_browser(port: u16, headless: bool, _json: bool) -> Result<()> {
    // Respond to MCP initialize/tools/list immediately; Chrome starts on first tool use.
    eprintln!("Browser MCP server listening (Chrome launches on first tool call, port {port})");
    let browser = Mutex::new(LazyBrowser::new(port, headless));
    agent_doctor_mcp::run_mcp_server(&browser)?;
    Ok(())
}

pub fn run_status(port: u16, json: bool) -> Result<()> {
    let status = browser_mcp_status(port);

    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }

    println!("Agent Doctor — browser MCP status\n");
    if status.chrome_found {
        println!(
            "Chrome: {} (version: {})",
            status.binary.as_deref().unwrap_or("unknown"),
            status.version.as_deref().unwrap_or("unknown")
        );
        if let Some(dir) = &status.user_data_dir {
            println!("User data: {dir}");
        }
    } else {
        println!("Chrome: not found");
    }
    if status.cdp_connected {
        println!(
            "CDP: connected on port {port} (ws: {})",
            status.ws_endpoint.unwrap_or_default()
        );
    } else {
        println!("CDP: not connected on port {port}");
    }

    Ok(())
}

pub fn run_configure(runtime: &str, port: u16, headless: bool, json: bool) -> Result<()> {
    let discovery = discover_chrome()?;
    let binary = agent_doctor_core::resolve_agent_doctor_binary().unwrap_or_else(|_| {
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("agent-doctor-cli"))
    });

    let options = McpConfigureOptions {
        runtime: runtime.to_string(),
        port,
        headless,
        binary: binary.clone(),
        project_path: None,
        codex_home: std::env::var("CODEX_HOME")
            .ok()
            .map(std::path::PathBuf::from),
    };

    configure_for(&discovery, &options)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "configured",
                "runtime": runtime,
                "port": port,
                "headless": headless,
                "binary": binary.display().to_string(),
            }))?
        );
        return Ok(());
    }

    println!("✓ Browser MCP configured for {runtime}");
    println!("  Binary: {}", binary.display());
    println!("  Port: {port}");
    println!(
        "  UI: {}",
        if headless {
            "headless (no window)"
        } else {
            "visible window"
        }
    );
    println!("  Restart {runtime} for changes to take effect.");
    Ok(())
}
