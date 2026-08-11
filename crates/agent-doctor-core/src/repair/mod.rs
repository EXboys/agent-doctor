mod execute;
mod explain;
mod llm;
mod mask;
mod planner;
mod playbooks;
mod repair_loop;
mod restore;
mod suggested;
mod tools;

use serde::{Deserialize, Serialize};

pub use execute::{
    backups_root, execute_repair, probe_health_summary, probe_issue_score, RepairExecuteOptions,
    RepairExecuteReport, SkippedRepairAction,
};
pub use explain::{
    explain_runtime, ExplainCheck, ExplainInput, ExplainInstallFailure, ExplainReport,
    ExplainSuggestion,
};
pub use llm::LlmConfig;
pub use mask::{
    load_masked_config_snippets, mask_config_file, mask_env_file_content, mask_secret_value,
    merge_env_with_vault, unmask_file_content, MaskedFileSnippet, SecretVault,
};
pub use planner::{
    build_masked_repair_context, AiRepairPlanner, DeterministicPlanner, MaskedRepairContext,
    PlannerOptions, PlannerResult, RepairPlanner,
};
pub use playbooks::{
    apply_claude_code_playbook, apply_claude_code_playbook_filtered, apply_codex_playbook,
    apply_codex_playbook_filtered, apply_hermes_playbook, apply_hermes_playbook_filtered,
    apply_openclaw_playbook, apply_openclaw_playbook_filtered, suggest_claude_code_repairs,
    suggest_codex_repairs, suggest_hermes_repairs, suggest_openclaw_repairs, PlaybookApplyResult,
};
pub use repair_loop::{execute_repair_loop, RepairLoopOptions, RepairLoopReport, RepairLoopRound};
pub use restore::{
    list_runtime_backup_ids, load_backup_snapshot, restore_backup_snapshot, restore_runtime_backup,
    RestoreReport,
};
pub use suggested::SuggestedRepair;
pub use tools::{
    allowed_paths_for_runtime, bash_command_allowed, parse_tool_call, RepairToolCall,
    RepairToolExecutor, RepairToolKind, RepairToolResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensitivityLevel {
    Public,
    LocalPath,
    ConfigShape,
    Secret,
    SensitiveLog,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticFact {
    pub key: String,
    pub value: String,
    pub sensitivity: SensitivityLevel,
}

impl DiagnosticFact {
    pub fn new(
        key: impl Into<String>,
        value: impl Into<String>,
        sensitivity: SensitivityLevel,
    ) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            sensitivity,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactedFact {
    pub key: String,
    pub value: String,
    pub sensitivity: SensitivityLevel,
    pub redacted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticBundle {
    pub runtime_id: String,
    pub facts: Vec<DiagnosticFact>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RedactionPolicy {
    pub reveal_local_paths: bool,
    pub reveal_sensitive_logs: bool,
}

pub struct Redactor {
    policy: RedactionPolicy,
}

impl Redactor {
    pub fn new(policy: RedactionPolicy) -> Self {
        Self { policy }
    }

    pub fn redact_bundle(&self, bundle: &DiagnosticBundle) -> Vec<RedactedFact> {
        bundle
            .facts
            .iter()
            .map(|fact| self.redact_fact(fact))
            .collect()
    }

    pub fn redact_fact(&self, fact: &DiagnosticFact) -> RedactedFact {
        let (value, redacted) = match fact.sensitivity {
            SensitivityLevel::Public | SensitivityLevel::ConfigShape => {
                (redact_inline_secret_values(&fact.value), false)
            }
            SensitivityLevel::LocalPath if self.policy.reveal_local_paths => {
                (redact_inline_secret_values(&fact.value), false)
            }
            SensitivityLevel::LocalPath => (redact_home_path(&fact.value), true),
            SensitivityLevel::SensitiveLog if self.policy.reveal_sensitive_logs => {
                (redact_inline_secrets(&fact.key, &fact.value), false)
            }
            SensitivityLevel::SensitiveLog => (redact_inline_secrets(&fact.key, &fact.value), true),
            SensitivityLevel::Secret => ("[REDACTED]".to_string(), true),
        };

        RedactedFact {
            key: fact.key.clone(),
            value,
            sensitivity: fact.sensitivity,
            redacted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairRisk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairActionKind {
    BackupFile,
    RestoreBackup,
    PatchConfig,
    InstallRuntime,
    UpdateRuntime,
    RemoveBrokenSymlink,
    VerifyCommand,
    ManualReview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairAction {
    pub id: String,
    pub kind: RepairActionKind,
    pub title: String,
    pub description: String,
    pub risk: RepairRisk,
    pub requires_confirmation: bool,
    pub touches: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairPlan {
    pub runtime_id: String,
    pub summary: String,
    pub redacted_facts: Vec<RedactedFact>,
    pub actions: Vec<RepairAction>,
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotFile {
    pub original_path: String,
    pub snapshot_path: String,
    pub sensitivity: SensitivityLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSnapshot {
    pub id: String,
    pub runtime_id: String,
    pub root: String,
    pub files: Vec<SnapshotFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    pub id: String,
    pub runtime_id: String,
    pub redacted_facts: Vec<RedactedFact>,
    pub action_ids: Vec<String>,
    pub verification_summary: String,
    pub rollback_hint: String,
}

pub fn build_repair_preview(runtime_id: &str) -> RepairPlan {
    build_repair_preview_from_bundle(DiagnosticBundle {
        runtime_id: runtime_id.to_string(),
        facts: vec![
            DiagnosticFact::new("runtime.id", runtime_id, SensitivityLevel::Public),
            DiagnosticFact::new(
                "safety.mode",
                "AI can summarize redacted diagnostics, but execution is limited to typed repair actions.",
                SensitivityLevel::ConfigShape,
            ),
            DiagnosticFact::new(
                "default.backup_root",
                "~/.config/agent-doctor/backups/",
                SensitivityLevel::LocalPath,
            ),
        ],
        notes: vec![
            "This preview does not inspect private files yet.".to_string(),
            "Future repair runs must create a backup snapshot before modifying configs."
                .to_string(),
        ],
    })
}

pub fn build_repair_preview_from_bundle(bundle: DiagnosticBundle) -> RepairPlan {
    let redactor = Redactor::new(RedactionPolicy::default());
    let actions = vec![
        RepairAction {
            id: "backup-runtime-configs".to_string(),
            kind: RepairActionKind::BackupFile,
            title: "Create backup snapshot".to_string(),
            description:
                "Copy runtime configs into an Agent Doctor backup directory before repair."
                    .to_string(),
            risk: RepairRisk::Low,
            requires_confirmation: false,
            touches: vec!["runtime config files".to_string()],
        },
        RepairAction {
            id: "diagnose-known-issues".to_string(),
            kind: RepairActionKind::ManualReview,
            title: "Diagnose known runtime issues".to_string(),
            description:
                "Run deterministic probes first, then let AI rank causes from redacted findings."
                    .to_string(),
            risk: RepairRisk::Low,
            requires_confirmation: false,
            touches: Vec::new(),
        },
        RepairAction {
            id: "apply-confirmed-fixes".to_string(),
            kind: RepairActionKind::PatchConfig,
            title: "Apply confirmed fixes".to_string(),
            description:
                "Only typed, whitelisted repair actions may modify local files after confirmation."
                    .to_string(),
            risk: RepairRisk::Medium,
            requires_confirmation: true,
            touches: vec!["selected runtime config files".to_string()],
        },
        RepairAction {
            id: "verify-runtime".to_string(),
            kind: RepairActionKind::VerifyCommand,
            title: "Verify runtime health".to_string(),
            description: "Re-run version/config probes and produce a local audit report."
                .to_string(),
            risk: RepairRisk::Low,
            requires_confirmation: false,
            touches: Vec::new(),
        },
    ];

    RepairPlan {
        runtime_id: bundle.runtime_id.clone(),
        summary: "Safe repair preview: backup, diagnose with redacted facts, apply confirmed typed actions, verify, and keep rollback metadata.".to_string(),
        redacted_facts: redactor.redact_bundle(&bundle),
        requires_confirmation: actions.iter().any(|action| action.requires_confirmation),
        actions,
    }
}

fn redact_home_path(value: &str) -> String {
    let home = dirs::home_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    if !home.is_empty() && value.starts_with(&home) {
        return value.replacen(&home, "$HOME", 1);
    }
    value.to_string()
}

fn redact_inline_secrets(key: &str, value: &str) -> String {
    if looks_secret_key(key) {
        return "[REDACTED]".to_string();
    }
    redact_inline_secret_values(value)
}

fn redact_inline_secret_values(value: &str) -> String {
    value
        .split_whitespace()
        .map(|part| {
            if looks_secret_value(part) {
                "[REDACTED]".to_string()
            } else if let Some(token) = part.strip_prefix("Bearer ") {
                if looks_secret_value(token) {
                    "Bearer [REDACTED]".to_string()
                } else {
                    part.to_string()
                }
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn looks_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "api_key",
        "apikey",
        "token",
        "secret",
        "password",
        "authorization",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn looks_secret_value(value: &str) -> bool {
    let trimmed = value.trim_matches(|c: char| {
        matches!(
            c,
            '"' | '\'' | ',' | ';' | ')' | '(' | '[' | ']' | '{' | '}'
        )
    });
    trimmed.len() >= 12
        && (trimmed.starts_with("sk-")
            || trimmed.starts_with("ghp_")
            || trimmed.starts_with("gho_")
            || trimmed.starts_with("xox")
            || trimmed.starts_with("AKIA")
            || trimmed.contains("_API_KEY="))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secret_facts() {
        let fact = DiagnosticFact::new(
            "openai_api_key",
            "sk-secret-value",
            SensitivityLevel::Secret,
        );
        let redacted = Redactor::new(RedactionPolicy::default()).redact_fact(&fact);
        assert_eq!(redacted.value, "[REDACTED]");
        assert!(redacted.redacted);
    }

    #[test]
    fn redacts_inline_tokens_in_logs() {
        let fact = DiagnosticFact::new(
            "runtime.log",
            "request failed with sk-1234567890abcdef token",
            SensitivityLevel::SensitiveLog,
        );
        let redacted = Redactor::new(RedactionPolicy::default()).redact_fact(&fact);
        assert!(redacted.value.contains("[REDACTED]"));
        assert!(!redacted.value.contains("sk-1234567890abcdef"));
    }

    #[test]
    fn keeps_config_shape_api_key_metadata_visible() {
        let fact = DiagnosticFact::new(
            "hermes.api_key.env",
            "DEEPSEEK_API_KEY",
            SensitivityLevel::ConfigShape,
        );
        let redacted = Redactor::new(RedactionPolicy::default()).redact_fact(&fact);
        assert_eq!(redacted.value, "DEEPSEEK_API_KEY");
        assert!(!redacted.redacted);
    }

    #[test]
    fn repair_preview_requires_confirmation_for_writes() {
        let plan = build_repair_preview("openclaw");
        assert!(plan.requires_confirmation);
        assert!(plan
            .actions
            .iter()
            .any(|action| action.kind == RepairActionKind::PatchConfig));
    }
}
