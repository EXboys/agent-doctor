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
    let user_data_dir = default_user_data_dir();
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

fn default_user_data_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();

    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Google/Chrome")
    } else if cfg!(target_os = "linux") {
        home.join(".config/google-chrome")
    } else {
        home.join(r"AppData\Local\Google\Chrome\User Data")
    }
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
/// - Uses your real user data directory (authentic cookies, sessions)
/// - Disables AutomationControlled Blink feature
/// - Does NOT pass --enable-automation flag
/// - Does NOT pass --remote-debugging-pipe (less conspicuous)
pub fn launch_chrome(
    discovery: &BrowserDiscovery,
    port: u16,
    headless: bool,
) -> Result<ChromeInstance> {
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

    let child = cmd.spawn().with_context(|| {
        format!(
            "Failed to start Chrome: {}",
            discovery.binary_path.display()
        )
    })?;

    // Wait for Chrome to start listening
    std::thread::sleep(Duration::from_millis(1500));

    let ws_endpoint = find_ws_endpoint_http(port).ok();

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

/// Fetch the WebSocket debug URL from Chrome's DevTools Protocol endpoint.
fn find_ws_endpoint_http(port: u16) -> Result<String> {
    let addr = format!("127.0.0.1:{port}");
    let mut stream = TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_secs(5))
        .with_context(|| format!("Cannot connect to Chrome on {addr}"))?;

    let request = format!(
        "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .context("Failed to send HTTP request to Chrome")?;
    stream.flush()?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .context("Failed to read response from Chrome")?;

    let response_str = String::from_utf8_lossy(&response);

    // Find the JSON body after HTTP headers
    let body = response_str
        .split("\r\n\r\n")
        .nth(1)
        .context("No HTTP body in Chrome version response")?;

    let json: serde_json::Value =
        serde_json::from_str(body).context("Failed to parse Chrome version endpoint response")?;

    let ws = json
        .get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .context("Chrome version endpoint missing webSocketDebuggerUrl")?;

    Ok(ws)
}

/// Stop the Chrome instance gracefully.
pub fn stop_chrome(instance: &mut ChromeInstance) -> Result<()> {
    if let Some(mut child) = instance.process.take() {
        child.kill().context("Failed to kill Chrome process")?;
        child.wait().context("Failed to wait for Chrome to exit")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_user_data_dir_is_valid_path() {
        let dir = default_user_data_dir();
        assert!(
            dir.to_string_lossy().contains("Chrome")
                || dir.to_string_lossy().contains("google-chrome")
        );
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
