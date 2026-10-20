//! The per-runtime playbook table core consults through
//! [`agent_doctor_core::RuntimePlaybooks`].
//!
//! Lives here because the playbooks do. Core keeps the composition (config
//! syntax, install suggestions, dedup) and only asks this table for the
//! runtime-specific part, so `install` and the doctor flow keep working
//! without core depending on this crate.

use agent_doctor_core::probe::RuntimeProbeReport;
use agent_doctor_core::repair_types::{PlaybookApplyResult, SuggestedRepair};
use anyhow::{anyhow, Result};

use super::{
    apply_claude_code_playbook_filtered, apply_codex_playbook_filtered,
    apply_cursor_playbook_filtered, apply_deepseek_harness_playbook_filtered,
    apply_hermes_playbook_filtered, apply_openclaw_playbook_filtered,
    apply_qoder_playbook_filtered, apply_workbuddy_playbook_filtered,
    apply_zcode_playbook_filtered, suggest_claude_code_repairs, suggest_codex_repairs,
    suggest_cursor_repairs, suggest_deepseek_harness_repairs, suggest_hermes_repairs,
    suggest_openclaw_repairs, suggest_qoder_repairs, suggest_workbuddy_repairs,
    suggest_zcode_repairs,
};

/// One runtime row: id, suggest entry point, filtered-apply entry point.
type PlaybookRow = (
    &'static str,
    fn(&RuntimeProbeReport) -> Vec<SuggestedRepair>,
    fn(&RuntimeProbeReport, Option<&[String]>) -> Result<PlaybookApplyResult>,
);

/// Runtime-specific suggest/apply rows, mirroring the old registry fields.
const TABLE: &[PlaybookRow] = &[
    (
        "openclaw",
        suggest_openclaw_repairs,
        apply_openclaw_playbook_filtered,
    ),
    (
        "hermes",
        suggest_hermes_repairs,
        apply_hermes_playbook_filtered,
    ),
    (
        "deepseek-harness",
        suggest_deepseek_harness_repairs,
        apply_deepseek_harness_playbook_filtered,
    ),
    (
        "claude-code",
        suggest_claude_code_repairs,
        apply_claude_code_playbook_filtered,
    ),
    (
        "codex",
        suggest_codex_repairs,
        apply_codex_playbook_filtered,
    ),
    (
        "qoder",
        suggest_qoder_repairs,
        apply_qoder_playbook_filtered,
    ),
    (
        "workbuddy",
        suggest_workbuddy_repairs,
        apply_workbuddy_playbook_filtered,
    ),
    (
        "cursor",
        suggest_cursor_repairs,
        apply_cursor_playbook_filtered,
    ),
    (
        "zcode",
        suggest_zcode_repairs,
        apply_zcode_playbook_filtered,
    ),
];

fn row(runtime_id: &str) -> Option<&'static PlaybookRow> {
    TABLE.iter().find(|row| row.0 == runtime_id)
}

/// Registered into core from `main`/app setup via
/// `agent_doctor_repair::register_runtime_playbooks()`.
pub static AGENT_DOCTOR_PLAYBOOKS: AgentDoctorPlaybooks = AgentDoctorPlaybooks;

pub struct AgentDoctorPlaybooks;

impl AgentDoctorPlaybooks {
    pub fn suggest(&self, runtime_id: &str, probe: &RuntimeProbeReport) -> Vec<SuggestedRepair> {
        match row(runtime_id) {
            Some((_, suggest, _)) => suggest(probe),
            None => Vec::new(),
        }
    }

    pub fn apply(
        &self,
        runtime_id: &str,
        probe: &RuntimeProbeReport,
        only_ids: Option<&[String]>,
    ) -> Result<PlaybookApplyResult> {
        let (_, _, apply) = row(runtime_id)
            .ok_or_else(|| anyhow!("runtime '{runtime_id}' has no repair playbook"))?;
        apply(probe, only_ids)
    }

    pub fn supports(&self, runtime_id: &str) -> bool {
        row(runtime_id).is_some()
    }
}
