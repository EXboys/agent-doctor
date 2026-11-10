//! Import existing Claude Code transcripts (`~/.claude/projects/**.jsonl`)
//! into chat sessions.
//!
//! The jsonl lines carry the full API transcript: `user` rows hold prompts and
//! `tool_result` blocks, `assistant` rows hold `thinking`/`text`/`tool_use`
//! blocks, `summary` rows hold titles. Everything else (queue ops, environment
//! snapshots, cost states, sidechains) is Claude Code bookkeeping and is
//! skipped. Projection targets the desktop chat store's message shape, so the
//! renderer needs nothing new.

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

#[derive(Serialize)]
pub struct ClaudeImportCandidate {
    pub path: String,
    pub session_id: String,
    pub title: String,
    pub preview: String,
    pub updated_at: u64,
    pub size_bytes: u64,
}

#[derive(Serialize)]
pub struct ClaudeImportPayload {
    pub session_id: String,
    pub title: String,
    pub truncated: bool,
    pub messages: Vec<Value>,
}

fn home_projects_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "找不到你的个人文件夹，无法读取 Claude Code 会话。".to_string())?;
    Ok(PathBuf::from(home).join(".claude").join("projects"))
}

/// Scan `~/.claude/projects/**.jsonl` for importable sessions. Only the head
/// of each file is read, so a shelf of multi-megabyte transcripts still lists
/// quickly.
#[tauri::command]
pub async fn list_claude_import_sessions_command() -> Result<Vec<ClaudeImportCandidate>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let projects = home_projects_dir()?;
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
                let Some(meta) = fs::metadata(&path).ok() else {
                    continue;
                };
                let session_id = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("")
                    .to_string();
                if session_id.is_empty() {
                    continue;
                }
                let (title, preview) = scan_head(&path, HEAD_SCAN_BYTES);
                out.push(ClaudeImportCandidate {
                    path: path.display().to_string(),
                    session_id: session_id.clone(),
                    title: title.unwrap_or_else(|| preview.clone()),
                    preview,
                    updated_at: meta
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|delta| delta.as_millis() as u64)
                        .unwrap_or(0),
                    size_bytes: meta.len(),
                });
            }
        }
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(out)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// First title and first user text within the head of one transcript.
fn scan_head(path: &Path, max_bytes: u64) -> (Option<String>, String) {
    let Ok(file) = fs::File::open(path) else {
        return (None, String::new());
    };
    let mut reader = BufReader::new(file);
    let mut title = None;
    let mut preview = String::new();
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
    for line in buffer.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
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
                preview = user_text(&value).unwrap_or_default();
            }
            _ => {}
        }
        if title.is_some() && !preview.is_empty() {
            break;
        }
    }
    (title, clip(&preview, 120))
}

/// Project one transcript into chat-store messages.
#[tauri::command]
pub async fn import_claude_session_command(path: String) -> Result<ClaudeImportPayload, String> {
    tauri::async_runtime::spawn_blocking(move || import_claude_session_sync(&path))
        .await
        .map_err(|error| error.to_string())?
}

fn import_claude_session_sync(path: &str) -> Result<ClaudeImportPayload, String> {
    let file = fs::File::open(path).map_err(|error| format!("打不开这份会话记录：{error}"))?;
    let mut title = String::new();
    let mut messages: Vec<Value> = Vec::new();
    let mut truncated = false;
    let mut seq = 0usize;
    let next_id = |seq: usize| format!("imp-{seq}");

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
        title = messages
            .iter()
            .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .map(|text| first_line(text))
            .unwrap_or_else(|| "导入的 Claude Code 会话".into());
    }
    Ok(ClaudeImportPayload {
        session_id,
        title: clip(&title, 60),
        truncated,
        messages,
    })
}

fn push_message(messages: &mut Vec<Value>, truncated: &mut bool, message: Value) {
    if messages.len() >= MAX_MESSAGES {
        *truncated = true;
        return;
    }
    messages.push(message);
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
    if !text.trim().is_empty() {
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

/// Minimal RFC3339 parser for the one spelling Claude Code writes
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
    use serde_json::json;

    fn import(name: &str, jsonl: &str) -> ClaudeImportPayload {
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
}
