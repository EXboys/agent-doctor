pub mod browser_tools_gate;
mod deep_diagnose;
pub(crate) mod dispatch;
mod execute;
mod explain;
mod llm;
mod planner;
mod playbooks;
mod repair_loop;
mod restore;
mod tools;

pub use agent_doctor_core::mask::{
    load_masked_config_snippets, mask_config_file, mask_env_file_content, mask_secret_value,
    merge_env_with_vault, unmask_file_content, MaskedFileSnippet, SecretVault,
};
pub use browser_tools_gate::{browser_tools_allowed, HoldBrowserToolsOff};
pub use deep_diagnose::{
    deep_diagnose_llm_config, run_deep_diagnose_chat, run_deep_repair, DeepDiagnoseEvent,
    DeepDiagnoseOptions, DeepDiagnoseReport, DeepDiagnoseTurn, DeepRepairSummary,
    DEEP_DIAGNOSE_CANCELLED, DEEP_DIAGNOSE_NO_PROVIDER, DEEP_DIAGNOSE_UNSUPPORTED_PROTOCOL,
};
pub use execute::{execute_repair, RepairExecuteOptions, RepairExecuteReport};
pub use explain::explain_runtime;

use agent_doctor_core::repair_types::LlmConfig;
pub use planner::{
    build_masked_repair_context, AiRepairPlanner, DeterministicPlanner, PlannerOptions,
    PlannerResult, RepairPlanner,
};
pub use playbooks::{
    apply_claude_code_playbook, apply_claude_code_playbook_filtered, apply_codex_playbook,
    apply_codex_playbook_filtered, apply_cursor_playbook, apply_cursor_playbook_filtered,
    apply_deepseek_harness_playbook, apply_deepseek_harness_playbook_filtered,
    apply_hermes_playbook, apply_hermes_playbook_filtered, apply_openclaw_playbook,
    apply_openclaw_playbook_filtered, apply_qoder_playbook, apply_qoder_playbook_filtered,
    apply_workbuddy_playbook, apply_workbuddy_playbook_filtered, apply_zcode_playbook,
    apply_zcode_playbook_filtered, suggest_claude_code_repairs, suggest_codex_repairs,
    suggest_cursor_repairs, suggest_deepseek_harness_repairs, suggest_hermes_repairs,
    suggest_openclaw_repairs, suggest_qoder_repairs, suggest_workbuddy_repairs,
    suggest_zcode_repairs,
};
pub use repair_loop::{diff_probe_checks, execute_repair_loop};
pub use restore::{
    list_runtime_backup_ids, load_backup_snapshot, restore_backup_snapshot, restore_runtime_backup,
    RestoreReport,
};
pub use tools::{
    allowed_paths_for_runtime, bash_command_allowed, parse_tool_call, RepairToolCall,
    RepairToolExecutor, RepairToolKind, RepairToolResult,
};

// The redaction/diagnostic/action vocabulary moved to `agent_doctor_core::repair_types`
// so probe, install, workspace, and setup can share it without the repair
// execution loop. The re-export keeps every existing `crate::repair::*` path.
pub use agent_doctor_core::repair_types::{
    backups_root, build_repair_preview, build_repair_preview_from_bundle, AuditReport,
    BackupSnapshot, DiagnosticBundle, DiagnosticFact, ExplainCheck, ExplainInput,
    ExplainInstallFailure, ExplainReport, ExplainSuggestion, PlaybookApplyResult, RedactedFact,
    RedactionPolicy, Redactor, RepairAction, RepairActionKind, RepairPlan, RepairRisk,
    SensitivityLevel, SkippedRepairAction, SnapshotFile, SuggestedRepair,
};
