use super::{resolve_screenshot_path, write_screenshot_png, BrowserContext};
use anyhow::{Context, Result};
use serde_json::{json, Value};

impl BrowserContext {
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

    /// Screenshot result for MCP: default writes PNG and returns `{ path }`.
    ///
    /// Set `inline` to return `{ data }` base64 (can blow past context limits).
    pub fn screenshot_result(&mut self, path: Option<&str>, inline: bool) -> Result<Value> {
        let data = self.screenshot()?;
        if inline {
            return Ok(json!({ "data": data }));
        }
        let out = resolve_screenshot_path(path);
        write_screenshot_png(&data, &out)?;
        let bytes = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        Ok(json!({
            "path": out,
            "bytes": bytes,
            "format": "png",
        }))
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
}
