//! Repair playbooks, the execution loop, deep diagnose, and the LLM planner
//! on top of [`agent_doctor_core`]'s diagnostic vocabulary.
//!
//! Registers the runtime dispatch and install follow-ups into core at startup
//! via [`register`], so `agent_doctor_core::suggest_runtime_repairs`,
//! `apply_runtime_playbook`, and install's explain/repair-after keep working
//! for in-core callers.

pub mod repair;

pub use repair::{
    allowed_paths_for_runtime, bash_command_allowed, build_masked_repair_context,
    deep_diagnose_llm_config, diff_probe_checks, execute_repair, execute_repair_loop,
    explain_runtime, list_runtime_backup_ids, load_backup_snapshot, parse_tool_call,
    restore_backup_snapshot, restore_runtime_backup, run_deep_diagnose_chat, run_deep_repair,
    AiRepairPlanner, DeepDiagnoseEvent, DeepDiagnoseOptions, DeepDiagnoseReport, DeepDiagnoseTurn,
    DeepRepairSummary, DeterministicPlanner, ExplainInput, HoldBrowserToolsOff, MaskedFileSnippet,
    PlannerOptions, PlannerResult, RepairExecuteOptions, RepairExecuteReport, RepairPlanner,
    RepairToolCall, RepairToolExecutor, RestoreReport, DEEP_DIAGNOSE_CANCELLED,
    DEEP_DIAGNOSE_NO_PROVIDER, DEEP_DIAGNOSE_UNSUPPORTED_PROTOCOL,
};

use crate::repair::dispatch::AGENT_DOCTOR_PLAYBOOKS;

/// Register the playbook dispatch and install follow-ups into core. Call once
/// from `main` / app setup before any doctor/repair flow runs.
pub fn register() {
    agent_doctor_core::register_runtime_playbooks(&AGENT_DOCTOR_REPAIR);
    agent_doctor_core::register_install_repair_followups(&AGENT_DOCTOR_REPAIR);
}

/// Implements both core bridge traits on one static.
pub struct AgentDoctorRepair;

impl agent_doctor_core::RuntimePlaybooks for AgentDoctorRepair {
    fn suggest(
        &self,
        runtime_id: &str,
        probe: &agent_doctor_core::probe::RuntimeProbeReport,
    ) -> Vec<agent_doctor_core::repair_types::SuggestedRepair> {
        AGENT_DOCTOR_PLAYBOOKS.suggest(runtime_id, probe)
    }

    fn apply(
        &self,
        runtime_id: &str,
        probe: &agent_doctor_core::probe::RuntimeProbeReport,
        only_ids: Option<&[String]>,
    ) -> anyhow::Result<agent_doctor_core::repair_types::PlaybookApplyResult> {
        AGENT_DOCTOR_PLAYBOOKS.apply(runtime_id, probe, only_ids)
    }

    fn supports(&self, runtime_id: &str) -> bool {
        AGENT_DOCTOR_PLAYBOOKS.supports(runtime_id)
    }
}

impl agent_doctor_core::InstallRepairFollowups for AgentDoctorRepair {
    fn explain(
        &self,
        input: &agent_doctor_core::repair_types::ExplainInput,
    ) -> anyhow::Result<agent_doctor_core::repair_types::ExplainReport> {
        repair::explain_runtime(input)
    }

    fn run_repair_loop(
        &self,
        runtime_id: &str,
        options: &agent_doctor_core::repair_types::RepairLoopOptions,
    ) -> anyhow::Result<agent_doctor_core::repair_types::RepairLoopReport> {
        repair::execute_repair_loop(runtime_id, options)
    }
}

pub static AGENT_DOCTOR_REPAIR: AgentDoctorRepair = AgentDoctorRepair;
