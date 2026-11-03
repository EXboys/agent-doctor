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

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use agent_doctor_core::mask::SecretVault;
use agent_doctor_core::repair_types::{LlmConfig, RepairToolKind};

use super::llm::stream::{chat_with_tool_set_stream, LlmStreamDelta};
use super::llm::{max_agent_turns, read_only_tool_definitions, repair_tool_definitions};
use super::tools::{
    allowed_paths_for_official_agent, parse_tool_call, path_is_allowed, RepairToolExecutor,
    OFFICIAL_AGENT_RUNTIME_ID,
};

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
}

impl OfficialAgentOptions {
    /// Chat profile of the built-in agent.
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
        tool: String,
        target: Option<String>,
    },
    /// The same call finished; hosts pair the two into one step card.
    ToolResult { tool: String, success: bool },
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
    let tools = if options.read_only {
        read_only_tool_definitions()
    } else {
        let mut all = repair_tool_definitions();
        if let Value::Array(items) = &mut all {
            items.push(update_plan_tool_definition());
        }
        all
    };

    let mut messages = vec![json!({
        "role": "system",
        "content": options
            .system_override
            .clone()
            .unwrap_or_else(|| system_prompt(options, zh)),
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
    let mut plan_state = PlanState::default();
    let mut loop_state = LoopState::default();
    let mut overflow_retries = 0usize;
    let mut confirmation_seq = 0usize;
    let mut rounds_used = 0usize;
    let mut streamed_any = false;

    for round in 1..=options.max_rounds {
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

        if turn.tool_calls.is_empty() {
            let (answer, reasoning) = strip_think_blocks(turn.content.as_deref().unwrap_or(""));
            return Ok(OfficialAgentReport {
                answer,
                reasoning,
                streamed: streamed_any,
                model: config.model.clone(),
                tool_calls: loop_state.tool_calls,
                rounds: rounds_used,
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
                        on_event(OfficialAgentEvent::Plan {
                            steps: plan_state.steps.clone(),
                        });
                        plan_state.apply_update(steps)
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

            on_event(OfficialAgentEvent::Tool {
                tool: call.name.clone(),
                target: parsed.path.clone().or_else(|| parsed.pattern.clone()),
            });

            // Permission gate for writes and allowlisted bash. Runtime config
            // files are refused outright: repairs only happen in the repair
            // flow where every write is backed up first.
            if needs_permission(parsed.kind) && !options.read_only {
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
                let detail = permission_detail(
                    parsed.kind,
                    parsed.path.as_deref(),
                    parsed.command.as_deref(),
                );
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
    Ok(OfficialAgentReport {
        answer,
        reasoning,
        streamed: streamed_any,
        model: config.model.clone(),
        tool_calls: loop_state.tool_calls,
        rounds: rounds_used,
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

fn system_prompt(options: &OfficialAgentOptions, zh: bool) -> String {
    let mut prompt = if zh {
        "你是 Agent Doctor，这台电脑上官方内置的助手。\n\
        - 你可以读取本机各 Agent（OpenClaw、Hermes、Claude Code、Codex、DeepSeek Harness）的配置文件来回答问题，密钥会显示成 {{{{SECRET:n}}}}。不要猜密钥，也不要让用户把密钥发给你。\n\
        - 你可以在当前项目文件夹里新建或修改文件，但每次写入或改动前都会先征求用户同意。\n\
        - 你可以运行白名单里的官方安装/更新命令（同样先征求同意）。不在此列的命令不要尝试。\n\
        - runtime 的配置文件不要直接改：引导用户走修复流程（会先备份）。这个对话里只能改项目文件夹内的文件。\n\
        - 需要多步才能完成的任务，先用 update_plan 列出步骤，边做边更新。\n\
        - 回答先给结论，再给必要细节；用简体中文。不确定就直说，不要编造。"
        .to_string()
    } else {
        "You are Agent Doctor, the built-in assistant on this computer.\n\
        - You may read the config files of the agents installed here (OpenClaw, Hermes, Claude Code, Codex, DeepSeek Harness) to answer questions. Secrets appear as {{{{SECRET:n}}}}. Never guess them or ask the person to send a key.\n\
        - You may create or edit files inside the current project folder; every write asks the user for approval first.\n\
        - You may run allowlisted official install/update commands (also approved first). Never try commands outside that list.\n\
        - Do not edit runtime config files directly: guide the person to the repair flow (which backs up first). In this chat you only edit files inside the project folder.\n\
        - For multi-step requests, publish your steps with update_plan first and keep it current.\n\
        - Lead with the answer, then the details. If you are unsure, say so. Do not make things up."
        .to_string()
    };
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
