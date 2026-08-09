use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};

/// Information about a discovered Chrome installation.
#[derive(Debug, Clone)]
pub struct BrowserDiscovery {
    pub binary_path: PathBuf,
    pub user_data_dir: PathBuf,
    pub version: Option<String>,
}

/// A running Chrome instance controlled by agent-doctor.
#[derive(Debug)]
pub struct ChromeInstance {
    pub process: Option<Child>,
    pub debug_port: u16,
    pub user_data_dir: PathBuf,
    pub ws_endpoint: Option<String>,
}

/// Discover Chrome on the local machine (macOS first, Linux fallback).
pub fn discover_chrome() -> Result<BrowserDiscovery> {
    let binary_path = find_chrome_binary().context("Chrome not found on this system")?;
    let user_data_dir = resolve_user_data_dir(None, Some(&binary_path));
    let version = detect_chrome_version(&binary_path);

    Ok(BrowserDiscovery {
        binary_path,
        user_data_dir,
        version,
    })
}

fn find_chrome_binary() -> Result<PathBuf> {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chrome.app/Contents/MacOS/Chrome",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ]
    } else if cfg!(target_os = "linux") {
        &[
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "chrome",
        ]
    } else {
        &["chrome", "msedge"]
    };

    // Check absolute paths first
    for candidate in candidates {
        let p = PathBuf::from(candidate);
        if p.is_absolute() && p.exists() {
            return Ok(p);
        }
    }

    // Check PATH for non-absolute candidates
    if let Ok(path) = std::env::var("PATH") {
        for candidate in candidates {
            let p = PathBuf::from(candidate);
            if !p.is_absolute() {
                for dir in std::env::split_paths(&path) {
                    let full = dir.join(&p);
                    if full.exists() {
                        return Ok(full);
                    }
                }
            }
        }
    }

    anyhow::bail!("Could not find Chrome/Chromium binary. Install Google Chrome or chromium.")
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Everyday browser profile (Google Chrome / Brave / Edge / Chromium).
///
/// This is the user-data-dir parent (contains `Default`, `Profile 1`, …), not
/// `…/Default` itself.
pub fn system_chrome_user_data_dir(binary: Option<&PathBuf>) -> PathBuf {
    let home = home_dir();
    let kind = binary
        .map(|p| p.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    if cfg!(target_os = "macos") {
        if kind.contains("brave") {
            return home.join("Library/Application Support/BraveSoftware/Brave-Browser");
        }
        if kind.contains("edge") {
            return home.join("Library/Application Support/Microsoft Edge");
        }
        if kind.contains("chromium") {
            return home.join("Library/Application Support/Chromium");
        }
        home.join("Library/Application Support/Google/Chrome")
    } else if cfg!(target_os = "linux") {
        if kind.contains("brave") {
            return home.join(".config/BraveSoftware/Brave-Browser");
        }
        if kind.contains("edge") || kind.contains("msedge") {
            return home.join(".config/microsoft-edge");
        }
        if kind.contains("chromium") {
            return home.join(".config/chromium");
        }
        home.join(".config/google-chrome")
    } else {
        if kind.contains("brave") {
            return home.join(r"AppData\Local\BraveSoftware\Brave-Browser\User Data");
        }
        if kind.contains("edge") || kind.contains("msedge") {
            return home.join(r"AppData\Local\Microsoft\Edge\User Data");
        }
        if kind.contains("chromium") {
            return home.join(r"AppData\Local\Chromium\User Data");
        }
        home.join(r"AppData\Local\Google\Chrome\User Data")
    }
}

/// Isolated Agent Doctor profile (no shared cookies/login with everyday Chrome).
pub fn isolated_chrome_user_data_dir() -> PathBuf {
    let home = home_dir();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/agent-doctor/chrome-cdp")
    } else if cfg!(target_os = "linux") {
        home.join(".config/agent-doctor/chrome-cdp")
    } else {
        home.join(r"AppData\Local\agent-doctor\chrome-cdp")
    }
}

/// Resolve profile dir: explicit arg → env → everyday system Chrome profile.
pub fn resolve_user_data_dir(explicit: Option<&PathBuf>, binary: Option<&PathBuf>) -> PathBuf {
    if let Some(path) = explicit {
        if !path.as_os_str().is_empty() {
            return path.clone();
        }
    }
    if let Ok(custom) = std::env::var("AGENT_DOCTOR_CHROME_USER_DATA_DIR") {
        let path = PathBuf::from(custom);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    system_chrome_user_data_dir(binary)
}

/// Default profile used when launching Chrome for MCP (everyday Chrome).
fn default_user_data_dir() -> PathBuf {
    resolve_user_data_dir(None, find_chrome_binary().ok().as_ref())
}

fn detect_chrome_version(binary: &PathBuf) -> Option<String> {
    let output = Command::new(binary).arg("--version").output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Launch Chrome with anti-detection settings.
///
/// Anti-detection measures:
/// - Disables AutomationControlled Blink feature
/// - Does NOT pass --enable-automation flag
/// - Does NOT pass --remote-debugging-pipe (less conspicuous)
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
        .arg(format!(
            "--user-data-dir={}",
            discovery.user_data_dir.display()
        ))
        // Anti-detection: disable Blink automation flag
        .arg("--disable-blink-features=AutomationControlled")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--window-size=1280,800");

    if headless {
        cmd.arg("--headless=new");
    }

    // NOTE: Deliberately omit --enable-automation so navigator.webdriver is not set
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "Failed to start Chrome: {}",
            discovery.binary_path.display()
        )
    })?;

    // Wait for Chrome to start listening (cold profile can be slow).
    let mut ws_endpoint = None;
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(500));
        if let Ok(endpoint) = find_ws_endpoint_http(port) {
            ws_endpoint = Some(endpoint);
            break;
        }
        if let Some(status) = child.try_wait().ok().flatten() {
            anyhow::bail!(
                "Chrome exited early ({status}) while starting CDP on port {port}. \
                 If you need your everyday profile, quit Chrome and relaunch with \
                 --remote-debugging-port={port}, or set AGENT_DOCTOR_CHROME_USER_DATA_DIR."
            );
        }
    }

    Ok(ChromeInstance {
        process: Some(child),
        debug_port: port,
        user_data_dir: discovery.user_data_dir.clone(),
        ws_endpoint,
    })
}

/// Connect to an already-running Chrome instance on the given port.
pub fn connect_chrome(port: u16) -> Result<ChromeInstance> {
    let ws_endpoint = find_ws_endpoint_http(port)?;
    let user_data_dir = default_user_data_dir();

    Ok(ChromeInstance {
        process: None,
        debug_port: port,
        user_data_dir,
        ws_endpoint: Some(ws_endpoint),
    })
}

/// Fetch a *page* WebSocket debug URL (not the browser-level endpoint).
///
/// Page domains like `Page.enable` only work on page targets. Prefer an existing
/// page from `/json/list`, otherwise create one via `/json/new`.
fn find_ws_endpoint_http(port: u16) -> Result<String> {
    if let Ok(list) = chrome_http_json(port, "/json/list") {
        if let Some(arr) = list.as_array() {
            for target in arr {
                let is_page = target
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(|t| t == "page")
                    .unwrap_or(true);
                if !is_page {
                    continue;
                }
                if let Some(ws) = target
                    .get("webSocketDebuggerUrl")
                    .and_then(|v| v.as_str())
                {
                    return Ok(ws.to_string());
                }
            }
        }
    }

    // Create a blank page target and use its debugger URL.
    let created = chrome_http_json(port, "/json/new?about:blank")
        .context("Failed to create a Chrome page target via /json/new")?;
    created
        .get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .context("Chrome /json/new response missing webSocketDebuggerUrl")
}

/// GET a Chrome DevTools HTTP endpoint and parse the JSON body.
///
/// Chrome's CDP HTTP server may keep the socket open; reads until
/// Content-Length bytes are received instead of waiting for EOF.
pub(crate) fn chrome_http_json(port: u16, path: &str) -> Result<serde_json::Value> {
    let addr = format!("127.0.0.1:{port}");
    let mut stream = TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(800))
        .with_context(|| format!("Cannot connect to Chrome on {addr}"))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(800)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_millis(800)))
        .ok();

    // Chrome's CDP HTTP server expects HTTP/1.1 and may keep the socket open;
    // read until Content-Length bytes are received instead of waiting for EOF.
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nAccept: */*\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .context("Failed to send HTTP request to Chrome")?;
    stream.flush()?;

    let mut response = Vec::new();
    let mut buf = [0u8; 8192];
    let mut content_length: Option<usize> = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                if content_length.is_some() {
                    break;
                }
                continue;
            }
            Err(err) => return Err(err).context("Failed to read response from Chrome"),
        }

        if let Some(headers) = split_headers(&response) {
            if content_length.is_none() {
                content_length = parse_content_length(headers);
            }
            let body_start = headers.len() + 4; // \r\n\r\n
            if let Some(len) = content_length {
                if response.len().saturating_sub(body_start) >= len {
                    break;
                }
            }
        }
    }

    let response_str = String::from_utf8_lossy(&response);
    if !response_str.contains("200") {
        anyhow::bail!("Chrome CDP HTTP {path} failed: {}", &response_str[..response_str.len().min(200)]);
    }
    let raw_body = response_str
        .split("\r\n\r\n")
        .nth(1)
        .context("No HTTP body in Chrome CDP response")?;
    let body = match content_length {
        Some(len) => {
            let bytes = raw_body.as_bytes();
            std::str::from_utf8(&bytes[..len.min(bytes.len())]).unwrap_or(raw_body)
        }
        None => raw_body.trim(),
    };

    serde_json::from_str(body.trim()).context("Failed to parse Chrome CDP JSON response")
}

fn split_headers(response: &[u8]) -> Option<&[u8]> {
    response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|idx| &response[..idx])
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(headers);
    for line in text.split("\r\n") {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

/// Stop the Chrome instance gracefully.
pub fn stop_chrome(instance: &mut ChromeInstance) -> Result<()> {
    if let Some(mut child) = instance.process.take() {
        child.kill().context("Failed to kill Chrome process")?;
        child.wait().context("Failed to wait for Chrome to exit")?;
    }
    Ok(())
}

/// PIDs currently listening on a TCP port (best-effort via `lsof`).
fn pids_listening_on_port(port: u16) -> Vec<u32> {
    let output = Command::new("lsof")
        .args([
            "-nP",
            &format!("-iTCP:{port}"),
            "-sTCP:LISTEN",
            "-t",
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

fn process_command_line(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-ww", "-o", "command="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let cmd = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if cmd.is_empty() {
        None
    } else {
        Some(cmd)
    }
}

/// Whether the Chrome (or Chromium) listening on `port` was started with `--headless`.
/// Returns `None` if nothing is listening or the mode cannot be determined.
pub fn cdp_port_is_headless(port: u16) -> Option<bool> {
    let mut saw_browser = false;
    for pid in pids_listening_on_port(port) {
        let Some(cmd) = process_command_line(pid) else {
            continue;
        };
        let lower = cmd.to_ascii_lowercase();
        let is_browser = lower.contains("chrome")
            || lower.contains("chromium")
            || lower.contains("msedge")
            || lower.contains("brave");
        if !is_browser {
            continue;
        }
        saw_browser = true;
        if cmd.contains("--headless") {
            return Some(true);
        }
    }
    if saw_browser {
        Some(false)
    } else {
        None
    }
}

/// Kill processes listening on the CDP port so a fresh Chrome can bind it.
pub fn kill_chrome_on_port(port: u16) -> Result<()> {
    let pids = pids_listening_on_port(port);
    if pids.is_empty() {
        return Ok(());
    }
    for pid in &pids {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(200));
        if pids_listening_on_port(port).is_empty() {
            return Ok(());
        }
    }
    // Last resort
    for pid in pids_listening_on_port(port) {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
    std::thread::sleep(Duration::from_millis(300));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_user_data_dir_is_valid_path() {
        let dir = default_user_data_dir();
        let s = dir.to_string_lossy();
        assert!(
            s.contains("Chrome")
                || s.contains("google-chrome")
                || s.contains("Chromium")
                || s.contains("Brave")
                || s.contains("Edge")
                || s.contains("chrome-cdp"),
            "unexpected user-data-dir: {s}"
        );
    }

    #[test]
    fn test_system_profile_is_not_isolated_by_default() {
        let system = system_chrome_user_data_dir(None);
        let isolated = isolated_chrome_user_data_dir();
        assert_ne!(system, isolated);
        assert!(!system.to_string_lossy().contains("chrome-cdp"));
    }

    #[test]
    fn test_discover_chrome_finds_binary_when_installed() {
        if let Ok(discovery) = discover_chrome() {
            assert!(discovery.binary_path.exists(), "Chrome binary should exist");
            println!(
                "Found Chrome: {:?} version={:?}",
                discovery.binary_path, discovery.version
            );
        }
    }
}
