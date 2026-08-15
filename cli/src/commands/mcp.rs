use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use agent_doctor_mcp::{
    browser_mcp_status, cdp_user_data_dir, configure_for, connect_chrome, discover_chrome,
    isolated_chrome_user_data_dir, kill_chrome_on_port, launch_chrome, parse_browser_family,
    resolve_profile_directory, resolve_user_data_dir, smoke_browser_navigate,
    system_chrome_user_data_dir, BrowserDiscovery, LazyBrowser, McpConfigureOptions, SmokeOptions,
};
use anyhow::{bail, Context, Result};

pub fn run_browser(
    port: u16,
    headless: bool,
    user_data_dir: Option<PathBuf>,
    profile_directory: Option<String>,
    _json: bool,
) -> Result<()> {
    // Respond to MCP initialize/tools/list immediately; Chrome starts on first tool use.
    let discovery = discover_chrome().ok();
    let resolved_dir = user_data_dir.clone().or_else(|| {
        discovery
            .as_ref()
            .map(|d| resolve_user_data_dir(None, Some(&d.binary_path)))
    });
    let resolved_profile = resolve_profile_directory(profile_directory.as_deref());
    eprintln!(
        "Browser MCP server listening (Chrome launches on first tool call, port {port}, profile: {} / {})",
        resolved_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(default)".into()),
        resolved_profile
    );
    let browser = Mutex::new(LazyBrowser::with_options(
        port,
        headless,
        user_data_dir,
        profile_directory,
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
        println!("Profile directory: {}", status.profile_directory);
        println!(
            "Isolated (no daily impact): {}",
            status.isolated_user_data_dir
        );
        println!("Everyday Chrome: {}", status.system_user_data_dir);
    } else {
        println!("Chrome: not found");
    }
    if status.cdp_connected {
        println!(
            "CDP: connected on port {port} (ws: {})",
            status.ws_endpoint.unwrap_or_default()
        );
        if let Some(dir) = cdp_user_data_dir(port) {
            println!("CDP profile: {}", dir.display());
        }
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
    profile_directory: Option<String>,
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
    let profile_directory = Some(resolve_profile_directory(profile_directory.as_deref()));

    let options = McpConfigureOptions {
        runtime: runtime.to_string(),
        port,
        headless,
        user_data_dir: user_data_dir.clone(),
        profile_directory: profile_directory.clone(),
        binary: binary.clone(),
        project_path: None,
        codex_home: std::env::var("CODEX_HOME")
            .ok()
            .map(std::path::PathBuf::from),
        hermes_home: std::env::var("HERMES_HOME")
            .ok()
            .map(std::path::PathBuf::from),
        openclaw_workspace: std::env::var("OPENCLAW_WORKSPACE")
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
                "profile_directory": profile_directory,
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
        println!(
            "  Profile: {} / {}",
            dir.display(),
            profile_directory.as_deref().unwrap_or("Default")
        );
    }
    println!("  Restart {runtime} for changes to take effect.");
    Ok(())
}

/// Launch/reuse isolated chrome-cdp — does not touch everyday Chrome.
pub fn run_chrome_ensure(port: u16, headless: bool, json: bool) -> Result<()> {
    if let Ok(existing) = connect_chrome(port) {
        let dir = cdp_user_data_dir(port).unwrap_or_else(isolated_chrome_user_data_dir);
        let isolated = isolated_chrome_user_data_dir();
        if dir != isolated {
            kill_chrome_on_port(port)?;
        } else {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": "ready",
                        "mode": "isolated",
                        "port": port,
                        "user_data_dir": dir,
                        "ws_endpoint": existing.ws_endpoint,
                        "reused": true,
                    }))?
                );
            } else {
                println!("✓ CDP already ready on :{port}");
                println!("  Mode: isolated (everyday Chrome untouched)");
                println!("  Profile: {}", dir.display());
            }
            return Ok(());
        }
    }

    let discovery = discover_chrome()?;
    let discovery = BrowserDiscovery {
        user_data_dir: isolated_chrome_user_data_dir(),
        profile_directory: "Default".into(),
        ..discovery
    };
    let instance = launch_chrome(&discovery, port, headless)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "ready",
                "mode": "isolated",
                "port": port,
                "user_data_dir": discovery.user_data_dir,
                "ws_endpoint": instance.ws_endpoint,
                "reused": false,
            }))?
        );
    } else {
        println!("✓ Isolated Chrome ready on :{port}");
        println!("  Everyday Chrome was not touched");
        println!("  Profile: {}", discovery.user_data_dir.display());
        println!("  Tip: log into sites once in this window if you need cookies.");
    }
    // Keep Chrome alive — detach by forgetting the Child without kill.
    std::mem::forget(instance);
    Ok(())
}

/// Quit everyday Chrome and relaunch it with CDP so MCP can attach (shared logins).
pub fn run_chrome_attach_daily(port: u16, profile_directory: &str, json: bool) -> Result<()> {
    let discovery = discover_chrome()?;
    let user_data_dir = system_chrome_user_data_dir(Some(&discovery.binary_path));
    let profile_directory = resolve_profile_directory(Some(profile_directory));

    let _ = kill_chrome_on_port(port);
    quit_everyday_chrome(&discovery)?;
    thread::sleep(Duration::from_millis(800));

    let discovery = BrowserDiscovery {
        user_data_dir: user_data_dir.clone(),
        profile_directory: profile_directory.clone(),
        ..discovery
    };
    let instance = launch_chrome(&discovery, port, false)?;
    for _ in 0..30 {
        if connect_chrome(port).is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    if connect_chrome(port).is_err() {
        bail!("Chrome started but CDP on :{port} is not ready yet");
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "ready",
                "mode": "attach-daily",
                "port": port,
                "user_data_dir": user_data_dir,
                "profile_directory": profile_directory,
                "ws_endpoint": instance.ws_endpoint,
            }))?
        );
    } else {
        println!("✓ Everyday Chrome relaunched with debugging on :{port}");
        println!(
            "  Profile: {} / {}",
            user_data_dir.display(),
            profile_directory
        );
        println!("  Your logins stay. MCP should only connect — not open a second Chrome.");
        println!("  Tip: write MCP config with this --user-data-dir, then restart Claude.");
    }
    std::mem::forget(instance);
    Ok(())
}

/// Headless navigate smoke for CI / release verification (Chrome or Edge).
pub fn run_smoke(
    browser: &str,
    url: &str,
    port: Option<u16>,
    headed: bool,
    json: bool,
) -> Result<()> {
    let family = parse_browser_family(browser)?;
    let options = SmokeOptions {
        family,
        url: url.to_string(),
        headless: !headed,
        port,
    };
    let report = smoke_browser_navigate(&options)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report.to_json_value())?);
    } else {
        println!("✓ Browser CDP smoke OK ({browser})");
        println!("  Binary: {}", report.binary);
        if let Some(version) = &report.version {
            println!("  Version: {version}");
        }
        println!("  Port: {}", report.port);
        println!(
            "  URL: {}",
            report.final_url.as_deref().unwrap_or(&report.url)
        );
        if let Some(title) = &report.title {
            println!("  Title: {title}");
        }
    }
    Ok(())
}

fn quit_everyday_chrome(discovery: &BrowserDiscovery) -> Result<()> {
    if cfg!(target_os = "macos") {
        let _ = Command::new("osascript")
            .args(["-e", "tell application \"Google Chrome\" to quit"])
            .status();
        thread::sleep(Duration::from_millis(500));
    }
    let output = Command::new("pgrep")
        .args(["-f", &discovery.binary_path.display().to_string()])
        .output()
        .context("pgrep Chrome")?;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Ok(pid) = line.trim().parse::<i32>() {
            let _ = Command::new("kill").args([pid.to_string()]).status();
        }
    }
    Ok(())
}
