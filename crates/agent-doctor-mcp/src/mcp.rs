use std::io::{self, BufRead, Read, Write};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::tools::BrowserContext;

/// A tool definition exposed by this MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
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

/// Run the MCP server over stdio.
///
/// Reads JSON-RPC requests from stdin, dispatches to a shared `BrowserContext`,
/// and writes responses to stdout.
pub fn run_mcp_server(browser: &Mutex<BrowserContext>) -> Result<()> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut buffer = String::new();
    let mut content_length: Option<usize> = None;

    loop {
        buffer.clear();
        if reader.read_line(&mut buffer)? == 0 {
            return Ok(());
        }

        let line = buffer.trim().to_string();

        if line.is_empty() {
            let len = content_length
                .take()
                .context("Missing Content-Length header in MCP request")?;
            let mut body = vec![0u8; len];
            reader
                .read_exact(&mut body)
                .context("Failed to read MCP request body")?;

            let body_str = String::from_utf8_lossy(&body);
            let request: McpRequest = match serde_json::from_str(&body_str) {
                Ok(req) => req,
                Err(e) => {
                    let resp = McpResponse {
                        id: None,
                        result: None,
                        error: Some(McpError {
                            code: -32700,
                            message: format!("Parse error: {e}"),
                            data: None,
                        }),
                    };
                    write_response(&resp)?;
                    continue;
                }
            };

            match handle_request(&request, browser)? {
                HandleResult::Respond(resp) => write_response(&resp)?,
                HandleResult::Shutdown => return Ok(()),
                HandleResult::NoResponse => {} // notifications
            }
        } else if let Some(len_str) = line
            .strip_prefix("Content-Length:")
            .or_else(|| line.strip_prefix("content-length:"))
        {
            content_length = Some(len_str.trim().parse().context("Invalid Content-Length")?);
        }
    }
}

fn handle_request(request: &McpRequest, browser: &Mutex<BrowserContext>) -> Result<HandleResult> {
    let id = request.id.clone();

    let response = match request.method.as_str() {
        "initialize" => McpResponse {
            id,
            result: Some(json!({
                "protocolVersion": "0.1.0",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "agent-doctor-browser",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
            error: None,
        },
        // Notifications have no 'id' — we just ignore them
        "notifications/initialized" | "initialized" => {
            return Ok(HandleResult::NoResponse);
        }
        "tools/list" => {
            let tools = tool_definitions();
            McpResponse {
                id,
                result: Some(json!({ "tools": tools })),
                error: None,
            }
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
                    McpResponse {
                        id,
                        result: Some(json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": text,
                                }
                            ],
                            "isError": false,
                        })),
                        error: None,
                    }
                }
                Err(e) => McpResponse {
                    id,
                    result: Some(json!({
                        "content": [
                            {
                                "type": "text",
                                "text": format!("{e:#}"),
                            }
                        ],
                        "isError": true,
                    })),
                    error: None,
                },
            }
        }
        "shutdown" => {
            return Ok(HandleResult::Shutdown);
        }
        "exit" => {
            return Ok(HandleResult::Shutdown);
        }
        _ => McpResponse {
            id,
            result: None,
            error: Some(McpError {
                code: -32601,
                message: format!("Method not found: {}", request.method),
                data: None,
            }),
        },
    };

    Ok(HandleResult::Respond(response))
}

fn execute_tool(name: &str, args: &Value, browser: &Mutex<BrowserContext>) -> Result<Value> {
    let mut ctx = browser.lock().unwrap();

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

fn write_response(response: &McpResponse) -> Result<()> {
    let body = serde_json::to_string(response)?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    out.flush()?;
    Ok(())
}
