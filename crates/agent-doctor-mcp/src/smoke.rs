//! CDP navigate smoke checks for CI and local verification.
//!
//! Launches an isolated Chromium-family browser, creates a page target
//! (PUT/GET `/json/new` fallback), navigates once, then tears down.

use std::net::TcpListener;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::json;

use crate::browser::{
    discover_browser, kill_chrome_on_port, launch_chrome, stop_chrome, BrowserDiscovery,
    BrowserFamily, ChromeInstance,
};
use crate::tools::BrowserContext;

const DEFAULT_SMOKE_URL: &str = "https://example.com/";

#[derive(Debug, Clone)]
pub struct SmokeOptions {
    pub family: BrowserFamily,
    pub url: String,
    pub headless: bool,
    /// When `None`, pick an ephemeral free TCP port.
    pub port: Option<u16>,
}

impl Default for SmokeOptions {
    fn default() -> Self {
        Self {
            family: BrowserFamily::Auto,
            url: DEFAULT_SMOKE_URL.into(),
            headless: true,
            port: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SmokeReport {
    pub ok: bool,
    pub browser: String,
    pub binary: String,
    pub version: Option<String>,
    pub port: u16,
    pub url: String,
    pub title: Option<String>,
    pub final_url: Option<String>,
    pub detail: String,
}

/// End-to-end: launch → `/json/new` → `Page.navigate` → assert load.
pub fn smoke_browser_navigate(options: &SmokeOptions) -> Result<SmokeReport> {
    let discovery = discover_browser(options.family)?;
    let port = match options.port {
        Some(port) => port,
        None => free_tcp_port().context("allocate free CDP port for smoke")?,
    };

    let user_data_dir = smoke_user_data_dir(options.family, port);
    let _ = std::fs::remove_dir_all(&user_data_dir);
    std::fs::create_dir_all(&user_data_dir)
        .with_context(|| format!("create smoke profile {}", user_data_dir.display()))?;

    let discovery = BrowserDiscovery {
        user_data_dir: user_data_dir.clone(),
        profile_directory: "Default".into(),
        ..discovery
    };

    let _ = kill_chrome_on_port(port);
    let mut instance = launch_chrome(&discovery, port, options.headless)?;
    let result = run_navigate(&mut instance, &discovery, options);
    let _ = stop_chrome(&mut instance);
    let _ = kill_chrome_on_port(port);
    let _ = std::fs::remove_dir_all(&user_data_dir);
    result
}

fn run_navigate(
    instance: &mut ChromeInstance,
    discovery: &BrowserDiscovery,
    options: &SmokeOptions,
) -> Result<SmokeReport> {
    let ws = instance
        .ws_endpoint
        .clone()
        .context("Chrome started but page WebSocket endpoint is missing")?;
    let mut ctx = BrowserContext::connect(&ws)?;
    let nav = ctx
        .navigate(&options.url)
        .with_context(|| format!("browser_navigate to {}", options.url))?;

    let title = nav
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let final_url = nav.get("url").and_then(|v| v.as_str()).map(str::to_string);

    if final_url
        .as_deref()
        .map(|u| u.starts_with("chrome-error://") || u.starts_with("edge-error://"))
        .unwrap_or(false)
    {
        bail!(
            "navigation landed on error page ({})",
            final_url.as_deref().unwrap_or("?")
        );
    }
    if title.as_deref().unwrap_or("").is_empty()
        && final_url
            .as_deref()
            .map(|u| u == "about:blank" || u.is_empty())
            .unwrap_or(true)
    {
        bail!("navigation produced empty title and blank URL");
    }

    let snap = ctx
        .snapshot(true, false, None)
        .context("browser_snapshot after navigate")?;
    let count = snap.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
    if count == 0 {
        bail!("browser_snapshot returned 0 interactive refs");
    }
    let first_ref = snap
        .pointer("/elements/0/ref")
        .and_then(|v| v.as_str())
        .unwrap_or("@e1")
        .to_string();
    let waited = ctx
        .wait(Some(&first_ref), None, None, 3_000)
        .context("wait for first snapshot ref")?;
    if waited.get("found").and_then(|v| v.as_bool()) != Some(true) {
        bail!("first snapshot ref {first_ref} was not present");
    }

    let _ = ctx
        .wait(None, None, Some("networkidle"), 5_000)
        .context("wait networkidle")?;

    // Semantic find: example.com exposes a single link.
    if options.url.contains("example.com") {
        let found = ctx
            .find("role", "link", false, None, None)
            .or_else(|_| ctx.find("text", "information", false, None, None))
            .context("browser_find role=link")?;
        if found.get("ref").and_then(|v| v.as_str()).is_none() {
            bail!("browser_find did not return a ref on example.com");
        }
    }

    Ok(SmokeReport {
        ok: true,
        browser: options.family.as_str().into(),
        binary: discovery.binary_path.display().to_string(),
        version: discovery.version.clone(),
        port: instance.debug_port,
        url: options.url.clone(),
        title,
        final_url,
        detail: format!("navigate ok; snapshot refs={count}; find+networkidle ok"),
    })
}

fn free_tcp_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn smoke_user_data_dir(family: BrowserFamily, port: u16) -> PathBuf {
    std::env::temp_dir().join(format!(
        "agent-doctor-smoke-{}-{port}-{}",
        family.as_str(),
        std::process::id()
    ))
}

/// Parse `chrome` / `edge` / `auto` (case-insensitive).
pub fn parse_browser_family(raw: &str) -> Result<BrowserFamily> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "auto" | "" => Ok(BrowserFamily::Auto),
        "chrome" | "google-chrome" | "google chrome" => Ok(BrowserFamily::Chrome),
        "edge" | "msedge" | "microsoft-edge" | "microsoft edge" => Ok(BrowserFamily::Edge),
        "chromium" => Ok(BrowserFamily::Chromium),
        other => bail!("unknown browser family '{other}' (expected chrome, edge, chromium, auto)"),
    }
}

impl SmokeReport {
    pub fn to_json_value(&self) -> serde_json::Value {
        json!({
            "ok": self.ok,
            "browser": self.browser,
            "binary": self.binary,
            "version": self.version,
            "port": self.port,
            "url": self.url,
            "title": self.title,
            "final_url": self.final_url,
            "detail": self.detail,
        })
    }
}
