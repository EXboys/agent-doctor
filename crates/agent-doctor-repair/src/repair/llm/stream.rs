//! Streaming LLM responses for the official agent loop.
//!
//! Both protocols stream over SSE with the blocking client (`Response: Read`).
//! Text deltas run through [`ThinkFilter`] so reasoning models that inline
//! `<think>…</think>` become separate reasoning deltas instead of raw tags in
//! the chat bubble. Tool-call fragments are reassembled into the same
//! [`LlmTurn`] the non-streaming path returns, so the loop stays identical.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use agent_doctor_core::repair_types::{LlmConfig, LlmProtocol};

use super::anthropic;
use super::{chat_with_tool_set, LlmToolCall, LlmTurn};

/// Streaming reads can outlast the one-shot request budget (reasoning models
/// think for a long time between deltas).
const STREAM_TIMEOUT: Duration = Duration::from_secs(600);
const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

/// One incremental piece of a reply. `Reasoning` is a thinking step the host
/// may show collapsed; `Text` is user-visible reply body.
#[derive(Debug, Clone)]
pub enum LlmStreamDelta {
    Text(String),
    Reasoning(String),
}

/// Splits inline `<think>…</think>` blocks out of streamed text, tolerating
/// tags that arrive split across chunk boundaries (a chunk may end mid-tag;
/// the ambiguous tail is held back until the next feed decides it).
#[derive(Default)]
pub struct ThinkFilter {
    in_think: bool,
    pending: String,
}

impl ThinkFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, chunk: &str, out: &mut dyn FnMut(LlmStreamDelta)) {
        let mut buf = std::mem::take(&mut self.pending) + chunk;
        loop {
            if self.in_think {
                match buf.find(THINK_CLOSE) {
                    Some(end) => {
                        let reasoning = buf[..end].trim().to_string();
                        if !reasoning.is_empty() {
                            out(LlmStreamDelta::Reasoning(reasoning));
                        }
                        buf = buf[end + THINK_CLOSE.len()..].to_string();
                        self.in_think = false;
                    }
                    None => {
                        let hold = partial_suffix_len(&buf, THINK_CLOSE);
                        let emit_end = buf.len() - hold;
                        if emit_end > 0 {
                            let reasoning = buf[..emit_end].trim().to_string();
                            if !reasoning.is_empty() {
                                out(LlmStreamDelta::Reasoning(reasoning));
                            }
                        }
                        self.pending = buf[emit_end..].to_string();
                        return;
                    }
                }
            } else {
                match buf.find('<') {
                    Some(start) => {
                        if start > 0 {
                            out(LlmStreamDelta::Text(buf[..start].to_string()));
                        }
                        let rest = &buf[start..];
                        if let Some(stripped) = rest.strip_prefix(THINK_OPEN) {
                            self.in_think = true;
                            buf = stripped.to_string();
                        } else if rest.len() >= THINK_OPEN.len() {
                            // A '<' that is not our tag: literal text.
                            out(LlmStreamDelta::Text("<".to_string()));
                            buf = rest[1..].to_string();
                        } else {
                            // Too short to decide — hold until more arrives.
                            self.pending = rest.to_string();
                            return;
                        }
                    }
                    None => {
                        if !buf.is_empty() {
                            out(LlmStreamDelta::Text(buf));
                        }
                        return;
                    }
                }
            }
        }
    }

    /// Flush the held tail at end of stream.
    pub fn finish(&mut self, out: &mut dyn FnMut(LlmStreamDelta)) {
        let buf = std::mem::take(&mut self.pending);
        if buf.is_empty() {
            return;
        }
        if self.in_think {
            let reasoning = buf.trim().to_string();
            if !reasoning.is_empty() {
                out(LlmStreamDelta::Reasoning(reasoning));
            }
        } else {
            out(LlmStreamDelta::Text(buf));
        }
    }
}

/// Longest prefix of `tag` that is a suffix of `buf` (both ASCII tags), so a
/// split closing tag is not emitted as reasoning text.
fn partial_suffix_len(buf: &str, tag: &str) -> usize {
    for len in (1..tag.len()).rev() {
        if buf.ends_with(&tag[..len]) {
            return len;
        }
    }
    0
}

/// Streamed turn with text/reasoning deltas pushed through `on_delta` as they
/// arrive. `Ok(None)` means the provider would not stream (rejected setup) and
/// the caller should retry with the one-shot request; mid-stream failures are
/// hard errors.
pub fn chat_with_tool_set_stream(
    config: &LlmConfig,
    messages: &[Value],
    tools: Option<&Value>,
    on_delta: &mut dyn FnMut(LlmStreamDelta),
) -> Result<LlmTurn> {
    let established = match config.protocol {
        LlmProtocol::OpenAi => openai_chat_stream(config, messages, tools, on_delta),
        LlmProtocol::Anthropic => anthropic_chat_stream(config, messages, tools, on_delta),
    };
    match established {
        Ok(Some(turn)) => Ok(turn),
        Ok(None) => chat_with_tool_set(config, messages, tools),
        Err(error) => Err(error),
    }
}

fn stream_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(STREAM_TIMEOUT)
        .build()
        .context("failed to build LLM HTTP client")
}

/// Returns `Ok(None)` when the request could not start streaming at all.
fn post_stream(
    client: &reqwest::blocking::Client,
    url: &str,
    headers: Vec<(&str, String)>,
    body: Value,
) -> Result<Option<BufReader<reqwest::blocking::Response>>> {
    let mut request = client.post(url).json(&body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = match request.send() {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };
    if !response.status().is_success() {
        return Ok(None);
    }
    Ok(Some(BufReader::new(response)))
}

/// One SSE `data:` payload as JSON; `None` for comments, blanks and `[DONE]`.
fn sse_data(line: &str) -> Result<Option<Value>> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(':') {
        return Ok(None);
    }
    let payload = trimmed.strip_prefix("data:").map(str::trim_start);
    let Some(payload) = payload else {
        return Ok(None); // `event:` lines are handled by the caller for Anthropic
    };
    if payload == "[DONE]" {
        return Ok(None);
    }
    serde_json::from_str(payload)
        .map(Some)
        .context("failed to parse SSE data line")
}

// ─── OpenAI-compatible ──────────────────────────────────────────────────────

#[derive(Default)]
struct OpenAiStreamAcc {
    content: String,
    tools: BTreeMap<usize, LlmToolCall>,
}

fn openai_feed_data(
    payload: &Value,
    acc: &mut OpenAiStreamAcc,
    filter: &mut ThinkFilter,
    out: &mut dyn FnMut(LlmStreamDelta),
) -> Result<bool> {
    if payload.get("error").is_some() {
        bail!("LLM stream error: {payload}");
    }
    let Some(delta) = payload.pointer("/choices/0/delta") else {
        return Ok(false);
    };
    if let Some(reasoning) = delta.get("reasoning_content").and_then(Value::as_str) {
        if !reasoning.is_empty() {
            out(LlmStreamDelta::Reasoning(reasoning.to_string()));
        }
    }
    if let Some(text) = delta.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            acc.content.push_str(text);
            filter.feed(text, out);
        }
    }
    if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let entry = acc.tools.entry(index).or_insert_with(|| LlmToolCall {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                entry.id = id.to_string();
            }
            if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                entry.name = name.to_string();
            }
            if let Some(arguments) = call.pointer("/function/arguments").and_then(Value::as_str) {
                entry.arguments.push_str(arguments);
            }
        }
    }
    Ok(false)
}

fn openai_chat_stream(
    config: &LlmConfig,
    messages: &[Value],
    tools: Option<&Value>,
    on_delta: &mut dyn FnMut(LlmStreamDelta),
) -> Result<Option<LlmTurn>> {
    let client = stream_client()?;
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "temperature": 0.1,
        "stream": true,
    });
    if let Some(tools) = tools {
        body["tools"] = tools.clone();
        body["tool_choice"] = json!("auto");
    }
    let reader = match post_stream(
        &client,
        &config.api_url,
        vec![("Authorization", format!("Bearer {}", config.api_key))],
        body,
    )? {
        Some(reader) => reader,
        None => return Ok(None),
    };

    let mut acc = OpenAiStreamAcc::default();
    let mut filter = ThinkFilter::new();
    for line in reader.lines() {
        let line = line.context("LLM stream read failed")?;
        let Some(payload) = sse_data(&line)? else {
            continue;
        };
        if openai_feed_data(&payload, &mut acc, &mut filter, on_delta)? {
            break;
        }
    }
    filter.finish(on_delta);

    let tool_calls = acc.tools.into_values().collect();
    let content = if acc.content.is_empty() {
        None
    } else {
        Some(acc.content)
    };
    Ok(Some(LlmTurn {
        content,
        tool_calls,
    }))
}

// ─── Anthropic ──────────────────────────────────────────────────────────────

#[derive(Default)]
struct AnthropicStreamAcc {
    content: String,
    tool_order: Vec<u64>,
    tools: BTreeMap<u64, LlmToolCall>,
}

fn anthropic_feed_event(
    event: &str,
    payload: &Value,
    acc: &mut AnthropicStreamAcc,
    filter: &mut ThinkFilter,
    out: &mut dyn FnMut(LlmStreamDelta),
) -> Result<bool> {
    match event {
        "content_block_start" => {
            if let Some(block) = payload.get("content_block") {
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    let index = payload.get("index").and_then(Value::as_u64).unwrap_or(0);
                    acc.tools.entry(index).or_insert_with(|| LlmToolCall {
                        id: block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("tool-call")
                            .to_string(),
                        name: block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        arguments: String::new(),
                    });
                    acc.tool_order.push(index);
                }
            }
        }
        "content_block_delta" => {
            let Some(delta) = payload.get("delta") else {
                return Ok(false);
            };
            match delta.get("type").and_then(Value::as_str) {
                Some("text_delta") => {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        if !text.is_empty() {
                            acc.content.push_str(text);
                            filter.feed(text, out);
                        }
                    }
                }
                Some("thinking_delta") => {
                    if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                        if !text.is_empty() {
                            out(LlmStreamDelta::Reasoning(text.to_string()));
                        }
                    }
                }
                Some("input_json_delta") => {
                    let index = payload.get("index").and_then(Value::as_u64).unwrap_or(0);
                    if let Some(partial) = delta.get("partial_json").and_then(Value::as_str) {
                        if let Some(tool) = acc.tools.get_mut(&index) {
                            tool.arguments.push_str(partial);
                        }
                    }
                }
                _ => {}
            }
        }
        "message_stop" => return Ok(true),
        _ => {}
    }
    Ok(false)
}

/// Normalize a parsed `data:` payload plus the SSE event name that preceded it.
fn anthropic_event_name(payload: &Value) -> String {
    payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Anthropic stream entry. Returns `Ok(None)` when streaming could not start.
pub(crate) fn anthropic_chat_stream(
    config: &LlmConfig,
    messages: &[Value],
    tools: Option<&Value>,
    on_delta: &mut dyn FnMut(LlmStreamDelta),
) -> Result<Option<LlmTurn>> {
    let client = stream_client()?;
    let body = anthropic::build_stream_request_body(config, messages, tools);
    let url = anthropic::messages_url(&config.api_url);
    let headers = vec![
        ("x-api-key", config.api_key.clone()),
        (
            "anthropic-version",
            anthropic::ANTHROPIC_VERSION.to_string(),
        ),
        ("Authorization", format!("Bearer {}", config.api_key)),
    ];
    let reader = match post_stream(&client, &url, headers, body)? {
        Some(reader) => reader,
        None => return Ok(None),
    };

    let mut acc = AnthropicStreamAcc::default();
    let mut filter = ThinkFilter::new();
    for line in reader.lines() {
        let line = line.context("LLM stream read failed")?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') {
            continue;
        }
        let Some(payload) = sse_data(&line)? else {
            continue; // `event:` name lines; the data payload carries `type`
        };
        let event = anthropic_event_name(&payload);
        if anthropic_feed_event(&event, &payload, &mut acc, &mut filter, on_delta)? {
            break;
        }
    }
    filter.finish(on_delta);

    let tool_calls = acc
        .tool_order
        .iter()
        .filter_map(|index| acc.tools.remove(index))
        .collect();
    let content = if acc.content.is_empty() {
        None
    } else {
        Some(acc.content)
    };
    Ok(Some(LlmTurn {
        content,
        tool_calls,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn collect(filter: &mut ThinkFilter, chunks: &[&str]) -> (Vec<String>, Vec<String>) {
        let texts = RefCell::new(Vec::new());
        let reasonings = RefCell::new(Vec::new());
        {
            let texts_ref = &texts;
            let reasonings_ref = &reasonings;
            let mut out = |delta: LlmStreamDelta| match delta {
                LlmStreamDelta::Text(text) => texts_ref.borrow_mut().push(text),
                LlmStreamDelta::Reasoning(text) => reasonings_ref.borrow_mut().push(text),
            };
            for chunk in chunks {
                filter.feed(chunk, &mut out);
            }
            filter.finish(&mut out);
        }
        (texts.into_inner(), reasonings.into_inner())
    }

    #[test]
    fn plain_text_passes_through_untouched() {
        let mut filter = ThinkFilter::new();
        let (texts, reasonings) = collect(&mut filter, &["你好", "世界"]);
        assert_eq!(texts.join(""), "你好世界");
        assert!(reasonings.is_empty());
    }

    #[test]
    fn whole_think_block_becomes_reasoning() {
        let mut filter = ThinkFilter::new();
        let (texts, reasonings) = collect(&mut filter, &["<think>先想想</think>答案"]);
        assert_eq!(texts.join(""), "答案");
        assert_eq!(reasonings.join(""), "先想想");
    }

    #[test]
    fn tags_split_across_chunks_are_detected() {
        let mut filter = ThinkFilter::new();
        let (texts, reasonings) = collect(&mut filter, &["<thi", "nk>推理", "中</th", "ink>结论"]);
        assert_eq!(texts.join(""), "结论");
        assert_eq!(reasonings.join(""), "推理中");
    }

    #[test]
    fn unclosed_think_drains_as_reasoning_at_finish() {
        let mut filter = ThinkFilter::new();
        let (texts, reasonings) = collect(&mut filter, &["<think>只写一半"]);
        assert!(texts.is_empty());
        assert_eq!(reasonings.join(""), "只写一半");
    }

    #[test]
    fn literal_angle_bracket_survives() {
        let mut filter = ThinkFilter::new();
        let (texts, reasonings) = collect(&mut filter, &["a < b <think>x</think>"]);
        assert_eq!(texts.join(""), "a < b ");
        assert_eq!(reasonings.join(""), "x");
    }

    #[test]
    fn openai_deltas_reassemble_tool_calls() {
        let mut acc = OpenAiStreamAcc::default();
        let mut filter = ThinkFilter::new();
        let mut out = |_| {};
        let lines = [
            r##"data: {"choices": [{"delta": {"role": "assistant"}}]}"##,
            r##"data: {"choices": [{"delta": {"content": "\u68c0\u67e5\u4e2d"}}]}"##,
            r##"data: {"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "t1", "function": {"name": "read_file", "arguments": "{\"pa"}}]}}]}"##,
            r##"data: {"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "th\":\"/a\""}}]}}]}"##,
            r##"data: {"choices": [{"delta": {"tool_calls": [{"index": 1, "id": "t2", "function": {"name": "list_dir", "arguments": "{}"}}]}}]}"##,
            r##"data: {"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "}"}}]}}]}"##,
            "data: [DONE]",
        ];
        for line in lines {
            let Some(payload) = sse_data(line).unwrap() else {
                continue;
            };
            let _ = openai_feed_data(&payload, &mut acc, &mut filter, &mut out).unwrap();
        }
        let tools: Vec<_> = acc.tools.into_values().collect();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].id, "t1");
        assert_eq!(tools[0].name, "read_file");
        assert_eq!(tools[0].arguments, r#"{"path":"/a"}"#);
        assert_eq!(tools[1].name, "list_dir");
        assert_eq!(acc.content, "检查中");
    }

    #[test]
    fn anthropic_events_reassemble_text_and_tools() {
        let mut acc = AnthropicStreamAcc::default();
        let mut filter = ThinkFilter::new();
        let mut out = |_| {};
        let events = [
            (
                "content_block_start",
                r#"{"index":0,"content_block":{"type":"text"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"index":0,"delta":{"type":"text_delta","text":"看配置"}}"#,
            ),
            (
                "content_block_start",
                r#"{"index":1,"content_block":{"type":"tool_use","id":"t9","name":"grep_files"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"index":1,"delta":{"type":"input_json_delta","partial_json":"{\"pat"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"index":1,"delta":{"type":"input_json_delta","partial_json":"tern\":\"x\"}"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"index":2,"delta":{"type":"thinking_delta","thinking":"想一想"}}"#,
            ),
            ("message_stop", "{}"),
        ];
        for (event, data) in events {
            let payload: Value = serde_json::from_str(data).unwrap();
            let done =
                anthropic_feed_event(event, &payload, &mut acc, &mut filter, &mut out).unwrap();
            assert!(!(event == "message_stop") || done);
        }
        assert_eq!(acc.content, "看配置");
        let tool = &acc.tools[&1];
        assert_eq!(tool.id, "t9");
        assert_eq!(tool.name, "grep_files");
        assert_eq!(tool.arguments, r#"{"pattern":"x"}"#);
    }

    #[test]
    fn sse_lines_comments_and_done_are_skipped() {
        assert!(sse_data("").unwrap().is_none());
        assert!(sse_data(": keep-alive").unwrap().is_none());
        assert!(sse_data("data: [DONE]").unwrap().is_none());
        assert!(sse_data("event: content_block_delta").unwrap().is_none());
        assert!(sse_data("data: {\"a\":1}").unwrap().is_some());
    }
}
