use agent_doctor_core::{
    add_host, add_project, bootstrap_and_add_host, load_remote_hosts, probe_remote_host,
    remove_host, remove_project, run_remote_doctor, BootstrapHostOptions, RemoteDoctorOptions,
    RemoteDoctorReport, RemoteHostProbeReport, RemoteHostsDocument,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RemoteHostRow {
    pub host_id: String,
    pub target: String,
    pub managed: bool,
    pub project_count: usize,
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
    load_remote_hosts().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_remote_host_rows_command() -> Result<Vec<RemoteHostRow>, String> {
    let doc = load_remote_hosts().map_err(|error| error.to_string())?;
    let mut rows: Vec<RemoteHostRow> = doc
        .hosts
        .iter()
        .map(|(host_id, host)| RemoteHostRow {
            host_id: host_id.clone(),
            target: host.display_target(),
            managed: host.is_managed(),
            project_count: host.projects.len(),
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
