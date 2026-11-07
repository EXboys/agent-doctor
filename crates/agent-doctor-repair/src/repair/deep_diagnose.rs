//! Deep diagnose: Agent Doctor's own agent loop answers questions about one
//! runtime. It never asks the runtime under diagnosis to diagnose itself.
//!
//! Q&A uses read-only tools (list/grep/read with secrets masked). Fixing goes
//! through `execute_repair_loop` (backup → plan → apply → re-probe).

use agent_doctor_core::repair_types::LlmConfig;
use agent_doctor_core::repair_types::MaskedRepairContext;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::llm::{max_agent_turns, official_llm_config, OFFICIAL_PROVIDER_MISSING};
use super::official_agent::{
    run_official_agent_turn, AutoApproveGate, OfficialAgentEvent, OfficialAgentOptions,
};
use super::planner::build_masked_repair_context;
use super::repair_loop::{diff_probe_checks, execute_repair_loop};
use super::tools::allowed_paths_for_runtime;
use super::SkippedRepairAction;
use agent_doctor_core::probe::{probe_runtime, ProbeStatus};
use agent_doctor_core::repair_types::probe_issue_score;
#[cfg(test)]
use agent_doctor_core::repair_types::RepairToolKind;
use agent_doctor_core::repair_types::{CheckChange, RepairLoopOptions};
use agent_doctor_core::runtime::suggest_runtime_repairs;

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
    /// Incremental reply text while the model writes it.
    Delta { text: String },
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
    /// Fix actions that actually ran and were kept (excludes the backup step).
    pub executed: Vec<String>,
    pub skipped: Vec<SkippedRepairAction>,
    /// Checks that were warn/fail before the repair and pass now.
    pub fixed: Vec<CheckChange>,
    /// Checks the kept changes made worse.
    pub new_issues: Vec<CheckChange>,
    /// New problems from changes that were undone.
    pub rolled_back_issues: Vec<CheckChange>,
}

/// Model for deep diagnose: the official built-in agent channel. Both OpenAI-
/// compatible and Anthropic protocols work here; without an official sign-in
/// the desktop points the user at login.
pub fn deep_diagnose_llm_config() -> Result<LlmConfig> {
    official_llm_config().map_err(|error| {
        let message = format!("{error:#}");
        if message.contains(OFFICIAL_PROVIDER_MISSING) {
            anyhow::anyhow!("{DEEP_DIAGNOSE_NO_PROVIDER}: official account is not signed in")
        } else {
            anyhow::anyhow!(message)
        }
    })
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
    let mut prelude = vec![json!({ "role": "system", "content": context_message(&context)? })];
    prelude.extend(history_messages(&options.history));

    let official_options = OfficialAgentOptions {
        prompt: question.to_string(),
        locale: options.locale.clone(),
        workspace: None,
        read_only: true,
        apply_writes: false,
        max_rounds: max_agent_turns(),
        system_override: Some(system_prompt(&options.runtime_id, zh, repair_offered)),
        system_extra: None,
        allowed_paths_override: Some(allowed_paths_for_runtime(&options.runtime_id)),
        vault: context.take_vault(),
        prelude_messages: prelude,
        browser_tools: false,
        computer_tools: false,
    };

    let report = run_official_agent_turn(
        &official_options,
        config,
        cancel,
        |event| match event {
            OfficialAgentEvent::Thinking { round } => {
                on_event(DeepDiagnoseEvent::Thinking { round })
            }
            OfficialAgentEvent::Tool { tool, target, .. } => {
                on_event(DeepDiagnoseEvent::Tool { tool, target })
            }
            OfficialAgentEvent::Delta { text } => {
                on_event(DeepDiagnoseEvent::Delta { text });
            }
            OfficialAgentEvent::ToolResult { .. }
            | OfficialAgentEvent::Reasoning { .. }
            | OfficialAgentEvent::Plan { .. }
            | OfficialAgentEvent::Confirmation { .. } => {}
        },
        &mut AutoApproveGate,
    )?;

    Ok(DeepDiagnoseReport {
        answer: report.answer,
        model: report.model,
        tool_calls: report.tool_calls,
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
    let diff = diff_probe_checks(&report.before_probe, &report.after_probe);
    let rolled_back_issues = report
        .rounds
        .iter()
        .filter(|round| round.rolled_back)
        .flat_map(|round| round.new_issues.iter().cloned())
        .collect();
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
        fixed: diff.fixed,
        new_issues: diff.new_issues,
        rolled_back_issues,
    })
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
            - 结论必须分级，不许越级：有报错日志、401/403、启动失败或实际调用失败证据的，才是「已确认故障」；只有静态配置差异、没有运行失败证据的，写「待验证」，不是故障；配置偏宽松但功能正常，是「安全加固」建议。用户亲测能用是最直接的运行证据，优先于扫描发现的配置缺口。待验证项要给出具体验证办法（比如让它实际跑一条消息，或点「实际测一下」按钮）。\n\
            - 规则检查结果里的 [fault]/[unverified]/[hardening] 标签就是类别：[fault]=已确认故障；[unverified]=静态差异待验证，不是故障；[hardening]=安全加固建议，不是故障。按标签下结论，不要把 [unverified] 说成坏了。\n\
            - 凭据来源不止 .env：进程继承的环境变量、Shell 启动文件、凭据存储、应用自身的凭据机制、外部启动器都可能提供密钥。「配置文件里没看到某个变量」只能写「凭据来源未确认，属待验证」，禁止写成「无法调用」或「高概率失败」。\n\
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
            - Grade every conclusion, never escalate: a confirmed fault needs evidence — error logs, 401/403, a startup failure, or a failed real call. A static config difference with no runtime failure evidence is \"unverified\", not a fault. A loose config that still works is a hardening tip. First-hand \"it works\" from the person outranks a gap found by static scanning. Give each unverified item a concrete way to check it (for example, send one real message, or tap the live-probe button).\n\
            - The rule-check notes carry a category tag: [fault] = confirmed failure; [unverified] = a static difference, still unverified, not a fault; [hardening] = a hardening tip, not a fault. Conclude by the tag; never present [unverified] as broken.\n\
            - Credentials do not live only in the .env: inherited process env, shell startup files, credential stores, the agent's own lookup, or external launchers can all supply keys. \"The variable is not in this file\" may only be worded as \"credential source unverified\" — never as \"cannot call\" or \"likely failing\".\n\
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

    fn is_read_only(kind: RepairToolKind) -> bool {
        matches!(
            kind,
            RepairToolKind::Read | RepairToolKind::ListDir | RepairToolKind::GrepFiles
        )
    }

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
        let tools = crate::repair::llm::read_only_tool_definitions();
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
    fn prompt_grades_evidence_before_calling_something_broken() {
        for prompt in [
            system_prompt("hermes", true, true),
            system_prompt("hermes", false, false),
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
    fn cancel_flag_stops_the_loop() {
        let cancel = AtomicBool::new(true);
        let error = ensure_not_cancelled(&cancel).unwrap_err().to_string();
        assert!(error.starts_with(DEEP_DIAGNOSE_CANCELLED));
    }
}
