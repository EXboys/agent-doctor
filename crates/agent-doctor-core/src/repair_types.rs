//! The redaction, diagnostic, and typed-action vocabulary of the repair flow.
//!
//! Lives in core because probe, install, workspace, and setup all speak these
//! types; only the playbooks and execution loop belong to the repair crate.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::mask::{MaskedFileSnippet, SecretVault};
use crate::probe::{ProbeStatus, RuntimeProbeReport};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedRepair {
    pub id: String,
    pub title: String,
    pub description: String,
    pub auto_fixable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedRepairAction {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PlaybookApplyResult {
    pub executed: Vec<String>,
    pub skipped: Vec<SkippedRepairAction>,
    /// When API key scaffold is created, path to the generated guide document.
    pub guide_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainReport {
    pub summary: String,
    pub likely_causes: Vec<String>,
    pub recommended_action_ids: Vec<String>,
    pub user_next_steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainInput {
    pub runtime_id: String,
    pub probe_summary: String,
    pub issue_score: u32,
    pub checks: Vec<ExplainCheck>,
    pub suggested_repairs: Vec<ExplainSuggestion>,
    pub install_failure: Option<ExplainInstallFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainCheck {
    pub title: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainSuggestion {
    pub id: String,
    pub title: String,
    pub auto_fixable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainInstallFailure {
    pub action_id: String,
    pub reason: String,
    pub log_path: Option<String>,
    pub log_tail: String,
}

/// Root directory every runtime backup snapshot lives under.
pub fn backups_root() -> Result<PathBuf> {
    let root = dirs::config_dir()
        .map(|dir| dir.join("agent-doctor").join("backups"))
        .context("could not resolve config directory")?;
    std::fs::create_dir_all(&root)?;
    Ok(root)
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

// ===== Loop, tool, and planner vocabulary (moved with the repair split) =====

/// One repair tool an LLM planner may call, mirrored from tool-call JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairToolKind {
    Read,
    ListDir,
    GrepFiles,
    WriteFile,
    SearchReplace,
    PatchConfig,
    Bash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairToolResult {
    pub kind: RepairToolKind,
    pub success: bool,
    pub masked_output: String,
    pub applied: bool,
    pub preview_diff: Option<String>,
    pub error: Option<String>,
}

/// Input for repair planning (LLM or deterministic). Secrets appear as vault tokens only.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MaskedRepairContext {
    pub runtime_id: String,
    pub redacted_facts: Vec<RedactedFact>,
    pub masked_files: Vec<MaskedFileSnippet>,
    pub suggested_repairs: Vec<SuggestedRepair>,
    /// Local-only vault for merge after planning; never serialized.
    #[serde(skip)]
    vault: SecretVault,
}

impl MaskedRepairContext {
    /// Struct-literal construction stays possible across crates; the vault
    /// itself is only reachable through [`Self::set_vault`] / [`Self::take_vault`].
    #[allow(clippy::needless_update)]
    pub fn new(
        runtime_id: String,
        redacted_facts: Vec<RedactedFact>,
        masked_files: Vec<MaskedFileSnippet>,
        suggested_repairs: Vec<SuggestedRepair>,
        vault: SecretVault,
    ) -> Self {
        Self {
            runtime_id,
            redacted_facts,
            masked_files,
            suggested_repairs,
            vault,
        }
    }

    pub fn set_vault(&mut self, vault: SecretVault) {
        self.vault = vault;
    }

    /// Takes the vault out for the merge-back phase.
    pub fn take_vault(&mut self) -> SecretVault {
        std::mem::take(&mut self.vault)
    }
}

/// OpenAI-compatible gateway endpoint plus credentials. `Debug` never shows the key.
#[derive(Clone)]
pub struct LlmConfig {
    pub api_url: String,
    pub api_key: String,
    pub model: String,
}

impl std::fmt::Debug for LlmConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmConfig")
            .field("api_url", &self.api_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .finish()
    }
}

impl LlmConfig {
    /// OpenAI-compatible gateway base (e.g. `https://api.deepseek.com/v1`) or a full
    /// `.../chat/completions` URL.
    pub fn from_gateway(gateway_url: &str, api_key: &str, model: &str) -> Self {
        let base = gateway_url.trim().trim_end_matches('/');
        let api_url = if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{base}/chat/completions")
        };
        Self {
            api_url,
            api_key: api_key.trim().to_string(),
            model: model.trim().to_string(),
        }
    }

    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("AGENT_DOCTOR_LLM_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .ok()?;
        if api_key.trim().is_empty() {
            return None;
        }
        let api_url = std::env::var("AGENT_DOCTOR_LLM_API_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions".to_string());
        let model =
            std::env::var("AGENT_DOCTOR_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
        Some(Self {
            api_url,
            api_key,
            model,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairLoopRound {
    pub round: u32,
    pub probe_summary: String,
    pub issue_score: u32,
    pub planned_action_ids: Vec<String>,
    pub executed_action_ids: Vec<String>,
    pub skipped_actions: Vec<SkippedRepairAction>,
    pub tool_trace: Vec<RepairToolResult>,
    pub masked_context: MaskedRepairContext,
    /// Checks this round made worse; non-empty with `rolled_back` means its edits were undone.
    #[serde(default)]
    pub new_issues: Vec<CheckChange>,
    #[serde(default)]
    pub rolled_back: bool,
}

/// One check whose state differs between two probes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckChange {
    pub id: String,
    pub title: String,
    pub status: ProbeStatus,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckDiff {
    /// Was warn/fail before, now passes (or no longer reported).
    pub fixed: Vec<CheckChange>,
    /// New warn/fail, or a check that got worse.
    pub new_issues: Vec<CheckChange>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepairLoopOptions {
    pub apply_confirmed_writes: bool,
    pub max_rounds: Option<u32>,
    pub use_ai_planner: bool,
    /// Model for the AI planner; `None` reads it from the environment.
    #[serde(skip)]
    pub llm: Option<LlmConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairLoopReport {
    pub runtime_id: String,
    pub plan: RepairPlan,
    pub backup: BackupSnapshot,
    pub before_probe: RuntimeProbeReport,
    pub after_probe: RuntimeProbeReport,
    pub rounds: Vec<RepairLoopRound>,
    pub executed_action_ids: Vec<String>,
    pub skipped_actions: Vec<SkippedRepairAction>,
    pub audit: AuditReport,
    pub guide_path: Option<String>,
}

pub fn probe_health_summary(report: &RuntimeProbeReport) -> String {
    let mut pass = 0usize;
    let mut warn = 0usize;
    let mut fail = 0usize;
    let mut not_checked = 0usize;
    let mut not_applicable = 0usize;

    for check in &report.checks {
        match check.status {
            ProbeStatus::Pass => pass += 1,
            ProbeStatus::Warn => warn += 1,
            ProbeStatus::Fail => fail += 1,
            ProbeStatus::NotChecked => not_checked += 1,
            ProbeStatus::NotApplicable => not_applicable += 1,
        }
    }

    format!("pass={pass} warn={warn} fail={fail} not_checked={not_checked} n/a={not_applicable}")
}

pub fn probe_issue_score(report: &RuntimeProbeReport) -> u32 {
    let mut score = 0u32;
    for check in &report.checks {
        score += match check.status {
            ProbeStatus::Fail => 100,
            ProbeStatus::Warn => 10,
            ProbeStatus::NotChecked => 1,
            ProbeStatus::Pass | ProbeStatus::NotApplicable => 0,
        };
    }
    score
}

/// Deterministic explanation used when no LLM is configured or reachable.
pub fn fallback_explain(input: &ExplainInput) -> ExplainReport {
    let mut likely_causes = Vec::new();
    let mut next_steps = Vec::new();

    for check in &input.checks {
        if check.status == "fail" || check.status == "warn" {
            likely_causes.push(format!("{}: {}", check.title, check.message));
        }
    }

    if let Some(failure) = &input.install_failure {
        likely_causes.push(format!("Install failed: {}", failure.reason));
        next_steps.push(format!(
            "Inspect install log: {}",
            failure.log_path.as_deref().unwrap_or("(not saved)")
        ));
        next_steps.push(format!("Retry: agent-doctor install {}", input.runtime_id));
    }

    if next_steps.is_empty() {
        for suggestion in &input.suggested_repairs {
            if suggestion.auto_fixable {
                next_steps.push(format!(
                    "Run: agent-doctor repair {} --apply",
                    input.runtime_id
                ));
                break;
            }
        }
    }

    if next_steps.is_empty() && input.issue_score == 0 {
        next_steps.push("No action required.".to_string());
    }

    ExplainReport {
        summary: format!(
            "{} health score {} — {}",
            input.runtime_id, input.issue_score, input.probe_summary
        ),
        likely_causes,
        recommended_action_ids: input
            .suggested_repairs
            .iter()
            .filter(|item| item.auto_fixable)
            .map(|item| item.id.clone())
            .collect(),
        user_next_steps: next_steps,
    }
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
