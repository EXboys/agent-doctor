use super::{url_matches, BrowserContext};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

impl BrowserContext {
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
}
