use super::BrowserContext;
use anyhow::Result;
use serde_json::{json, Value};
use std::time::Duration;

impl BrowserContext {
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
}
