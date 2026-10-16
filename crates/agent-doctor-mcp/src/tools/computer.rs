//! Desktop computer tools. The model calls these the same way it calls browser tools.
//!
//! The desktop app does the click. macOS talks over a local socket. Windows talks
//! over a named pipe. Each `screen_snapshot` numbers the front window's controls as `@eN`.

#[cfg(unix)]
use std::io::BufReader;
use std::io::{self, BufRead, Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::mcp::{HandleResult, McpRequest, McpResponse, ToolDefinition};

struct Target {
    x: f64,
    y: f64,
    label: String,
}

static TARGETS: Mutex<Vec<Target>> = Mutex::new(Vec::new());

pub fn run_computer_mcp_server() -> Result<()> {
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
            if !dispatch(&String::from_utf8_lossy(&body), true)? {
                return Ok(());
            }
        } else if let Some(len_str) = line
            .strip_prefix("Content-Length:")
            .or_else(|| line.strip_prefix("content-length:"))
        {
            content_length = Some(len_str.trim().parse().context("Invalid Content-Length")?);
        } else if line.starts_with('{') && !dispatch(&line, false)? {
            return Ok(());
        }
    }
}

fn dispatch(body: &str, content_length: bool) -> Result<bool> {
    let request: McpRequest = match serde_json::from_str(body) {
        Ok(req) => req,
        Err(e) => {
            write_response(
                &err_response(None, -32700, format!("Parse error: {e}")),
                content_length,
            )?;
            return Ok(true);
        }
    };
    match handle(&request)? {
        HandleResult::Respond(resp) => write_response(&resp, content_length)?,
        HandleResult::Shutdown => return Ok(false),
        HandleResult::NoResponse => {}
    }
    Ok(true)
}

fn handle(request: &McpRequest) -> Result<HandleResult> {
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
                        "name": "agent-doctor-computer",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
        }
        "notifications/initialized" | "initialized" => return Ok(HandleResult::NoResponse),
        "tools/list" => ok_response(id, json!({ "tools": tool_definitions() })),
        "resources/list" => ok_response(id, json!({ "resources": [] })),
        "resources/templates/list" => ok_response(id, json!({ "resourceTemplates": [] })),
        "prompts/list" => ok_response(id, json!({ "prompts": [] })),
        "ping" => ok_response(id, json!({})),
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
            match execute(tool_name, &args) {
                Ok(result) => ok_response(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&result)? }],
                        "isError": false,
                    }),
                ),
                Err(e) => ok_response(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": format!("{e:#}") }],
                        "isError": true,
                    }),
                ),
            }
        }
        "shutdown" | "exit" => return Ok(HandleResult::Shutdown),
        _ if request.method.starts_with("notifications/") => return Ok(HandleResult::NoResponse),
        _ => err_response(id, -32601, format!("Method not found: {}", request.method)),
    };
    Ok(HandleResult::Respond(response))
}

fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "screen_apps".into(),
            description: "List the apps open on this computer.".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDefinition {
            name: "screen_snapshot".into(),
            description: "See the screen. Returns a PNG path plus point size. Read the PNG. Click x,y are points: divide image pixels by scale.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "App name. Empty uses the front app." }
                }
            }),
        },
        ToolDefinition {
            name: "screen_click".into(),
            description: "Click a control from the latest screen_snapshot, or a point.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "ref": { "type": "string", "description": "@eN from screen_snapshot" },
                    "x": { "type": "number" },
                    "y": { "type": "number" }
                }
            }),
        },
        ToolDefinition {
            name: "screen_type".into(),
            description: "Type text into the focused control. Pass ref to click that field first.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string" },
                    "ref": { "type": "string" }
                },
                "required": ["text"]
            }),
        },
        ToolDefinition {
            name: "screen_press".into(),
            description: "Press a key: return, escape, tab, space, up, down, left, right, or command+a style.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "key": { "type": "string" } },
                "required": ["key"]
            }),
        },
    ]
}

fn execute(name: &str, args: &Value) -> Result<Value> {
    match name {
        "screen_apps" => screen_apps(),
        "screen_snapshot" => screen_snapshot(args.get("app").and_then(Value::as_str).unwrap_or("")),
        "screen_click" => screen_click(args),
        "screen_type" => screen_type(args),
        "screen_press" => screen_press(args.get("key").and_then(Value::as_str).unwrap_or("")),
        _ => bail!("Unknown tool: {name}"),
    }
}

fn screen_apps() -> Result<Value> {
    ask_hand("screen_apps", &json!({}))
}

fn screen_snapshot(app: &str) -> Result<Value> {
    let mut result = ask_hand("screen_look", &json!({ "app": app }))?;
    let raw = result
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let (text, targets) = number_controls(&raw);
    *TARGETS.lock().unwrap_or_else(|e| e.into_inner()) = targets;
    if let Some(obj) = result.as_object_mut() {
        obj.insert("text".into(), json!(text));
    }
    Ok(result)
}

fn number_controls(text: &str) -> (String, Vec<Target>) {
    let mut targets = Vec::new();
    let mut lines = Vec::new();
    for line in text.lines() {
        let mut parts = line.split('\t');
        let role = parts.next().unwrap_or("").trim();
        if !matches!(role, "button" | "text field" | "text") {
            lines.push(line.to_string());
            continue;
        }
        let label = parts.next().unwrap_or("").trim().to_string();
        let x: f64 = parts
            .next()
            .unwrap_or("")
            .trim()
            .parse()
            .unwrap_or(f64::NAN);
        let y: f64 = parts
            .next()
            .unwrap_or("")
            .trim()
            .parse()
            .unwrap_or(f64::NAN);
        if label.is_empty() || !x.is_finite() || !y.is_finite() {
            lines.push(line.to_string());
            continue;
        }
        targets.push(Target {
            x,
            y,
            label: label.clone(),
        });
        lines.push(format!("@e{} {role} {label} {x} {y}", targets.len()));
    }
    (lines.join("\n"), targets)
}

fn ask_hand(tool: &str, args: &Value) -> Result<Value> {
    let line = json!({ "tool": tool, "args": args }).to_string();
    #[cfg(unix)]
    {
        let reply = ask_hand_unix(&line)?;
        parse_hand_reply(&reply)
    }
    #[cfg(windows)]
    {
        let reply = ask_hand_windows(&line)?;
        parse_hand_reply(&reply)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = line;
        bail!("clicking and typing run on macOS");
    }
}

fn parse_hand_reply(line: &str) -> Result<Value> {
    let reply: Value = serde_json::from_str(line).context("empty reply from the app")?;
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(reply.get("result").cloned().unwrap_or(json!({})))
    } else {
        bail!(
            "{}",
            reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("the click did not go through")
        );
    }
}

#[cfg(unix)]
fn ask_hand_unix(line: &str) -> Result<String> {
    let dir = agent_doctor_core::store::agent_doctor_config_dir()
        .context("could not find the app config folder")?;
    let path = dir.join("computer.sock");
    let stream = UnixStream::connect(&path).context(
        "the app is not ready to click. Keep Agent Doctor open, and allow it to control this computer.",
    )?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(8)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(8)))?;
    let mut writer = stream.try_clone()?;
    writeln!(writer, "{line}")?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    Ok(reply)
}

#[cfg(windows)]
fn ask_hand_windows(line: &str) -> Result<String> {
    use std::time::{Duration, Instant};
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_MODE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Pipes::PeekNamedPipe;

    let name = HSTRING::from(agent_doctor_core::COMPUTER_HAND_PIPE);
    let pipe = unsafe {
        CreateFileW(
            &name,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_MODE(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0),
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            HANDLE::default(),
        )
    }
    .context(
        "the app is not ready to click. Keep Agent Doctor open, and allow it to control this computer.",
    )?;
    let body = format!("{line}\n");
    let mut written = 0u32;
    let wrote = unsafe { WriteFile(pipe, Some(body.as_bytes()), Some(&mut written), None) };
    if let Err(err) = wrote {
        unsafe {
            let _ = CloseHandle(pipe);
        }
        return Err(err).context("the click did not go through");
    }
    let mut reply = Vec::new();
    let mut buf = [0u8; 4096];
    let started = Instant::now();
    let result = (|| {
        while !reply.contains(&b'\n') && reply.len() < 1024 * 1024 {
            if started.elapsed() > Duration::from_secs(8) {
                bail!("the click did not go through");
            }
            let mut available = 0u32;
            unsafe { PeekNamedPipe(pipe, None, 0, None, Some(&mut available), None) }
                .context("empty reply from the app")?;
            if available == 0 {
                std::thread::sleep(Duration::from_millis(40));
                continue;
            }
            let mut read = 0u32;
            unsafe { ReadFile(pipe, Some(&mut buf), Some(&mut read), None) }
                .context("empty reply from the app")?;
            if read == 0 {
                break;
            }
            reply.extend_from_slice(&buf[..read as usize]);
        }
        Ok(String::from_utf8_lossy(&reply).to_string())
    })();
    unsafe {
        let _ = CloseHandle(pipe);
    }
    result
}

fn screen_click(args: &Value) -> Result<Value> {
    if let (Some(x), Some(y)) = (
        args.get("x").and_then(Value::as_f64),
        args.get("y").and_then(Value::as_f64),
    ) {
        return ask_hand("screen_click", &json!({ "x": x, "y": y }));
    }
    let refer = args.get("ref").and_then(Value::as_str).unwrap_or("");
    let (x, y, label) = lookup(refer)?;
    ask_hand("screen_click", &json!({ "x": x, "y": y }))?;
    Ok(json!({ "clicked": refer, "label": label }))
}

fn screen_type(args: &Value) -> Result<Value> {
    let text = args.get("text").and_then(Value::as_str).unwrap_or("");
    if text.is_empty() {
        bail!("text is empty");
    }
    if let Some(refer) = args
        .get("ref")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        screen_click(&json!({ "ref": refer }))?;
    }
    ask_hand("screen_type", &json!({ "text": text }))
}

fn screen_press(key: &str) -> Result<Value> {
    let key = key.trim().to_ascii_lowercase();
    if key.is_empty() {
        bail!("key is empty");
    }
    ask_hand("screen_press", &json!({ "key": key }))
}

fn lookup(refer: &str) -> Result<(f64, f64, String)> {
    let n: usize = refer
        .trim()
        .trim_start_matches('@')
        .trim_start_matches('e')
        .trim_start_matches('E')
        .parse()
        .context("ref must look like @e1 from screen_snapshot")?;
    let guard = TARGETS.lock().unwrap_or_else(|e| e.into_inner());
    let target = guard
        .get(n.checked_sub(1).unwrap_or(usize::MAX))
        .context("that ref is stale. Call screen_snapshot again.")?;
    Ok((target.x, target.y, target.label.clone()))
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
    use crate::mcp::McpError;
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

fn write_response(response: &McpResponse, content_length: bool) -> Result<()> {
    let body = serde_json::to_string(response)?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if content_length {
        write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    } else {
        writeln!(out, "{body}")?;
    }
    out.flush()?;
    Ok(())
}
