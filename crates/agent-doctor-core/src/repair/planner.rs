use serde::{Deserialize, Serialize};

use super::llm::{chat_with_tools, max_agent_turns, LlmConfig, LlmTurn};
use super::mask::{load_masked_config_snippets, MaskedFileSnippet, SecretVault};
use super::tools::{parse_tool_call, RepairToolExecutor, RepairToolResult};
use super::{RedactedFact, RedactionPolicy, Redactor, SuggestedRepair};
use crate::probe::RuntimeProbeReport;
use crate::runtime::{adapter_by_id, runtime_allowed_bash_commands};

/// Input for repair planning (LLM or deterministic). Secrets appear as vault tokens only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskedRepairContext {
    pub runtime_id: String,
    pub redacted_facts: Vec<RedactedFact>,
    pub masked_files: Vec<MaskedFileSnippet>,
    pub suggested_repairs: Vec<SuggestedRepair>,
    /// Local-only vault for merge after planning; never serialized.
    #[serde(skip)]
    pub(crate) vault: SecretVault,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlannerOptions {
    pub apply_tool_writes: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlannerResult {
    pub action_ids: Vec<String>,
    pub tool_trace: Vec<RepairToolResult>,
}

pub fn build_masked_repair_context(
    runtime_id: &str,
    probe: &RuntimeProbeReport,
    suggested: Vec<SuggestedRepair>,
) -> MaskedRepairContext {
    let redactor = Redactor::new(RedactionPolicy::default());
    let bundle = probe.to_diagnostic_bundle();
    let config_paths = adapter_by_id(runtime_id)
        .map(|adapter| adapter.all_config_paths())
        .unwrap_or_default();
    let (masked_files, vault) = load_masked_config_snippets(&config_paths);

    MaskedRepairContext {
        runtime_id: runtime_id.to_string(),
        redacted_facts: redactor.redact_bundle(&bundle),
        masked_files,
        suggested_repairs: suggested,
        vault,
    }
}

pub trait RepairPlanner {
    fn plan(
        &self,
        context: &mut MaskedRepairContext,
        options: &PlannerOptions,
    ) -> anyhow::Result<PlannerResult>;
}

/// Default planner: run all auto-fixable suggestions in stable order (no LLM).
#[derive(Debug, Default, Clone, Copy)]
pub struct DeterministicPlanner;

impl RepairPlanner for DeterministicPlanner {
    fn plan(
        &self,
        context: &mut MaskedRepairContext,
        _options: &PlannerOptions,
    ) -> anyhow::Result<PlannerResult> {
        Ok(PlannerResult {
            action_ids: context
                .suggested_repairs
                .iter()
                .filter(|item| item.auto_fixable)
                .map(|item| item.id.clone())
                .collect(),
            tool_trace: Vec::new(),
        })
    }
}

/// LLM agent planner: read/edit/bash tools with mask on the wire, unmask locally before apply.
#[derive(Debug, Default, Clone, Copy)]
pub struct AiRepairPlanner;

impl RepairPlanner for AiRepairPlanner {
    fn plan(
        &self,
        context: &mut MaskedRepairContext,
        options: &PlannerOptions,
    ) -> anyhow::Result<PlannerResult> {
        let Some(config) = LlmConfig::from_env() else {
            return DeterministicPlanner.plan(context, options);
        };

        run_agent_loop(context, options, &config)
    }
}

fn run_agent_loop(
    context: &mut MaskedRepairContext,
    options: &PlannerOptions,
    config: &LlmConfig,
) -> anyhow::Result<PlannerResult> {
    use serde_json::json;

    let allowed_bash = runtime_allowed_bash_commands(&context.runtime_id);
    let bash_allowlist = if allowed_bash.is_empty() {
        "none registered".to_string()
    } else {
        allowed_bash.join("; ")
    };

    let mut messages = vec![
        json!({
            "role": "system",
            "content": format!(
                "You are Agent Doctor repair planner. Start with list_dir or grep_files to locate config, then read_file. \
                Prefer search_replace for surgical edits; use patch_config for YAML/JSON/TOML keys; use write_file only for new or tiny files. \
                read_file line numbers must NOT appear in old_string. Use {{SECRET:n}} tokens for secrets. \
                When the binary is missing, use bash with an allowlisted install command. \
                bash allowlist for this runtime: {bash_allowlist}. \
                When done, reply with JSON: {{\"action_ids\":[\"fix-...\"]}}"
            )
        }),
        json!({
            "role": "user",
            "content": serde_json::to_string(&MaskedRepairContext {
                vault: SecretVault::default(),
                ..context.clone()
            })?
        }),
    ];

    let mut executor = RepairToolExecutor::new(
        &context.runtime_id,
        std::mem::take(&mut context.vault),
        options.apply_tool_writes,
    );
    let mut tool_trace = Vec::new();

    for _ in 0..max_agent_turns() {
        let turn = chat_with_tools(config, &messages)?;
        if turn.tool_calls.is_empty() {
            let action_ids = parse_action_ids_from_turn(&turn);
            context.vault = executor.into_vault();
            return Ok(PlannerResult {
                action_ids,
                tool_trace,
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
            let parsed = parse_tool_call(&call.name, &call.arguments)?;
            let result = executor.execute(&parsed)?;
            tool_trace.push(result.clone());
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call.id,
                "content": serde_json::to_string(&result)?
            }));
        }
    }

    context.vault = executor.into_vault();
    Ok(PlannerResult {
        action_ids: DeterministicPlanner.plan(context, options)?.action_ids,
        tool_trace,
    })
}

fn parse_action_ids_from_turn(turn: &LlmTurn) -> Vec<String> {
    let Some(content) = turn.content.as_deref() else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    value
        .get("action_ids")
        .and_then(|item| item.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}
