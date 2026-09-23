//! CDP `BrowserContext`: navigate, snapshot, input, waits, tabs, and state.

mod capture;
mod interact;
mod navigate;
mod tabs_state;
mod wait_find;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use tungstenite::{client::IntoClientRequest, Message};

use crate::state::ensure_parent_dir;

/// Parse `@e12` / `e12` → 0-based index into `window.__agentDoctorRefs`.
pub(crate) fn parse_ref_index(target: &str) -> Option<usize> {
    let t = target.trim();
    let body = t.strip_prefix('@').unwrap_or(t);
    let digits = body.strip_prefix('e').or_else(|| body.strip_prefix('E'))?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: usize = digits.parse().ok()?;
    n.checked_sub(1)
}

/// JS prelude that binds `el` from a CSS selector or snapshot ref (`@eN`).
pub(crate) fn js_bind_el(target: &str) -> String {
    if let Some(idx) = parse_ref_index(target) {
        format!(
            r#"const el = (window.__agentDoctorRefs && window.__agentDoctorRefs[{idx}]);
            if (!el || !document.contains(el)) {{
                return {{ error: "Stale or unknown ref: {t}. Call browser_snapshot again." }};
            }}"#,
            idx = idx,
            t = target.replace('\\', "\\\\").replace('"', "\\\"")
        )
    } else {
        format!(
            r#"const el = document.querySelector({s:?});
            if (!el) {{
                return {{ error: "Element not found: " + {s:?} }};
            }}"#,
            s = target
        )
    }
}

pub(crate) fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn screenshot_stamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub(crate) fn resolve_screenshot_path(path: Option<&str>) -> PathBuf {
    if let Some(p) = path.map(str::trim).filter(|s| !s.is_empty()) {
        return PathBuf::from(p);
    }
    std::env::temp_dir().join(format!(
        "agent-doctor-screenshot-{}-{}.png",
        std::process::id(),
        screenshot_stamp_ms()
    ))
}

pub(crate) fn write_screenshot_png(data_b64: &str, path: &Path) -> Result<()> {
    ensure_parent_dir(path)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64)
        .context("decode screenshot base64")?;
    std::fs::write(path, &bytes).with_context(|| format!("write screenshot {}", path.display()))?;
    Ok(())
}

/// A CDP connection to a Chrome DevTools Protocol endpoint.
pub struct BrowserContext {
    ws_connection: tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    next_id: u64,
    /// DevTools HTTP port (parsed from the page WebSocket URL).
    debug_port: Option<u16>,
    /// In-flight network request ids → first-seen time (when Network domain is enabled).
    network_inflight: HashMap<String, Instant>,
    last_network_activity: Option<Instant>,
    network_tracking: bool,
}

impl BrowserContext {
    /// Connect to Chrome's CDP WebSocket endpoint.
    pub fn connect(ws_endpoint: &str) -> Result<Self> {
        let request = ws_endpoint
            .into_client_request()
            .context("Failed to create WebSocket request")?;
        let (ws_connection, _) = tungstenite::connect(request)
            .with_context(|| format!("Failed to connect to Chrome CDP at {ws_endpoint}"))?;

        let mut ctx = BrowserContext {
            ws_connection,
            next_id: 1,
            debug_port: parse_debug_port(ws_endpoint),
            network_inflight: HashMap::new(),
            last_network_activity: None,
            network_tracking: false,
        };

        // Enable CDP domains we need. `Input` has no enable method.
        ctx.enable_domain("Page")?;
        ctx.enable_domain("Runtime")?;
        ctx.enable_domain("DOM")?;
        // Needed for Target.activateTarget / getTargets from a page session.
        let _ = ctx.enable_domain("Target");
        let _ = ctx.ensure_network_tracking();

        Ok(ctx)
    }

    fn reconnect(&mut self, ws_endpoint: &str) -> Result<()> {
        let request = ws_endpoint
            .into_client_request()
            .context("Failed to create WebSocket request")?;
        let (ws_connection, _) = tungstenite::connect(request)
            .with_context(|| format!("Failed to reconnect Chrome CDP at {ws_endpoint}"))?;
        self.ws_connection = ws_connection;
        self.next_id = 1;
        self.debug_port = parse_debug_port(ws_endpoint);
        self.network_inflight.clear();
        self.last_network_activity = None;
        self.network_tracking = false;
        self.enable_domain("Page")?;
        self.enable_domain("Runtime")?;
        self.enable_domain("DOM")?;
        let _ = self.enable_domain("Target");
        let _ = self.ensure_network_tracking();
        Ok(())
    }

    fn enable_domain(&mut self, domain: &str) -> Result<Value> {
        self.send_command(&format!("{domain}.enable"), json!({}))
    }

    fn ensure_network_tracking(&mut self) -> Result<()> {
        if self.network_tracking {
            return Ok(());
        }
        self.enable_domain("Network")?;
        self.network_tracking = true;
        Ok(())
    }

    fn handle_cdp_event(&mut self, method: &str, params: Option<&Value>) {
        let Some(params) = params else {
            return;
        };
        match method {
            "Network.requestWillBeSent" => {
                if let Some(id) = params.get("requestId").and_then(Value::as_str) {
                    self.network_inflight
                        .entry(id.to_string())
                        .or_insert_with(Instant::now);
                    self.last_network_activity = Some(Instant::now());
                }
            }
            "Network.loadingFinished" | "Network.loadingFailed" => {
                if let Some(id) = params.get("requestId").and_then(Value::as_str) {
                    self.network_inflight.remove(id);
                    self.last_network_activity = Some(Instant::now());
                }
            }
            "Network.requestServedFromCache" => {
                if let Some(id) = params.get("requestId").and_then(Value::as_str) {
                    self.network_inflight.remove(id);
                    self.last_network_activity = Some(Instant::now());
                }
            }
            _ => {}
        }
    }

    fn send_command(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let cmd = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        let cmd_str = serde_json::to_string(&cmd)?;
        let msg = Message::Text(cmd_str.into());
        self.ws_connection
            .send(msg)
            .context("Failed to send CDP command")?;

        // Read responses until we find the one matching our id
        loop {
            let received = self
                .ws_connection
                .read()
                .context("Failed to read CDP response")?;

            let text = match received {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
                _ => continue,
            };

            let response: Value = serde_json::from_str(&text)?;

            if let Some(event_method) = response.get("method").and_then(Value::as_str) {
                self.handle_cdp_event(event_method, response.get("params"));
                continue;
            }

            if response.get("id") == Some(&json!(id)) {
                if let Some(error) = response.get("error") {
                    anyhow::bail!(
                        "CDP error for {method}: {}",
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                    );
                }
                return Ok(response.get("result").cloned().unwrap_or(json!({})));
            }
        }
    }
}

pub(crate) fn url_matches(current: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    if current == pattern || current.contains(pattern) {
        return true;
    }
    wildcard_match(current, pattern)
}

pub(crate) fn wildcard_match(text: &str, pattern: &str) -> bool {
    fn go(t: &[u8], p: &[u8]) -> bool {
        if p.is_empty() {
            return t.is_empty();
        }
        if p[0] == b'*' {
            let rest = if p.len() > 1 && p[1] == b'*' {
                &p[2..]
            } else {
                &p[1..]
            };
            if go(t, rest) {
                return true;
            }
            if !t.is_empty() && go(&t[1..], p) {
                return true;
            }
            return false;
        }
        if t.is_empty() {
            return false;
        }
        if p[0] == t[0] && go(&t[1..], &p[1..]) {
            return true;
        }
        false
    }
    go(text.as_bytes(), pattern.as_bytes())
}

pub(crate) fn parse_debug_port(ws_endpoint: &str) -> Option<u16> {
    let rest = ws_endpoint
        .strip_prefix("ws://")
        .or_else(|| ws_endpoint.strip_prefix("wss://"))?;
    let hostport = rest.split('/').next()?;
    let port = hostport.rsplit(':').next()?;
    port.parse().ok()
}

pub(crate) fn page_ws_for_target(port: u16, target_id: &str) -> Result<String> {
    let list = crate::browser::chrome_http_json(port, "/json/list")
        .with_context(|| format!("Failed to list Chrome targets on port {port}"))?;
    let arr = list
        .as_array()
        .with_context(|| "Chrome /json/list is not an array")?;
    for target in arr {
        let id = target.get("id").and_then(Value::as_str).unwrap_or("");
        let is_page = target
            .get("type")
            .and_then(Value::as_str)
            .map(|t| t == "page")
            .unwrap_or(false);
        if id == target_id && is_page {
            if let Some(ws) = target.get("webSocketDebuggerUrl").and_then(Value::as_str) {
                return Ok(ws.to_string());
            }
        }
    }
    // Some Chrome builds expose target id only under "targetId".
    for target in arr {
        let id = target
            .get("id")
            .or_else(|| target.get("targetId"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if id != target_id {
            continue;
        }
        if let Some(ws) = target.get("webSocketDebuggerUrl").and_then(Value::as_str) {
            return Ok(ws.to_string());
        }
    }
    anyhow::bail!("target {target_id} not found in /json/list");
}

#[cfg(test)]
mod ref_tests {
    use super::*;

    #[test]
    fn parses_snapshot_refs() {
        assert_eq!(parse_ref_index("@e1"), Some(0));
        assert_eq!(parse_ref_index("e12"), Some(11));
        assert_eq!(parse_ref_index("@E3"), Some(2));
        assert_eq!(parse_ref_index("#main"), None);
        assert_eq!(parse_ref_index("@e"), None);
    }

    #[test]
    fn url_glob_matches() {
        assert!(url_matches(
            "https://example.com/a/b",
            "https://example.com/**"
        ));
        assert!(url_matches("https://example.com/login", "login"));
        assert!(!url_matches("https://example.com", "https://other.com"));
    }

    #[test]
    fn screenshot_path_defaults_to_temp_png() {
        let path = resolve_screenshot_path(None);
        assert!(path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("agent-doctor-screenshot-") && n.ends_with(".png")));
    }

    #[test]
    fn screenshot_path_honors_explicit() {
        let path = resolve_screenshot_path(Some("/tmp/shot.png"));
        assert_eq!(path, PathBuf::from("/tmp/shot.png"));
    }

    #[test]
    fn write_screenshot_png_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("page.png");
        // 1x1 PNG
        let b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        write_screenshot_png(b64, &out).unwrap();
        assert!(out.is_file());
        assert!(std::fs::metadata(&out).unwrap().len() > 0);
    }
}
