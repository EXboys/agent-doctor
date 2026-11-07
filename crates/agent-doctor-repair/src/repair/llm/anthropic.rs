//! Anthropic Messages-protocol client for the repair/official-agent LLM layer.
//!
//! Converts the OpenAI-shaped message list the loop builds into Anthropic
//! content blocks, and maps the reply back to the protocol-agnostic
//! [`LlmTurn`]. Headers and URL shape follow `verify_anthropic_endpoint`
//! in `agent-doctor-core::setup::personal`.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use agent_doctor_core::repair_types::LlmConfig;

use super::{LlmToolCall, LlmTurn};

/// Anthropic requires an explicit output budget; tool-heavy repair turns need
/// room for several tool calls plus prose.
const MAX_TOKENS: u32 = 8192;
pub(crate) const ANTHROPIC_VERSION: &str = "2023-06-01";

pub fn messages_url(gateway_base: &str) -> String {
    let base = gateway_base.trim().trim_end_matches('/');
    if base.ends_with("/messages") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{base}/messages")
    } else {
        format!("{base}/v1/messages")
    }
}

/// POST one turn to the Anthropic Messages API and normalize the reply.
pub fn chat(config: &LlmConfig, messages: &[Value], tools: Option<&Value>) -> Result<LlmTurn> {
    let client = reqwest::blocking::Client::builder()
        .timeout(super::REQUEST_TIMEOUT)
        .build()
        .context("failed to build LLM HTTP client")?;

    let body = build_request_body(config, messages, tools);
    let url = messages_url(&config.api_url);
    let mut request = client
        .post(&url)
        .header("x-api-key", &config.api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        // Some Anthropic-compatible gateways also accept Bearer.
        .header("Authorization", format!("Bearer {}", config.api_key));
    if let Some(turn_id) = config
        .turn_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        request = request.header("x-ad-turn", turn_id);
    }
    let response = request.json(&body).send().context("LLM request failed")?;

    let status = response.status();
    let payload: Value = response
        .json()
        .context("failed to parse LLM response JSON")?;
    if !status.is_success() {
        bail!("LLM HTTP {status}: {payload}");
    }

    parse_messages_response(&payload)
}

fn build_request_body(config: &LlmConfig, messages: &[Value], tools: Option<&Value>) -> Value {
    let mut system_parts: Vec<String> = Vec::new();
    let mut converted: Vec<Value> = Vec::new();

    for message in messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        let content = message.get("content");
        match role {
            "system" => {
                if let Some(text) = content.and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        system_parts.push(text.to_string());
                    }
                }
            }
            "user" => {
                let text = content.and_then(Value::as_str).unwrap_or("").to_string();
                converted.push(json!({ "role": "user", "content": text }));
            }
            "assistant" => {
                converted.push(convert_assistant_message(message, content));
            }
            "tool" => {
                // OpenAI tool results become user-turn tool_result blocks.
                let tool_call_id = message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let text = content.and_then(Value::as_str).unwrap_or("");
                push_tool_result(&mut converted, tool_call_id, text);
            }
            _ => {}
        }
    }

    // Adjacent tool_result blocks must land in the same user turn per the
    // Anthropic wire format.
    let mut merged: Vec<Value> = Vec::new();
    for message in converted {
        let is_tool_result = message
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                blocks
                    .first()
                    .is_some_and(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            });
        if is_tool_result {
            if let Some(last) = merged.last_mut() {
                if last.get("role").and_then(Value::as_str) == Some("user")
                    && last.get("content").is_some_and(Value::is_array)
                {
                    if let (Some(last_blocks), Some(blocks)) = (
                        last.get_mut("content").and_then(Value::as_array_mut),
                        message.get("content").and_then(Value::as_array),
                    ) {
                        last_blocks.extend(blocks.iter().cloned());
                        continue;
                    }
                }
            }
        }
        merged.push(message);
    }

    let mut body = json!({
        "model": config.model,
        "max_tokens": MAX_TOKENS,
        "temperature": 0.1,
        "messages": merged,
    });
    if !system_parts.is_empty() {
        body["system"] = json!(system_parts.join("\n\n"));
    }
    if let Some(tools) = tools {
        body["tools"] = convert_tool_definitions(tools);
        body["tool_choice"] = json!({ "type": "auto" });
    }
    body
}

/// OpenAI assistant turn (optional text + tool_calls array) → content blocks.
fn convert_assistant_message(message: &Value, content: Option<&Value>) -> Value {
    let mut blocks: Vec<Value> = Vec::new();
    if let Some(text) = content.and_then(Value::as_str) {
        if !text.is_empty() {
            blocks.push(json!({ "type": "text", "text": text }));
        }
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let function = call.get("function");
            let name = function
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let arguments = function
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let input = serde_json::from_str::<Value>(arguments).unwrap_or_else(|_| json!({}));
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("tool-call");
            blocks.push(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input,
            }));
        }
    }
    if blocks.is_empty() {
        blocks.push(json!({ "type": "text", "text": "" }));
    }
    json!({ "role": "assistant", "content": blocks })
}

/// Append a tool_result block, starting a new user turn when the previous
/// message is not an open user turn.
fn push_tool_result(converted: &mut Vec<Value>, tool_call_id: &str, text: &str) {
    let block = json!({
        "type": "tool_result",
        "tool_use_id": tool_call_id,
        "content": text,
    });
    if let Some(last) = converted.last_mut() {
        if last.get("role").and_then(Value::as_str) == Some("user")
            && last.get("content").is_some_and(Value::is_array)
        {
            if let Some(blocks) = last.get_mut("content").and_then(Value::as_array_mut) {
                blocks.push(block);
                return;
            }
        }
    }
    converted.push(json!({ "role": "user", "content": [block] }));
}

/// OpenAI function-tool JSON array → Anthropic `{name, description, input_schema}`.
fn convert_tool_definitions(tools: &Value) -> Value {
    let Value::Array(all) = tools else {
        return json!([]);
    };
    Value::Array(
        all.iter()
            .filter_map(|tool| {
                let function = tool.get("function")?;
                Some(json!({
                    "name": function.get("name")?.as_str()? ,
                    "description": function.get("description").and_then(Value::as_str).unwrap_or(""),
                    "input_schema": function.get("parameters").cloned().unwrap_or_else(|| json!({"type": "object", "properties": {}})),
                }))
            })
            .collect(),
    )
}

/// Same body with `stream: true` for the SSE endpoint.
pub(crate) fn build_stream_request_body(
    config: &LlmConfig,
    messages: &[Value],
    tools: Option<&Value>,
) -> Value {
    let mut body = build_request_body(config, messages, tools);
    body["stream"] = Value::Bool(true);
    body
}

fn parse_messages_response(payload: &Value) -> Result<LlmTurn> {
    let blocks = payload
        .get("content")
        .and_then(Value::as_array)
        .context("LLM response missing content blocks")?;

    let mut content_parts: Vec<String> = Vec::new();
    let mut tool_calls = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    content_parts.push(text.to_string());
                }
            }
            Some("tool_use") => {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool-call")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .context("tool_use block missing name")?
                    .to_string();
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                tool_calls.push(LlmToolCall {
                    id,
                    name,
                    arguments: serde_json::to_string(&input).unwrap_or_else(|_| "{}".into()),
                });
            }
            _ => {}
        }
    }

    let content = if content_parts.is_empty() {
        None
    } else {
        Some(content_parts.join("\n"))
    };
    Ok(LlmTurn {
        content,
        tool_calls,
        usage: payload
            .get("usage")
            .and_then(agent_doctor_core::usage::usage_from_value)
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_url_appends_v1_messages() {
        assert_eq!(
            messages_url("https://host/api/gateway/anthropic"),
            "https://host/api/gateway/anthropic/v1/messages"
        );
        assert_eq!(
            messages_url("https://host/api/gateway/anthropic/"),
            "https://host/api/gateway/anthropic/v1/messages"
        );
        assert_eq!(messages_url("https://host/v1"), "https://host/v1/messages");
        assert_eq!(
            messages_url("https://host/v1/messages"),
            "https://host/v1/messages"
        );
    }

    #[test]
    fn request_body_moves_system_and_keeps_tool_results_adjacent() {
        let config = LlmConfig::from_anthropic_gateway("https://host/anthropic", "k", "m");
        let messages = vec![
            json!({"role": "system", "content": "be terse"}),
            json!({"role": "user", "content": "hi"}),
            json!({"role": "assistant", "tool_calls": [
                {"id": "t1", "function": {"name": "read_file", "arguments": "{\"path\":\"/a\"}"}}
            ]}),
            json!({"role": "tool", "tool_call_id": "t1", "content": "line 1"}),
            json!({"role": "tool", "tool_call_id": "t2", "content": "line 2"}),
            json!({"role": "user", "content": "go on"}),
        ];
        let body = build_request_body(&config, &messages, None);
        assert_eq!(body["system"], json!("be terse"));
        let turn_messages = body["messages"].as_array().unwrap();
        assert_eq!(
            turn_messages.len(),
            4,
            "tool results merge into one user turn"
        );
        assert_eq!(turn_messages[1]["role"], "assistant");
        assert_eq!(turn_messages[1]["content"][0]["type"], "tool_use");
        assert_eq!(turn_messages[2]["role"], "user");
        let results = turn_messages[2]["content"].as_array().unwrap();
        assert_eq!(results.len(), 2, "both tool results share one turn");
        assert_eq!(results[1]["tool_use_id"], "t2");
        assert_eq!(
            turn_messages[3],
            json!({"role": "user", "content": "go on"})
        );
    }

    #[test]
    fn request_body_converts_openai_tool_definitions() {
        let tools = json!([
            {
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "read",
                    "parameters": {"type": "object", "properties": {"path": {"type": "string"}}}
                }
            }
        ]);
        let config = LlmConfig::from_anthropic_gateway("https://host/anthropic", "k", "m");
        let body = build_request_body(
            &config,
            &[json!({"role": "user", "content": "x"})],
            Some(&tools),
        );
        assert_eq!(
            body["tools"],
            json!([{
                "name": "read_file",
                "description": "read",
                "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}}
            }])
        );
        assert_eq!(body["tool_choice"], json!({"type": "auto"}));
    }

    #[test]
    fn parse_maps_text_and_tool_use_blocks() {
        let payload = json!({
            "content": [
                {"type": "text", "text": "checking"},
                {"type": "tool_use", "id": "t9", "name": "grep_files", "input": {"pattern": "url"}}
            ]
        });
        let turn = parse_messages_response(&payload).unwrap();
        assert_eq!(turn.content.as_deref(), Some("checking"));
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].id, "t9");
        assert_eq!(turn.tool_calls[0].name, "grep_files");
        assert_eq!(turn.tool_calls[0].arguments, r#"{"pattern":"url"}"#);
    }

    #[test]
    fn parse_all_text_turn_has_no_tool_calls() {
        let payload = json!({"content": [{"type": "text", "text": "done"}]});
        let turn = parse_messages_response(&payload).unwrap();
        assert_eq!(turn.content.as_deref(), Some("done"));
        assert!(turn.tool_calls.is_empty());
    }
}
