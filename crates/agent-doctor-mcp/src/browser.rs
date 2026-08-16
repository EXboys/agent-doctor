use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};

/// Information about a discovered Chrome installation.
#[derive(Debug, Clone)]
pub struct BrowserDiscovery {
    pub binary_path: PathBuf,
    pub user_data_dir: PathBuf,
    /// Chrome profile directory name inside user-data-dir (`Default`, `Profile 2`, …).
    pub profile_directory: String,
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

/// Which Chromium-family browser to prefer when discovering a binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowserFamily {
    /// Prefer Chrome, then Edge, then Chromium/Brave.
    #[default]
    Auto,
    Chrome,
    Edge,
    Chromium,
}

impl BrowserFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Chrome => "chrome",
            Self::Edge => "edge",
            Self::Chromium => "chromium",
        }
    }
}

/// Discover Chrome on the local machine (macOS first, Linux fallback).
pub fn discover_chrome() -> Result<BrowserDiscovery> {
    discover_browser(BrowserFamily::Auto)
}

/// Discover a Chromium-family browser, optionally pinning Chrome vs Edge.
pub fn discover_browser(family: BrowserFamily) -> Result<BrowserDiscovery> {
    if let Ok(custom) = std::env::var("AGENT_DOCTOR_BROWSER_BINARY") {
        let path = PathBuf::from(custom.trim());
        if path.as_os_str().is_empty() {
            // fall through
        } else if path.exists() {
            return discovery_from_binary(path);
        } else {
            anyhow::bail!(
                "AGENT_DOCTOR_BROWSER_BINARY points to missing binary: {}",
                path.display()
            );
        }
    }

    let binary_path = find_chrome_binary(family).with_context(|| {
        format!(
            "{} not found on this system",
            match family {
                BrowserFamily::Edge => "Microsoft Edge",
                BrowserFamily::Chromium => "Chromium",
                BrowserFamily::Chrome => "Google Chrome",
                BrowserFamily::Auto => "Chrome/Edge/Chromium",
            }
        )
    })?;
    discovery_from_binary(binary_path)
}

fn discovery_from_binary(binary_path: PathBuf) -> Result<BrowserDiscovery> {
    let user_data_dir = resolve_user_data_dir(None, Some(&binary_path));
    let profile_directory = resolve_profile_directory(None);
    let version = detect_chrome_version(&binary_path);

    Ok(BrowserDiscovery {
        binary_path,
        user_data_dir,
        profile_directory,
        version,
    })
}

fn find_chrome_binary(family: BrowserFamily) -> Result<PathBuf> {
    let candidates = browser_binary_candidates(family);

    // Check absolute paths first
    for candidate in &candidates {
        let p = PathBuf::from(candidate);
        if p.is_absolute() && p.exists() {
            return Ok(p);
        }
    }

    // Check PATH for non-absolute candidates
    if let Ok(path) = std::env::var("PATH") {
        for candidate in &candidates {
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

    anyhow::bail!(
        "Could not find {} binary. Install Google Chrome, Microsoft Edge, or Chromium.",
        family.as_str()
    )
}

fn browser_binary_candidates(family: BrowserFamily) -> Vec<&'static str> {
    if cfg!(target_os = "macos") {
        match family {
            BrowserFamily::Chrome => vec![
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Chrome.app/Contents/MacOS/Chrome",
            ],
            BrowserFamily::Edge => {
                vec!["/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"]
            }
            BrowserFamily::Chromium => vec![
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
                "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            ],
            BrowserFamily::Auto => vec![
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Chrome.app/Contents/MacOS/Chrome",
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
                "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
            ],
        }
    } else if cfg!(target_os = "linux") {
        match family {
            BrowserFamily::Chrome => vec!["google-chrome", "google-chrome-stable", "chrome"],
            BrowserFamily::Edge => vec!["microsoft-edge", "microsoft-edge-stable", "msedge"],
            BrowserFamily::Chromium => vec!["chromium", "chromium-browser"],
            BrowserFamily::Auto => vec![
                "google-chrome",
                "google-chrome-stable",
                "microsoft-edge",
                "microsoft-edge-stable",
                "msedge",
                "chromium",
                "chromium-browser",
                "chrome",
            ],
        }
    } else {
        match family {
            BrowserFamily::Chrome => vec!["chrome", "google-chrome"],
            BrowserFamily::Edge => vec!["msedge", "microsoft-edge"],
            BrowserFamily::Chromium => vec!["chromium"],
            BrowserFamily::Auto => vec!["chrome", "msedge", "chromium"],
        }
    }
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

/// Resolve profile dir: explicit arg → env → isolated automation profile.
///
/// Default is the Agent Doctor chrome-cdp dir so everyday Chrome is never
/// locked or duplicated. Pass an explicit path (or use desktop "日常 Chrome")
/// when you intentionally want the shared login profile.
pub fn resolve_user_data_dir(explicit: Option<&PathBuf>, binary: Option<&PathBuf>) -> PathBuf {
    let _ = binary;
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
    isolated_chrome_user_data_dir()
}

/// Resolve profile directory: explicit → env → `Default`.
///
/// Without this, multi-profile Chrome shows the account picker on launch.
pub fn resolve_profile_directory(explicit: Option<&str>) -> String {
    if let Some(name) = explicit {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(from_env) = std::env::var("AGENT_DOCTOR_CHROME_PROFILE_DIRECTORY") {
        let trimmed = from_env.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    "Default".to_string()
}

/// Default profile used when launching Chrome for MCP (everyday Chrome).
fn default_user_data_dir() -> PathBuf {
    resolve_user_data_dir(None, find_chrome_binary(BrowserFamily::Auto).ok().as_ref())
}

fn detect_chrome_version(binary: &Path) -> Option<String> {
    // Never spawn Google Chrome / Chromium just to read --version.
    // Executing the app binary on macOS often handoffs to a running instance
    // and focuses a browser window (Resources tab / status checks).
    chrome_version_from_app_bundle(binary).or_else(|| chrome_version_via_plutil(binary))
}

fn chrome_version_via_plutil(binary: &Path) -> Option<String> {
    let mut dir = binary.parent()?.to_path_buf();
    for _ in 0..4 {
        if dir.extension().and_then(|e| e.to_str()) == Some("app") {
            let plist = dir.join("Contents/Info.plist");
            if !plist.exists() {
                return None;
            }
            let output = Command::new("plutil")
                .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
                .arg(&plist)
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if version.is_empty() {
                return None;
            }
            return Some(format!("Google Chrome {version}"));
        }
        dir = dir.parent()?.to_path_buf();
    }
    None
}

fn chrome_version_from_app_bundle(binary: &Path) -> Option<String> {
    let mut dir = binary.parent()?.to_path_buf();
    // .../Foo.app/Contents/MacOS/Google Chrome → climb to Foo.app
    for _ in 0..4 {
        if dir.extension().and_then(|e| e.to_str()) == Some("app") {
            let plist = dir.join("Contents/Info.plist");
            return read_bundle_short_version(&plist);
        }
        dir = dir.parent()?.to_path_buf();
    }
    None
}

fn read_bundle_short_version(plist_path: &std::path::Path) -> Option<String> {
    let raw = std::fs::read_to_string(plist_path).ok()?;
    // Keep this dependency-free: Info.plist for Chrome is XML with the key nearby.
    let key = "<key>CFBundleShortVersionString</key>";
    let idx = raw.find(key)?;
    let after = &raw[idx + key.len()..];
    let start = after.find("<string>")? + "<string>".len();
    let end = after[start..].find("</string>")?;
    let version = after[start..start + end].trim();
    if version.is_empty() {
        None
    } else {
        Some(format!("Google Chrome {version}"))
    }
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
                if let Some(ws) = target.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                    return Ok(ws.to_string());
                }
            }
        }
    }

    create_blank_page_ws_endpoint(port)
}

/// Create a blank page target and return its page WebSocket URL.
///
/// Compatibility notes (C-end / multi-Chrome):
/// - Chromium **111+** (Chrome ~111+, Edge matching) requires `PUT /json/new`.
/// - Older Chromium accepted `GET /json/new`.
/// - We try PUT first, then GET, so one code path covers common Stable/Beta/Edge builds
///   without hard-coding major versions.
fn create_blank_page_ws_endpoint(port: u16) -> Result<String> {
    let mut errors = Vec::new();
    for method in ["PUT", "GET"] {
        match chrome_http_json_method(port, method, "/json/new?about:blank") {
            Ok(created) => {
                if let Some(ws) = created
                    .get("webSocketDebuggerUrl")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                {
                    return Ok(ws);
                }
                errors.push(format!(
                    "{method} /json/new returned JSON without webSocketDebuggerUrl"
                ));
            }
            Err(err) => errors.push(format!("{method} /json/new: {err:#}")),
        }
    }

    let version_hint = chrome_devtools_version_summary(port).unwrap_or_else(|| "unknown".into());
    anyhow::bail!(
        "Failed to create a Chrome page target via /json/new (Chrome DevTools {version_hint}). \
         Tried PUT then GET. Details: {}",
        errors.join(" | ")
    )
}

/// Best-effort Chrome DevTools version string from `/json/version` for diagnostics.
pub(crate) fn chrome_devtools_version_summary(port: u16) -> Option<String> {
    let value = chrome_http_json(port, "/json/version").ok()?;
    let browser = value
        .get("Browser")
        .and_then(|v| v.as_str())
        .unwrap_or("Chrome");
    let proto = value
        .get("Protocol-Version")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    Some(format!("{browser}, CDP {proto}"))
}

/// GET a Chrome DevTools HTTP endpoint and parse the JSON body.
pub(crate) fn chrome_http_json(port: u16, path: &str) -> Result<serde_json::Value> {
    chrome_http_json_method(port, "GET", path)
}

/// Call a Chrome DevTools HTTP endpoint with an explicit method and parse JSON.
///
/// Chrome's CDP HTTP server may keep the socket open; reads until
/// Content-Length bytes are received instead of waiting for EOF.
pub(crate) fn chrome_http_json_method(
    port: u16,
    method: &str,
    path: &str,
) -> Result<serde_json::Value> {
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
    let method = method.trim().to_ascii_uppercase();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nAccept: */*\r\nContent-Length: 0\r\n\r\n"
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
        anyhow::bail!(
            "Chrome CDP HTTP {method} {path} failed: {}",
            &response_str[..response_str.len().min(200)]
        );
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
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
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

fn process_parent_pid(pid: u32) -> Option<u32> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "ppid="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

fn automation_markers_from_command(cmd: &str, ancestor: bool) -> Vec<String> {
    let lower = cmd.to_ascii_lowercase();
    let mut markers = Vec::new();
    if lower.contains("--enable-automation") {
        markers.push("--enable-automation".to_string());
    }
    if lower.contains("--test-type=webdriver") {
        markers.push("--test-type=webdriver".to_string());
    }
    if lower.contains("chromedriver") {
        markers.push(if ancestor {
            "ChromeDriver ancestor process".to_string()
        } else {
            "ChromeDriver listener process".to_string()
        });
    }
    markers
}

/// Return strong signs that the browser listening on `port` belongs to
/// ChromeDriver rather than a Chrome process launched directly for CDP.
///
/// This is intentionally conservative: generic headless/CDP flags are not
/// considered suspicious because Agent Doctor uses them itself.
pub fn cdp_automation_markers(port: u16) -> Vec<String> {
    let mut markers = Vec::new();
    for pid in pids_listening_on_port(port) {
        if let Some(cmd) = process_command_line(pid) {
            markers.extend(automation_markers_from_command(&cmd, false));
        }

        // ChromeDriver normally remains an ancestor of the browser process.
        // Walk a few levels to account for shell/wrapper processes.
        let mut current = pid;
        for _ in 0..4 {
            let Some(parent) = process_parent_pid(current).filter(|parent| *parent > 1) else {
                break;
            };
            if let Some(cmd) = process_command_line(parent) {
                markers.extend(automation_markers_from_command(&cmd, true));
            }
            current = parent;
        }
    }
    markers.sort();
    markers.dedup();
    markers
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

/// Parse `--user-data-dir=` from the Chrome process listening on `port`.
pub fn cdp_user_data_dir(port: u16) -> Option<PathBuf> {
    for pid in pids_listening_on_port(port) {
        let Some(cmd) = process_command_line(pid) else {
            continue;
        };
        if let Some(dir) = parse_user_data_dir_flag(&cmd) {
            return Some(dir);
        }
    }
    None
}

fn parse_user_data_dir_flag(cmd: &str) -> Option<PathBuf> {
    // Handles `--user-data-dir=/path` and `--user-data-dir /path`.
    let bytes = cmd.as_bytes();
    let key = b"--user-data-dir";
    let mut i = 0;
    while i + key.len() <= bytes.len() {
        if &bytes[i..i + key.len()] == key {
            let rest = &cmd[i + key.len()..];
            let path = if let Some(stripped) = rest.strip_prefix('=') {
                stripped.split_whitespace().next().unwrap_or("")
            } else {
                rest.split_whitespace().next().unwrap_or("")
            };
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
        i += 1;
    }
    None
}

/// True if a non-CDP Chrome already holds `user_data_dir` (profile lock).
pub fn profile_locked_by_other_chrome(user_data_dir: &PathBuf, cdp_port: u16) -> bool {
    let want = user_data_dir.to_string_lossy();
    let cdp_pids: std::collections::HashSet<u32> =
        pids_listening_on_port(cdp_port).into_iter().collect();
    let output = Command::new("ps")
        .args(["-ax", "-ww", "-o", "pid=,command="])
        .output();
    let Ok(output) = output else {
        return false;
    };
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((pid_str, cmd)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid_str.trim().parse::<u32>() else {
            continue;
        };
        if cdp_pids.contains(&pid) {
            continue;
        }
        let lower = cmd.to_ascii_lowercase();
        if !(lower.contains("google chrome")
            || lower.contains("chromium")
            || lower.contains("/chrome"))
        {
            continue;
        }
        if cmd.contains("--type=") {
            continue; // helper/renderer processes
        }
        if let Some(dir) = parse_user_data_dir_flag(cmd) {
            if dir == *user_data_dir {
                return true;
            }
        } else if cmd.contains(want.as_ref()) {
            // Everyday Chrome often omits --user-data-dir (uses default).
            return true;
        } else if want.contains("Google/Chrome")
            && !cmd.contains("--user-data-dir")
            && (lower.contains("google chrome") || lower.contains("google chrome.app"))
        {
            // Default profile lock: main Chrome process without explicit dir.
            return true;
        }
    }
    false
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
        assert!(isolated.to_string_lossy().contains("chrome-cdp"));
        assert_eq!(resolve_user_data_dir(None, None), isolated);
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

    #[test]
    fn detects_strong_chromedriver_process_markers() {
        assert_eq!(
            automation_markers_from_command(
                "/tmp/chrome --remote-debugging-port=9222 --enable-automation",
                false
            ),
            vec!["--enable-automation"]
        );
        assert_eq!(
            automation_markers_from_command("/opt/bin/chromedriver --port=9515", true),
            vec!["ChromeDriver ancestor process"]
        );
    }

    #[test]
    fn accepts_direct_cdp_chrome_command() {
        assert!(automation_markers_from_command(
            "/Applications/Google Chrome --remote-debugging-port=9222 --headless=new",
            false
        )
        .is_empty());
    }
}
