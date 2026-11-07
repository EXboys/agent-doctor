use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use agent_doctor_core::repair_types::{LlmConfig, LlmProtocol};
use agent_doctor_core::usage::{usage_from_value, TokenUsage};
use serde_json::{json, Value};

pub mod anthropic;
pub mod stream;

const MAX_AGENT_TURNS: usize = 8;
/// Reasoning models can think for a while before the first token.
/// Reasoning models (MiniMax-M3, o-series, DeepSeek-R1) can think for
/// several minutes before the first token; 3 minutes was not enough.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

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
    /// What this one model call billed. Empty for providers that omit usage.
    pub usage: TokenUsage,
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

/// Browser tools for the chat: same names and contracts as the
/// `agent-doctor-mcp` stdio server, executed in-process against Agent
/// Doctor's isolated automation Chrome. Never offered to the repair planner
/// or deep diagnose.
pub fn browser_tool_definitions() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "browser_navigate",
                "description": "Open a URL in Agent Doctor's browser and wait for the page to load. Returns { title, url }. Call browser_snapshot next before clicking or filling.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "The URL to open (https://…)" }
                    },
                    "required": ["url"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "browser_snapshot",
                "description": "List the page's interactive elements with stable @eN refs. REQUIRED before browser_click/browser_fill — re-run after navigation or DOM changes because old refs go stale. `selector` optionally narrows the scope to one subtree.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "selector": { "type": "string", "description": "Optional CSS scope; snapshot only inside this element" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "browser_click",
                "description": "Click a page element by snapshot ref (@e1, preferred) or CSS selector.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "ref": { "type": "string", "description": "Snapshot ref like @e1 from browser_snapshot" },
                        "selector": { "type": "string", "description": "CSS selector fallback when no ref" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "browser_fill",
                "description": "Clear and fill an input by snapshot ref (@eN, preferred) or CSS selector.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "ref": { "type": "string", "description": "Snapshot ref like @e2" },
                        "selector": { "type": "string", "description": "CSS selector fallback" },
                        "text": { "type": "string", "description": "Text to fill in" }
                    },
                    "required": ["text"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "browser_find",
                "description": "Find an element by role/label/text/placeholder/testid when refs are awkward; optionally click/fill/type it immediately via `action`.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "strategy": { "type": "string", "description": "role | label | text | placeholder | testid" },
                        "query": { "type": "string", "description": "Text / role name / test id to match" },
                        "action": { "type": "string", "description": "Optional: click | fill | type" },
                        "text": { "type": "string", "description": "Text for the fill/type action" }
                    },
                    "required": ["strategy", "query"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "browser_screenshot",
                "description": "Screenshot the current page as a PNG and return { path }. Tell the person where the file is; use it when they ask for a screenshot or visual proof.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Optional PNG path; defaults to a temp file" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "browser_get_text",
                "description": "Read the current page's visible text. Cheapest way to check search results or page content.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }
    ])
}

/// Computer-use tools for the chat: same names and contracts as the
/// `agent-doctor-mcp` stdio computer server, executed in-process. The click
/// happens in the desktop app over its local socket. Never offered to the
/// repair planner or deep diagnose.
pub fn computer_tool_definitions() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "screen_apps",
                "description": "List the apps open on this computer.",
                "parameters": { "type": "object", "properties": {} }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "screen_snapshot",
                "description": "See one app's window as text: its controls numbered @e1, @e2 … with screen points. Call it again after anything changes — old refs go stale. `app` empty uses the front app.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "app": { "type": "string", "description": "App name. Empty uses the front app." }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "screen_click",
                "description": "Click inside an app that is already open, by @eN ref from screen_snapshot (preferred), by label from screen_state, or by x,y points. To open or switch apps call screen_open instead — never click the Dock, Spotlight, or Launchpad.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "ref": { "type": "string", "description": "@eN from screen_snapshot" },
                        "label": { "type": "string", "description": "Control name from screen_state" },
                        "app": { "type": "string" },
                        "x": { "type": "number" },
                        "y": { "type": "number" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "screen_type",
                "description": "Type text into the focused control. Pass ref to click that field first.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" },
                        "ref": { "type": "string" }
                    },
                    "required": ["text"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "screen_open",
                "description": "Open an app by its exact name and bring it forward. Use this instead of clicking the Dock. 微信 is not 微信开发者工具.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "app": { "type": "string", "description": "Exact app name, such as 微信 or WeChat." }
                    },
                    "required": ["app"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "screen_scroll",
                "description": "Scroll a list up or down when the row is not on screen. x,y are points on that list.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "direction": { "type": "string", "description": "up or down" },
                        "x": { "type": "number" },
                        "y": { "type": "number" },
                        "amount": { "type": "integer" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "screen_press",
                "description": "Press a key inside the current app, such as return, escape, tab, backspace, delete, or command+a.",
                "parameters": {
                    "type": "object",
                    "properties": { "key": { "type": "string" } },
                    "required": ["key"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "screen_windows",
                "description": "List an app's window titles. Empty app uses the front app.",
                "parameters": {
                    "type": "object",
                    "properties": { "app": { "type": "string" } }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "screen_state",
                "description": "Read the front window's buttons and text fields, with screen points.",
                "parameters": {
                    "type": "object",
                    "properties": { "app": { "type": "string" } }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "screen_set",
                "description": "Write into a named text field from screen_state. If that window has no text field, click the place and use screen_type.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "app": { "type": "string" },
                        "label": { "type": "string" },
                        "text": { "type": "string" }
                    },
                    "required": ["label", "text"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "screen_menu",
                "description": "Open a menu by its names from the menu bar down, such as [\"文件\", \"新建\"]. Does not click a guessed point.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "app": { "type": "string" },
                        "path": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["path"]
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
    match config.protocol {
        LlmProtocol::OpenAi => chat_openai(config, messages, tools),
        LlmProtocol::Anthropic => anthropic::chat(config, messages, tools),
    }
}

fn chat_openai(config: &LlmConfig, messages: &[Value], tools: Option<&Value>) -> Result<LlmTurn> {
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

    let mut request = client.post(&config.api_url).bearer_auth(&config.api_key);
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
        usage: payload
            .get("usage")
            .and_then(usage_from_value)
            .unwrap_or_default(),
    })
}

pub const fn max_agent_turns() -> usize {
    MAX_AGENT_TURNS
}

/// Stable marker for "the official account is not signed in". Callers match on
/// it to point the user at login instead of generic provider setup.
pub const OFFICIAL_PROVIDER_MISSING: &str = "official_provider_missing";

/// Chat pin marker for the monthly-gift entry: rides the dedicated
/// `/api/v1/agent` channel and spends the gift, never purchased tokens.
pub const OFFICIAL_GIFT_MODEL: &str = "official-gift";

/// LLM config for the official built-in agent. The chat's pinned provider wins
/// when it names a saved personal service. Pinned to the official row itself,
/// the model decides the line: the gift marker (or no model at all) rides the
/// free `/api/v1/agent` channel, any other model rides the stored
/// `/api/v1/official` gateway and spends purchased tokens — so a personal key
/// is never spent silently. A missing official row with no pin means "sign in".
pub fn official_llm_config_for(
    provider_id: Option<&str>,
    model: Option<&str>,
) -> Result<LlmConfig> {
    let pinned = provider_id.map(str::trim).filter(|id| !id.is_empty());
    let pinned_model = model.map(str::trim).filter(|m| !m.is_empty());
    let entry = match pinned {
        None | Some(agent_doctor_core::setup::TEAMUPS_OFFICIAL_PROVIDER_ID) => {
            agent_doctor_core::setup::load_personal_provider(
                agent_doctor_core::setup::TEAMUPS_OFFICIAL_PROVIDER_ID,
            )?
            .ok_or_else(|| {
                anyhow!("{OFFICIAL_PROVIDER_MISSING}: official account is not signed in")
            })?
        }
        Some(id) => agent_doctor_core::setup::load_personal_provider(id)?
            .ok_or_else(|| anyhow!("pinned provider not found: {id}"))?,
    };
    let on_official_row =
        pinned.is_none() || pinned == Some(agent_doctor_core::setup::TEAMUPS_OFFICIAL_PROVIDER_ID);
    let on_gift = on_official_row
        && pinned_model
            .map(|m| m == OFFICIAL_GIFT_MODEL)
            .unwrap_or(true);
    let url = if on_gift {
        official_agent_gateway_url(&entry)
    } else {
        entry.url.clone()
    };
    let model = if on_gift {
        entry.model.clone()
    } else {
        pinned_model.unwrap_or(&entry.model).to_string()
    };
    if agent_doctor_core::setup::normalize_protocol(&entry.protocol)
        == agent_doctor_core::setup::PROTOCOL_ANTHROPIC
    {
        Ok(LlmConfig::from_anthropic_gateway(
            &url,
            &entry.api_key,
            &model,
        ))
    } else {
        Ok(LlmConfig::from_gateway(&url, &entry.api_key, &model))
    }
}

/// Deep diagnose and other in-process callers: always the official gift line.
pub fn official_llm_config() -> Result<LlmConfig> {
    official_llm_config_for(None, None)
}

/// `…/api/v1/official` → `…/api/v1/agent` (OpenAI wire) or `…/api/v1/agent/anthropic`
/// (Anthropic wire). Anything else — custom edits included — stays as configured.
fn official_agent_gateway_url(entry: &agent_doctor_core::setup::PersonalProviderEntry) -> String {
    if entry.id != agent_doctor_core::setup::TEAMUPS_OFFICIAL_PROVIDER_ID {
        return entry.url.clone();
    }
    let trimmed = entry.url.trim().trim_end_matches('/');
    let Some(base) = trimmed.strip_suffix("/api/v1/official") else {
        return entry.url.clone();
    };
    if agent_doctor_core::setup::normalize_protocol(&entry.protocol)
        == agent_doctor_core::setup::PROTOCOL_ANTHROPIC
    {
        format!("{base}/api/v1/agent/anthropic")
    } else {
        format!("{base}/api/v1/agent")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(
        id: &str,
        url: &str,
        protocol: &str,
    ) -> agent_doctor_core::setup::PersonalProviderEntry {
        agent_doctor_core::setup::PersonalProviderEntry {
            id: id.to_string(),
            name: "test".to_string(),
            url: url.to_string(),
            api_key: "k".to_string(),
            model: "m".to_string(),
            protocol: protocol.to_string(),
        }
    }

    #[test]
    fn official_provider_rides_the_agent_channel() {
        let openai = provider(
            "teamups-official",
            "https://agentdoctor.cn/api/v1/official",
            "openai",
        );
        assert_eq!(
            official_agent_gateway_url(&openai),
            "https://agentdoctor.cn/api/v1/agent"
        );
        let anthropic = provider(
            "teamups-official",
            "https://agentdoctor.cn/api/v1/official/",
            "anthropic",
        );
        assert_eq!(
            official_agent_gateway_url(&anthropic),
            "https://agentdoctor.cn/api/v1/agent/anthropic"
        );
    }

    #[test]
    fn personal_providers_and_custom_urls_stay_untouched() {
        let deepseek = provider("deepseek", "https://api.deepseek.com/v1", "openai");
        assert_eq!(
            official_agent_gateway_url(&deepseek),
            "https://api.deepseek.com/v1"
        );
        // 手改过的官方行（marker 不在末尾）不动，避免悄悄改用户的自定义配置
        let edited = provider(
            "teamups-official",
            "https://mirror.example/official/api/v1",
            "openai",
        );
        assert_eq!(
            official_agent_gateway_url(&edited),
            "https://mirror.example/official/api/v1"
        );
    }

    #[test]
    fn browser_definitions_cover_the_chat_tool_set() {
        let tools = browser_tool_definitions();
        let names: Vec<&str> = tools
            .as_array()
            .expect("array")
            .iter()
            .map(|tool| {
                tool.pointer("/function/name")
                    .and_then(Value::as_str)
                    .expect("name")
            })
            .collect();
        assert_eq!(
            names,
            [
                "browser_navigate",
                "browser_snapshot",
                "browser_click",
                "browser_fill",
                "browser_find",
                "browser_screenshot",
                "browser_get_text"
            ]
        );
    }

    #[test]
    fn computer_definitions_cover_the_screen_tool_set() {
        let tools = computer_tool_definitions();
        let names: Vec<&str> = tools
            .as_array()
            .expect("array")
            .iter()
            .map(|tool| {
                tool.pointer("/function/name")
                    .and_then(Value::as_str)
                    .expect("name")
            })
            .collect();
        assert_eq!(
            names,
            [
                "screen_apps",
                "screen_snapshot",
                "screen_click",
                "screen_type",
                "screen_open",
                "screen_scroll",
                "screen_press",
                "screen_windows",
                "screen_state",
                "screen_set",
                "screen_menu",
            ]
        );
    }

    #[test]
    fn repair_and_read_only_definitions_stay_hands_free() {
        let hands_free = |tools: &Value| {
            tools.as_array().expect("array").iter().all(|tool| {
                let name = tool
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .expect("name");
                !name.starts_with("browser_") && !name.starts_with("screen_")
            })
        };
        assert!(hands_free(&repair_tool_definitions()));
        assert!(hands_free(&read_only_tool_definitions()));
    }

    #[test]
    fn hand_definitions_do_not_bleed_into_each_other() {
        let names = |tools: &Value| -> Vec<String> {
            tools
                .as_array()
                .expect("array")
                .iter()
                .map(|tool| {
                    tool.pointer("/function/name")
                        .and_then(Value::as_str)
                        .expect("name")
                        .to_string()
                })
                .collect()
        };
        assert!(names(&browser_tool_definitions())
            .iter()
            .all(|name| !name.starts_with("screen_")));
        assert!(names(&computer_tool_definitions())
            .iter()
            .all(|name| !name.starts_with("browser_")));
    }
}
