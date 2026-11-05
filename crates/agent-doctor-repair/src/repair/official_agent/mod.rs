//! The official built-in agent loop ("Agent Doctor").
//!
//! One engine for two entry points: the desktop chat (runtime id `doctor`,
//! tools with permission gating) and deep diagnose (read-only profile). The
//! mechanics — failure bookkeeping with loop detection, per-tool result
//! profiles, context-overflow recovery, truncated-JSON recovery, a typed
//! plan-control tool — are ported from SkillLite's `skilllite-agent`.
//!
//! Safety model unchanged: secrets stay masked in the vault, bash stays on the
//! allowlist, runtime config files are only ever written by the repair flow
//! (backup → typed actions → re-probe → audit).

mod plan;
mod recover;
mod text;

pub use plan::{
    parse_plan_steps, planning_control_for, update_plan_tool_definition, PlanState, PlanStep,
    PlanningControl,
};
pub use recover::{is_recoverable_write_tool, recover_truncated_arguments};
pub use text::{head_tail, process_tool_result, READ_FILE_RESULT_MAX_CHARS, TOOL_RESULT_MAX_CHARS};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use agent_doctor_core::mask::SecretVault;
use agent_doctor_core::repair_types::{LlmConfig, RepairToolKind};
use agent_doctor_core::usage::TokenUsage;
use agent_doctor_mcp::{LazyBrowser, SharedBrowser, DEFAULT_BROWSER_MCP_PORT};

use super::llm::stream::{chat_with_tool_set_stream, LlmStreamDelta};
use super::llm::{
    browser_tool_definitions, computer_tool_definitions, max_agent_turns,
    read_only_tool_definitions, repair_tool_definitions,
};
use super::tools::{
    allowed_paths_for_official_agent, parse_tool_call, path_is_allowed, RepairToolExecutor,
    OFFICIAL_AGENT_RUNTIME_ID,
};

/// One browser session for the whole app process. Chrome starts on the first
/// browser tool call and stays open across turns — the page the person asked
/// for must not close when the turn ends — and concurrent chat sessions share
/// one managed instance instead of killing each other's browser. The
/// `LazyBrowser` inside never drops, so Chrome outlives the app; the next
/// app run reuses it over the same CDP port.
fn chat_browser_session() -> &'static Arc<SharedBrowser> {
    static SESSION: OnceLock<Arc<SharedBrowser>> = OnceLock::new();
    SESSION.get_or_init(|| {
        Arc::new(SharedBrowser::new(LazyBrowser::new(
            DEFAULT_BROWSER_MCP_PORT,
            false,
        )))
    })
}

/// Tool rounds per turn (same bound as the repair planner).
pub const OFFICIAL_AGENT_MAX_ROUNDS: usize = max_agent_turns();
/// Consecutive failed tool calls before the loop stops and answers anyway.
const MAX_CONSECUTIVE_FAILURES: usize = 5;
/// Repeat signature threshold that appends the loop-detected warning.
const REPEATED_FAILURE_THRESHOLD: usize = 2;
/// Context-overflow recovery: at most this many shrink-and-retry attempts.
const MAX_OVERFLOW_RETRIES: usize = 3;
/// Tool-message char budget the overflow recovery truncates down to.
const OVERFLOW_RECOVERY_BUDGET: usize = 3_000;
/// Browser flows need more rounds than file repairs (navigate → snapshot →
/// click → snapshot …), so chat turns with browser tools get this floor.
const BROWSER_AGENT_MIN_ROUNDS: usize = 16;

#[derive(Debug, Clone)]
pub struct OfficialAgentOptions {
    /// Full user turn (chat passes the transcript-prefixed prompt).
    pub prompt: String,
    /// `en` for English replies; anything else answers in Simplified Chinese.
    pub locale: Option<String>,
    /// Project folder for this session; the only place this agent writes files.
    pub workspace: Option<PathBuf>,
    /// Read-only profile (deep diagnose): no write tools, no bash, no plan tool.
    pub read_only: bool,
    /// Apply approved writes; `false` keeps every write as a preview.
    pub apply_writes: bool,
    pub max_rounds: usize,
    /// Replace the default chat system prompt (deep diagnose keeps its own).
    pub system_override: Option<String>,
    /// Extra system-prompt block appended after the base prompt.
    pub system_extra: Option<String>,
    /// Narrow the tool path scope (deep diagnose: one runtime's config roots).
    /// `None` uses the official-agent union of all runtime roots + workspace.
    pub allowed_paths_override: Option<Vec<PathBuf>>,
    /// Session vault to keep unmasking across tool calls.
    pub vault: SecretVault,
    /// Messages inserted between the system prompt and the user turn
    /// (deep diagnose: masked rule-check context + trimmed history).
    pub prelude_messages: Vec<Value>,
    /// Offer the browser tools (Settings → 对话「操作网页」). Deep diagnose
    /// ignores this via `read_only`.
    pub browser_tools: bool,
    /// Offer the `screen_*` computer-use tools (Settings → 对话「操作电脑」).
    /// Off by default: these drive the person's real screen.
    pub computer_tools: bool,
}

impl OfficialAgentOptions {
    /// Chat profile of the built-in agent. Browser follows the switch default
    /// (on unless held off); computer use must be explicitly enabled.
    pub fn for_chat(prompt: String, locale: Option<String>, workspace: Option<PathBuf>) -> Self {
        Self {
            prompt,
            locale,
            workspace,
            read_only: false,
            apply_writes: true,
            max_rounds: OFFICIAL_AGENT_MAX_ROUNDS,
            system_override: None,
            system_extra: None,
            allowed_paths_override: None,
            vault: SecretVault::default(),
            prelude_messages: Vec::new(),
            browser_tools: true,
            computer_tools: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OfficialAgentEvent {
    /// Waiting on the model.
    Thinking { round: u32 },
    /// The agent is reading, searching, editing, or running a command.
    Tool {
        id: String,
        tool: String,
        target: Option<String>,
    },
    /// The same call finished; hosts pair the two by `id` into one step card.
    ToolResult {
        id: String,
        tool: String,
        success: bool,
    },
    /// Reasoning text that a model inlined as `<think>…</think>`.
    Reasoning { text: String },
    /// Incremental reply text while the model writes it.
    Delta { text: String },
    /// Plan steps for the chat plan UI.
    Plan { steps: Vec<PlanStep> },
    /// A write or allowlisted command needs the user's approval.
    Confirmation {
        request_id: String,
        tool: String,
        detail: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfficialAgentReport {
    pub answer: String,
    /// Reasoning models sometimes inline `<think>…</think>` in the content;
    /// the loop strips it out of the answer and hands it back separately so
    /// hosts can show it as a thinking step.
    pub reasoning: Option<String>,
    /// The reply already streamed as `Delta` events; hosts must not paint the
    /// full answer again.
    pub streamed: bool,
    pub model: String,
    pub tool_calls: usize,
    pub rounds: usize,
    /// Every model call in the turn added up, so the chat can bill the turn.
    #[serde(default)]
    pub usage: TokenUsage,
}

/// Split inline `<think>…</think>` blocks out of model content. Handles
/// several blocks and an unclosed block that runs to the end of the reply.
pub fn strip_think_blocks(content: &str) -> (String, Option<String>) {
    let mut reasoning_parts: Vec<String> = Vec::new();
    let mut clean = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("<think>") {
        clean.push_str(&rest[..start]);
        let after = &rest[start + "<think>".len()..];
        match after.find("</think>") {
            Some(end) => {
                reasoning_parts.push(after[..end].trim().to_string());
                rest = &after[end + "</think>".len()..];
            }
            None => {
                reasoning_parts.push(after.trim().to_string());
                rest = "";
            }
        }
    }
    clean.push_str(rest);
    let clean = clean.trim().to_string();
    let reasoning = if reasoning_parts.is_empty() {
        None
    } else {
        Some(reasoning_parts.join("\n\n").trim().to_string())
    };
    (clean, reasoning)
}

/// Approval hook for writes and allowlisted bash. Blocks until the user
/// answers; returning `false` skips the call without counting as a failure.
pub trait PermissionGate {
    fn confirm(&mut self, request_id: &str, tool: &str, detail: &str) -> bool;
}

/// Gate for callers that pre-approved everything (repair planner, tests).
pub struct AutoApproveGate;

impl PermissionGate for AutoApproveGate {
    fn confirm(&mut self, _request_id: &str, _tool: &str, _detail: &str) -> bool {
        true
    }
}

/// Failure bookkeeping ported from SkillLite's `ExecutionState`: repeated
/// same-failure signatures surface a loop warning, consecutive failures stop
/// the loop.
#[derive(Default)]
struct LoopState {
    tool_calls: usize,
    consecutive_failures: usize,
    repeated_sig: Option<(String, String)>,
    repeated_count: usize,
}

impl LoopState {
    fn signature(tool: &str, content: &str) -> (String, String) {
        let head: String = content.chars().take(80).collect();
        (tool.to_string(), head)
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.repeated_sig = None;
        self.repeated_count = 0;
    }

    /// Returns the loop-detected warning to append when this failure keeps
    /// repeating.
    fn record_failure(&mut self, tool: &str, content: &str) -> Option<&'static str> {
        self.consecutive_failures += 1;
        let sig = Self::signature(tool, content);
        if self.repeated_sig.as_ref() == Some(&sig) {
            self.repeated_count += 1;
        } else {
            self.repeated_sig = Some(sig);
            self.repeated_count = 1;
        }
        (self.repeated_count >= REPEATED_FAILURE_THRESHOLD)
            .then_some("⚠️ loop detected: this tool call keeps failing the same way; change approach or answer without it.")
    }
}

fn is_context_overflow_error(message: &str) -> bool {
    let lowered = message.to_lowercase();
    [
        "context_length_exceeded",
        "maximum context length",
        "token limit",
        "too many tokens",
        "context window",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

/// Shrink every tool result in the transcript to `max_chars` (head+tail).
fn truncate_tool_messages(messages: &mut [Value], max_chars: usize) {
    for message in messages.iter_mut() {
        if message.get("role").and_then(Value::as_str) == Some("tool") {
            if let Some(content) = message.get("content").and_then(Value::as_str) {
                let truncated = head_tail(content, max_chars);
                message["content"] = json!(truncated);
            }
        }
    }
}

pub fn run_official_agent_turn(
    options: &OfficialAgentOptions,
    config: &LlmConfig,
    cancel: &AtomicBool,
    mut on_event: impl FnMut(OfficialAgentEvent),
    gate: &mut dyn PermissionGate,
) -> Result<OfficialAgentReport> {
    let prompt = options.prompt.trim();
    if prompt.is_empty() {
        bail!("prompt must not be empty");
    }
    ensure_not_cancelled(cancel)?;

    let zh = options.locale.as_deref() != Some("en");
    // Hands tools ride only on full chat turns: deep diagnose stays
    // file-read-only, the thread-local gate can still hold them off for a
    // turn, and computer use additionally needs its explicit switch.
    let browser_enabled =
        !options.read_only && options.browser_tools && crate::repair::browser_tools_allowed();
    let computer_enabled =
        !options.read_only && options.computer_tools && crate::repair::browser_tools_allowed();
    let hands_enabled = browser_enabled || computer_enabled;
    let tools = if options.read_only {
        read_only_tool_definitions()
    } else {
        let mut all = repair_tool_definitions();
        if let Value::Array(items) = &mut all {
            items.push(update_plan_tool_definition());
            if browser_enabled {
                if let Value::Array(browser_items) = browser_tool_definitions() {
                    items.extend(browser_items);
                }
            }
            if computer_enabled {
                if let Value::Array(screen_items) = computer_tool_definitions() {
                    items.extend(screen_items);
                }
            }
        }
        all
    };

    let mut messages = vec![json!({
        "role": "system",
        "content": options
            .system_override
            .clone()
            .unwrap_or_else(|| system_prompt(options, zh, browser_enabled, computer_enabled)),
    })];
    messages.extend(options.prelude_messages.iter().cloned());
    messages.push(json!({ "role": "user", "content": prompt }));

    let allowed_paths = options
        .allowed_paths_override
        .clone()
        .unwrap_or_else(|| allowed_paths_for_official_agent(options.workspace.as_deref()));
    let mut executor = RepairToolExecutor::with_allowed_paths(
        OFFICIAL_AGENT_RUNTIME_ID,
        options.vault.clone(),
        options.apply_writes,
        allowed_paths,
    );
    if browser_enabled {
        executor.set_browser(chat_browser_session().clone());
    }
    executor.set_screen_enabled(computer_enabled);
    let mut plan_state = PlanState::default();
    let mut loop_state = LoopState::default();
    let mut overflow_retries = 0usize;
    let mut confirmation_seq = 0usize;
    let mut rounds_used = 0usize;
    let mut streamed_any = false;
    // Every model call in this turn, added up for the usage ledger.
    let mut used = TokenUsage::default();
    let max_rounds = if hands_enabled {
        options.max_rounds.max(BROWSER_AGENT_MIN_ROUNDS)
    } else {
        options.max_rounds
    };

    for round in 1..=max_rounds {
        ensure_not_cancelled(cancel)?;
        on_event(OfficialAgentEvent::Thinking {
            round: round as u32,
        });

        let turn = {
            let mut saw_text = false;
            let call_result = {
                let mut on_delta = |delta: LlmStreamDelta| match delta {
                    LlmStreamDelta::Text(text) => {
                        saw_text = true;
                        on_event(OfficialAgentEvent::Delta { text });
                    }
                    LlmStreamDelta::Reasoning(text) => {
                        on_event(OfficialAgentEvent::Reasoning { text });
                    }
                };
                chat_with_tool_set_stream(config, &messages, Some(&tools), &mut on_delta)
            };
            streamed_any = streamed_any || saw_text;
            match call_result {
                Ok(turn) => turn,
                Err(error) => {
                    let message = format!("{error:#}");
                    if is_context_overflow_error(&message)
                        && overflow_retries < MAX_OVERFLOW_RETRIES
                    {
                        overflow_retries += 1;
                        let budget = OVERFLOW_RECOVERY_BUDGET / (1 << (overflow_retries - 1));
                        truncate_tool_messages(&mut messages, budget);
                        continue;
                    }
                    return Err(error);
                }
            }
        };
        rounds_used = round;
        used.add(&turn.usage);

        if turn.tool_calls.is_empty() {
            let (answer, reasoning) = strip_think_blocks(turn.content.as_deref().unwrap_or(""));
            return Ok(OfficialAgentReport {
                answer,
                reasoning,
                streamed: streamed_any,
                model: config.model.clone(),
                tool_calls: loop_state.tool_calls,
                rounds: rounds_used,
                usage: used,
            });
        }

        let (clean_content, inline_reasoning) =
            strip_think_blocks(turn.content.as_deref().unwrap_or(""));
        if let Some(reasoning) = inline_reasoning {
            on_event(OfficialAgentEvent::Reasoning { text: reasoning });
        }
        messages.push(json!({
            "role": "assistant",
            "content": clean_content,
            "tool_calls": turn.tool_calls.iter().map(|call| json!({
                "id": call.id,
                "type": "function",
                "function": { "name": call.name, "arguments": call.arguments }
            })).collect::<Vec<_>>()
        }));

        let mut stopped_for_failures = false;
        for call in turn.tool_calls {
            ensure_not_cancelled(cancel)?;
            loop_state.tool_calls += 1;

            // Typed planning control first: the loop never dispatches these
            // through the executor.
            if let Some(PlanningControl::UpdatePlan) = planning_control_for(&call.name) {
                let content = match parse_plan_steps(&call.arguments) {
                    Ok(steps) => {
                        // Merge first, emit the merged snapshot: the desktop
                        // paints exactly what it receives, and emitting the
                        // pre-update state left it one update behind — the
                        // final "all done" call still showed every step open.
                        let ack = plan_state.apply_update(steps);
                        on_event(OfficialAgentEvent::Plan {
                            steps: plan_state.steps.clone(),
                        });
                        ack
                    }
                    Err(error) => json!({ "error": error }).to_string(),
                };
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": content
                }));
                continue;
            }

            let mut recovered_note: Option<String> = None;
            let mut parsed = match parse_tool_call(&call.name, &call.arguments) {
                Ok(parsed) => parsed,
                // Model output budget may cut write-tool arguments mid-string;
                // salvage what we can rather than losing the call.
                Err(error) if is_recoverable_write_tool(&call.name) => {
                    match recover_truncated_arguments(&call.name, &call.arguments) {
                        Some(recovered) => {
                            let arguments = recovered.to_string();
                            match parse_tool_call(&call.name, &arguments) {
                                Ok(parsed) => {
                                    recovered_note = Some(
                                        "Recovered from a truncated JSON call: the written content may be incomplete. Keep future files smaller or split them.".to_string(),
                                    );
                                    parsed
                                }
                                Err(retry_error) => {
                                    messages.push(json!({
                                        "role": "tool",
                                        "tool_call_id": call.id,
                                        "content": json!({ "error": retry_error.to_string() }).to_string()
                                    }));
                                    continue;
                                }
                            }
                        }
                        None => {
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": call.id,
                                "content": json!({ "error": error.to_string() }).to_string()
                            }));
                            continue;
                        }
                    }
                }
                Err(error) => {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call.id,
                        "content": json!({ "error": error.to_string() }).to_string()
                    }));
                    continue;
                }
            };

            if options.read_only && !is_read_only(parsed.kind) {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": json!({ "error": "deep diagnose can only read files" }).to_string()
                }));
                continue;
            }

            // Models often pass relative paths; resolve them against the
            // workspace so the executor's absolute-path validation accepts them.
            if let Some(path) = parsed.path.as_deref() {
                if !path.trim().is_empty() && !Path::new(path).is_absolute() {
                    if let Some(workspace) = options.workspace.as_deref() {
                        parsed.path = Some(workspace.join(path).to_string_lossy().into_owned());
                    }
                }
            }

            let tool_target_label = parsed
                .path
                .clone()
                .or_else(|| parsed.pattern.clone())
                .or_else(|| {
                    parsed.browser.as_ref().and_then(|browser| {
                        browser
                            .url
                            .clone()
                            .or_else(|| browser.target.clone())
                            .or_else(|| browser.query.clone())
                    })
                })
                .or_else(|| {
                    parsed.computer.as_ref().and_then(|computer| {
                        ["app", "label", "text", "ref"].iter().find_map(|key| {
                            computer
                                .arguments
                                .get(*key)
                                .and_then(Value::as_str)
                                .filter(|t| !t.trim().is_empty())
                                .map(str::to_string)
                        })
                    })
                });
            let call_id = format!("call-{}", loop_state.tool_calls);
            on_event(OfficialAgentEvent::Tool {
                id: call_id.clone(),
                tool: call.name.clone(),
                target: tool_target_label,
            });

            // Permission gate for writes, allowlisted bash, and screen actions.
            // Runtime config files are refused outright: repairs only happen in
            // the repair flow where every write is backed up first. Screen
            // read-only looks (snapshot/apps/windows/state) skip the card;
            // screen actions drive the real desktop, so they ask first.
            let requires_approval = if parsed.kind == RepairToolKind::Screen {
                parsed
                    .computer
                    .as_ref()
                    .map_or(true, |computer| screen_tool_needs_approval(&computer.tool))
            } else {
                needs_permission(parsed.kind)
            };
            if requires_approval && !options.read_only {
                if let Err(refuse) = check_write_scope(
                    parsed.kind,
                    parsed.path.as_deref(),
                    options.workspace.as_deref(),
                    zh,
                ) {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call.id,
                        "content": json!({ "error": refuse }).to_string()
                    }));
                    continue;
                }
                confirmation_seq += 1;
                let request_id = format!("oc-{confirmation_seq}");
                let detail = match parsed.computer.as_ref() {
                    Some(computer) => format!(
                        "{} {}",
                        computer.tool,
                        serde_json::to_string(&computer.arguments).unwrap_or_default()
                    ),
                    None => permission_detail(
                        parsed.kind,
                        parsed.path.as_deref(),
                        parsed.command.as_deref(),
                    ),
                };
                on_event(OfficialAgentEvent::Confirmation {
                    request_id: request_id.clone(),
                    tool: call.name.clone(),
                    detail: detail.clone(),
                });
                if !gate.confirm(&request_id, &call.name, &detail) {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call.id,
                        "content": json!({ "declined": true, "message": "User declined this action." }).to_string()
                    }));
                    continue;
                }
            }

            let result = executor.execute(&parsed)?;
            on_event(OfficialAgentEvent::ToolResult {
                id: call_id,
                tool: call.name.clone(),
                success: result.success,
            });
            let mut content = process_tool_result(&call.name, &serde_json::to_string(&result)?);
            if let Some(note) = &recovered_note {
                content = format!("⚠️ {note}\n\n{content}");
            }
            if !result.success {
                if let Some(warning) = loop_state.record_failure(&call.name, &content) {
                    content = format!("{content}\n{warning}");
                }
                if loop_state.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call.id,
                        "content": content
                    }));
                    stopped_for_failures = true;
                    break;
                }
            } else {
                loop_state.record_success();
            }
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call.id,
                "content": content
            }));
        }

        if stopped_for_failures {
            break;
        }
    }

    // Out of rounds (or out of patience): answer with what is known.
    ensure_not_cancelled(cancel)?;
    messages.push(json!({
        "role": "user",
        "content": if zh {
            "请根据目前掌握的信息直接回答，不要再调用工具。"
        } else {
            "Answer now with what you have. Do not call more tools."
        }
    }));
    let turn = {
        let mut saw_text = false;
        let mut on_delta = |delta: LlmStreamDelta| match delta {
            LlmStreamDelta::Text(text) => {
                saw_text = true;
                on_event(OfficialAgentEvent::Delta { text });
            }
            LlmStreamDelta::Reasoning(text) => {
                on_event(OfficialAgentEvent::Reasoning { text });
            }
        };
        let call_result = chat_with_tool_set_stream(config, &messages, None, &mut on_delta);
        streamed_any = streamed_any || saw_text;
        call_result?
    };
    let (answer, reasoning) = strip_think_blocks(turn.content.as_deref().unwrap_or(""));
    used.add(&turn.usage);
    Ok(OfficialAgentReport {
        answer,
        reasoning,
        streamed: streamed_any,
        model: config.model.clone(),
        tool_calls: loop_state.tool_calls,
        rounds: rounds_used,
        usage: used,
    })
}

fn ensure_not_cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::SeqCst) {
        bail!("cancelled: stopped by user");
    }
    Ok(())
}

fn is_read_only(kind: RepairToolKind) -> bool {
    matches!(
        kind,
        RepairToolKind::Read | RepairToolKind::ListDir | RepairToolKind::GrepFiles
    )
}

fn needs_permission(kind: RepairToolKind) -> bool {
    matches!(
        kind,
        RepairToolKind::WriteFile
            | RepairToolKind::SearchReplace
            | RepairToolKind::PatchConfig
            | RepairToolKind::Bash
    )
}

/// Screen read-only looks skip the approval card; everything that clicks,
/// types, or moves the real desktop asks first.
fn screen_tool_needs_approval(tool: &str) -> bool {
    !matches!(
        tool,
        "screen_snapshot" | "screen_apps" | "screen_windows" | "screen_state"
    )
}

/// Writes outside the project workspace are refused: runtime config edits must
/// go through the repair flow (backup → typed actions → audit).
fn check_write_scope(
    kind: RepairToolKind,
    path: Option<&str>,
    workspace: Option<&Path>,
    zh: bool,
) -> std::result::Result<(), String> {
    if kind == RepairToolKind::Bash {
        return Ok(());
    }
    let Some(path) = path else {
        return Ok(()); // executor reports the missing path itself
    };
    let in_workspace = workspace
        .map(|root| path_is_allowed(Path::new(path), &[root.to_path_buf()]))
        .unwrap_or(false);
    if in_workspace {
        return Ok(());
    }
    Err(if zh {
        "这个文件不在当前项目文件夹里。runtime 配置只能在修复流程里修改（会先备份）：诊断页「让 Agent Doctor 修」，或命令行 agent-doctor repair <runtime> --apply。".to_string()
    } else {
        "That file is outside the project folder. Runtime configs can only be changed in the repair flow (backed up first): the diagnose screen's \"Let Agent Doctor fix it\", or `agent-doctor repair <runtime> --apply`.".to_string()
    })
}

fn permission_detail(kind: RepairToolKind, path: Option<&str>, command: Option<&str>) -> String {
    match kind {
        RepairToolKind::Bash => command.unwrap_or_default().to_string(),
        RepairToolKind::PatchConfig => path.unwrap_or_default().to_string(),
        _ => path.unwrap_or_default().to_string(),
    }
}
fn system_prompt(
    options: &OfficialAgentOptions,
    zh: bool,
    browser_enabled: bool,
    computer_enabled: bool,
) -> String {
    let mut prompt = if zh {
        "你是 Agent Doctor，这台电脑上官方内置的助手。\n\
        - 你可以读取本机各 Agent（OpenClaw、Hermes、Claude Code、Codex、DeepSeek Harness）的配置文件来回答问题，密钥会显示成 {{{{SECRET:n}}}}。不要猜密钥，也不要让用户把密钥发给你。\n\
        - 下结论必须分级，不许越级：有报错日志、401/403、启动失败或实际调用失败证据的，才是「已确认故障」；只有静态配置差异、没有运行失败证据的，写「待验证」；配置偏宽松但功能正常，是「安全加固」建议。用户亲测能用是最直接的运行证据，优先于扫描发现的配置缺口。\n\
        - 凭据来源不止 .env：进程继承的环境变量、Shell 启动文件、凭据存储、应用自身的凭据机制、外部启动器都可能提供密钥。「配置文件里没看到某个变量」只能写「凭据来源未确认，属待验证」，禁止写成「无法调用」或「高概率失败」。\n\
        - 你可以在当前项目文件夹里新建或修改文件，但每次写入或改动前都会先征求用户同意。\n\
        - 你可以运行白名单里的官方安装/更新命令（同样先征求同意）。不在此列的命令不要尝试。\n\
        - runtime 的配置文件不要直接改：引导用户走修复流程（会先备份）。这个对话里只能改项目文件夹内的文件。\n\
        - 需要多步才能完成的任务，先用 update_plan 列出步骤，边做边更新。\n\
        - 工具调用的每一步界面都会实时显示，回复里不要再逐步复述（“我打开了…然后我查看了…”这类流水账不要写）。中间过程安静执行；正文只在有真正结论、发现或需要确认时才说话。\n\
        - 回答先给结论，再给必要细节；用简体中文。不确定就直说，不要编造。"
        .to_string()
    } else {
        "You are Agent Doctor, the built-in assistant on this computer.\n\
        - You may read the config files of the agents installed here (OpenClaw, Hermes, Claude Code, Codex, DeepSeek Harness) to answer questions. Secrets appear as {{{{SECRET:n}}}}. Never guess them or ask the person to send a key.\n\
        - Grade every conclusion, never escalate: a confirmed fault needs evidence — error logs, 401/403, a startup failure, or a failed real call. A static config difference with no runtime failure evidence is \"unverified\". A loose config that still works is a hardening tip. First-hand \"it works\" from the person outranks a gap found by static scanning.\n\
        - Credentials do not live only in the .env: inherited process env, shell startup files, credential stores, the agent's own lookup, or external launchers can all supply keys. \"The variable is not in this file\" may only be worded as \"credential source unverified\" — never as \"cannot call\" or \"likely failing\".\n\
        - You may create or edit files inside the current project folder; every write asks the user for approval first.\n\
        - You may run allowlisted official install/update commands (also approved first). Never try commands outside that list.\n\
        - Do not edit runtime config files directly: guide the person to the repair flow (which backs up first). In this chat you only edit files inside the project folder.\n\
        - For multi-step requests, publish your steps with update_plan first and keep it current.\n\
        - Every tool call is already shown live in the UI, so never narrate your steps back in the reply (no play-by-play like \"I opened… then I checked…\"). Work quietly; speak in the reply only when there is a real finding, conclusion, or something to confirm.\n\
        - Lead with the answer, then the details. If you are unsure, say so. Do not make things up."
        .to_string()
    };
    if browser_enabled {
        if zh {
            prompt.push_str(
                "\n- 你可以直接操作浏览器完成“打开某网站”“在网页上找按钮点一下”“截图”这类请求：browser_navigate 打开网页，browser_snapshot 获取可点击元素及其 @eN 引用（点击/填写前必须先调用），browser_click / browser_fill 点击和填写，browser_find 按文字或角色找元素，browser_screenshot 截图并告诉用户图片保存在哪，browser_get_text 读取页面文字。浏览器用独立的自动化配置，不共享日常浏览器的登录态。",
            );
        } else {
            prompt.push_str(
                "\n- You can drive the browser for requests like \"open this site\", \"find the login button and click it\", or \"take a screenshot\": browser_navigate opens a URL, browser_snapshot lists clickable elements with @eN refs (always call it before click/fill), browser_click / browser_fill act on elements, browser_find locates by text or role, browser_screenshot saves a PNG (tell the person where), browser_get_text reads page text. The browser runs an isolated automation profile; it does not share the everyday browser's logins.",
            );
        }
    }
    if computer_enabled {
        if zh {
            prompt.push_str(
                "\n- 你可以操作这台电脑上已打开的应用（用户已打开「操作电脑」开关）：screen_open 按确切名字打开/切换应用（不要点 Dock、Spotlight、启动台），screen_snapshot 把窗口控件编号成 @e1、@e2…（点击前必须先调用，界面变化后要重新调用），screen_click / screen_type / screen_set / screen_press / screen_scroll 操作控件，screen_apps / screen_windows / screen_state 查看现状。每次实际点击前都会先征求用户同意。",
            );
        } else {
            prompt.push_str(
                "\n- You can operate apps already open on this computer (the person turned the computer-use switch on): screen_open opens/switches apps by exact name (never the Dock, Spotlight, or Launchpad), screen_snapshot numbers a window's controls @e1, @e2… (always call it before clicking, and again after anything changes), screen_click / screen_type / screen_set / screen_press / screen_scroll act on controls, screen_apps / screen_windows / screen_state report the current state. Every real click asks the user first.",
            );
        }
    }
    if let Some(extra) = &options.system_extra {
        prompt.push_str("\n\n");
        prompt.push_str(extra);
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_offers_browser_only_when_enabled() {
        let options = OfficialAgentOptions::for_chat("hi".into(), None, None);
        let with_browser = system_prompt(&options, true, true, false);
        assert!(with_browser.contains("browser_navigate"));
        assert!(with_browser.contains("browser_snapshot"));
        assert!(!with_browser.contains("screen_snapshot"));
        let without_browser = system_prompt(&options, true, false, false);
        assert!(!without_browser.contains("browser_navigate"));
        let english = system_prompt(&options, false, true, false);
        assert!(english.contains("isolated automation profile"));
    }

    #[test]
    fn system_prompt_offers_screen_only_when_enabled() {
        let options = OfficialAgentOptions::for_chat("hi".into(), None, None);
        let with_screen = system_prompt(&options, true, false, true);
        assert!(with_screen.contains("screen_open"));
        assert!(with_screen.contains("screen_snapshot"));
        assert!(!with_screen.contains("browser_navigate"));
        let without_screen = system_prompt(&options, true, false, false);
        assert!(!without_screen.contains("screen_open"));
    }

    #[test]
    fn system_prompt_grades_evidence_before_calling_something_broken() {
        let options = OfficialAgentOptions::for_chat("hi".into(), None, None);
        for prompt in [
            system_prompt(&options, true, false, false),
            system_prompt(&options, false, false, false),
        ] {
            // A static gap alone is never a fault; user-verified wins.
            assert!(prompt.contains("401/403"), "evidence bar: {prompt}");
            assert!(prompt.contains("待验证") || prompt.contains("unverified"));
            assert!(
                prompt.contains("亲测能用") || prompt.contains("it works"),
                "user-verified outranks static gaps: {prompt}"
            );
            // Credential sources beyond the .env file, and the forbidden wording.
            assert!(prompt.contains(".env"), "credential sources: {prompt}");
            assert!(
                prompt.contains("凭据来源未确认")
                    || prompt.contains("credential source unverified"),
                "unverified wording: {prompt}"
            );
            assert!(
                prompt.contains("禁止写成「无法调用」")
                    || prompt.contains("never as \"cannot call\""),
                "forbidden phrasing: {prompt}"
            );
        }
    }

    #[test]
    fn screen_action_tools_ask_but_reads_do_not() {
        assert!(!screen_tool_needs_approval("screen_snapshot"));
        assert!(!screen_tool_needs_approval("screen_apps"));
        assert!(!screen_tool_needs_approval("screen_windows"));
        assert!(!screen_tool_needs_approval("screen_state"));
        assert!(screen_tool_needs_approval("screen_click"));
        assert!(screen_tool_needs_approval("screen_open"));
        assert!(screen_tool_needs_approval("screen_type"));
        assert!(screen_tool_needs_approval("screen_press"));
        assert!(screen_tool_needs_approval("screen_menu"));
    }

    #[test]
    fn failure_loop_detection_warns_on_repeat() {
        let mut state = LoopState::default();
        assert!(state
            .record_failure("bash", "command not on repair allowlist")
            .is_none());
        let warning = state
            .record_failure("bash", "command not on repair allowlist")
            .expect("second identical failure warns");
        assert!(warning.contains("loop detected"));
        assert!(state
            .record_failure("bash", "different error entirely")
            .is_none());
    }

    #[test]
    fn success_resets_failure_counters() {
        let mut state = LoopState::default();
        state.record_failure("bash", "boom");
        state.record_success();
        assert_eq!(state.consecutive_failures, 0);
        assert!(
            state.record_failure("bash", "boom").is_none(),
            "fresh failure starts over"
        );
    }

    #[test]
    fn consecutive_failure_limit_stops_the_loop() {
        let mut state = LoopState::default();
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            state.record_failure("bash", "boom");
        }
        assert!(state.consecutive_failures >= MAX_CONSECUTIVE_FAILURES);
    }

    #[test]
    fn overflow_detector_matches_provider_phrases() {
        assert!(is_context_overflow_error(
            "LLM HTTP 400: {\"error\":{\"code\":\"context_length_exceeded\"}}"
        ));
        assert!(is_context_overflow_error(
            "This model's maximum context length is 8192 tokens"
        ));
        assert!(!is_context_overflow_error("LLM HTTP 401: bad key"));
    }

    #[test]
    fn truncation_shrinks_only_tool_messages() {
        let mut messages = vec![
            json!({"role": "user", "content": "x".repeat(5_000)}),
            json!({"role": "tool", "tool_call_id": "t", "content": "y".repeat(5_000)}),
        ];
        truncate_tool_messages(&mut messages, 1_000);
        assert!(
            messages[0]["content"].as_str().unwrap().len() > 4_000,
            "user text untouched"
        );
        let squeezed = messages[1]["content"].as_str().unwrap();
        assert!(squeezed.chars().count() < 5_000);
        assert!(squeezed.contains("[content truncated"));
    }

    #[test]
    fn write_scope_refuses_runtime_configs_but_allows_workspace() {
        let workspace = std::env::temp_dir().join("ad-official-agent-scope-test");
        std::fs::create_dir_all(&workspace).unwrap();
        let inside = workspace.join("notes.md");
        std::fs::write(&inside, "x").unwrap();
        assert!(check_write_scope(
            RepairToolKind::SearchReplace,
            Some(&inside.to_string_lossy()),
            Some(&workspace),
            true
        )
        .is_ok());

        let outside = std::env::temp_dir().join("outside-scope.md");
        let error = check_write_scope(
            RepairToolKind::SearchReplace,
            Some(&outside.to_string_lossy()),
            Some(&workspace),
            true,
        )
        .unwrap_err();
        assert!(error.contains("修复流程") || error.contains("repair flow"));

        assert!(check_write_scope(RepairToolKind::Bash, None, Some(&workspace), true).is_ok());
        assert!(check_write_scope(
            RepairToolKind::SearchReplace,
            Some(&inside.to_string_lossy()),
            None,
            true
        )
        .is_err());
    }

    #[test]
    fn think_blocks_are_stripped_and_kept_as_reasoning() {
        let (clean, reasoning) = strip_think_blocks("<think>推理 A</think>回答正文");
        assert_eq!(clean, "回答正文");
        assert_eq!(reasoning.as_deref(), Some("推理 A"));

        let (clean, reasoning) = strip_think_blocks("<think>a</think>中<think>b</think>尾");
        assert_eq!(clean, "中尾");
        assert_eq!(reasoning.as_deref(), Some("a\n\nb"));

        // Unclosed block running to the end (truncated output budget).
        let (clean, reasoning) = strip_think_blocks("<think>只写了一半");
        assert_eq!(clean, "");
        assert_eq!(reasoning.as_deref(), Some("只写了一半"));

        let (clean, reasoning) = strip_think_blocks("没有标签");
        assert_eq!(clean, "没有标签");
        assert!(reasoning.is_none());
    }

    #[test]
    fn cancel_flag_stops_before_anything_runs() {
        let cancel = AtomicBool::new(true);
        let error = ensure_not_cancelled(&cancel).unwrap_err().to_string();
        assert!(error.starts_with("cancelled"));
    }

    #[test]
    fn chat_options_are_write_enabled_and_read_only_options_are_not() {
        let chat = OfficialAgentOptions::for_chat("hi".into(), None, None);
        assert!(!chat.read_only && chat.apply_writes);
    }
}
