use agent_doctor_core::{
    activate_personal_provider, apply_profile_model, build_repair_preview_from_bundle,
    delete_personal_provider, evotown_status, execute_evotown_onboarding,
    execute_install_with_progress, execute_personal_provider_setup, execute_repair, execute_sync,
    list_personal_providers, list_runtime_backup_ids, load_evotown_config, load_mode_status,
    load_personal_provider_status, load_profiles, load_workspaces, needs_binary_install,
    open_interactive_session, probe_runtime, restore_runtime_backup, run_doctor,
    runtime_supports_playbook, set_runtime_model, suggest_runtime_repairs,
    switch_to_personal_mode, switch_to_team_mode, upsert_personal_provider, use_profile,
    use_workspace_with_options, verify_personal_provider_with_protocol, workspace_doctor,
    workspace_fix, workspace_status, ApplyReport, DoctorReport, EvotownStatus, HermesAdapter,
    HermesProfilePreset, HermesSettings, InstallOptions, InstallProgressEvent, InstallReport,
    ModeStatus, ModeSwitchReport, OnboardingOptions, OnboardingReport, OpenSessionOptions,
    OpenSessionReport, PersonalProviderOptions, PersonalProviderSetupReport, PersonalProviderStatus,
    PersonalProviderVerifyReport, PersonalProvidersDocument, ProbeStatus, ProfilesDocument,
    RepairExecuteOptions, RepairExecuteReport, RestoreReport, RuntimeModelPreset,
    RuntimeProbeReport, SyncOptions, SyncReport, UpsertPersonalProviderOptions, UseProfileReport,
    UseWorkspaceOptions, UseWorkspaceReport, WorkspaceDoctorReport, WorkspaceFixOptions,
    WorkspaceFixReport, WorkspaceStatusReport, WorkspacesDocument,
};
use serde::Serialize;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn publish_doctor_report(app: &tauri::AppHandle, report: &DoctorReport) {
    show_main_window(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("doctor-report", report);
    }
}

#[tauri::command]
fn list_workspaces_command() -> WorkspacesDocument {
    load_workspaces().unwrap_or_default()
}

#[tauri::command]
fn use_workspace_command(
    name: String,
    app: tauri::AppHandle,
) -> Result<UseWorkspaceReport, String> {
    let report = use_workspace_with_options(
        &name,
        &UseWorkspaceOptions {
            backup: true,
            restart_gateways: false,
        },
    )
    .map_err(|error| error.to_string())?;
    update_tray_tooltip(&app);
    rebuild_tray_menu(&app);
    Ok(report)
}

#[tauri::command]
fn workspace_status_command() -> Result<WorkspaceStatusReport, String> {
    workspace_status(None).map_err(|error| error.to_string())
}

#[tauri::command]
fn workspace_doctor_command() -> Result<WorkspaceDoctorReport, String> {
    workspace_doctor().map_err(|error| error.to_string())
}

#[tauri::command]
fn workspace_fix_command(migrate_claude_mcp: bool) -> Result<WorkspaceFixReport, String> {
    workspace_fix(&WorkspaceFixOptions {
        dry_run: false,
        restart_gateways: false,
        migrate_claude_mcp,
    })
    .map_err(|error| error.to_string())
}

fn build_tray_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{IsMenuItem, Menu, MenuItem, Submenu};

    let doc = load_workspaces().unwrap_or_default();
    let show = MenuItem::with_id(app, "show", "Show Agent Doctor", true, None::<&str>)?;
    let ws_doctor = MenuItem::with_id(
        app,
        "workspace_doctor",
        "Workspace check",
        true,
        None::<&str>,
    )?;
    let doctor = MenuItem::with_id(app, "doctor", "Run doctor", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let mut switch_items: Vec<MenuItem<tauri::Wry>> = Vec::new();
    for name in doc.workspaces.keys() {
        let label = if doc.active.as_deref() == Some(name.as_str()) {
            format!("✓ {name}")
        } else {
            name.clone()
        };
        switch_items.push(MenuItem::with_id(
            app,
            format!("workspace:{name}"),
            &label,
            true,
            None::<&str>,
        )?);
    }

    let none_item = MenuItem::with_id(
        app,
        "workspace:none",
        "(no workspaces)",
        false,
        None::<&str>,
    )?;
    let switch_refs: Vec<&dyn IsMenuItem<tauri::Wry>> = if switch_items.is_empty() {
        vec![&none_item as &dyn IsMenuItem<tauri::Wry>]
    } else {
        switch_items
            .iter()
            .map(|item| item as &dyn IsMenuItem<tauri::Wry>)
            .collect()
    };
    let switch_sub = Submenu::with_id(app, "switch", "Switch workspace", true)?;
    if switch_items.is_empty() {
        switch_sub.append(&none_item)?;
    } else {
        switch_sub.append_items(&switch_refs)?;
    }
    Menu::with_items(app, &[&show, &switch_sub, &ws_doctor, &doctor, &quit])
}

fn rebuild_tray_menu(app: &tauri::AppHandle) {
    if let Ok(menu) = build_tray_menu(app) {
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn switch_workspace_from_tray(app: &tauri::AppHandle, name: &str) {
    if use_workspace_with_options(
        name,
        &UseWorkspaceOptions {
            backup: true,
            restart_gateways: false,
        },
    )
    .is_ok()
    {
        update_tray_tooltip(app);
        rebuild_tray_menu(app);
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.emit("workspace-changed", name);
        }
    }
}

fn publish_workspace_doctor_report(app: &tauri::AppHandle, report: &WorkspaceDoctorReport) {
    show_main_window(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("workspace-doctor-report", report);
    }
}

fn update_tray_tooltip(app: &tauri::AppHandle) {
    let doc = load_workspaces().unwrap_or_default();
    let label = match doc.active.as_deref() {
        Some(name) => format!("Agent Doctor · workspace: {name}"),
        None => "Agent Doctor".to_string(),
    };
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(&label));
    }
}

#[tauri::command]
fn get_evotown_status_command() -> EvotownStatus {
    evotown_status().unwrap_or(EvotownStatus {
        configured: false,
        base_url: None,
        api_key_hint: None,
        config_source: None,
        runtime_target: None,
        bundle_id: None,
    })
}

#[tauri::command]
fn run_evotown_onboarding_command(
    url: String,
    key: String,
    sync_skills: bool,
    pull_policies: bool,
) -> Result<OnboardingReport, String> {
    execute_evotown_onboarding(&OnboardingOptions {
        url,
        api_key: key,
        hermes_provider: "openai".to_string(),
        sync_skills,
        pull_policies,
    })
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn run_sync_command() -> Result<SyncReport, String> {
    let config = load_evotown_config().map_err(|error| error.to_string())?;
    execute_sync(
        &config,
        &SyncOptions {
            dry_run: false,
            only_skills: Vec::new(),
            runtime_target: None,
            bundle_id: None,
        },
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_personal_provider_status_command() -> PersonalProviderStatus {
    load_personal_provider_status().unwrap_or(PersonalProviderStatus {
        configured: false,
        gateway_url: None,
        model: None,
        api_key_hint: None,
        profile_env_path: None,
        active_id: None,
        active_name: None,
        protocol: None,
    })
}

#[tauri::command]
fn list_personal_providers_command() -> PersonalProvidersDocument {
    list_personal_providers().unwrap_or(PersonalProvidersDocument {
        active_id: None,
        providers: Vec::new(),
        store_path: String::new(),
    })
}

#[tauri::command]
fn upsert_personal_provider_command(
    id: Option<String>,
    name: String,
    url: String,
    key: String,
    model: String,
    protocol: String,
    activate: bool,
) -> Result<PersonalProvidersDocument, String> {
    upsert_personal_provider(&UpsertPersonalProviderOptions {
        id,
        name,
        url,
        api_key: key,
        model,
        protocol,
        activate,
    })
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_personal_provider_command(id: String) -> Result<PersonalProvidersDocument, String> {
    delete_personal_provider(&id).map_err(|error| error.to_string())
}

#[tauri::command]
fn activate_personal_provider_command(id: String) -> Result<PersonalProviderSetupReport, String> {
    activate_personal_provider(&id).map_err(|error| error.to_string())
}

#[tauri::command]
fn verify_personal_provider_command(
    url: String,
    key: String,
    protocol: String,
) -> Result<PersonalProviderVerifyReport, String> {
    verify_personal_provider_with_protocol(&url, &key, &protocol).map_err(|error| error.to_string())
}

#[tauri::command]
fn apply_personal_provider_command(
    url: String,
    key: String,
    model: String,
    protocol: String,
) -> Result<PersonalProviderSetupReport, String> {
    execute_personal_provider_setup(&PersonalProviderOptions {
        url,
        api_key: key,
        model,
        protocol,
    })
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_mode_status_command() -> ModeStatus {
    load_mode_status().unwrap_or(ModeStatus {
        mode: "unset".to_string(),
        personal_ready: false,
        team_ready: false,
        active_label: None,
        active_gateway_url: None,
        active_key_hint: None,
        personal_active_id: None,
        personal_active_name: None,
        team_base_url: None,
    })
}

#[tauri::command]
fn switch_to_personal_mode_command(
    provider_id: Option<String>,
) -> Result<ModeSwitchReport, String> {
    switch_to_personal_mode(provider_id.as_deref()).map_err(|error| error.to_string())
}

#[tauri::command]
fn switch_to_team_mode_command() -> Result<ModeSwitchReport, String> {
    switch_to_team_mode().map_err(|error| error.to_string())
}

#[tauri::command]
fn run_doctor_command() -> DoctorReport {
    run_doctor()
}

#[tauri::command]
fn list_profiles_command() -> ProfilesDocument {
    load_profiles().unwrap_or(ProfilesDocument {
        active: None,
        profiles: Default::default(),
    })
}

#[tauri::command]
fn use_profile_command(name: String) -> Result<UseProfileReport, String> {
    use_profile(&name).map_err(|error| error.to_string())
}

#[tauri::command]
fn get_hermes_model_command() -> Result<HermesSettings, String> {
    HermesAdapter
        .read_settings()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_hermes_model_command(
    provider: String,
    model: String,
    base_url: String,
    api_key: Option<String>,
) -> Result<ApplyReport, String> {
    set_runtime_model(
        "hermes",
        RuntimeModelPreset {
            provider,
            model,
            base_url,
        },
        api_key.as_deref(),
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn apply_profile_model_command(
    profile: String,
    provider: String,
    model: String,
    base_url: String,
) -> Result<ApplyReport, String> {
    apply_profile_model(
        &profile,
        HermesProfilePreset {
            provider,
            model,
            base_url,
        },
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn run_repair_preview_command(runtime: String) -> Result<RepairPreviewResponse, String> {
    let report = probe_runtime(&runtime).map_err(|error| error.to_string())?;
    Ok(build_repair_preview_response(report, None))
}

#[tauri::command]
fn run_repair_execute_command(runtime: String) -> Result<RepairPreviewResponse, String> {
    let result = execute_repair(
        &runtime,
        &RepairExecuteOptions {
            apply_confirmed_writes: true,
        },
    )
    .map_err(|error| error.to_string())?;
    let execute = RepairExecuteSummary::from(&result);
    Ok(build_repair_preview_response(
        result.after_probe,
        Some(execute),
    ))
}

#[tauri::command]
fn run_repair_rollback_command(
    runtime: String,
    backup: Option<String>,
) -> Result<RestoreSummary, String> {
    let report =
        restore_runtime_backup(&runtime, backup.as_deref()).map_err(|error| error.to_string())?;
    Ok(RestoreSummary::from(&report))
}

#[tauri::command]
async fn install_runtime_command(
    app: tauri::AppHandle,
    runtime: String,
) -> Result<InstallRuntimeResponse, String> {
    let app_for_emit = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
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
        Ok(InstallRuntimeResponse::from(&report))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn open_path_command(path: String, app: tauri::AppHandle) -> Result<(), String> {
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn open_session_command(
    runtime: String,
    cwd: Option<String>,
    prompt: Option<String>,
    terminal: Option<bool>,
) -> Result<OpenSessionReport, String> {
    open_interactive_session(&OpenSessionOptions {
        runtime,
        cwd: cwd.map(std::path::PathBuf::from),
        prompt,
        prefer_deep_link: !terminal.unwrap_or(false),
    })
    .map_err(|err| format!("{err:#}"))
}

fn build_repair_preview_response(
    report: RuntimeProbeReport,
    last_execute: Option<RepairExecuteSummary>,
) -> RepairPreviewResponse {
    let plan = build_repair_preview_from_bundle(report.to_diagnostic_bundle());
    let suggested = suggest_runtime_repairs(&report.runtime_id, &report);
    let can_apply_repair = runtime_supports_playbook(&report.runtime_id)
        || suggested.iter().any(|item| item.auto_fixable);
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
struct RepairExecuteSummary {
    backup_id: String,
    backup_root: String,
    executed: Vec<String>,
    skipped: Vec<SkippedRepairItem>,
    verification_summary: String,
    rollback_hint: String,
    guide_path: Option<String>,
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
        }
    }
}

#[derive(Debug, Serialize)]
struct RestoreSummary {
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
struct InstallRuntimeResponse {
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
struct RepairPreviewResponse {
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

fn setup_tray(app: &tauri::App) {
    use tauri::tray::TrayIconBuilder;

    let Ok(menu) = build_tray_menu(app.handle()) else {
        return;
    };

    let tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Agent Doctor")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "workspace_doctor" => {
                if let Ok(report) = workspace_doctor() {
                    publish_workspace_doctor_report(app, &report);
                }
            }
            "doctor" => {
                let report = run_doctor();
                publish_doctor_report(app, &report);
            }
            "quit" => {
                app.exit(0);
            }
            id if id.starts_with("workspace:") => {
                if let Some(name) = id.strip_prefix("workspace:") {
                    if name != "none" {
                        switch_workspace_from_tray(app, name);
                    }
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    let tray = if let Some(icon) = app.default_window_icon() {
        tray.icon(icon.clone())
    } else {
        tray
    };

    if tray.build(app).is_ok() {
        update_tray_tooltip(app.handle());
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            show_main_window(app.handle());
            setup_tray(app);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_evotown_status_command,
            run_evotown_onboarding_command,
            run_sync_command,
            get_personal_provider_status_command,
            list_personal_providers_command,
            upsert_personal_provider_command,
            delete_personal_provider_command,
            activate_personal_provider_command,
            verify_personal_provider_command,
            apply_personal_provider_command,
            get_mode_status_command,
            switch_to_personal_mode_command,
            switch_to_team_mode_command,
            run_doctor_command,
            list_profiles_command,
            list_workspaces_command,
            use_workspace_command,
            workspace_status_command,
            workspace_doctor_command,
            workspace_fix_command,
            use_profile_command,
            get_hermes_model_command,
            set_hermes_model_command,
            apply_profile_model_command,
            run_repair_preview_command,
            run_repair_execute_command,
            run_repair_rollback_command,
            install_runtime_command,
            open_path_command,
            open_session_command
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
