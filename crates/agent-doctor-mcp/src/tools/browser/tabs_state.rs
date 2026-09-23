use super::{page_ws_for_target, parse_ref_index, unix_timestamp, BrowserContext};
use crate::state::{ensure_parent_dir, resolve_state_path};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::time::Duration;

impl BrowserContext {
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

    pub(crate) fn target_present(&mut self, target: &str) -> Result<bool> {
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
