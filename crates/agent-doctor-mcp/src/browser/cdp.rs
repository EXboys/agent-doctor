use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};

use super::discover::default_user_data_dir;
use super::types::ChromeInstance;

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
pub(crate) fn find_ws_endpoint_http(port: u16) -> Result<String> {
    if let Ok(list) = chrome_http_json(port, "/json/list") {
        if let Some(arr) = list.as_array() {
            for target in arr {
                let ty = target
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("page");
                // Edge headless may expose `webview` / `tab` before a classic `page`.
                if !matches!(ty, "page" | "webview" | "tab") {
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
pub(crate) fn create_blank_page_ws_endpoint(port: u16) -> Result<String> {
    let mut errors = Vec::new();
    for path in ["/json/new?about:blank", "/json/new"] {
        for method in ["PUT", "GET"] {
            match chrome_http_json_method(port, method, path) {
                Ok(created) => {
                    if let Some(ws) = created
                        .get("webSocketDebuggerUrl")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                    {
                        return Ok(ws);
                    }
                    errors.push(format!(
                        "{method} {path} returned JSON without webSocketDebuggerUrl"
                    ));
                }
                Err(err) => errors.push(format!("{method} {path}: {err:#}")),
            }
        }
    }

    let version_hint = chrome_devtools_version_summary(port).unwrap_or_else(|| "unknown".into());
    anyhow::bail!(
        "Failed to create a Chrome page target via /json/new (Chrome DevTools {version_hint}). \
         Tried PUT/GET on /json/new?about:blank and /json/new. Details: {}",
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
pub fn chrome_http_json(port: u16, path: &str) -> Result<serde_json::Value> {
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

pub(crate) fn split_headers(response: &[u8]) -> Option<&[u8]> {
    response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|idx| &response[..idx])
}

pub(crate) fn parse_content_length(headers: &[u8]) -> Option<usize> {
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
pub(crate) fn pids_listening_on_port(port: u16) -> Vec<u32> {
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

pub(crate) fn process_command_line(pid: u32) -> Option<String> {
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

pub(crate) fn process_parent_pid(pid: u32) -> Option<u32> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "ppid="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

pub(crate) fn automation_markers_from_command(cmd: &str, ancestor: bool) -> Vec<String> {
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

pub(crate) fn parse_user_data_dir_flag(cmd: &str) -> Option<PathBuf> {
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
