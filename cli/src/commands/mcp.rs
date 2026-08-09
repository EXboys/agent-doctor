use std::path::PathBuf;
use std::sync::Mutex;

use agent_doctor_mcp::{
    browser_mcp_status, configure_for, discover_chrome, resolve_user_data_dir, LazyBrowser,
    McpConfigureOptions,
};
use anyhow::Result;

pub fn run_browser(
    port: u16,
    headless: bool,
    user_data_dir: Option<PathBuf>,
    _json: bool,
) -> Result<()> {
    // Respond to MCP initialize/tools/list immediately; Chrome starts on first tool use.
    let resolved = user_data_dir.clone().or_else(|| {
        discover_chrome()
            .ok()
            .map(|d| resolve_user_data_dir(None, Some(&d.binary_path)))
    });
    eprintln!(
        "Browser MCP server listening (Chrome launches on first tool call, port {port}, profile: {})",
        resolved
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(default)".into())
    );
    let browser = Mutex::new(LazyBrowser::with_user_data_dir(
        port,
        headless,
        user_data_dir,
    ));
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

pub fn run_configure(
    runtime: &str,
    port: u16,
    headless: bool,
    user_data_dir: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    let discovery = discover_chrome()?;
    let binary = agent_doctor_core::resolve_agent_doctor_binary().unwrap_or_else(|_| {
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("agent-doctor-cli"))
    });
    let user_data_dir = Some(resolve_user_data_dir(
        user_data_dir.as_ref(),
        Some(&discovery.binary_path),
    ));

    let options = McpConfigureOptions {
        runtime: runtime.to_string(),
        port,
        headless,
        user_data_dir: user_data_dir.clone(),
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
                "user_data_dir": user_data_dir,
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
    if let Some(dir) = &user_data_dir {
        println!("  Profile: {}", dir.display());
    }
    println!("  Restart {runtime} for changes to take effect.");
    Ok(())
}
