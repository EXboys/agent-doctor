use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tungstenite::{client::IntoClientRequest, Message};

/// A CDP connection to a Chrome DevTools Protocol endpoint.
pub struct BrowserContext {
    ws_connection: tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    next_id: u64,
    /// DevTools HTTP port (parsed from the page WebSocket URL).
    debug_port: Option<u16>,
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
        };

        // Enable CDP domains we need. `Input` has no enable method.
        ctx.enable_domain("Page")?;
        ctx.enable_domain("Runtime")?;
        ctx.enable_domain("DOM")?;
        // Needed for Target.activateTarget / getTargets from a page session.
        let _ = ctx.enable_domain("Target");

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
        self.enable_domain("Page")?;
        self.enable_domain("Runtime")?;
        self.enable_domain("DOM")?;
        let _ = self.enable_domain("Target");
        Ok(())
    }

    fn enable_domain(&mut self, domain: &str) -> Result<Value> {
        self.send_command(&format!("{domain}.enable"), json!({}))
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

    // ─── Browser tools ─────────────────────────────────────────

    /// Navigate to a URL and wait for the page to load.
    pub fn navigate(&mut self, url: &str) -> Result<Value> {
        self.send_command("Page.navigate", json!({ "url": url }))?;
        std::thread::sleep(Duration::from_millis(2000));

        for _ in 0..30 {
            let result = self.send_command(
                "Runtime.evaluate",
                json!({
                    "expression": "document.readyState",
                    "returnByValue": true,
                }),
            )?;
            if let Some(state) = result.pointer("/result/value").and_then(Value::as_str) {
                if state == "complete" {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }

        let title = self
            .send_command(
                "Runtime.evaluate",
                json!({
                    "expression": "document.title",
                    "returnByValue": true,
                }),
            )
            .ok()
            .and_then(|v| v.pointer("/result/value").cloned());

        let url_current = self
            .send_command(
                "Runtime.evaluate",
                json!({
                    "expression": "window.location.href",
                    "returnByValue": true,
                }),
            )
            .ok()
            .and_then(|v| v.pointer("/result/value").cloned());

        Ok(json!({
            "title": title,
            "url": url_current,
        }))
    }

    /// Click on an element identified by a CSS selector.
    pub fn click(&mut self, selector: &str) -> Result<Value> {
        // Scroll first, then read fresh coordinates (old code scrolled AFTER measuring → miss).
        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"(function() {{
                        const el = document.querySelector({s:?});
                        if (!el) return {{ error: "Element not found: " + {s:?} }};
                        el.scrollIntoView({{ block: "center", inline: "center" }});
                        const rect = el.getBoundingClientRect();
                        if (rect.width <= 0 || rect.height <= 0) {{
                            return {{
                                error: "Element has zero size (not visible): " + {s:?},
                                tag: el.tagName
                            }};
                        }}
                        return {{
                            x: rect.left + rect.width / 2,
                            y: rect.top + rect.height / 2,
                            visible: true,
                            tag: el.tagName,
                            text: (el.innerText || el.textContent || "").substring(0, 100)
                        }};
                    }})()"#,
                    s = selector,
                ),
                "returnByValue": true,
            }),
        )?;

        let coords = result
            .pointer("/result/value")
            .context("Failed to evaluate click target")?;

        if let Some(err) = coords.get("error").and_then(Value::as_str) {
            anyhow::bail!("{err}");
        }

        let x = coords
            .get("x")
            .and_then(Value::as_f64)
            .context("Missing x coordinate")?;
        let y = coords
            .get("y")
            .and_then(Value::as_f64)
            .context("Missing y coordinate")?;

        std::thread::sleep(Duration::from_millis(50));

        self.send_command(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseMoved",
                "x": x,
                "y": y,
            }),
        )?;
        self.send_command(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mousePressed",
                "x": x,
                "y": y,
                "button": "left",
                "buttons": 1,
                "clickCount": 1,
            }),
        )?;
        self.send_command(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseReleased",
                "x": x,
                "y": y,
                "button": "left",
                "buttons": 0,
                "clickCount": 1,
            }),
        )?;

        // DOM click only if mouse missed the target (overlay / wrong node).
        // Always doing both double-toggles checkboxes and radios.
        let hit = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"(function() {{
                        const el = document.querySelector({s:?});
                        if (!el) return {{ method: "mouse", matched: false }};
                        const top = document.elementFromPoint({x}, {y});
                        if (top && (top === el || el.contains(top) || top.contains(el))) {{
                            return {{ method: "mouse", matched: true }};
                        }}
                        el.click();
                        return {{
                            method: "dom-fallback",
                            matched: false,
                            atPoint: top ? top.tagName : null
                        }};
                    }})()"#,
                    s = selector,
                    x = x,
                    y = y,
                ),
                "returnByValue": true,
            }),
        )?;
        let method = hit
            .pointer("/result/value/method")
            .and_then(Value::as_str)
            .unwrap_or("mouse");

        Ok(json!({
            "selector": selector,
            "x": x,
            "y": y,
            "tag": coords.get("tag"),
            "method": method,
        }))
    }

    /// Type text into an element identified by a CSS selector.
    ///
    /// Clears existing content first, then inserts. `Input.insertText` alone
    /// often leaves the previous value (select + insertText does not reliably
    /// replace), so we verify and fall back to a native value setter.
    pub fn type_text(&mut self, selector: &str, text: &str) -> Result<Value> {
        let focused = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"(function() {{
                        const el = document.querySelector({s:?});
                        if (!el) return {{ ok: false, error: "Element not found: " + {s:?} }};
                        el.scrollIntoView({{ block: "center", inline: "center" }});
                        el.focus();
                        if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {{
                            const proto = el instanceof HTMLTextAreaElement
                                ? HTMLTextAreaElement.prototype
                                : HTMLInputElement.prototype;
                            const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
                            if (setter) setter.call(el, ""); else el.value = "";
                            el.dispatchEvent(new Event("input", {{ bubbles: true }}));
                        }} else if (el.isContentEditable) {{
                            el.textContent = "";
                            const range = document.createRange();
                            range.selectNodeContents(el);
                            const sel = window.getSelection();
                            sel.removeAllRanges();
                            sel.addRange(range);
                        }}
                        return {{
                            ok: true,
                            tag: el.tagName,
                            focused: document.activeElement === el
                        }};
                    }})()"#,
                    s = selector,
                ),
                "returnByValue": true,
            }),
        )?;

        let focus_info = focused
            .pointer("/result/value")
            .cloned()
            .unwrap_or(json!({}));
        if focus_info.get("ok") != Some(&json!(true)) {
            let err = focus_info
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Failed to focus element");
            anyhow::bail!("{err}");
        }

        std::thread::sleep(Duration::from_millis(30));

        let mut method = "insertText";
        if self
            .send_command("Input.insertText", json!({ "text": text }))
            .is_err()
        {
            method = "dom-set";
            self.set_element_value(selector, text)?;
        }

        let mut verify = self.read_element_value(selector)?;
        let matches = verify.as_str().map(|v| v == text).unwrap_or(false);
        if !matches {
            method = "dom-set";
            self.set_element_value(selector, text)?;
            verify = self.read_element_value(selector)?;
        }

        Ok(json!({
            "selector": selector,
            "typed_length": text.chars().count(),
            "value": verify,
            "method": method,
        }))
    }

    fn set_element_value(&mut self, selector: &str, text: &str) -> Result<()> {
        self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"(function() {{
                        const el = document.querySelector({s:?});
                        if (!el) return false;
                        el.focus();
                        const value = {t:?};
                        if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {{
                            const proto = el instanceof HTMLTextAreaElement
                                ? HTMLTextAreaElement.prototype
                                : HTMLInputElement.prototype;
                            const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
                            if (setter) setter.call(el, value); else el.value = value;
                            el.dispatchEvent(new Event("input", {{ bubbles: true }}));
                            el.dispatchEvent(new Event("change", {{ bubbles: true }}));
                        }} else if (el.isContentEditable) {{
                            el.textContent = value;
                            el.dispatchEvent(new InputEvent("input", {{ bubbles: true, data: value }}));
                        }}
                        return true;
                    }})()"#,
                    s = selector,
                    t = text,
                ),
                "returnByValue": true,
            }),
        )?;
        Ok(())
    }

    fn read_element_value(&mut self, selector: &str) -> Result<Value> {
        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"(function() {{
                        const el = document.querySelector({s:?});
                        if (!el) return null;
                        if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {{
                            return el.value;
                        }}
                        if (el.isContentEditable) return el.textContent || "";
                        return null;
                    }})()"#,
                    s = selector,
                ),
                "returnByValue": true,
            }),
        )?;
        Ok(result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Take a screenshot (returns base64-encoded PNG).
    pub fn screenshot(&mut self) -> Result<String> {
        let result = self.send_command(
            "Page.captureScreenshot",
            json!({
                "format": "png",
                "fromSurface": true,
            }),
        )?;

        result
            .get("data")
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("Screenshot response missing 'data' field")
    }

    /// Get the visible text content of the page.
    pub fn get_text(&mut self) -> Result<String> {
        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": "document.body?.innerText || '(no body)'",
                "returnByValue": true,
            }),
        )?;

        result
            .pointer("/result/value")
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("Failed to extract page text")
    }

    /// Get the current page URL.
    pub fn get_url(&mut self) -> Result<String> {
        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": "window.location.href",
                "returnByValue": true,
            }),
        )?;

        result
            .pointer("/result/value")
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("Failed to extract URL")
    }

    /// Get HTML content of a specific element (or full page).
    pub fn get_html(&mut self, selector: Option<&str>) -> Result<String> {
        let expr = if let Some(sel) = selector {
            format!(
                r#"document.querySelector({s:?})?.outerHTML || "Element not found: " + {s:?}"#,
                s = sel
            )
        } else {
            "document.documentElement?.outerHTML || '(no html)'".to_string()
        };

        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": expr,
                "returnByValue": true,
            }),
        )?;

        result
            .pointer("/result/value")
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("Failed to extract HTML")
    }

    /// Evaluate arbitrary JavaScript in the page context.
    pub fn evaluate(&mut self, js: &str) -> Result<Value> {
        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": js,
                "returnByValue": true,
                "awaitPromise": true,
            }),
        )?;

        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(json!(null)))
    }

    /// Detect globals injected by ChromeDriver's automation bootstrap.
    pub fn chrome_driver_artifacts(&mut self) -> Result<Vec<String>> {
        let value = self.evaluate(
            r#"Object.getOwnPropertyNames(globalThis)
                .filter((name) => /^cdc_[A-Za-z0-9_]{10,}$/.test(name))
                .sort()"#,
        )?;
        Ok(value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect())
    }

    /// Scroll the page by a delta.
    pub fn scroll(&mut self, delta_x: f64, delta_y: f64) -> Result<Value> {
        self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!("window.scrollBy({delta_x}, {delta_y})"),
                "returnByValue": true,
            }),
        )?;

        Ok(json!({ "scrolled_by": { "x": delta_x, "y": delta_y } }))
    }

    /// Wait for an element to appear.
    pub fn wait_for_selector(&mut self, selector: &str, timeout_ms: u64) -> Result<Value> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_millis(timeout_ms);

        while start.elapsed() < timeout {
            let result = self.send_command(
                "Runtime.evaluate",
                json!({
                    "expression": format!(r#"!!document.querySelector({s:?})"#, s = selector),
                    "returnByValue": true,
                }),
            )?;

            if result.pointer("/result/value") == Some(&json!(true)) {
                return Ok(json!({ "found": true, "selector": selector }));
            }

            std::thread::sleep(Duration::from_millis(200));
        }

        anyhow::bail!("Element '{selector}' not found within {timeout_ms}ms")
    }

    /// Get all links on the page.
    pub fn get_links(&mut self) -> Result<Value> {
        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": r#"
                    Array.from(document.querySelectorAll('a[href]')).map(a => ({
                        text: a.textContent?.trim()?.substring(0, 100) || '',
                        href: a.href,
                        title: a.title || '',
                    }))
                "#,
                "returnByValue": true,
            }),
        )?;

        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(json!([])))
    }

    /// Open a new browser tab and navigate to a URL.
    pub fn new_tab(&mut self, url: &str) -> Result<Value> {
        let result = self.send_command(
            "Target.createTarget",
            json!({
                "url": url,
                "newWindow": false,
            }),
        )?;

        let target_id = result
            .get("targetId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("Failed to get targetId")?;

        std::thread::sleep(Duration::from_millis(500));
        // Attach subsequent commands to the new tab's page session.
        let switched = self.switch_tab(&target_id)?;
        Ok(json!({
            "target_id": target_id,
            "url": url,
            "status": switched.get("status"),
        }))
    }

    /// Switch to a browser tab by target ID.
    ///
    /// `Target.activateTarget` only brings the tab to the front; the existing page
    /// WebSocket stays bound to the old document. Reconnect CDP to the target's
    /// page debugger URL so subsequent tools operate on the selected tab.
    pub fn switch_tab(&mut self, target_id: &str) -> Result<Value> {
        let _ = self.send_command(
            "Target.activateTarget",
            json!({
                "targetId": target_id,
            }),
        );

        let port = self.debug_port.context(
            "Cannot switch tabs: DevTools port unknown (CDP was not connected via ws://host:port)",
        )?;
        let ws = page_ws_for_target(port, target_id)
            .with_context(|| format!("No page WebSocket for target {target_id}"))?;
        self.reconnect(&ws)?;

        let url = self.get_url().unwrap_or_default();
        Ok(json!({
            "target_id": target_id,
            "status": "switched",
            "url": url,
            "ws_endpoint": ws,
        }))
    }

    /// Close a browser tab by target ID.
    pub fn close_tab(&mut self, target_id: &str) -> Result<Value> {
        self.send_command(
            "Target.closeTarget",
            json!({
                "targetId": target_id,
            }),
        )?;
        Ok(json!({ "target_id": target_id, "status": "closed" }))
    }

    /// List all open browser tabs/targets.
    pub fn list_tabs(&mut self) -> Result<Value> {
        let result = self.send_command("Target.getTargets", json!({}))?;
        let targets = result.get("targetInfos").cloned().unwrap_or(json!([]));
        Ok(json!({
            "targets": targets,
            "count": targets.as_array().map(|a| a.len()).unwrap_or(0),
        }))
    }
}

fn parse_debug_port(ws_endpoint: &str) -> Option<u16> {
    let rest = ws_endpoint
        .strip_prefix("ws://")
        .or_else(|| ws_endpoint.strip_prefix("wss://"))?;
    let hostport = rest.split('/').next()?;
    let port = hostport.rsplit(':').next()?;
    port.parse().ok()
}

fn page_ws_for_target(port: u16, target_id: &str) -> Result<String> {
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
