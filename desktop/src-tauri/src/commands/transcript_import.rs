//! Import existing conversations from external agent CLIs into chat sessions.
//!
//! One source per agent runtime, keyed by the runtime id the chat store
//! already uses (`claude-code`, `codex`): each source knows where that CLI
//! keeps its transcripts, how to project one into the desktop chat message
//! shapes, and which session/thread id its runtime can resume.
//!
//! Claude Code: `~/.claude/projects/**.jsonl`. The jsonl lines carry the full
//! API transcript: `user` rows hold prompts and `tool_result` blocks,
//! `assistant` rows hold `thinking`/`text`/`tool_use` blocks, `summary` rows
//! hold titles. Everything else (queue ops, environment snapshots, cost
//! states, sidechains) is Claude Code bookkeeping and is skipped.
//!
//! Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. `session_meta`
//! carries the thread id; `response_item` payloads carry `message` rows.
//! Only text is projected — Codex's rollout format interleaves reasoning and
//! function-call items in shapes that have changed across CLI versions, and
//! a resume replays the original thread server-side anyway, so imported rows
//! are the readable conversation, not a forensic reconstruction.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

const MAX_MESSAGES: usize = 2_000;
const MAX_TEXT_CHARS: usize = 24_000;
const MAX_TOOL_OUTPUT_CHARS: usize = 8_000;
const MAX_TOOL_INPUT_CHARS: usize = 4_000;
const HEAD_SCAN_BYTES: u64 = 256 * 1024;

/// Text the harness (Ask wiring or the CLI itself) injects before the
/// person's own words. It must never become a title, a preview, or an
/// imported visible message — sessions made of nothing else are the app's
/// own Ask turns and are filtered out of the lists entirely.
const INJECTED_MARKERS: [&str; 7] = [
    "BROWSER MCP REQUIRED",
    "COMPUTER MCP REQUIRED",
    "<user_instructions>",
    "<environment_context>",
    "<ENVIRONMENT_CONTEXT>",
    "<turn_context>",
    "<user_shell_command>",
];

fn injected_user_text(text: &str) -> bool {
    let trimmed = text.trim_start();
    INJECTED_MARKERS
        .iter()
        .any(|marker| trimmed.starts_with(marker))
}

#[derive(Serialize)]
pub struct ImportCandidate {
    pub path: String,
    pub session_id: String,
    pub title: String,
    pub preview: String,
    pub updated_at: u64,
    pub size_bytes: u64,
}

#[derive(Serialize)]
pub struct ImportPayload {
    pub session_id: String,
    pub title: String,
    pub truncated: bool,
    pub messages: Vec<Value>,
}

/// Scan a source's transcript store for importable sessions. Only the head of
/// each file is read, so a shelf of multi-megabyte transcripts still lists
/// quickly.
#[tauri::command]
pub async fn list_import_sessions_command(source: String) -> Result<Vec<ImportCandidate>, String> {
    tauri::async_runtime::spawn_blocking(move || match source.as_str() {
        "claude-code" => list_claude_sessions(),
        "codex" => list_codex_sessions(),
        other => Err(format!("未知的导入来源：{other}")),
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Project one transcript into chat-store messages.
#[tauri::command]
pub async fn import_session_command(source: String, path: String) -> Result<ImportPayload, String> {
    tauri::async_runtime::spawn_blocking(move || match source.as_str() {
        "claude-code" => import_claude_session_sync(&path),
        "codex" => import_codex_session_sync(&path),
        other => Err(format!("未知的导入来源：{other}")),
    })
    .await
    .map_err(|error| error.to_string())?
}

fn home_dir(note: &str) -> Result<PathBuf, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| note.to_string())?;
    Ok(PathBuf::from(home))
}

fn file_row(
    path: &Path,
    session_id: String,
    title: Option<String>,
    preview: String,
) -> ImportCandidate {
    let meta = fs::metadata(path).ok();
    ImportCandidate {
        path: path.display().to_string(),
        session_id,
        title: title.unwrap_or_else(|| preview.clone()),
        preview,
        updated_at: meta
            .as_ref()
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|delta| delta.as_millis() as u64)
            .unwrap_or(0),
        size_bytes: meta.map(|meta| meta.len()).unwrap_or(0),
    }
}

// ---------------------------------------------------------------------------
// Claude Code source
// ---------------------------------------------------------------------------

fn list_claude_sessions() -> Result<Vec<ImportCandidate>, String> {
    let projects = home_dir("找不到你的个人文件夹，无法读取 Claude Code 会话。")
        .map(|home| home.join(".claude").join("projects"))?;
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&projects) else {
        return Ok(out);
    };
    for project in entries.flatten() {
        let Ok(files) = fs::read_dir(project.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            let session_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("")
                .to_string();
            if session_id.is_empty() {
                continue;
            }
            // A transcript whose only words are the harness's injected wiring
            // prefixes is one of this app's own Ask turns: already managed
            // here, and its title would be noise. Skip it.
            let Some((title, preview)) = scan_claude_head(&path, HEAD_SCAN_BYTES) else {
                continue;
            };
            out.push(file_row(&path, session_id, title, preview));
        }
    }
    out.sort_by_key(|candidate| std::cmp::Reverse(candidate.updated_at));
    Ok(out)
}

/// First title and first human user text within the head of one transcript.
/// `None` when the head holds only injected harness text: that transcript is
/// one of this app's own Ask turns and must not offer itself for import.
fn scan_claude_head(path: &Path, max_bytes: u64) -> Option<(Option<String>, String)> {
    let head = read_head_lines(path, max_bytes);
    let mut title = None;
    let mut preview = String::new();
    for line in head {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("summary") => {
                if title.is_none() {
                    title = value
                        .get("summary")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
            }
            Some("user") if preview.is_empty() && !is_true(&value, "isMeta") => {
                let text = user_text(&value).unwrap_or_default();
                if !injected_user_text(&text) {
                    preview = text;
                }
            }
            _ => {}
        }
        if title.is_some() && !preview.is_empty() {
            break;
        }
    }
    if title.is_none() && preview.is_empty() {
        return None;
    }
    Some((title, clip(&preview, 120)))
}

fn import_claude_session_sync(path: &str) -> Result<ImportPayload, String> {
    let file = fs::File::open(path).map_err(|error| format!("打不开这份会话记录：{error}"))?;
    let mut title = String::new();
    let mut messages: Vec<Value> = Vec::new();
    let mut truncated = false;
    let mut seq = 0usize;

    let reader = BufReader::new(file);
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if is_true(&value, "isSidechain") || is_true(&value, "isMeta") {
            continue;
        }
        let at = timestamp_ms(&value).unwrap_or(0);
        match value.get("type").and_then(Value::as_str) {
            Some("summary") => {
                if title.is_empty() {
                    title = value
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                }
            }
            Some("user") => {
                for message in user_messages(&value, at, &mut seq) {
                    push_message(&mut messages, &mut truncated, message);
                }
            }
            Some("assistant") => {
                for message in assistant_messages(&value, at, &mut seq) {
                    push_message(&mut messages, &mut truncated, message);
                }
            }
            _ => {}
        }
    }

    let session_id = Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .to_string();
    if title.is_empty() {
        title =
            first_human_user_text(&messages).unwrap_or_else(|| "导入的 Claude Code 会话".into());
    }
    Ok(ImportPayload {
        session_id,
        title: clip(&title, 60),
        truncated,
        messages,
    })
}

// ---------------------------------------------------------------------------
// Codex source
// ---------------------------------------------------------------------------

fn codex_sessions_dir() -> Result<PathBuf, String> {
    home_dir("找不到你的个人文件夹，无法读取 Codex 会话。")
        .map(|home| home.join(".codex").join("sessions"))
}

fn list_codex_sessions() -> Result<Vec<ImportCandidate>, String> {
    let root = codex_sessions_dir()?;
    let mut out = Vec::new();
    collect_codex_files(&root, 0, &mut out);
    out.sort_by_key(|candidate| std::cmp::Reverse(candidate.updated_at));
    Ok(out)
}

/// The store nests one `rollout-*.jsonl` per session under `YYYY/MM/DD`.
fn collect_codex_files(dir: &Path, depth: usize, out: &mut Vec<ImportCandidate>) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_codex_files(&path, depth + 1, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            let Some(candidate) = scan_codex_head(&path, HEAD_SCAN_BYTES) else {
                continue;
            };
            out.push(candidate);
        }
    }
}

/// Thread id, title, and first human user text within the head of one
/// rollout. `None` when nothing human-turned inside the head (a scripted
/// `codex exec` run or one of this app's own wired Ask turns).
fn scan_codex_head(path: &Path, max_bytes: u64) -> Option<ImportCandidate> {
    let head = read_head_lines(path, max_bytes);
    let mut session_id = codex_thread_id_from_name(path);
    let mut preview = String::new();
    for line in head {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("response_item") {
            if session_id.is_none() {
                session_id = value
                    .pointer("/payload/session_id")
                    .or_else(|| value.pointer("/payload/id"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            continue;
        }
        if !preview.is_empty() {
            continue;
        }
        let payload = value.get("payload")?;
        if payload.get("type").and_then(Value::as_str) != Some("message")
            || payload.get("role").and_then(Value::as_str) != Some("user")
        {
            continue;
        }
        let text = codex_message_text(payload);
        if !injected_user_text(&text) {
            preview = text;
        }
    }
    if preview.is_empty() {
        return None;
    }
    let session_id = session_id?;
    Some(file_row(path, session_id, None, clip(&preview, 120)))
}

/// The rollout name ends with the thread id:
/// `rollout-2026-08-10T08-33-35-<uuid>.jsonl`.
fn codex_thread_id_from_name(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let uuid = stem.rsplit('-').take(5).collect::<Vec<_>>();
    if uuid.len() < 5 {
        return None;
    }
    // rsplit gives the segments right to left: hex, 4, 4, 8, prefix.
    let id = format!(
        "{}-{}-{}-{}-{}",
        uuid[4].rsplit('-').next().unwrap_or(""),
        uuid[3],
        uuid[2],
        uuid[1],
        uuid[0]
    );
    let hex = |segment: &str| {
        !segment.is_empty() && segment.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
    };
    (id.chars().filter(|c| *c == '-').count() == 4 && uuid.iter().all(|segment| hex(segment)))
        .then_some(id)
}

fn import_codex_session_sync(path: &str) -> Result<ImportPayload, String> {
    let file = fs::File::open(path).map_err(|error| format!("打不开这份会话记录：{error}"))?;
    let mut session_id = codex_thread_id_from_name(Path::new(path)).unwrap_or_default();
    let mut title = String::new();
    let mut messages: Vec<Value> = Vec::new();
    let mut truncated = false;
    let mut seq = 0usize;

    let reader = BufReader::new(file);
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if session_id.is_empty() {
            session_id = value
                .pointer("/payload/session_id")
                .or_else(|| value.pointer("/payload/id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
        }
        if value.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let payload = match value.get("payload") {
            Some(payload) if payload.get("type").and_then(Value::as_str) == Some("message") => {
                payload
            }
            _ => continue,
        };
        let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
        let at = timestamp_ms(&value).unwrap_or(0);
        let text = codex_message_text(payload);
        if text.trim().is_empty() {
            continue;
        }
        if role == "user" {
            if injected_user_text(&text) {
                continue;
            }
            if title.is_empty() {
                title = first_line(&text);
            }
            let id = next_id(seq);
            seq += 1;
            push_message(
                &mut messages,
                &mut truncated,
                json!({"id": id, "role": "user", "content": clip(&text, MAX_TEXT_CHARS), "at": at}),
            );
        } else if role == "assistant" {
            let id = next_id(seq);
            seq += 1;
            push_message(
                &mut messages,
                &mut truncated,
                json!({"id": id, "role": "assistant", "content": clip(&text, MAX_TEXT_CHARS), "at": at}),
            );
        }
    }

    if title.is_empty() {
        title = first_human_user_text(&messages).unwrap_or_else(|| "导入的 Codex 会话".into());
    }
    Ok(ImportPayload {
        session_id,
        title: clip(&title, 60),
        truncated,
        messages,
    })
}

/// Text of a rollout `message` payload: the joined `text` of its content items.
fn codex_message_text(payload: &Value) -> String {
    match payload.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Shared projection helpers
// ---------------------------------------------------------------------------

fn read_head_lines(path: &Path, max_bytes: u64) -> Vec<String> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut reader = BufReader::new(file);
    let mut taken: u64 = 0;
    let mut buffer = String::new();
    while taken < max_bytes {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(n) => taken += n as u64,
        }
        buffer.push_str(&line);
    }
    buffer.lines().map(str::to_string).collect()
}

fn push_message(messages: &mut Vec<Value>, truncated: &mut bool, message: Value) {
    if messages.len() >= MAX_MESSAGES {
        *truncated = true;
        return;
    }
    messages.push(message);
}

/// The person's opening line, for the fallback title when the store kept no
/// summary: the first visible user row, human text only.
fn first_human_user_text(messages: &[Value]) -> Option<String> {
    messages
        .iter()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(first_line)
}

fn next_id(seq: usize) -> String {
    format!("imp-{seq}")
}

fn is_true(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool) == Some(true)
}

/// The user's own words on one row: string content or the text blocks of a
/// content array, with injected `<system-reminder>` noise removed.
fn user_text(value: &Value) -> Option<String> {
    let content = value.pointer("/message/content");
    let mut texts: Vec<String> = Vec::new();
    if let Some(text) = content.and_then(Value::as_str) {
        texts.push(text.to_string());
    }
    for block in content_blocks(content) {
        if block.get("type").and_then(Value::as_str) == Some("tool_result") {
            continue;
        }
        if let Some(text) = block.get("text").and_then(Value::as_str) {
            texts.push(text.to_string());
        }
    }
    let text = strip_system_reminders(&texts.join("\n"));
    (!text.is_empty()).then_some(text)
}

fn user_messages(value: &Value, at: u64, seq: &mut usize) -> Vec<Value> {
    let content = value.pointer("/message/content");
    let mut out = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    for block in content_blocks(content) {
        if block.get("type").and_then(Value::as_str) == Some("tool_result") {
            let Some(tool_use_id) = block
                .get("tool_use_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let is_error = block.get("is_error").and_then(Value::as_bool) == Some(true);
            let output = block_content_text(block.get("content"));
            out.push(json!({
                "id": next_id(*seq),
                "role": "tool",
                "content": "",
                "at": at,
                "step": {
                    "id": tool_use_id,
                    "status": if is_error { "failed" } else { "done" },
                    "output": clip(&output, MAX_TOOL_OUTPUT_CHARS),
                },
            }));
            *seq += 1;
            continue;
        }
        if let Some(text) = block.get("text").and_then(Value::as_str) {
            texts.push(text.to_string());
        }
    }
    if let Some(text) = content.and_then(Value::as_str) {
        texts.push(text.to_string());
    }
    let text = strip_system_reminders(&texts.join("\n"));
    if !text.trim().is_empty() && !injected_user_text(&text) {
        let id = next_id(*seq);
        *seq += 1;
        out.push(json!({
            "id": id,
            "role": "user",
            "content": clip(&text, MAX_TEXT_CHARS),
            "at": at,
        }));
    }
    out
}

fn assistant_messages(value: &Value, at: u64, seq: &mut usize) -> Vec<Value> {
    let content = value.pointer("/message/content");
    let mut out = Vec::new();
    for block in content_blocks(content) {
        let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "thinking" => {
                let text = block
                    .get("thinking")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if text.is_empty() {
                    continue;
                }
                let id = next_id(*seq);
                *seq += 1;
                out.push(json!({
                    "id": id,
                    "role": "thinking",
                    "content": clip(&text, MAX_TEXT_CHARS),
                    "at": at,
                }));
            }
            "text" => {
                let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                if text.trim().is_empty() {
                    continue;
                }
                let id = next_id(*seq);
                *seq += 1;
                out.push(json!({
                    "id": id,
                    "role": "assistant",
                    "content": clip(text, MAX_TEXT_CHARS),
                    "at": at,
                }));
            }
            "tool_use" => {
                let Some(tool_use_id) = block
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                else {
                    continue;
                };
                let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                let id = next_id(*seq);
                *seq += 1;
                out.push(json!({
                    "id": id,
                    "role": "tool",
                    "content": "",
                    "at": at,
                    "step": {
                        "id": tool_use_id,
                        "name": name,
                        "kind": tool_kind(name),
                        "status": "running",
                        "input": clip(&tool_arguments(&input), MAX_TOOL_INPUT_CHARS),
                    },
                }));
            }
            _ => {}
        }
    }
    out
}

fn content_blocks(content: Option<&Value>) -> Vec<&Value> {
    match content {
        Some(Value::Array(items)) => items.iter().collect(),
        _ => Vec::new(),
    }
}

fn block_content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn tool_kind(name: &str) -> &'static str {
    match name {
        "Bash" => "terminal",
        "Read" => "read",
        "Edit" | "Write" | "NotebookEdit" => "edit",
        "Grep" | "Glob" | "WebSearch" => "search",
        _ => "other",
    }
}

fn tool_arguments(raw: &Value) -> String {
    match raw {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Claude Code injects `<system-reminder>` blocks into user turns; they are
/// app noise, not what the person typed.
fn strip_system_reminders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<system-reminder>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</system-reminder>") {
            Some(end) => rest = &rest[start + end + "</system-reminder>".len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}…［已截断］")
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let mut out = line.trim().to_string();
    if out.chars().count() > 60 {
        out = out.chars().take(60).collect();
        out.push('…');
    }
    out
}

fn timestamp_ms(value: &Value) -> Option<u64> {
    let raw = value.get("timestamp").and_then(Value::as_str)?;
    parse_rfc3339_ms(raw)
}

/// Minimal RFC3339 parser for the one spelling Claude Code and Codex write
/// (`2026-11-02T06:28:15.294Z`); anything it cannot read costs the row its
/// timestamp, never the row itself.
fn parse_rfc3339_ms(raw: &str) -> Option<u64> {
    let bytes = raw.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let num = |from: usize, to: usize| raw.get(from..to)?.parse::<u64>().ok();
    let year = num(0, 4)?;
    let month = num(5, 7)?;
    let day = num(8, 10)?;
    let hour = num(11, 13)?;
    let minute = num(14, 16)?;
    let second = num(17, 19)?;
    let millis = {
        let rest = &raw[19..];
        if let Some(frac) = rest.strip_prefix('.') {
            let digits: String = frac.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                0
            } else {
                let mut millis: u64 = digits.parse().ok()?;
                for _ in digits.len()..3 {
                    millis *= 10;
                }
                for _ in 3..digits.len() {
                    millis /= 10;
                }
                millis
            }
        } else {
            0
        }
    };
    // Days from civil (Howard Hinnant's algorithm), valid for any year.
    let years = year as i64 - if month <= 2 { 1 } else { 0 };
    let era = if years >= 0 { years } else { years - 399 } / 400;
    let year_of_era = years - era * 400;
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) as i64 + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let secs = days * 86_400 + hour as i64 * 3_600 + minute as i64 * 60 + second as i64;
    u64::try_from(secs * 1_000 + millis as i64).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import(name: &str, jsonl: &str) -> ImportPayload {
        let dir = std::env::temp_dir().join(format!("ad-import-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.jsonl"));
        std::fs::write(&path, jsonl).unwrap();
        let payload = import_claude_session_sync(&path.display().to_string()).unwrap();
        let _ = std::fs::remove_file(&path);
        payload
    }

    #[test]
    fn a_transcript_projects_into_the_chat_message_shapes() {
        let payload = import(
            "projection",
            &format!(
                "{}\n{}\n{}\n{}\n{}\n{}\n",
                r#"{"type":"summary","summary":"重构登录流程"}"#,
                r#"{"type":"queue-operation","operation":"enqueue"}"#,
                r#"{"parentUuid":null,"type":"user","timestamp":"2026-11-02T06:28:15.294Z","message":{"role":"user","content":"帮我重构登录流程 <system-reminder>别管这个</system-reminder>"}}"#,
                r#"{"type":"assistant","timestamp":"2026-11-02T06:28:20.000Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"先看代码"},{"type":"text","text":"好的，我先读一下登录模块。"},{"type":"tool_use","id":"toolu-1","name":"Read","input":{"file_path":"/w/login.rs"}}]}}"#,
                r#"{"type":"user","timestamp":"2026-11-02T06:28:22.000Z","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":[{"type":"text","text":"fn login() {}"}]}]}}"#,
                r#"{"type":"assistant","timestamp":"2026-11-02T06:28:25.000Z","message":{"role":"assistant","content":[{"type":"text","text":"读完了，方案如下。"}]}}"#,
            ),
        );
        assert_eq!(payload.title, "重构登录流程");
        assert_eq!(payload.messages.len(), 6);
        let roles: Vec<&str> = payload
            .messages
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(
            roles,
            vec!["user", "thinking", "assistant", "tool", "tool", "assistant"]
        );
        let user = &payload.messages[0];
        assert_eq!(user["content"], "帮我重构登录流程");
        assert_eq!(user["at"], 1793600895294u64);
        let step = &payload.messages[3]["step"];
        assert_eq!(step["id"], "toolu-1");
        assert_eq!(step["name"], "Read");
        assert_eq!(step["kind"], "read");
        assert_eq!(step["status"], "running");
        let done = &payload.messages[4]["step"];
        assert_eq!(done["status"], "done");
        assert_eq!(done["output"], "fn login() {}");
        let _ = std::fs::remove_file("/tmp/ad-import-test-cleanup");
    }

    #[test]
    fn sidechains_meta_rows_and_empty_text_are_dropped() {
        let payload = import(
            "sidechain",
            &format!(
                "{}\n{}\n{}\n{}\n",
                r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"子代理内部"}} "#,
                r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<system-reminder>内部注入</system-reminder>"}}"#,
                r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"x","type":"tool_result","content":"ok"}]}}"#,
                r#"{"type":"user","message":{"role":"user","content":"  "}}"#,
            ),
        );
        assert_eq!(payload.messages.len(), 1, "only the tool result survives");
        assert_eq!(payload.messages[0]["role"], "tool");
    }

    #[test]
    fn the_wiring_prefix_never_becomes_the_title_or_a_visible_row() {
        let payload = import(
            "prefix",
            &format!(
                "{}\n{}\n",
                r#"{"type":"user","timestamp":"2026-11-10T09:26:02.000Z","message":{"role":"user","content":"BROWSER MCP REQUIRED for this turn:\n- Use the configured MCP server named `browser`.\n帮我打开 example.com"}}"#,
                r#"{"type":"assistant","timestamp":"2026-11-10T09:26:20.000Z","message":{"role":"assistant","content":[{"type":"text","text":"已打开。"}]}}"#,
            ),
        );
        assert_eq!(payload.title, "导入的 Claude Code 会话");
        assert!(
            payload.messages.iter().all(|m| m["role"] != "user"),
            "the injected prefix row is dropped; a human-typed one stays"
        );
    }

    #[test]
    fn a_human_line_after_the_prefix_becomes_the_fallback_title() {
        let payload = import(
            "prefix-then-human",
            &format!(
                "{}\n{}\n{}\n",
                r#"{"type":"user","timestamp":"2026-11-10T09:26:02.000Z","message":{"role":"user","content":"COMPUTER MCP REQUIRED for this turn:\n- The app switch for controlling this computer is ON."}}"#,
                r#"{"type":"user","timestamp":"2026-11-10T09:26:05.000Z","message":{"role":"user","content":"随机操作2下电脑，不要太多"}}"#,
                r#"{"type":"assistant","timestamp":"2026-11-10T09:26:20.000Z","message":{"role":"assistant","content":[{"type":"text","text":"好。"}]}}"#,
            ),
        );
        assert_eq!(payload.title, "随机操作2下电脑，不要太多");
        let user_rows: Vec<_> = payload
            .messages
            .iter()
            .filter(|m| m["role"] == "user")
            .collect();
        assert_eq!(user_rows.len(), 1, "only the human turn is projected");
        assert_eq!(user_rows[0]["content"], "随机操作2下电脑，不要太多");
    }

    #[test]
    fn rfc3339_timestamps_survive_fraction_and_plain_forms() {
        assert_eq!(
            parse_rfc3339_ms("2026-11-02T06:28:15.294Z"),
            Some(1793600895294)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-11-02T06:28:15Z"),
            Some(1793600895000)
        );
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:01Z"), Some(1000));
        assert_eq!(parse_rfc3339_ms("not-a-time"), None);
    }

    fn codex_import(jsonl: &str, file_name: &str) -> ImportPayload {
        let dir = std::env::temp_dir().join(format!("ad-import-codex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(file_name);
        std::fs::write(&path, jsonl).unwrap();
        let payload = import_codex_session_sync(&path.display().to_string()).unwrap();
        let _ = std::fs::remove_file(&path);
        payload
    }

    #[test]
    fn a_codex_rollout_projects_its_text_conversation() {
        let payload = codex_import(
            &format!(
                "{}\n{}\n{}\n{}\n{}\n",
                r#"{"timestamp":"2026-08-10T00:33:35.331Z","type":"session_meta","payload":{"session_id":"019fe916-d43b-73b3-82bd-234ccfe48547","id":"019fe916-d43b-73b3-82bd-234ccfe48547","cwd":"/tmp"}}"#,
                r#"{"timestamp":"2026-08-10T00:33:36.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<user_instructions># AGENTS.md\n内部指令</user_instructions>"}]}}"#,
                r#"{"timestamp":"2026-08-10T00:34:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"帮我把构建脚本改成增量"}]}}"#,
                r#"{"timestamp":"2026-08-10T00:34:10.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"已改成增量编译。"}]}}"#,
                r#"{"timestamp":"2026-08-10T00:34:12.000Z","type":"event_msg","payload":{"type":"user_message","message":"帮我把构建脚本改成增量"}}"#,
            ),
            "rollout-2026-08-10T08-33-35-019fe916-d43b-73b3-82bd-234ccfe48547.jsonl",
        );
        assert_eq!(payload.session_id, "019fe916-d43b-73b3-82bd-234ccfe48547");
        assert_eq!(payload.title, "帮我把构建脚本改成增量");
        assert_eq!(
            payload.messages.len(),
            2,
            "text only, no duplicate event rows"
        );
        assert_eq!(payload.messages[0]["role"], "user");
        assert_eq!(payload.messages[0]["content"], "帮我把构建脚本改成增量");
        assert_eq!(payload.messages[1]["role"], "assistant");
        assert_eq!(payload.messages[1]["content"], "已改成增量编译。");
    }

    #[test]
    fn a_codex_thread_id_falls_back_to_the_file_name() {
        let path =
            Path::new("/x/rollout-2026-08-10T08-33-35-019fe916-d43b-73b3-82bd-234ccfe48547.jsonl");
        assert_eq!(
            codex_thread_id_from_name(path).as_deref(),
            Some("019fe916-d43b-73b3-82bd-234ccfe48547")
        );
        assert_eq!(codex_thread_id_from_name(Path::new("/x/notes.jsonl")), None);
    }

    #[test]
    fn an_all_injected_codex_head_is_not_importable() {
        let dir = std::env::temp_dir().join(format!("ad-import-codex-scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path =
            dir.join("rollout-2026-08-10T08-33-35-019fe916-d43b-73b3-82bd-234ccfe48547.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"timestamp":"2026-08-10T00:33:35.331Z","type":"session_meta","payload":{"session_id":"t0"}}"#,
                "\n",
                r#"{"timestamp":"2026-08-10T00:33:36.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<user_instructions>AGENTS</user_instructions>"}]}}"#,
                "\n",
                r#"{"timestamp":"2026-08-10T00:33:37.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd=/tmp</environment_context>"}]}}"#,
            ),
        )
        .unwrap();
        assert!(scan_codex_head(&path, HEAD_SCAN_BYTES).is_none());
        let _ = std::fs::remove_file(&path);
    }
}
