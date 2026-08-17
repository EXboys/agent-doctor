use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tungstenite::{client::IntoClientRequest, Message};

use crate::state::{ensure_parent_dir, resolve_state_path};

/// Parse `@e12` / `e12` → 0-based index into `window.__agentDoctorRefs`.
fn parse_ref_index(target: &str) -> Option<usize> {
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
fn js_bind_el(target: &str) -> String {
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

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

    // ─── Browser tools ─────────────────────────────────────────

    /// Navigate to a URL and wait for the page to load.
    pub fn navigate(&mut self, url: &str) -> Result<Value> {
        // Snapshot refs are page-local; always invalidate on navigation.
        let _ = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": "window.__agentDoctorRefs = undefined",
                "returnByValue": true,
            }),
        );

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

        // Best-effort quiet window so SPA shells settle before the model snapshots.
        let _ = self.wait(None, None, Some("networkidle"), 8_000);

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
            "hint": "Call browser_snapshot next to get @eN refs before click/fill.",
        }))
    }

    /// Capture interactive elements and assign `@eN` refs (agent-browser-style).
    ///
    /// Workflow: navigate → snapshot → click/fill `@eN` → re-snapshot after DOM changes.
    pub fn snapshot(
        &mut self,
        interactive: bool,
        cursor_interactive: bool,
        scope: Option<&str>,
    ) -> Result<Value> {
        let expression = format!(
            r#"(function() {{
                const interactiveOnly = {interactive};
                const includeCursor = {cursor};
                const scopeSel = {scope:?};
                let root = document;
                if (scopeSel) {{
                    const scoped = document.querySelector(scopeSel);
                    if (!scoped) return {{ error: "Scope not found: " + scopeSel }};
                    root = scoped;
                }}
                const sel = interactiveOnly
                    ? 'a[href], button, input:not([type=hidden]), textarea, select, summary, [role="button"], [role="link"], [role="textbox"], [role="checkbox"], [role="radio"], [role="menuitem"], [role="tab"], [role="switch"], [role="combobox"], [role="option"], [role="searchbox"], [contenteditable="true"], [contenteditable=""], [tabindex]:not([tabindex="-1"])'
                    : 'a, button, input, textarea, select, [role], [contenteditable], [tabindex], label, summary';
                const seen = new Set();
                const els = [];
                const push = (el) => {{
                    if (!el || seen.has(el)) return;
                    const style = window.getComputedStyle(el);
                    if (style.display === 'none' || style.visibility === 'hidden') return;
                    if (Number(style.opacity) === 0) return;
                    const r = el.getBoundingClientRect();
                    if (r.width <= 0 || r.height <= 0) return;
                    seen.add(el);
                    els.push(el);
                }};
                root.querySelectorAll(sel).forEach(push);
                if (includeCursor) {{
                    root.querySelectorAll('*').forEach((el) => {{
                        try {{
                            if (window.getComputedStyle(el).cursor === 'pointer') push(el);
                        }} catch (_) {{}}
                    }});
                }}
                window.__agentDoctorRefs = els;
                const items = els.map((el, i) => {{
                    const tag = el.tagName.toLowerCase();
                    const role = el.getAttribute('role') || '';
                    const type = (el.getAttribute('type') || '').toLowerCase();
                    let text = (el.getAttribute('aria-label')
                        || el.getAttribute('placeholder')
                        || el.getAttribute('title')
                        || '').trim();
                    if (!text) {{
                        const raw = (el.innerText || el.textContent || '').replace(/\s+/g, ' ').trim();
                        text = raw.slice(0, 80);
                    }}
                    if (!text && el.id) {{
                        const lab = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
                        if (lab) text = (lab.innerText || '').replace(/\s+/g, ' ').trim().slice(0, 80);
                    }}
                    const name = el.getAttribute('name') || '';
                    let value = '';
                    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {{
                        value = String(el.value || '').slice(0, 60);
                    }} else if (el instanceof HTMLSelectElement) {{
                        value = el.options[el.selectedIndex] ? el.options[el.selectedIndex].text : '';
                    }}
                    const checked = (el instanceof HTMLInputElement
                        && (el.type === 'checkbox' || el.type === 'radio'))
                        ? el.checked
                        : undefined;
                    const href = (el instanceof HTMLAnchorElement) ? el.href : undefined;
                    const ref = '@e' + (i + 1);
                    let line = ref + ' [' + tag;
                    if (type) line += ' type=' + type;
                    if (role) line += ' role=' + role;
                    line += ']';
                    if (name) line += ' name=' + JSON.stringify(name);
                    if (text) line += ' ' + JSON.stringify(text);
                    if (value) line += ' value=' + JSON.stringify(value);
                    if (href) line += ' ' + href;
                    if (checked === true) line += ' checked';
                    if (checked === false) line += ' unchecked';
                    return {{
                        ref,
                        tag,
                        role: role || undefined,
                        type: type || undefined,
                        name: name || undefined,
                        text: text || undefined,
                        value: value || undefined,
                        href,
                        checked,
                        line,
                    }};
                }});
                return {{
                    count: items.length,
                    elements: items,
                    snapshot: items.map((it) => it.line).join('\\n'),
                    hint: 'Use @eN with browser_click / browser_fill / browser_type. Re-snapshot after navigation or DOM changes.',
                }};
            }})()"#,
            interactive = if interactive { "true" } else { "false" },
            cursor = if cursor_interactive { "true" } else { "false" },
            scope = scope.unwrap_or(""),
        );

        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
            }),
        )?;
        let value = result
            .pointer("/result/value")
            .cloned()
            .context("snapshot evaluate returned no value")?;
        if let Some(err) = value.get("error").and_then(Value::as_str) {
            anyhow::bail!("{err}");
        }
        // Empty scope string means we passed "" — treat as no scope (already handled).
        Ok(value)
    }

    /// Click an element by CSS selector or snapshot ref (`@eN`).
    pub fn click(&mut self, target: &str) -> Result<Value> {
        let bind = js_bind_el(target);
        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"(function() {{
                        {bind}
                        el.scrollIntoView({{ block: "center", inline: "center" }});
                        const rect = el.getBoundingClientRect();
                        if (rect.width <= 0 || rect.height <= 0) {{
                            return {{
                                error: "Element has zero size (not visible): " + {t:?},
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
                    bind = bind,
                    t = target,
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

        let bind2 = js_bind_el(target);
        let hit = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"(function() {{
                        {bind}
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
                    bind = bind2,
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
            "target": target,
            "x": x,
            "y": y,
            "tag": coords.get("tag"),
            "method": method,
            "hint": "If the page changed, call browser_snapshot again before the next click.",
        }))
    }

    /// Type text into an element (CSS selector or `@eN`). Clears existing content first.
    pub fn type_text(&mut self, target: &str, text: &str) -> Result<Value> {
        let bind = js_bind_el(target);
        let focused = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"(function() {{
                        {bind}
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
                    bind = bind,
                ),
                "returnByValue": true,
            }),
        )?;

        let focus_info = focused
            .pointer("/result/value")
            .cloned()
            .unwrap_or(json!({}));
        if let Some(err) = focus_info.get("error").and_then(Value::as_str) {
            anyhow::bail!("{err}");
        }
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
            self.set_element_value(target, text)?;
        }

        let mut verify = self.read_element_value(target)?;
        let matches = verify.as_str().map(|v| v == text).unwrap_or(false);
        if !matches {
            method = "dom-set";
            self.set_element_value(target, text)?;
            verify = self.read_element_value(target)?;
        }

        Ok(json!({
            "target": target,
            "typed_length": text.chars().count(),
            "value": verify,
            "method": method,
        }))
    }

    /// Fill = clear + type (alias for models that know agent-browser's `fill`).
    pub fn fill(&mut self, target: &str, text: &str) -> Result<Value> {
        let mut out = self.type_text(target, text)?;
        if let Some(obj) = out.as_object_mut() {
            obj.insert("action".into(), json!("fill"));
        }
        Ok(out)
    }

    fn set_element_value(&mut self, target: &str, text: &str) -> Result<()> {
        let bind = js_bind_el(target);
        self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"(function() {{
                        {bind}
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
                    bind = bind,
                    t = text,
                ),
                "returnByValue": true,
            }),
        )?;
        Ok(())
    }

    fn read_element_value(&mut self, target: &str) -> Result<Value> {
        let bind = js_bind_el(target);
        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"(function() {{
                        {bind}
                        if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {{
                            return el.value;
                        }}
                        if (el.isContentEditable) return el.textContent || "";
                        return null;
                    }})()"#,
                    bind = bind,
                ),
                "returnByValue": true,
            }),
        )?;
        let value = result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null);
        if value.get("error").is_some() {
            anyhow::bail!(
                "{}",
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("read failed")
            );
        }
        Ok(value)
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

    /// Wait for selector/`@eN`, URL pattern, load state (`load` / `domcontentloaded` /
    /// `networkidle`), or a fixed timeout.
    pub fn wait(
        &mut self,
        selector: Option<&str>,
        url: Option<&str>,
        load: Option<&str>,
        timeout_ms: u64,
    ) -> Result<Value> {
        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms);

        if selector.is_none() && url.is_none() && load.is_none() {
            std::thread::sleep(timeout.min(Duration::from_millis(timeout_ms)));
            return Ok(json!({ "waited_ms": timeout_ms, "reason": "timeout" }));
        }

        while start.elapsed() < timeout {
            if let Some(state) = load {
                if self.load_state_reached(state)? {
                    return Ok(json!({
                        "load": state,
                        "inflight": self.network_inflight.len(),
                        "elapsed_ms": start.elapsed().as_millis(),
                    }));
                }
            }
            if let Some(sel) = selector {
                if self.target_present(sel)? {
                    return Ok(json!({
                        "found": true,
                        "selector": sel,
                        "elapsed_ms": start.elapsed().as_millis(),
                    }));
                }
            }
            if let Some(pattern) = url {
                let current = self.get_url().unwrap_or_default();
                if url_matches(&current, pattern) {
                    return Ok(json!({
                        "matched": true,
                        "url": current,
                        "pattern": pattern,
                        "elapsed_ms": start.elapsed().as_millis(),
                    }));
                }
            }
            // Pump CDP events (network counters) via a cheap evaluate.
            let _ = self.send_command(
                "Runtime.evaluate",
                json!({ "expression": "1", "returnByValue": true }),
            );
            std::thread::sleep(Duration::from_millis(100));
        }

        anyhow::bail!(
            "wait timed out after {timeout_ms}ms (selector={selector:?}, url={url:?}, load={load:?}, inflight={})",
            self.network_inflight.len()
        )
    }

    /// Wait for an element to appear (CSS or `@eN`).
    pub fn wait_for_selector(&mut self, selector: &str, timeout_ms: u64) -> Result<Value> {
        self.wait(Some(selector), None, None, timeout_ms)
    }

    fn load_state_reached(&mut self, state: &str) -> Result<bool> {
        let normalized = state.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "load" => self.document_ready_at_least("complete"),
            "domcontentloaded" | "domcontent" => {
                let ready = self.document_ready_state()?;
                Ok(ready == "interactive" || ready == "complete")
            }
            "networkidle" | "network-idle" | "network_almost_idle" | "networkalmostidle" => {
                let _ = self.ensure_network_tracking();
                if !self.document_ready_at_least("complete")? {
                    return Ok(false);
                }
                // Drop long-poll / hanging requests so idle can still settle.
                let stale_after = Duration::from_secs(3);
                self.network_inflight
                    .retain(|_, started| started.elapsed() < stale_after);
                if !self.network_inflight.is_empty() {
                    return Ok(false);
                }
                match self.last_network_activity {
                    Some(t) => Ok(t.elapsed() >= Duration::from_millis(500)),
                    None => Ok(true),
                }
            }
            other => anyhow::bail!(
                "unknown load state '{other}' (expected load, domcontentloaded, networkidle)"
            ),
        }
    }

    fn document_ready_state(&mut self) -> Result<String> {
        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": "document.readyState",
                "returnByValue": true,
            }),
        )?;
        Ok(result
            .pointer("/result/value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    fn document_ready_at_least(&mut self, want: &str) -> Result<bool> {
        let ready = self.document_ready_state()?;
        Ok(match want {
            "complete" => ready == "complete",
            "interactive" => ready == "interactive" || ready == "complete",
            _ => ready == want,
        })
    }

    /// Semantic locator (role / label / text / placeholder / testid).
    ///
    /// Adds the match to `__agentDoctorRefs` and returns `@eN`. Optional `action`
    /// of `click` / `fill` / `type` performs the action immediately.
    pub fn find(
        &mut self,
        strategy: &str,
        query: &str,
        exact: bool,
        action: Option<&str>,
        text: Option<&str>,
    ) -> Result<Value> {
        let strategy = strategy.trim().to_ascii_lowercase();
        let expression = format!(
            r#"(function() {{
                const strategy = {strategy:?};
                const query = {query:?};
                const exact = {exact};
                const norm = (s) => (s || '').replace(/\s+/g, ' ').trim().toLowerCase();
                const q = norm(query);
                const visible = (el) => {{
                    const st = getComputedStyle(el);
                    if (st.display === 'none' || st.visibility === 'hidden' || Number(st.opacity) === 0) return false;
                    const r = el.getBoundingClientRect();
                    return r.width > 0 && r.height > 0;
                }};
                const matchText = (hay) => {{
                    const h = norm(hay);
                    if (!h) return false;
                    return exact ? h === q : h.includes(q);
                }};
                let candidates = [];
                if (strategy === 'testid') {{
                    candidates = [...document.querySelectorAll(
                        '[data-testid], [data-test], [data-cy], [data-test-id]'
                    )].filter((el) => {{
                        const id = el.getAttribute('data-testid')
                            || el.getAttribute('data-test')
                            || el.getAttribute('data-cy')
                            || el.getAttribute('data-test-id')
                            || '';
                        return exact ? id === query : norm(id).includes(q);
                    }});
                }} else if (strategy === 'placeholder') {{
                    candidates = [...document.querySelectorAll('input, textarea, [contenteditable="true"], [contenteditable=""]')]
                        .filter((el) => matchText(el.getAttribute('placeholder') || ''));
                }} else if (strategy === 'label') {{
                    const byFor = [...document.querySelectorAll('label')].flatMap((lab) => {{
                        if (!matchText(lab.innerText || lab.textContent || '')) return [];
                        if (lab.htmlFor) {{
                            const el = document.getElementById(lab.htmlFor);
                            return el ? [el] : [];
                        }}
                        const nested = lab.querySelector('input, textarea, select');
                        return nested ? [nested] : [];
                    }});
                    const byAria = [...document.querySelectorAll('[aria-label]')]
                        .filter((el) => matchText(el.getAttribute('aria-label') || ''));
                    candidates = [...byFor, ...byAria];
                }} else if (strategy === 'role') {{
                    const role = q;
                    const implicit = {{
                        button: 'button, input[type=button], input[type=submit], input[type=reset], summary',
                        link: 'a[href]',
                        textbox: 'input:not([type=button]):not([type=submit]):not([type=checkbox]):not([type=radio]):not([type=hidden]), textarea, [contenteditable="true"]',
                        checkbox: 'input[type=checkbox]',
                        radio: 'input[type=radio]',
                        combobox: 'select, [role=combobox]',
                        heading: 'h1,h2,h3,h4,h5,h6',
                    }};
                    const sel = implicit[role] || '';
                    const withRole = [...document.querySelectorAll('[role]')].filter((el) =>
                        norm(el.getAttribute('role')) === role
                    );
                    const implied = sel ? [...document.querySelectorAll(sel)] : [];
                    candidates = [...withRole, ...implied];
                }} else if (strategy === 'text') {{
                    candidates = [...document.querySelectorAll(
                        'a, button, label, summary, [role=button], [role=link], [role=menuitem], [role=tab], option, td, th, span, p, h1, h2, h3, h4, h5, h6'
                    )].filter((el) => matchText(el.innerText || el.textContent || ''));
                }} else {{
                    return {{ error: "Unknown find strategy: " + strategy + " (use role|label|text|placeholder|testid)" }};
                }}
                candidates = candidates.filter(visible);
                // De-dupe while preserving order.
                const seen = new Set();
                candidates = candidates.filter((el) => {{
                    if (seen.has(el)) return false;
                    seen.add(el);
                    return true;
                }});
                if (!candidates.length) {{
                    return {{ error: "No element matched " + strategy + "=" + JSON.stringify(query) }};
                }}
                const el = candidates[0];
                if (!window.__agentDoctorRefs) window.__agentDoctorRefs = [];
                let idx = window.__agentDoctorRefs.indexOf(el);
                if (idx < 0) {{
                    window.__agentDoctorRefs.push(el);
                    idx = window.__agentDoctorRefs.length - 1;
                }}
                const ref = '@e' + (idx + 1);
                return {{
                    ref,
                    tag: el.tagName.toLowerCase(),
                    role: el.getAttribute('role') || undefined,
                    text: ((el.innerText || el.textContent || '').replace(/\s+/g, ' ').trim()).slice(0, 80),
                    matches: candidates.length,
                    strategy,
                    query,
                }};
            }})()"#,
            strategy = strategy,
            query = query,
            exact = if exact { "true" } else { "false" },
        );

        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
            }),
        )?;
        let value = result
            .pointer("/result/value")
            .cloned()
            .context("find evaluate returned no value")?;
        if let Some(err) = value.get("error").and_then(Value::as_str) {
            anyhow::bail!("{err}");
        }
        let ref_id = value
            .get("ref")
            .and_then(Value::as_str)
            .context("find did not return a ref")?
            .to_string();

        let action = action.map(str::trim).filter(|s| !s.is_empty());
        if let Some(act) = action {
            let act_l = act.to_ascii_lowercase();
            let acted = match act_l.as_str() {
                "click" => self.click(&ref_id)?,
                "fill" | "type" => {
                    let t = text.context("find action fill/type requires text")?;
                    if act_l == "fill" {
                        self.fill(&ref_id, t)?
                    } else {
                        self.type_text(&ref_id, t)?
                    }
                }
                other => anyhow::bail!("unknown find action '{other}' (use click, fill, type)"),
            };
            return Ok(json!({
                "ref": ref_id,
                "found": value,
                "action": act_l,
                "result": acted,
            }));
        }

        Ok(value)
    }

    /// Save cookies + localStorage (+ sessionStorage) for reuse.
    pub fn state_save(&mut self, path: Option<&str>, session: Option<&str>) -> Result<Value> {
        let _ = self.ensure_network_tracking();
        let out = resolve_state_path(path, session)?;
        ensure_parent_dir(&out)?;

        let cookies = self
            .send_command("Network.getAllCookies", json!({}))?
            .get("cookies")
            .cloned()
            .unwrap_or(json!([]));

        let storage = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": r#"(function() {
                    const dump = (store) => {
                        const out = {};
                        try {
                            for (let i = 0; i < store.length; i++) {
                                const k = store.key(i);
                                out[k] = store.getItem(k);
                            }
                        } catch (_) {}
                        return out;
                    };
                    return {
                        origin: location.origin,
                        url: location.href,
                        localStorage: dump(localStorage),
                        sessionStorage: dump(sessionStorage),
                    };
                })()"#,
                "returnByValue": true,
            }),
        )?;
        let storage_val = storage
            .pointer("/result/value")
            .cloned()
            .unwrap_or(json!({}));

        let payload = json!({
            "version": 1,
            "saved_at": unix_timestamp(),
            "cookies": cookies,
            "origin": storage_val.get("origin"),
            "url": storage_val.get("url"),
            "localStorage": storage_val.get("localStorage").cloned().unwrap_or(json!({})),
            "sessionStorage": storage_val.get("sessionStorage").cloned().unwrap_or(json!({})),
        });
        std::fs::write(&out, serde_json::to_vec_pretty(&payload)?)
            .with_context(|| format!("write state {}", out.display()))?;

        Ok(json!({
            "status": "saved",
            "path": out,
            "cookies": cookies.as_array().map(|a| a.len()).unwrap_or(0),
        }))
    }

    /// Restore cookies + storage from a previous `state_save`.
    pub fn state_load(&mut self, path: Option<&str>, session: Option<&str>) -> Result<Value> {
        let _ = self.ensure_network_tracking();
        let path = resolve_state_path(path, session)?;
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("read state {}", path.display()))?;
        let payload: Value = serde_json::from_str(&raw)
            .with_context(|| format!("parse state {}", path.display()))?;

        if let Some(cookies) = payload.get("cookies").and_then(Value::as_array) {
            let _ = self.send_command("Network.clearBrowserCookies", json!({}));
            if !cookies.is_empty() {
                self.send_command("Network.setCookies", json!({ "cookies": cookies }))?;
            }
        }

        let local = payload.get("localStorage").cloned().unwrap_or(json!({}));
        let session_store = payload.get("sessionStorage").cloned().unwrap_or(json!({}));
        self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    r#"(function() {{
                        const local = {local};
                        const session = {session};
                        try {{ localStorage.clear(); }} catch (_) {{}}
                        try {{ sessionStorage.clear(); }} catch (_) {{}}
                        for (const [k, v] of Object.entries(local || {{}})) {{
                            try {{ localStorage.setItem(k, String(v)); }} catch (_) {{}}
                        }}
                        for (const [k, v] of Object.entries(session || {{}})) {{
                            try {{ sessionStorage.setItem(k, String(v)); }} catch (_) {{}}
                        }}
                        return {{
                            localKeys: Object.keys(local || {{}}).length,
                            sessionKeys: Object.keys(session || {{}}).length,
                        }};
                    }})()"#,
                    local = local,
                    session = session_store,
                ),
                "returnByValue": true,
            }),
        )?;

        Ok(json!({
            "status": "loaded",
            "path": path,
            "url": payload.get("url"),
            "hint": "If the page was already open, reload or navigate so the app picks up storage/cookies.",
        }))
    }

    fn target_present(&mut self, target: &str) -> Result<bool> {
        let expression = if let Some(idx) = parse_ref_index(target) {
            format!(
                r#"(function() {{
                    const el = window.__agentDoctorRefs && window.__agentDoctorRefs[{idx}];
                    return !!(el && document.contains(el));
                }})()"#,
                idx = idx
            )
        } else {
            format!(
                r#"(function() {{ return !!document.querySelector({s:?}); }})()"#,
                s = target
            )
        };
        let result = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
            }),
        )?;
        Ok(result.pointer("/result/value") == Some(&json!(true)))
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
        let switched = self.switch_tab(&target_id)?;
        Ok(json!({
            "target_id": target_id,
            "url": url,
            "status": switched.get("status"),
        }))
    }

    /// Switch to a browser tab by target ID.
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

/// Match URL with exact, substring, or simple `*` / `**` wildcards.
fn url_matches(current: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    if current == pattern || current.contains(pattern) {
        return true;
    }
    wildcard_match(current, pattern)
}

fn wildcard_match(text: &str, pattern: &str) -> bool {
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
}
