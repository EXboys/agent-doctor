use agent_doctor_core::{
    add_host, add_project, bootstrap_and_add_host, load_remote_hosts, probe_remote_host,
    publish_managed_logins, remove_host, remove_project, run_remote_doctor, BootstrapHostOptions,
    LinkDirEntry, LinkDoctorSummary, LinkFilePayload, LinkFixSummary, LinkHello, LinkInitResult,
    LinkWorkspaceRow, RemoteDoctorOptions, RemoteDoctorReport, RemoteHostProbeReport,
    RemoteHostsDocument,
};
use serde::Serialize;
use tauri::{Emitter, State};

use crate::link_session::{LinkConnectReport, LinkHostStatus, LinkSessionState};

#[derive(Debug, Clone, Serialize)]
pub struct RemoteHostRow {
    pub host_id: String,
    pub target: String,
    pub managed: bool,
    pub project_count: usize,
    pub display_name: String,
    pub linked_client: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteProjectRow {
    pub host_id: String,
    pub project_id: String,
    pub path: String,
    pub runtimes: Vec<String>,
    /// Display label (managed user@host or legacy ssh config Host).
    pub ssh_config_host: String,
    pub managed: bool,
}

#[tauri::command]
pub fn list_remote_hosts_command() -> Result<RemoteHostsDocument, String> {
    let _ = publish_managed_logins();
    load_remote_hosts().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_remote_host_rows_command() -> Result<Vec<RemoteHostRow>, String> {
    let _ = publish_managed_logins();
    let doc = load_remote_hosts().map_err(|error| error.to_string())?;
    let mut rows: Vec<RemoteHostRow> = doc
        .hosts
        .iter()
        .map(|(host_id, host)| RemoteHostRow {
            host_id: host_id.clone(),
            target: host.display_target(),
            managed: host.is_managed(),
            project_count: host.projects.len(),
            display_name: if host.display_name.trim().is_empty() {
                host_id.clone()
            } else {
                host.display_name.clone()
            },
            linked_client: host.linked_client,
        })
        .collect();
    rows.sort_by(|a, b| a.host_id.cmp(&b.host_id));
    Ok(rows)
}

#[tauri::command]
pub fn list_remote_projects_command() -> Result<Vec<RemoteProjectRow>, String> {
    let doc = load_remote_hosts().map_err(|error| error.to_string())?;
    let mut rows = Vec::new();
    for (host_id, host) in &doc.hosts {
        for (project_id, project) in &host.projects {
            rows.push(RemoteProjectRow {
                host_id: host_id.clone(),
                project_id: project_id.clone(),
                path: project.path.clone(),
                runtimes: project.runtimes.clone(),
                ssh_config_host: host.display_target(),
                managed: host.is_managed(),
            });
        }
    }
    rows.sort_by(|a, b| (&a.host_id, &a.project_id).cmp(&(&b.host_id, &b.project_id)));
    Ok(rows)
}

#[tauri::command]
pub async fn bootstrap_remote_host_command(
    id: String,
    hostname: String,
    user: String,
    port: u16,
    password: String,
) -> Result<RemoteHostsDocument, String> {
    tauri::async_runtime::spawn_blocking(move || {
        bootstrap_and_add_host(BootstrapHostOptions {
            id,
            hostname,
            user,
            port,
            password,
            label: None,
        })
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn add_remote_host_command(
    id: String,
    ssh_config_host: String,
) -> Result<RemoteHostsDocument, String> {
    add_host(&id, &ssh_config_host).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn add_remote_project_command(
    host: String,
    name: String,
    path: String,
    runtimes: Vec<String>,
) -> Result<RemoteHostsDocument, String> {
    add_project(&host, &name, &path, runtimes).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn remove_remote_host_command(id: String) -> Result<RemoteHostsDocument, String> {
    remove_host(&id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn remove_remote_project_command(
    host: String,
    name: String,
) -> Result<RemoteHostsDocument, String> {
    remove_project(&host, &name).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn probe_remote_host_command(id: String) -> Result<RemoteHostProbeReport, String> {
    // SSH must not run on the UI / IPC thread — same pattern as repair commands.
    tauri::async_runtime::spawn_blocking(move || {
        probe_remote_host(&id).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn run_remote_doctor_command(
    target: String,
    runtime: Option<String>,
) -> Result<RemoteDoctorReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_remote_doctor(
            &target,
            RemoteDoctorOptions {
                runtime_filter: runtime,
                save_report: true,
            },
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn connect_linked_host_command(
    id: String,
    state: State<'_, LinkSessionState>,
) -> Result<LinkConnectReport, String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || state.connect(&id))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn nearby_sharing_command() -> bool {
    agent_doctor_core::nearby_sharing()
}

#[tauri::command]
pub fn set_nearby_sharing_command(enabled: bool) {
    agent_doctor_core::set_nearby_sharing(enabled);
}

#[tauri::command]
pub fn this_computer_name_command() -> String {
    agent_doctor_core::this_computer_name()
}

#[tauri::command]
pub fn list_nearby_computers_command() -> Result<Vec<agent_doctor_core::NearbyComputer>, String> {
    Ok(agent_doctor_core::list_nearby())
}

#[tauri::command]
pub fn pending_nearby_asks_command() -> Result<Vec<agent_doctor_core::NearbyAsk>, String> {
    Ok(agent_doctor_core::pending_asks())
}

#[tauri::command]
pub fn answer_nearby_ask_command(id: String, allow: bool) -> Result<(), String> {
    agent_doctor_core::decide_ask(&id, allow);
    Ok(())
}

#[tauri::command]
pub async fn use_nearby_computer_command(
    id: String,
    state: State<'_, LinkSessionState>,
    app: tauri::AppHandle,
) -> Result<LinkConnectReport, String> {
    let state = (*state).clone();
    let report = tauri::async_runtime::spawn_blocking(move || state.connect_nearby(&id))
        .await
        .map_err(|error| error.to_string())?;
    if report.ok {
        let _ = app.emit("workspace-changed", &report.host_id);
    }
    Ok(report)
}

#[tauri::command]
pub fn disconnect_linked_host_command(
    id: String,
    state: State<'_, LinkSessionState>,
) -> Result<(), String> {
    state.disconnect(&id);
    Ok(())
}

#[tauri::command]
pub fn list_linked_host_status_command(
    state: State<'_, LinkSessionState>,
) -> Result<Vec<LinkHostStatus>, String> {
    Ok(state.status_rows())
}

#[tauri::command]
pub async fn linked_list_workspaces_command(
    host_id: String,
    state: State<'_, LinkSessionState>,
) -> Result<Vec<LinkWorkspaceRow>, String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_client(&host_id, |client| client.list_workspaces())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn linked_init_workspace_command(
    host_id: String,
    path: String,
    name: Option<String>,
    state: State<'_, LinkSessionState>,
) -> Result<LinkInitResult, String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_client(&host_id, |client| {
            client.init_workspace(&path, name.as_deref())
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn linked_list_dir_command(
    host_id: String,
    path: String,
    state: State<'_, LinkSessionState>,
) -> Result<Vec<LinkDirEntry>, String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_client(&host_id, |client| client.list_absolute_dir(&path))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn linked_list_project_dir_command(
    host_id: String,
    root: String,
    relative: Option<String>,
    state: State<'_, LinkSessionState>,
) -> Result<Vec<LinkDirEntry>, String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_client(&host_id, |client| {
            client.list_project_dir(&root, relative.as_deref().unwrap_or(""))
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn linked_read_file_command(
    host_id: String,
    root: String,
    relative: String,
    state: State<'_, LinkSessionState>,
) -> Result<LinkFilePayload, String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_client(&host_id, |client| client.read_file(&root, &relative))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn linked_write_file_command(
    host_id: String,
    root: String,
    relative: String,
    content: String,
    state: State<'_, LinkSessionState>,
) -> Result<(), String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_client(&host_id, |client| {
            client.write_file(&root, &relative, &content)
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn linked_workspace_doctor_command(
    host_id: String,
    state: State<'_, LinkSessionState>,
) -> Result<LinkDoctorSummary, String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_client(&host_id, |client| client.workspace_doctor())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn linked_workspace_fix_command(
    host_id: String,
    migrate_claude_mcp: bool,
    state: State<'_, LinkSessionState>,
) -> Result<LinkFixSummary, String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_client(&host_id, |client| client.workspace_fix(migrate_claude_mcp))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn linked_ping_command(
    host_id: String,
    state: State<'_, LinkSessionState>,
) -> Result<LinkHello, String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_client(&host_id, |client| client.ping())
    })
    .await
    .map_err(|error| error.to_string())?
}
