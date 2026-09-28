//! Deep diagnose: Agent Doctor's own agent loop answers questions about one
//! runtime. It never asks the runtime under diagnosis to diagnose itself.
//!
//! Q&A uses read-only tools (list/grep/read with secrets masked). Fixing goes
//! through `execute_repair_loop` (backup → plan → apply → re-probe).

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::execute::probe_issue_score;
use super::llm::{chat_with_tool_set, max_agent_turns, read_only_tool_definitions, LlmConfig};
use super::planner::{build_masked_repair_context, MaskedRepairContext};
use super::repair_loop::{execute_repair_loop, RepairLoopOptions};
use super::tools::{parse_tool_call, RepairToolExecutor, RepairToolKind};
use super::SkippedRepairAction;
use crate::probe::{probe_runtime, ProbeStatus};
use crate::runtime::suggest_runtime_repairs;
use crate::setup::{load_active_personal_provider, normalize_protocol, PROTOCOL_ANTHROPIC};

/// Error prefixes the desktop maps to plain-language guidance.
pub const DEEP_DIAGNOSE_NO_PROVIDER: &str = "deep_diagnose_no_provider";
pub const DEEP_DIAGNOSE_UNSUPPORTED_PROTOCOL: &str = "deep_diagnose_unsupported_protocol";
pub const DEEP_DIAGNOSE_CANCELLED: &str = "deep_diagnose_cancelled";

const MAX_HISTORY_TURNS: usize = 12;
const MAX_HISTORY_CHARS: usize = 4000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepDiagnoseTurn {
    /// `user` or `assistant`.
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepDiagnoseOptions {
    pub runtime_id: String,
    pub question: String,
    #[serde(default)]
    pub history: Vec<DeepDiagnoseTurn>,
    /// `zh` or `en`; controls the answer language.
    #[serde(default)]
    pub locale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeepDiagnoseEvent {
    /// Re-running the rule checks for fresh context.
    Checking,
    /// Waiting on the model.
    Thinking { round: u32 },
    /// The model is looking at a config file or searching.
    Tool {
        tool: String,
        target: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepDiagnoseReport {
    pub answer: String,
    pub model: String,
    pub tool_calls: usize,
    /// Rule checks still failing or warning when the question was asked.
    pub open_issues: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepRepairSummary {
    pub runtime_id: String,
    pub backup_id: String,
    pub issue_score_before: u32,
    pub issue_score_after: u32,
    /// Fix actions that actually ran (excludes the backup step).
    pub executed: Vec<String>,
    pub skipped: Vec<SkippedRepairAction>,
}

/// Model for deep diagnose: the provider the user already wired in Agent Doctor.
pub fn deep_diagnose_llm_config() -> Result<LlmConfig> {
    let Some(entry) = load_active_personal_provider()? else {
        bail!("{DEEP_DIAGNOSE_NO_PROVIDER}: no active provider with an API key");
    };
    if normalize_protocol(&entry.protocol) == PROTOCOL_ANTHROPIC {
        bail!(
            "{DEEP_DIAGNOSE_UNSUPPORTED_PROTOCOL}: provider '{}' uses the Anthropic protocol",
            entry.name
        );
    }
    Ok(LlmConfig::from_gateway(
        &entry.url,
        &entry.api_key,
        &entry.model,
    ))
}

pub fn run_deep_diagnose_chat(
    options: &DeepDiagnoseOptions,
    config: &LlmConfig,
    cancel: &AtomicBool,
    mut on_event: impl FnMut(DeepDiagnoseEvent),
) -> Result<DeepDiagnoseReport> {
    let question = options.question.trim();
    if question.is_empty() {
        bail!("question must not be empty");
    }

    on_event(DeepDiagnoseEvent::Checking);
    let probe = probe_runtime(&options.runtime_id)?;
    let open_issues = probe
        .checks
        .iter()
        .filter(|check| matches!(check.status, ProbeStatus::Fail | ProbeStatus::Warn))
        .count();
    let suggested = suggest_runtime_repairs(&options.runtime_id, &probe);
    let mut context = build_masked_repair_context(&options.runtime_id, &probe, suggested);
    ensure_not_cancelled(cancel)?;

    let zh = options.locale.as_deref() != Some("en");
    // The desktop only shows the repair button when checks still fail or warn.
    let repair_offered = open_issues > 0;
    let mut messages = vec![
        json!({ "role": "system", "content": system_prompt(&options.runtime_id, zh, repair_offered) }),
        json!({ "role": "system", "content": context_message(&context)? }),
    ];
    messages.extend(history_messages(&options.history));
    messages.push(json!({ "role": "user", "content": question }));

    let tools = read_only_tool_definitions();
    let mut executor = RepairToolExecutor::new(
        &options.runtime_id,
        std::mem::take(&mut context.vault),
        false,
    );
    let mut tool_calls = 0usize;

    for round in 1..=max_agent_turns() {
        ensure_not_cancelled(cancel)?;
        on_event(DeepDiagnoseEvent::Thinking {
            round: round as u32,
        });
        let turn = chat_with_tool_set(config, &messages, Some(&tools))?;
        if turn.tool_calls.is_empty() {
            return Ok(DeepDiagnoseReport {
                answer: turn.content.unwrap_or_default().trim().to_string(),
                model: config.model.clone(),
                tool_calls,
                open_issues,
            });
        }

        messages.push(json!({
            "role": "assistant",
            "content": turn.content,
            "tool_calls": turn.tool_calls.iter().map(|call| json!({
                "id": call.id,
                "type": "function",
                "function": { "name": call.name, "arguments": call.arguments }
            })).collect::<Vec<_>>()
        }));

        for call in turn.tool_calls {
            ensure_not_cancelled(cancel)?;
            tool_calls += 1;
            let content = match parse_tool_call(&call.name, &call.arguments) {
                Ok(parsed) if is_read_only(parsed.kind) => {
                    on_event(DeepDiagnoseEvent::Tool {
                        tool: call.name.clone(),
                        target: parsed.path.clone().or_else(|| parsed.pattern.clone()),
                    });
                    serde_json::to_string(&executor.execute(&parsed)?)?
                }
                Ok(_) => json!({ "error": "deep diagnose can only read files" }).to_string(),
                Err(error) => json!({ "error": error.to_string() }).to_string(),
            };
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call.id,
                "content": content
            }));
        }
    }

    // Out of tool rounds: ask for the answer with what it has.
    ensure_not_cancelled(cancel)?;
    messages.push(json!({
        "role": "user",
        "content": if zh {
            "请根据目前看到的信息，直接给出结论，不要再调用工具。"
        } else {
            "Answer now with what you have. Do not call more tools."
        }
    }));
    let turn = chat_with_tool_set(config, &messages, None)?;
    Ok(DeepDiagnoseReport {
        answer: turn.content.unwrap_or_default().trim().to_string(),
        model: config.model.clone(),
        tool_calls,
        open_issues,
    })
}

/// Agent Doctor's bounded repair loop with the AI planner on the user's provider.
pub fn run_deep_repair(runtime_id: &str, config: &LlmConfig) -> Result<DeepRepairSummary> {
    let report = execute_repair_loop(
        runtime_id,
        &RepairLoopOptions {
            apply_confirmed_writes: true,
            max_rounds: None,
            use_ai_planner: true,
            llm: Some(config.clone()),
        },
    )?;
    Ok(DeepRepairSummary {
        runtime_id: report.runtime_id,
        backup_id: report.backup.id,
        issue_score_before: probe_issue_score(&report.before_probe),
        issue_score_after: probe_issue_score(&report.after_probe),
        executed: report
            .executed_action_ids
            .into_iter()
            .filter(|id| id != "backup-runtime-configs")
            .collect(),
        skipped: report.skipped_actions,
    })
}

fn is_read_only(kind: RepairToolKind) -> bool {
    matches!(
        kind,
        RepairToolKind::Read | RepairToolKind::ListDir | RepairToolKind::GrepFiles
    )
}

fn ensure_not_cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::SeqCst) {
        bail!("{DEEP_DIAGNOSE_CANCELLED}: stopped by user");
    }
    Ok(())
}

fn context_message(context: &MaskedRepairContext) -> Result<String> {
    Ok(format!(
        "Rule-check results for this machine (secrets masked as {{{{SECRET:n}}}}):\n{}",
        serde_json::to_string(context)?
    ))
}

fn history_messages(history: &[DeepDiagnoseTurn]) -> Vec<Value> {
    let start = history.len().saturating_sub(MAX_HISTORY_TURNS);
    history[start..]
        .iter()
        .filter(|turn| matches!(turn.role.as_str(), "user" | "assistant"))
        .filter(|turn| !turn.content.trim().is_empty())
        .map(|turn| {
            let content: String = turn.content.chars().take(MAX_HISTORY_CHARS).collect();
            json!({ "role": turn.role, "content": content })
        })
        .collect()
}

fn system_prompt(runtime_id: &str, zh: bool, repair_offered: bool) -> String {
    if zh {
        let repair = if repair_offered {
            "- 如果用户想直接修好，告诉他点回答下面的「让 Agent Doctor 修」：会先备份，改完再检查一次。"
        } else {
            "- 这次规则检查全部通过，回答下面不会有「让 Agent Doctor 修」按钮，不要让用户去点它。需要改设置时，引导他去界面里的「服务商」等页面。"
        };
        format!(
            "你是 Agent Doctor 的「深度诊断」助手，帮不懂技术的用户看懂这台电脑上 Agent（{runtime_id}）的检查结果。\n\
            - 先根据给你的规则检查结果回答。需要确认细节时，再用 list_dir / grep_files / read_file 看配置文件。你只能看，不能改文件，也不能运行命令。\n\
            - 密钥会显示成 {{{{SECRET:n}}}}。不要猜密钥，也不要让用户把密钥发给你。\n\
            - 用简体中文、短句回答三件事：问题是什么、要不要紧、下一步点哪里。优先说界面上的按钮，例如「自动修好设置」「强制重装」「服务商」。命令和文件路径只能作补充，不能只给命令。\n\
            {repair}\n\
            - 不确定就直说，不要编造。"
        )
    } else {
        let repair = if repair_offered {
            "- If they want it fixed, tell them to tap \"Let Agent Doctor fix it\" under the answer: it backs up first and checks again afterwards."
        } else {
            "- Every rule check passed this time, so there is no \"Let Agent Doctor fix it\" button under the answer. Do not tell them to tap it; point them to screens such as Provider instead."
        };
        format!(
            "You are Agent Doctor's Full diagnose assistant. Help a non-technical person understand the checks for the agent ({runtime_id}) on this computer.\n\
            - Answer from the rule-check results first. Use list_dir / grep_files / read_file only to confirm details. You can only read; you cannot change files or run commands.\n\
            - Secrets appear as {{{{SECRET:n}}}}. Never guess them or ask the person to send a key.\n\
            - Reply in short plain sentences: what is wrong, whether it matters, and what to tap next. Prefer buttons in the app, such as Auto-fix setup, Force reinstall, or Provider. Commands and file paths are extra detail only.\n\
            {repair}\n\
            - If you are unsure, say so. Do not make things up."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_url_gets_chat_completions_suffix() {
        let config = LlmConfig::from_gateway("https://api.deepseek.com/v1/", " k ", " m ");
        assert_eq!(
            config.api_url,
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(config.api_key, "k");
        assert_eq!(config.model, "m");

        let full = LlmConfig::from_gateway("https://x.test/v1/chat/completions", "k", "m");
        assert_eq!(full.api_url, "https://x.test/v1/chat/completions");
    }

    #[test]
    fn debug_never_prints_the_key() {
        let config = LlmConfig::from_gateway("https://x.test/v1", "sk-secret-value", "m");
        assert!(!format!("{config:?}").contains("sk-secret-value"));
    }

    #[test]
    fn only_read_tools_are_offered() {
        let tools = read_only_tool_definitions();
        let names: Vec<&str> = tools
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool.pointer("/function/name").and_then(Value::as_str))
            .collect();
        assert_eq!(names, vec!["read_file", "list_dir", "grep_files"]);
    }

    #[test]
    fn write_tools_are_rejected() {
        assert!(is_read_only(RepairToolKind::Read));
        assert!(is_read_only(RepairToolKind::GrepFiles));
        assert!(!is_read_only(RepairToolKind::WriteFile));
        assert!(!is_read_only(RepairToolKind::SearchReplace));
        assert!(!is_read_only(RepairToolKind::PatchConfig));
        assert!(!is_read_only(RepairToolKind::Bash));
    }

    #[test]
    fn history_keeps_only_recent_user_and_assistant_turns() {
        let mut history: Vec<DeepDiagnoseTurn> = (0..20)
            .map(|i| DeepDiagnoseTurn {
                role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                content: format!("turn {i}"),
            })
            .collect();
        history.push(DeepDiagnoseTurn {
            role: "meta".to_string(),
            content: "status".to_string(),
        });
        let messages = history_messages(&history);
        assert!(messages.len() <= MAX_HISTORY_TURNS);
        assert!(messages.iter().all(|m| m["role"] != "meta"));
    }

    #[test]
    fn prompt_mentions_repair_button_only_when_it_is_shown() {
        let offered = system_prompt("openclaw", true, true);
        assert!(offered.contains("点回答下面的「让 Agent Doctor 修」"));
        let hidden = system_prompt("openclaw", true, false);
        assert!(hidden.contains("不要让用户去点它"));
        assert!(!hidden.contains("点回答下面的「让 Agent Doctor 修」"));
        assert!(system_prompt("openclaw", false, false).contains("Do not tell them to tap it"));
    }

    #[test]
    fn cancel_flag_stops_the_loop() {
        let cancel = AtomicBool::new(true);
        let error = ensure_not_cancelled(&cancel).unwrap_err().to_string();
        assert!(error.starts_with(DEEP_DIAGNOSE_CANCELLED));
    }
}
