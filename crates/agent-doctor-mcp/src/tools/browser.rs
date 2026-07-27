use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tungstenite::{client::IntoClientRequest, Message};

/// A CDP connection to a Chrome DevTools Protocol endpoint.
pub struct BrowserContext {
    ws_connection: tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    next_id: u64,
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
        };

        // Enable CDP domains we need
        ctx.enable_domain("Page")?;
        ctx.enable_domain("Runtime")?;
        ctx.enable_domain("DOM")?;
        ctx.enable_domain("Input")?;

        Ok(ctx)
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
            if let Some(state) = result
                .pointer("/result/value")
                .and_then(Value::as_str)
            {
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
        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"(function() {{
                        const el = document.querySelector({s:?});
                        if (!el) return {{ error: "Element not found: {s:?}" }};
                        const rect = el.getBoundingClientRect();
                        return {{
                            x: rect.x + rect.width / 2,
                            y: rect.y + rect.height / 2,
                            visible: rect.width > 0 && rect.height > 0,
                            tag: el.tagName,
                            text: el.textContent?.substring(0, 100) || ''
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

        if coords.get("error").and_then(Value::as_str).is_some() {
            let err = coords.get("error").and_then(Value::as_str).unwrap_or("");
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

        self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"document.querySelector({s:?})?.scrollIntoView({{block:"center"}})"#,
                    s = selector
                ),
            }),
        )?;
        std::thread::sleep(Duration::from_millis(200));

        self.send_command("Input.dispatchMouseEvent", json!({
            "type": "mousePressed",
            "x": x,
            "y": y,
            "button": "left",
            "clickCount": 1,
        }))?;

        self.send_command("Input.dispatchMouseEvent", json!({
            "type": "mouseReleased",
            "x": x,
            "y": y,
            "button": "left",
            "clickCount": 1,
        }))?;

        Ok(json!({
            "selector": selector,
            "x": x,
            "y": y,
        }))
    }

    /// Type text into an element identified by a CSS selector.
    pub fn type_text(&mut self, selector: &str, text: &str) -> Result<Value> {
        self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"(function() {{
                        const el = document.querySelector({s:?});
                        if (!el) return false;
                        el.focus();
                        if (el.value !== undefined) el.value = '';
                        return true;
                    }})()"#,
                    s = selector,
                ),
                "returnByValue": true,
            }),
        )?;
        std::thread::sleep(Duration::from_millis(100));

        // Clear existing content
        self.send_command("Input.dispatchKeyEvent", json!({
            "type": "keyDown",
            "windowsVirtualKeyCode": 8,
            "key": "Backspace",
            "code": "Backspace",
        }))?;
        self.send_command("Input.dispatchKeyEvent", json!({
            "type": "keyUp",
            "windowsVirtualKeyCode": 8,
            "key": "Backspace",
            "code": "Backspace",
        }))?;

        for ch in text.chars() {
            let key = ch.to_string();
            self.send_command("Input.dispatchKeyEvent", json!({
                "type": "char",
                "text": key,
            }))
            .ok();
        }

        Ok(json!({
            "selector": selector,
            "typed_length": text.len(),
        }))
    }

    /// Take a screenshot (returns base64-encoded PNG).
    pub fn screenshot(&mut self) -> Result<String> {
        let result = self.send_command("Page.captureScreenshot", json!({
            "format": "png",
            "fromSurface": true,
        }))?;

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
                r#"document.querySelector({s:?})?.outerHTML || "Element not found: {s:?}""#,
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
        Ok(json!({ "target_id": target_id, "url": url }))
    }

    /// Switch to a browser tab by target ID.
    pub fn switch_tab(&mut self, target_id: &str) -> Result<Value> {
        self.send_command("Target.activateTarget", json!({
            "targetId": target_id,
        }))?;
        Ok(json!({ "target_id": target_id, "status": "activated" }))
    }

    /// Close a browser tab by target ID.
    pub fn close_tab(&mut self, target_id: &str) -> Result<Value> {
        self.send_command("Target.closeTarget", json!({
            "targetId": target_id,
        }))?;
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