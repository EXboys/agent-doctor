use agent_doctor_core::{
    build_repair_preview_from_bundle, execute_install_with_progress, execute_repair,
    list_runtime_backup_ids, needs_binary_install, probe_runtime, restore_runtime_backup,
    run_doctor, suggest_runtime_repairs, InstallOptions, InstallProgressEvent, InstallReport,
    ProbeStatus, RepairExecuteOptions, RepairExecuteReport, RestoreReport, RuntimeProbeReport,
};
use agent_doctor_mcp::{smoke_browser_navigate, SmokeOptions};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::{remember_tray_health, update_tray_tooltip};

#[derive(Debug, Default, Serialize)]
struct RepairPreviewSummary {
    pass: usize,
    warn: usize,
    fail: usize,
    not_applicable: usize,
    not_checked: usize,
}

#[derive(Debug, Serialize)]
struct RepairPreviewCheck {
    title: String,
    status: String,
    message: String,
    details: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SuggestedRepairItem {
    id: String,
    title: String,
    description: String,
    auto_fixable: bool,
}

#[derive(Debug, Serialize)]
struct SkippedRepairItem {
    id: String,
    reason: String,
}

#[derive(Debug, Serialize)]
pub struct BrowserSmokeSummary {
    ok: bool,
    detail: String,
}

#[derive(Debug, Serialize)]
struct RepairExecuteSummary {
    backup_id: String,
    backup_root: String,
    executed: Vec<String>,
    skipped: Vec<SkippedRepairItem>,
    verification_summary: String,
    rollback_hint: String,
    guide_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    browser_smoke: Option<BrowserSmokeSummary>,
}

impl From<&RepairExecuteReport> for RepairExecuteSummary {
    fn from(report: &RepairExecuteReport) -> Self {
        Self {
            backup_id: report.backup.id.clone(),
            backup_root: report.backup.root.clone(),
            executed: report.executed_action_ids.clone(),
            skipped: report
                .skipped_actions
                .iter()
                .map(|item| SkippedRepairItem {
                    id: item.id.clone(),
                    reason: item.reason.clone(),
                })
                .collect(),
            verification_summary: report.audit.verification_summary.clone(),
            rollback_hint: report.audit.rollback_hint.clone(),
            guide_path: report.guide_path.clone(),
            browser_smoke: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RestoreSummary {
    backup_id: String,
    backup_root: String,
    restored_files: Vec<String>,
}

impl From<&RestoreReport> for RestoreSummary {
    fn from(report: &RestoreReport) -> Self {
        Self {
            backup_id: report.backup_id.clone(),
            backup_root: report.backup_root.clone(),
            restored_files: report.restored_files.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct InstallRuntimeResponse {
    runtime_id: String,
    install_needed: bool,
    install_succeeded: bool,
    install_attempts: u8,
    install_log_path: Option<String>,
    manual_fallback: Vec<String>,
    skipped: Vec<SkippedRepairItem>,
    after_installed: bool,
}

impl From<&InstallReport> for InstallRuntimeResponse {
    fn from(report: &InstallReport) -> Self {
        Self {
            runtime_id: report.runtime_id.clone(),
            install_needed: report.install_needed,
            install_succeeded: report.install_succeeded,
            install_attempts: report.install_attempts,
            install_log_path: report.install_log_path.clone(),
            manual_fallback: report.manual_fallback.clone(),
            skipped: report
                .skipped_actions
                .iter()
                .map(|item| SkippedRepairItem {
                    id: item.id.clone(),
                    reason: item.reason.clone(),
                })
                .collect(),
            after_installed: !needs_binary_install(&report.after_probe),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RepairPreviewResponse {
    runtime_id: String,
    display_name: String,
    summary: RepairPreviewSummary,
    checks: Vec<RepairPreviewCheck>,
    plan_summary: String,
    suggested_repairs: Vec<SuggestedRepairItem>,
    can_apply_repair: bool,
    backup_ids: Vec<String>,
    last_execute: Option<RepairExecuteSummary>,
}

fn probe_status_label(status: ProbeStatus) -> &'static str {
    match status {
        ProbeStatus::Pass => "pass",
        ProbeStatus::Warn => "warn",
        ProbeStatus::Fail => "fail",
        ProbeStatus::NotApplicable => "n/a",
        ProbeStatus::NotChecked => "not checked",
    }
}

fn build_repair_preview_response(
    report: RuntimeProbeReport,
    last_execute: Option<RepairExecuteSummary>,
) -> RepairPreviewResponse {
    let plan = build_repair_preview_from_bundle(report.to_diagnostic_bundle());
    let suggested = suggest_runtime_repairs(&report.runtime_id, &report);
    // Show Apply only when there is at least one auto-fixable suggestion.
    // Playbook registration alone is not enough (avoids empty "Apply" on healthy runtimes).
    let can_apply_repair = suggested.iter().any(|item| item.auto_fixable);
    let backup_ids = list_runtime_backup_ids(&report.runtime_id).unwrap_or_default();
    let mut summary = RepairPreviewSummary::default();
    let checks = report
        .checks
        .into_iter()
        .map(|check| {
            match check.status {
                ProbeStatus::Pass => summary.pass += 1,
                ProbeStatus::Warn => summary.warn += 1,
                ProbeStatus::Fail => summary.fail += 1,
                ProbeStatus::NotApplicable => summary.not_applicable += 1,
                ProbeStatus::NotChecked => summary.not_checked += 1,
            }
            RepairPreviewCheck {
                title: check.title,
                status: probe_status_label(check.status).to_string(),
                message: check.message,
                details: check.details,
            }
        })
        .collect();

    RepairPreviewResponse {
        runtime_id: report.runtime_id,
        display_name: report.display_name,
        summary,
        checks,
        plan_summary: plan.summary,
        suggested_repairs: suggested
            .into_iter()
            .map(|item| SuggestedRepairItem {
                id: item.id,
                title: item.title,
                description: item.description,
                auto_fixable: item.auto_fixable,
            })
            .collect(),
        can_apply_repair,
        backup_ids,
        last_execute,
    }
}

#[tauri::command]
pub async fn run_repair_preview_command(runtime: String) -> Result<RepairPreviewResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let report = probe_runtime(&runtime).map_err(|error| error.to_string())?;
        Ok(build_repair_preview_response(report, None))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn run_repair_execute_command(
    app: AppHandle,
    runtime: String,
) -> Result<RepairPreviewResponse, String> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        execute_repair(
            &runtime,
            &RepairExecuteOptions {
                apply_confirmed_writes: true,
            },
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    let mut execute = RepairExecuteSummary::from(&result);
    // Do not auto-launch Chrome here — CDP smoke can block the UI for ~10s.
    // The diagnose panel has an explicit "Browser CDP smoke" button.
    execute.browser_smoke = None;
    // Tray refresh can spawn npm/version probes; never block Apply Repair on it.
    let app_tray = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let doctor = run_doctor();
        remember_tray_health(&app_tray, &doctor);
        update_tray_tooltip(&app_tray);
    });
    Ok(build_repair_preview_response(
        result.after_probe,
        Some(execute),
    ))
}

/// CDP navigate smoke (Chrome launch). Keep this off the Repair/Diagnose hot path.
fn run_browser_smoke_summary() -> BrowserSmokeSummary {
    match smoke_browser_navigate(&SmokeOptions {
        headless: true,
        ..SmokeOptions::default()
    }) {
        Ok(report) => BrowserSmokeSummary {
            ok: report.ok,
            detail: if report.ok {
                format!(
                    "CDP navigate ok → {} ({}) — not an MCP tool-path check",
                    report.title.unwrap_or_else(|| report.url.clone()),
                    report.final_url.unwrap_or(report.url)
                )
            } else {
                format!("CDP navigate failed: {}", report.detail)
            },
        },
        Err(err) => BrowserSmokeSummary {
            ok: false,
            detail: format!("CDP navigate failed: {err}"),
        },
    }
}

#[tauri::command]
pub async fn run_browser_smoke_command() -> Result<BrowserSmokeSummary, String> {
    tauri::async_runtime::spawn_blocking(run_browser_smoke_summary)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn run_repair_rollback_command(
    app: tauri::AppHandle,
    runtime: String,
    backup: Option<String>,
) -> Result<RestoreSummary, String> {
    let report =
        restore_runtime_backup(&runtime, backup.as_deref()).map_err(|error| error.to_string())?;
    let doctor = run_doctor();
    remember_tray_health(&app, &doctor);
    update_tray_tooltip(&app);
    Ok(RestoreSummary::from(&report))
}

#[tauri::command]
pub async fn install_runtime_command(
    app: AppHandle,
    runtime: String,
) -> Result<InstallRuntimeResponse, String> {
    let app_for_emit = app.clone();
    let response = tauri::async_runtime::spawn_blocking(move || {
        let report = execute_install_with_progress(
            &runtime,
            &InstallOptions {
                explain: false,
                plan_ai_repair: false,
                repair_after: false,
                retry_count: 0,
            },
            |event: InstallProgressEvent| {
                let _ = app_for_emit.emit("install-progress", &event);
            },
        )
        .map_err(|error| error.to_string())?;
        Ok::<InstallRuntimeResponse, String>(InstallRuntimeResponse::from(&report))
    })
    .await
    .map_err(|error| error.to_string())??;
    let doctor = run_doctor();
    remember_tray_health(&app, &doctor);
    update_tray_tooltip(&app);
    Ok(response)
}
