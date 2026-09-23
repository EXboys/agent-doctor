use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};

use super::cdp::{chrome_devtools_version_summary, find_ws_endpoint_http};
use super::types::{BrowserDiscovery, ChromeInstance};

/// Launch Chrome for direct CDP control.
///
/// Keep the command line close to a normal Chrome launch:
/// - Does NOT pass --enable-automation
/// - Does NOT alter Blink feature flags (Chrome warns about unsupported flags)
/// - Uses a fixed CDP port instead of --remote-debugging-port=0
///
/// Uses `discovery.user_data_dir` (defaults to the everyday Chrome profile).
/// That profile cannot be opened while a normal Chrome instance already holds
/// the lock — quit Chrome first, or connect to an existing CDP port.
pub fn launch_chrome(
    discovery: &BrowserDiscovery,
    port: u16,
    headless: bool,
) -> Result<ChromeInstance> {
    std::fs::create_dir_all(&discovery.user_data_dir).with_context(|| {
        format!(
            "Failed to create Chrome user data dir {}",
            discovery.user_data_dir.display()
        )
    })?;

    let mut cmd = Command::new(&discovery.binary_path);

    cmd.arg(format!("--remote-debugging-port={}", port))
        // Required by Chromium/Edge 111+: without this, CDP HTTP/WS from non-Chrome
        // clients often never yields a usable page endpoint (CI Edge smoke fails here).
        .arg("--remote-allow-origins=*")
        .arg(format!(
            "--user-data-dir={}",
            discovery.user_data_dir.display()
        ))
        // Skip the multi-account picker when Default / Profile N exist.
        .arg(format!(
            "--profile-directory={}",
            discovery.profile_directory
        ))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--window-size=1280,800");

    if headless {
        cmd.arg("--headless=new");
        // Edge/Chromium on GitHub-hosted Linux runners can stall creating the first
        // page target without these (GPU / dbus probes).
        cmd.arg("--disable-gpu");
        cmd.arg("--disable-software-rasterizer");
    }

    // GitHub Actions / containers often lack a usable sandbox user namespace.
    if std::env::var_os("CI").is_some()
        || std::env::var_os("AGENT_DOCTOR_CHROME_NO_SANDBOX").is_some()
    {
        cmd.arg("--no-sandbox");
        cmd.arg("--disable-dev-shm-usage");
    }

    // Deliberately omit --enable-automation. CDP does not require it, and Chrome
    // would otherwise expose automation UI and navigator.webdriver.
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "Failed to start Chrome: {}",
            discovery.binary_path.display()
        )
    })?;

    // Wait for DevTools HTTP, then a page WebSocket (cold Edge profile can be slow).
    let mut ws_endpoint = None;
    let mut last_err = None;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(500));
        match find_ws_endpoint_http(port) {
            Ok(endpoint) => {
                ws_endpoint = Some(endpoint);
                break;
            }
            Err(err) => last_err = Some(err),
        }
        if let Some(status) = child.try_wait().ok().flatten() {
            anyhow::bail!(
                "Chrome exited early ({status}) while starting CDP on port {port}. \
                 If you need your everyday profile, quit Chrome and relaunch with \
                 --remote-debugging-port={port}, or set AGENT_DOCTOR_CHROME_USER_DATA_DIR."
            );
        }
    }

    let Some(ws_endpoint) = ws_endpoint else {
        let version = chrome_devtools_version_summary(port).unwrap_or_else(|| "unreachable".into());
        let detail = last_err
            .map(|e| format!("{e:#}"))
            .unwrap_or_else(|| "no page target yet".into());
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!(
            "browser started but page WebSocket endpoint is missing on :{port} ({version}). {detail}"
        );
    };

    Ok(ChromeInstance {
        process: Some(child),
        debug_port: port,
        user_data_dir: discovery.user_data_dir.clone(),
        ws_endpoint: Some(ws_endpoint),
    })
}
