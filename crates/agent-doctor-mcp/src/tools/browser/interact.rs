use super::{js_bind_el, BrowserContext};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::time::Duration;

impl BrowserContext {
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
}
