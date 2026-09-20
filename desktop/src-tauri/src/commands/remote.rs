use agent_doctor_core::{
    add_host, add_project, bootstrap_and_add_host, load_remote_hosts, remove_host, remove_project,
    run_remote_doctor, BootstrapHostOptions, RemoteDoctorOptions, RemoteDoctorReport,
    RemoteHostsDocument,
};
use serde::Serialize;

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
pub fn bootstrap_remote_host_command(
    id: String,
    hostname: String,
    user: String,
    port: u16,
    password: String,
) -> Result<RemoteHostsDocument, String> {
    bootstrap_and_add_host(BootstrapHostOptions {
        id,
        hostname,
        user,
        port,
        password,
        label: None,
    })
    .map_err(|error| error.to_string())
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
pub fn run_remote_doctor_command(
    target: String,
    runtime: Option<String>,
) -> Result<RemoteDoctorReport, String> {
    run_remote_doctor(
        &target,
        RemoteDoctorOptions {
            runtime_filter: runtime,
            save_report: true,
        },
    )
    .map_err(|error| error.to_string())
}
