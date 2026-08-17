use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{
    build_repair_preview_from_bundle, AuditReport, BackupSnapshot, RedactionPolicy, Redactor,
    RepairAction, RepairActionKind, RepairPlan, SensitivityLevel, SnapshotFile,
};
use crate::probe::{probe_runtime, ProbeStatus, RuntimeProbeReport};
use crate::runtime::{
    adapter_by_id, apply_runtime_playbook, run_runtime_lifecycle, runtime_supports_lifecycle,
    runtime_supports_playbook, RuntimeLifecycleAction,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepairExecuteOptions {
    /// When true, run write actions such as `PatchConfig` that require confirmation.
    pub apply_confirmed_writes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedRepairAction {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairExecuteReport {
    pub runtime_id: String,
    pub plan: RepairPlan,
    pub backup: BackupSnapshot,
    pub before_probe: RuntimeProbeReport,
    pub after_probe: RuntimeProbeReport,
    pub executed_action_ids: Vec<String>,
    pub skipped_actions: Vec<SkippedRepairAction>,
    pub audit: AuditReport,
    /// Local guide document when API key scaffold was created.
    pub guide_path: Option<String>,
}

/// Run the repair pipeline: probe → plan → backup → apply actions → re-probe → audit.
pub fn execute_repair(
    runtime_id: &str,
    options: &RepairExecuteOptions,
) -> Result<RepairExecuteReport> {
    let before_probe = probe_runtime(runtime_id)?;
    let plan = build_repair_preview_from_bundle(before_probe.to_diagnostic_bundle());
    let backup = create_runtime_backup_snapshot(runtime_id)?;

    let mut executed_action_ids = vec!["backup-runtime-configs".to_string()];
    let mut skipped_actions = Vec::new();

    let mut guide_path = None;
    if options.apply_confirmed_writes && runtime_supports_playbook(runtime_id) {
        let playbook = apply_runtime_playbook(runtime_id, &before_probe)?;
        guide_path = playbook.guide_path.map(|path| path.display().to_string());
        executed_action_ids.extend(playbook.executed);
        skipped_actions.extend(playbook.skipped);
    }

    for action in &plan.actions {
        if is_internal_plan_action(action) {
            continue;
        }
        match execute_planned_action(runtime_id, action, options, &backup) {
            ActionRunResult::Executed => executed_action_ids.push(action.id.clone()),
            ActionRunResult::Skipped(reason) => skipped_actions.push(SkippedRepairAction {
                id: action.id.clone(),
                reason,
            }),
        }
    }

    let after_probe = probe_runtime(runtime_id)?;
    let audit = build_audit_report(
        runtime_id,
        &plan,
        &backup,
        &before_probe,
        &after_probe,
        &executed_action_ids,
    );

    Ok(RepairExecuteReport {
        runtime_id: runtime_id.to_string(),
        plan,
        backup,
        before_probe,
        after_probe,
        executed_action_ids,
        skipped_actions,
        audit,
        guide_path,
    })
}

enum ActionRunResult {
    Executed,
    Skipped(String),
}

fn is_internal_plan_action(action: &RepairAction) -> bool {
    matches!(
        action.kind,
        RepairActionKind::BackupFile
            | RepairActionKind::ManualReview
            | RepairActionKind::VerifyCommand
    ) || action.id == "apply-confirmed-fixes"
}

fn execute_planned_action(
    runtime_id: &str,
    action: &RepairAction,
    options: &RepairExecuteOptions,
    backup: &BackupSnapshot,
) -> ActionRunResult {
    match action.kind {
        RepairActionKind::BackupFile => {
            ActionRunResult::Skipped(format!("backup already created at {}", backup.root))
        }
        RepairActionKind::ManualReview => {
            ActionRunResult::Skipped("manual review is advisory only".to_string())
        }
        RepairActionKind::VerifyCommand => ActionRunResult::Skipped(
            "verification runs after all actions via probe_runtime".to_string(),
        ),
        RepairActionKind::PatchConfig => {
            if !options.apply_confirmed_writes {
                return ActionRunResult::Skipped(
                    "pass --apply to run confirmed config patches".to_string(),
                );
            }
            if runtime_supports_playbook(runtime_id) && action.id == "apply-confirmed-fixes" {
                return ActionRunResult::Skipped(
                    "runtime rule playbook already evaluated probe findings".to_string(),
                );
            }
            if action.requires_confirmation {
                match apply_patch_config(runtime_id, action) {
                    Ok(()) => ActionRunResult::Executed,
                    Err(error) => ActionRunResult::Skipped(error.to_string()),
                }
            } else {
                ActionRunResult::Skipped("automatic PatchConfig is not implemented yet".to_string())
            }
        }
        RepairActionKind::RestoreBackup => ActionRunResult::Skipped(
            "restore is only available as an explicit rollback workflow".to_string(),
        ),
        RepairActionKind::InstallRuntime | RepairActionKind::UpdateRuntime => {
            if !options.apply_confirmed_writes {
                return ActionRunResult::Skipped(
                    "pass --apply to run confirmed install/update actions".to_string(),
                );
            }
            if !runtime_supports_lifecycle(runtime_id) {
                return ActionRunResult::Skipped(format!(
                    "{} is not implemented for runtime '{runtime_id}'",
                    action.title
                ));
            }
            let lifecycle = match action.kind {
                RepairActionKind::InstallRuntime => RuntimeLifecycleAction::Install,
                RepairActionKind::UpdateRuntime => RuntimeLifecycleAction::Update,
                _ => unreachable!("matched install/update arm"),
            };
            match run_runtime_lifecycle(runtime_id, lifecycle) {
                Ok(()) => ActionRunResult::Executed,
                Err(error) => ActionRunResult::Skipped(error.to_string()),
            }
        }
        RepairActionKind::RemoveBrokenSymlink => {
            ActionRunResult::Skipped("symlink cleanup playbook not implemented yet".to_string())
        }
    }
}

fn apply_patch_config(_runtime_id: &str, _action: &RepairAction) -> Result<()> {
    anyhow::bail!("rule-based PatchConfig playbooks are not implemented yet")
}

pub fn backups_root() -> Result<PathBuf> {
    let root = dirs::config_dir()
        .map(|dir| dir.join("agent-doctor").join("backups"))
        .context("could not resolve config directory")?;
    fs::create_dir_all(&root)?;
    Ok(root)
}

pub fn create_runtime_backup_snapshot(runtime_id: &str) -> Result<BackupSnapshot> {
    let adapter =
        adapter_by_id(runtime_id).with_context(|| format!("unknown runtime '{runtime_id}'"))?;
    let config_paths = adapter.all_config_paths();
    let snapshot_id = format!("{runtime_id}-{}", unix_seconds());
    let snapshot_root = backups_root()?.join(&snapshot_id);
    fs::create_dir_all(&snapshot_root)?;

    let files = snapshot_config_files(&config_paths, &snapshot_root)?;

    Ok(BackupSnapshot {
        id: snapshot_id,
        runtime_id: runtime_id.to_string(),
        root: snapshot_root.display().to_string(),
        files,
    })
}

pub fn snapshot_config_files(
    config_paths: &[PathBuf],
    snapshot_root: &Path,
) -> Result<Vec<SnapshotFile>> {
    let mut files = Vec::new();
    for path in config_paths {
        if !path.exists() {
            continue;
        }
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "config".to_string());
        let dest = snapshot_root.join(file_name);
        fs::copy(path, &dest)
            .with_context(|| format!("failed to copy {} to {}", path.display(), dest.display()))?;
        files.push(SnapshotFile {
            original_path: path.display().to_string(),
            snapshot_path: dest.display().to_string(),
            sensitivity: SensitivityLevel::LocalPath,
        });
    }
    Ok(files)
}

pub(crate) fn build_audit_report(
    runtime_id: &str,
    _plan: &RepairPlan,
    backup: &BackupSnapshot,
    before_probe: &RuntimeProbeReport,
    after_probe: &RuntimeProbeReport,
    executed_action_ids: &[String],
) -> AuditReport {
    let redactor = Redactor::new(RedactionPolicy::default());
    let bundle = after_probe.to_diagnostic_bundle();
    AuditReport {
        id: format!("audit-{runtime_id}-{}", unix_seconds()),
        runtime_id: runtime_id.to_string(),
        redacted_facts: redactor.redact_bundle(&bundle),
        action_ids: executed_action_ids.to_vec(),
        verification_summary: format!(
            "before: {}; after: {}",
            probe_health_summary(before_probe),
            probe_health_summary(after_probe)
        ),
        rollback_hint: format!(
            "restore files from backup {} ({} file(s))",
            backup.root,
            backup.files.len()
        ),
    }
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

/// Lower is healthier — used by the repair loop to detect progress between rounds.
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

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn snapshot_config_files_copies_existing_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("config.yaml");
        let mut file = fs::File::create(&source).expect("create");
        writeln!(file, "model:").expect("write");

        let snapshot_root = temp.path().join("snapshot");
        fs::create_dir_all(&snapshot_root).expect("mkdir");

        let files =
            snapshot_config_files(std::slice::from_ref(&source), &snapshot_root).expect("snapshot");
        assert_eq!(files.len(), 1);
        assert!(Path::new(&files[0].snapshot_path).exists());
    }

    #[test]
    fn execute_repair_records_backup_without_apply_writes() {
        let report =
            execute_repair("hermes", &RepairExecuteOptions::default()).expect("execute repair");
        assert!(report.backup.runtime_id == "hermes");
        assert!(report.backup.root.contains("hermes"));
        assert!(report
            .executed_action_ids
            .iter()
            .any(|id| id == "backup-runtime-configs"));
        assert!(!report
            .skipped_actions
            .iter()
            .any(|item| item.id == "apply-confirmed-fixes"));
        assert!(report.audit.verification_summary.contains("before:"));
        assert!(report.audit.verification_summary.contains("after:"));
    }
}
