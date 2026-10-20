use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use agent_doctor_core::repair_types::LlmConfig;
use serde_json::{json, Value};

const MAX_AGENT_TURNS: usize = 8;
/// Reasoning models can think for a while before the first token.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct LlmTurn {
    pub content: Option<String>,
    pub tool_calls: Vec<LlmToolCall>,
}

pub fn repair_tool_definitions() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a runtime config file with line numbers (cat -n). Secrets are masked as {{SECRET:n}}. For search_replace, copy exact line text WITHOUT the line number prefix.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Absolute path to read" }
                    },
                    "required": ["path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_dir",
                "description": "List files and directories under an allowed repair path. Omit path to list all registered config roots for this runtime.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Optional absolute directory or file path" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "grep_files",
                "description": "Search for a literal text pattern in allowed config files. Returns path:line:masked_text matches.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "path": { "type": "string", "description": "Optional file or directory to narrow search" }
                    },
                    "required": ["pattern"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "search_replace",
                "description": "Replace exactly one unique occurrence in a file. Prefer this over write_file for edits. old_string/new_string may use {{SECRET:n}} tokens.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "old_string": { "type": "string" },
                        "new_string": { "type": "string" }
                    },
                    "required": ["path", "old_string", "new_string"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Write full file content. Use only for new files or very small files (e.g. .env). Secrets as {{SECRET:n}} tokens.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "patch_config",
                "description": "Set a YAML/JSON/TOML key by dot path (e.g. model.provider). More reliable than text replace for structured configs.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "key_path": { "type": "string" },
                        "value": { "type": "string", "description": "YAML/JSON scalar (quote strings)" }
                    },
                    "required": ["path", "key_path", "value"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "bash",
                "description": "Run an allowlisted repair shell command (Hermes/OpenClaw install/update, chmod .env, version checks).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" }
                    },
                    "required": ["command"]
                }
            }
        }
    ])
}

/// Tools that never change files or run commands (deep diagnose Q&A).
pub fn read_only_tool_definitions() -> Value {
    let Value::Array(all) = repair_tool_definitions() else {
        return json!([]);
    };
    Value::Array(
        all.into_iter()
            .filter(|tool| {
                matches!(
                    tool.pointer("/function/name").and_then(Value::as_str),
                    Some("read_file" | "list_dir" | "grep_files")
                )
            })
            .collect(),
    )
}

pub fn chat_with_tools(config: &LlmConfig, messages: &[Value]) -> Result<LlmTurn> {
    chat_with_tool_set(config, messages, Some(&repair_tool_definitions()))
}

/// `tools: None` asks for a plain answer (used to force a final reply).
pub fn chat_with_tool_set(
    config: &LlmConfig,
    messages: &[Value],
    tools: Option<&Value>,
) -> Result<LlmTurn> {
    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("failed to build LLM HTTP client")?;
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "temperature": 0.1,
    });
    if let Some(tools) = tools {
        body["tools"] = tools.clone();
        body["tool_choice"] = json!("auto");
    }

    let response = client
        .post(&config.api_url)
        .bearer_auth(&config.api_key)
        .json(&body)
        .send()
        .context("LLM request failed")?;

    let status = response.status();
    let payload: Value = response
        .json()
        .context("failed to parse LLM response JSON")?;
    if !status.is_success() {
        bail!("LLM HTTP {status}: {payload}");
    }

    parse_chat_completion(&payload)
}

fn parse_chat_completion(payload: &Value) -> Result<LlmTurn> {
    let message = payload
        .pointer("/choices/0/message")
        .context("LLM response missing message")?;

    let content = message
        .get("content")
        .and_then(|value| value.as_str())
        .map(str::to_string);

    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(|value| value.as_array()) {
        for call in calls {
            let id = call
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("tool-call")
                .to_string();
            let function = call.get("function").context("tool call missing function")?;
            let name = function
                .get("name")
                .and_then(|value| value.as_str())
                .context("tool call missing name")?
                .to_string();
            let arguments = function
                .get("arguments")
                .and_then(|value| value.as_str())
                .unwrap_or("{}")
                .to_string();
            tool_calls.push(LlmToolCall {
                id,
                name,
                arguments,
            });
        }
    }

    Ok(LlmTurn {
        content,
        tool_calls,
    })
}

pub fn max_agent_turns() -> usize {
    MAX_AGENT_TURNS
}
