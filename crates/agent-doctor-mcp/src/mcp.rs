use std::io::{self, BufRead, Read, Write};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::session::SharedBrowser;

/// A tool definition exposed by this MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// Incoming MCP request (JSON-RPC).
#[derive(Debug, Deserialize)]
pub struct McpRequest {
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// Outgoing MCP response (JSON-RPC).
#[derive(Debug, Serialize)]
pub struct McpResponse {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
}

fn ok_response(id: Option<Value>, result: Value) -> McpResponse {
    McpResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }
}

fn err_response(id: Option<Value>, code: i64, message: String) -> McpResponse {
    McpResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(McpError {
            code,
            message,
            data: None,
        }),
    }
}

#[derive(Debug, Serialize)]
pub struct McpError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Result of handling a single MCP request.
pub enum HandleResult {
    Respond(McpResponse),
    Shutdown,
    NoResponse, // For notifications
}

/// The tools exposed by this MCP server.
fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "browser_navigate".into(),
            description: "Navigate to a URL and wait for the page to load".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to navigate to"
                    }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "browser_click".into(),
            description: "Click an element identified by CSS selector".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for the element to click"
                    }
                },
                "required": ["selector"]
            }),
        },
        ToolDefinition {
            name: "browser_type".into(),
            description: "Type text into an element identified by CSS selector".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for the input element"
                    },
                    "text": {
                        "type": "string",
                        "description": "Text to type"
                    }
                },
                "required": ["selector", "text"]
            }),
        },
        ToolDefinition {
            name: "browser_screenshot".into(),
            description: "Take a screenshot of the current page (returns base64 PNG)".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_get_text".into(),
            description: "Get the visible text content of the current page".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_get_url".into(),
            description: "Get the current page URL".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_get_html".into(),
            description: "Get HTML content of a specific element or full page".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "Optional CSS selector. If omitted, returns full page HTML"
                    }
                }
            }),
        },
        ToolDefinition {
            name: "browser_evaluate".into(),
            description: "Execute arbitrary JavaScript in the page context".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "script": {
                        "type": "string",
                        "description": "JavaScript code to execute"
                    }
                },
                "required": ["script"]
            }),
        },
        ToolDefinition {
            name: "browser_scroll".into(),
            description: "Scroll the page by the given delta".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "delta_x": {
                        "type": "number",
                        "description": "Horizontal scroll delta",
                        "default": 0
                    },
                    "delta_y": {
                        "type": "number",
                        "description": "Vertical scroll delta",
                        "default": 300
                    }
                }
            }),
        },
        ToolDefinition {
            name: "browser_wait".into(),
            description: "Wait for an element to appear on the page".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector to wait for"
                    },
                    "timeout_ms": {
                        "type": "number",
                        "description": "Maximum wait time in milliseconds",
                        "default": 10000
                    }
                },
                "required": ["selector"]
            }),
        },
        ToolDefinition {
            name: "browser_get_links".into(),
            description: "Get all links on the current page".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_new_tab".into(),
            description: "Open a new browser tab and navigate to a URL".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "The URL to open in the new tab"}
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "browser_switch_tab".into(),
            description: "Switch to a different browser tab by target ID".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "target_id": {"type": "string", "description": "Target ID from list_tabs"}
                },
                "required": ["target_id"]
            }),
        },
        ToolDefinition {
            name: "browser_close_tab".into(),
            description: "Close a browser tab by target ID".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "target_id": {"type": "string", "description": "Target ID of the tab to close"}
                },
                "required": ["target_id"]
            }),
        },
        ToolDefinition {
            name: "browser_list_tabs".into(),
            description: "List all open browser tabs with target IDs, URLs, and titles".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
    ]
}

fn debug_log(msg: impl AsRef<str>) {
    if let Ok(path) = std::env::var("AGENT_DOCTOR_MCP_DEBUG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{}", msg.as_ref());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framing {
    /// LSP-style `Content-Length` headers (Codex / official MCP SDK).
    ContentLength,
    /// Newline-delimited JSON-RPC (Claude Code).
    Ndjson,
}

/// Run the MCP server over stdio.
///
/// Handshake/`tools/list` respond immediately; Chrome is started on first tool call.
pub fn run_mcp_server(browser: &SharedBrowser) -> Result<()> {
    debug_log(format!("run_mcp_server start pid={}", std::process::id()));
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut buffer = String::new();
    let mut content_length: Option<usize> = None;

    loop {
        buffer.clear();
        if reader.read_line(&mut buffer)? == 0 {
            debug_log("stdin EOF");
            return Ok(());
        }

        let line = buffer.trim().to_string();
        debug_log(format!("stdin line: {line:?}"));

        if line.is_empty() {
            let len = content_length
                .take()
                .context("Missing Content-Length header in MCP request")?;
            let mut body = vec![0u8; len];
            reader
                .read_exact(&mut body)
                .context("Failed to read MCP request body")?;
            debug_log(format!("body: {}", String::from_utf8_lossy(&body)));
            if !dispatch_raw(
                &String::from_utf8_lossy(&body),
                browser,
                Framing::ContentLength,
            )? {
                return Ok(());
            }
        } else if let Some(len_str) = line
            .strip_prefix("Content-Length:")
            .or_else(|| line.strip_prefix("content-length:"))
        {
            content_length = Some(len_str.trim().parse().context("Invalid Content-Length")?);
        } else if line.starts_with('{') {
            // Claude Code speaks newline-delimited JSON-RPC (no Content-Length).
            if !dispatch_raw(&line, browser, Framing::Ndjson)? {
                return Ok(());
            }
        }
    }
}

/// Returns `false` when the server should shut down.
fn dispatch_raw(body: &str, browser: &SharedBrowser, framing: Framing) -> Result<bool> {
    let request: McpRequest = match serde_json::from_str(body) {
        Ok(req) => req,
        Err(e) => {
            write_response(
                &err_response(None, -32700, format!("Parse error: {e}")),
                framing,
            )?;
            return Ok(true);
        }
    };

    match handle_request(&request, browser)? {
        HandleResult::Respond(resp) => write_response(&resp, framing)?,
        HandleResult::Shutdown => return Ok(false),
        HandleResult::NoResponse => {}
    }
    Ok(true)
}

fn handle_request(request: &McpRequest, browser: &SharedBrowser) -> Result<HandleResult> {
    let id = request.id.clone();

    let response = match request.method.as_str() {
        "initialize" => {
            let protocol = request
                .params
                .as_ref()
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or("2024-11-05");
            ok_response(
                id,
                json!({
                    "protocolVersion": protocol,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "agent-doctor-browser",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
        }
        // Notifications have no 'id' — we just ignore them
        "notifications/initialized" | "initialized" => {
            return Ok(HandleResult::NoResponse);
        }
        "tools/list" => {
            let tools = tool_definitions();
            ok_response(id, json!({ "tools": tools }))
        }
        "tools/call" => {
            let tool_name = request
                .params
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = request
                .params
                .as_ref()
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or(json!({}));

            match execute_tool(tool_name, &args, browser) {
                Ok(result) => {
                    let text = serde_json::to_string_pretty(&result)?;
                    ok_response(
                        id,
                        json!({
                            "content": [{ "type": "text", "text": text }],
                            "isError": false,
                        }),
                    )
                }
                Err(e) => ok_response(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": format!("{e:#}") }],
                        "isError": true,
                    }),
                ),
            }
        }
        "shutdown" | "exit" => {
            return Ok(HandleResult::Shutdown);
        }
        _ => err_response(id, -32601, format!("Method not found: {}", request.method)),
    };

    Ok(HandleResult::Respond(response))
}

fn execute_tool(name: &str, args: &Value, browser: &SharedBrowser) -> Result<Value> {
    let mut session = browser.lock().unwrap();
    let ctx = session.ensure()?;

    match name {
        "browser_navigate" => {
            let url = args
                .get("url")
                .and_then(Value::as_str)
                .context("Missing 'url' argument")?;
            ctx.navigate(url)
        }
        "browser_click" => {
            let selector = args
                .get("selector")
                .and_then(Value::as_str)
                .context("Missing 'selector' argument")?;
            ctx.click(selector)
        }
        "browser_type" => {
            let selector = args
                .get("selector")
                .and_then(Value::as_str)
                .context("Missing 'selector' argument")?;
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .context("Missing 'text' argument")?;
            ctx.type_text(selector, text)
        }
        "browser_screenshot" => {
            let data = ctx.screenshot()?;
            Ok(json!({ "data": data }))
        }
        "browser_get_text" => {
            let text = ctx.get_text()?;
            Ok(json!({ "text": text }))
        }
        "browser_get_url" => {
            let url = ctx.get_url()?;
            Ok(json!({ "url": url }))
        }
        "browser_get_html" => {
            let selector = args.get("selector").and_then(Value::as_str);
            let html = ctx.get_html(selector)?;
            Ok(json!({ "html": html }))
        }
        "browser_evaluate" => {
            let script = args
                .get("script")
                .and_then(Value::as_str)
                .context("Missing 'script' argument")?;
            ctx.evaluate(script)
        }
        "browser_scroll" => {
            let delta_x = args.get("delta_x").and_then(Value::as_f64).unwrap_or(0.0);
            let delta_y = args.get("delta_y").and_then(Value::as_f64).unwrap_or(300.0);
            ctx.scroll(delta_x, delta_y)
        }
        "browser_wait" => {
            let selector = args
                .get("selector")
                .and_then(Value::as_str)
                .context("Missing 'selector' argument")?;
            let timeout = args
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(10000);
            ctx.wait_for_selector(selector, timeout)
        }
        "browser_get_links" => ctx.get_links(),
        "browser_new_tab" => {
            let url = args
                .get("url")
                .and_then(Value::as_str)
                .context("Missing url")?;
            ctx.new_tab(url)
        }
        "browser_switch_tab" => {
            let tid = args
                .get("target_id")
                .and_then(Value::as_str)
                .context("Missing target_id")?;
            ctx.switch_tab(tid)
        }
        "browser_close_tab" => {
            let tid = args
                .get("target_id")
                .and_then(Value::as_str)
                .context("Missing target_id")?;
            ctx.close_tab(tid)
        }
        "browser_list_tabs" => ctx.list_tabs(),
        _ => anyhow::bail!("Unknown tool: {name}"),
    }
}

fn write_response(response: &McpResponse, framing: Framing) -> Result<()> {
    let body = serde_json::to_string(response)?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    match framing {
        Framing::ContentLength => {
            write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
        }
        Framing::Ndjson => {
            // Claude Code expects one JSON-RPC object per line.
            writeln!(out, "{body}")?;
        }
    }
    out.flush()?;
    debug_log(format!("wrote ({framing:?}): {body}"));
    Ok(())
}
