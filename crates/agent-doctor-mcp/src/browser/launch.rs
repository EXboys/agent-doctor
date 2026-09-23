use std::fs::File;
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};

use super::cdp::{chrome_devtools_version_summary, find_ws_endpoint_http};
use super::types::{BrowserDiscovery, ChromeInstance};

#[derive(Debug, Clone, Copy)]
enum HeadlessMode {
    New,
    Legacy,
    Off,
}

impl HeadlessMode {
    fn flag(self) -> Option<&'static str> {
        match self {
            Self::New => Some("--headless=new"),
            Self::Legacy => Some("--headless"),
            Self::Off => None,
        }
    }
}

fn binary_looks_like_edge(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name.contains("edge") || name.contains("msedge")
}

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
    let is_edge = binary_looks_like_edge(&discovery.binary_path);
    // Edge on GitHub Linux runners is flaky with `--headless=new` (CDP never binds).
    // Prefer legacy headless first for Edge; Chrome keeps the new mode.
    let modes = if !headless {
        vec![HeadlessMode::Off]
    } else if is_edge {
        vec![HeadlessMode::Legacy, HeadlessMode::New]
    } else {
        vec![HeadlessMode::New, HeadlessMode::Legacy]
    };

    let mut last_err = None;
    for (i, mode) in modes.into_iter().enumerate() {
        if i > 0 {
            // Let the previous attempt release the CDP port before retrying.
            std::thread::sleep(Duration::from_millis(500));
        }
        match launch_chrome_once(discovery, port, mode) {
            Ok(instance) => return Ok(instance),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("browser launch failed")))
}

fn launch_chrome_once(
    discovery: &BrowserDiscovery,
    port: u16,
    headless: HeadlessMode,
) -> Result<ChromeInstance> {
    std::fs::create_dir_all(&discovery.user_data_dir).with_context(|| {
        format!(
            "Failed to create Chrome user data dir {}",
            discovery.user_data_dir.display()
        )
    })?;

    let stderr_path = discovery
        .user_data_dir
        .join("agent-doctor-browser-stderr.log");
    let stderr_file = File::create(&stderr_path).with_context(|| {
        format!(
            "Failed to create browser stderr log {}",
            stderr_path.display()
        )
    })?;

    let mut cmd = Command::new(&discovery.binary_path);

    cmd.arg(format!("--remote-debugging-port={}", port))
        .arg("--remote-debugging-address=127.0.0.1")
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
        .arg("--disable-extensions")
        .arg("--disable-popup-blocking")
        .arg("--window-size=1280,800");

    if let Some(flag) = headless.flag() {
        cmd.arg(flag);
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

    // Force an initial target so `/json/list` is less likely to stay empty.
    cmd.arg("about:blank");

    // Deliberately omit --enable-automation. CDP does not require it, and Chrome
    // would otherwise expose automation UI and navigator.webdriver.
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::from(stderr_file));

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "Failed to start Chrome: {}",
            discovery.binary_path.display()
        )
    })?;

    // Wait for the TCP port, then a page WebSocket (cold Edge profile can be slow).
    let mut ws_endpoint = None;
    let mut last_err = None;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(400));
        if let Some(status) = child.try_wait().ok().flatten() {
            let stderr_tail = read_stderr_tail(&stderr_path);
            anyhow::bail!(
                "Chrome exited early ({status}) while starting CDP on port {port}. \
                 If you need your everyday profile, quit Chrome and relaunch with \
                 --remote-debugging-port={port}, or set AGENT_DOCTOR_CHROME_USER_DATA_DIR.\
                 {stderr_tail}"
            );
        }
        if !cdp_port_accepting(port) {
            continue;
        }
        match find_ws_endpoint_http(port) {
            Ok(endpoint) => {
                ws_endpoint = Some(endpoint);
                break;
            }
            Err(err) => last_err = Some(err),
        }
    }

    let Some(ws_endpoint) = ws_endpoint else {
        let version = chrome_devtools_version_summary(port).unwrap_or_else(|| "unreachable".into());
        let detail = last_err
            .map(|e| format!("{e:#}"))
            .unwrap_or_else(|| "no page target yet".into());
        let stderr_tail = read_stderr_tail(&stderr_path);
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!(
            "browser started but page WebSocket endpoint is missing on :{port} ({version}). {detail}\
             {stderr_tail}"
        );
    };

    Ok(ChromeInstance {
        process: Some(child),
        debug_port: port,
        user_data_dir: discovery.user_data_dir.clone(),
        ws_endpoint: Some(ws_endpoint),
    })
}

fn cdp_port_accepting(port: u16) -> bool {
    let addr = format!("127.0.0.1:{port}");
    TcpStream::connect_timeout(
        &addr.parse().expect("valid socket addr"),
        Duration::from_millis(200),
    )
    .is_ok()
}

fn read_stderr_tail(path: &Path) -> String {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let tail: String = trimmed
        .chars()
        .rev()
        .take(1200)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!(" Browser stderr (tail): {tail}")
}
