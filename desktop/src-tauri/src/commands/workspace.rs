use std::path::PathBuf;

use agent_doctor_core::{
    ensure_default_workspace, init_workspace, load_workspaces, use_workspace_with_options,
    workspace_doctor, workspace_fix, workspace_status, InitWorkspaceReport, UseWorkspaceOptions,
    UseWorkspaceReport, WorkspaceDoctorReport, WorkspaceFixOptions, WorkspaceFixReport,
    WorkspaceStatusReport, WorkspacesDocument,
};

use crate::{rebuild_tray_menu, update_tray_tooltip};

#[tauri::command]
pub fn list_workspaces_command() -> WorkspacesDocument {
    ensure_default_workspace().unwrap_or_else(|_| load_workspaces().unwrap_or_default())
}

#[tauri::command]
pub fn init_workspace_command(
    path: String,
    name: Option<String>,
    git_root: bool,
    app: tauri::AppHandle,
) -> Result<InitWorkspaceReport, String> {
    let report = init_workspace(Some(PathBuf::from(path)), name, git_root)
        .map_err(|error| error.to_string())?;
    let _ = use_workspace_with_options(
        &report.name,
        &UseWorkspaceOptions {
            backup: true,
            restart_gateways: false,
        },
    );
    update_tray_tooltip(&app);
    rebuild_tray_menu(&app);
    Ok(report)
}

#[tauri::command]
pub fn use_workspace_command(
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
pub fn workspace_status_command() -> Result<WorkspaceStatusReport, String> {
    workspace_status(None).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn workspace_doctor_command() -> Result<WorkspaceDoctorReport, String> {
    workspace_doctor().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn workspace_fix_command(migrate_claude_mcp: bool) -> Result<WorkspaceFixReport, String> {
    workspace_fix(&WorkspaceFixOptions {
        dry_run: false,
        restart_gateways: false,
        migrate_claude_mcp,
    })
    .map_err(|error| error.to_string())
}
