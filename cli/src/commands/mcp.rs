use std::sync::Mutex;

use agent_doctor_mcp::{
    connect_chrome, discover_chrome, launch_chrome, stop_chrome, BrowserContext, ChromeInstance,
};
use anyhow::Result;

pub fn run_browser(port: u16, headless: bool, json: bool) -> Result<()> {
    let discovery = discover_chrome()?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "discovered",
                "binary": discovery.binary_path.display().to_string(),
                "user_data_dir": discovery.user_data_dir.display().to_string(),
                "version": discovery.version,
            }))?
        );
    }

    let mut instance: ChromeInstance = if let Ok(existing) = connect_chrome(port) {
        if !json {
            eprintln!("Connected to existing Chrome on port {port}");
        }
        existing
    } else {
        if !json {
            eprintln!(
                "Starting Chrome: {} on port {port}",
                discovery.binary_path.display()
            );
        }
        launch_chrome(&discovery, port, headless)?
    };

    let ws_endpoint = instance
        .ws_endpoint
        .as_ref()
        .cloned()
        .or_else(|| {
            std::thread::sleep(std::time::Duration::from_secs(2));
            connect_chrome(port).ok().and_then(|c| c.ws_endpoint)
        })
        .expect("Failed to connect to Chrome CDP endpoint");

    let browser = Mutex::new(BrowserContext::connect(&ws_endpoint)?);

    if !json {
        eprintln!("Browser MCP server ready on port {port}");
        eprintln!("WebSocket endpoint: {ws_endpoint}");
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "running",
                "port": port,
                "ws_endpoint": ws_endpoint,
            }))?
        );
    }

    agent_doctor_mcp::run_mcp_server(&browser)?;

    stop_chrome(&mut instance)?;
    Ok(())
}

pub fn run_status(port: u16, json: bool) -> Result<()> {
    let discovery = discover_chrome().ok();
    let connected = connect_chrome(port).ok();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "chrome_found": discovery.is_some(),
                "binary": discovery.as_ref().map(|d| d.binary_path.display().to_string()),
                "version": discovery.as_ref().and_then(|d| d.version.as_ref()),
                "cdp_connected": connected.is_some(),
                "port": port,
            }))?
        );
        return Ok(());
    }

    println!("Agent Doctor — browser MCP status\n");
    match discovery {
        Some(d) => {
            println!(
                "Chrome: {} (version: {})",
                d.binary_path.display(),
                d.version.as_deref().unwrap_or("unknown")
            );
            println!("User data: {}", d.user_data_dir.display());
        }
        None => println!("Chrome: not found"),
    }
    match connected {
        Some(c) => println!(
            "CDP: connected on port {port} (ws: {})",
            c.ws_endpoint.unwrap_or_default()
        ),
        None => println!("CDP: not connected on port {port}"),
    }

    Ok(())
}

pub fn run_configure(runtime: &str, port: u16, json: bool) -> Result<()> {
    let discovery = discover_chrome()?;
    let binary = std::env::current_exe()?;

    let options = agent_doctor_mcp::McpConfigureOptions {
        runtime: runtime.to_string(),
        port,
        binary,
        project_path: None,
    };

    agent_doctor_mcp::configure_for(&discovery, &options)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "configured",
                "runtime": runtime,
                "port": port,
            }))?
        );
        return Ok(());
    }

    println!("✓ Browser MCP configured for {runtime}");
    println!("  Port: {port}");
    println!("  Restart {runtime} for changes to take effect.");
    Ok(())
}
