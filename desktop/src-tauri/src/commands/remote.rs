use agent_doctor_core::{
    account_host_id, account_ssh_public_key, add_host, add_project, bootstrap_and_add_host,
    choose_account_route, is_ssh_ed25519_pub, list_nearby, load_remote_hosts, probe_remote_host,
    public_ssh_entry, publish_managed_logins, remove_host, remove_project, run_remote_doctor,
    set_wan_share, sync_account_authorized_keys, teamups_channel_client_id,
    teamups_channel_signed_in, teamups_computers, wan_share_enabled, AccountRoute,
    BootstrapHostOptions, LinkDirEntry, LinkDoctorSummary, LinkFilePayload, LinkFixSummary,
    LinkHello, LinkInitResult, LinkWorkspaceRow, RemoteDoctorOptions, RemoteDoctorReport,
    RemoteHostProbeReport, RemoteHostsDocument,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountComputerRow {
    pub id: String,
    pub name: String,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountLinkStatus {
    pub signed_in: bool,
    pub sharing: bool,
    pub public_ip: String,
    pub keys_ready: bool,
    pub computers: Vec<AccountComputerRow>,
}

fn account_link_status(state: &LinkSessionState) -> Result<AccountLinkStatus, String> {
    let sharing = wan_share_enabled();
    if !teamups_channel_signed_in().unwrap_or(false) {
        return Ok(AccountLinkStatus {
            signed_in: false,
            sharing,
            public_ip: String::new(),
            keys_ready: true,
            computers: Vec::new(),
        });
    }
    let computers = teamups_computers()
        .map_err(|_| "没能看到同一账号的电脑。请确认已登录，稍后再看。".to_string())?;
    let self_id = teamups_channel_client_id().unwrap_or_default();
    let keys_ready = if sharing {
        let keys: Vec<(String, String)> = computers
            .iter()
            .filter(|computer| computer.id != self_id && is_ssh_ed25519_pub(&computer.ssh_pub))
            .map(|computer| (computer.id.clone(), computer.ssh_pub.clone()))
            .collect();
        sync_account_authorized_keys(&keys).is_ok()
    } else {
        true
    };
    let public_ip = computers
        .iter()
        .find(|computer| computer.id == self_id && computer.wan)
        .map(|computer| computer.public_ip.clone())
        .unwrap_or_default();
    let nearby: Vec<(String, String)> = list_nearby()
        .into_iter()
        .map(|peer| (peer.id, peer.account))
        .collect();
    let nearby_refs: Vec<(&str, &str)> = nearby
        .iter()
        .map(|(lan_id, account_id)| (lan_id.as_str(), account_id.as_str()))
        .collect();
    let connected: std::collections::HashSet<String> = state
        .status_rows()
        .into_iter()
        .filter(|row| row.connected && row.host_id.starts_with("acct-"))
        .map(|row| row.host_id)
        .collect();
    let mut rows = Vec::new();
    for computer in computers {
        if computer.id == self_id {
            continue;
        }
        if !matches!(
            choose_account_route(
                &computer.id,
                computer.wan,
                &computer.public_ip,
                &computer.ssh_user,
                &nearby_refs,
            ),
            AccountRoute::PublicSsh { .. }
        ) {
            continue;
        }
        let host_id = account_host_id(&computer.id).unwrap_or_default();
        rows.push(AccountComputerRow {
            connected: connected.contains(&host_id),
            name: if computer.name.trim().is_empty() {
                "这台电脑".into()
            } else {
                computer.name
            },
            id: computer.id,
        });
    }
    Ok(AccountLinkStatus {
        signed_in: true,
        sharing,
        public_ip,
        keys_ready,
        computers: rows,
    })
}

#[tauri::command]
pub fn set_wan_share_command(enabled: bool, app: tauri::AppHandle) {
    set_wan_share(enabled);
    if enabled {
        let _ = account_ssh_public_key();
    } else {
        let _ = sync_account_authorized_keys(&[]);
    }
    crate::channel_link::refresh_share(&app);
}

#[tauri::command]
pub async fn account_link_status_command(
    state: State<'_, LinkSessionState>,
) -> Result<AccountLinkStatus, String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || account_link_status(&state))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn connect_account_computer_command(
    id: String,
    state: State<'_, LinkSessionState>,
    app: tauri::AppHandle,
) -> Result<LinkConnectReport, String> {
    let state = (*state).clone();
    let report =
        tauri::async_runtime::spawn_blocking(move || connect_account_computer(&state, &id))
            .await
            .map_err(|error| error.to_string())?;
    if report.ok {
        let _ = app.emit("workspace-changed", &report.host_id);
    }
    Ok(report)
}

fn connect_account_computer(state: &LinkSessionState, id: &str) -> LinkConnectReport {
    let host_id = account_host_id(id).unwrap_or_else(|| format!("acct-{id}"));
    let fail = |kind: &str, message: &str| LinkConnectReport {
        host_id: host_id.clone(),
        ok: false,
        connected: false,
        computer_name: String::new(),
        message: message.into(),
        kind: kind.into(),
    };
    let Ok(computers) = teamups_computers() else {
        return fail("other", "没能看到同一账号的电脑。请确认已登录，稍后再看。");
    };
    let Some(computer) = computers.into_iter().find(|computer| computer.id == id) else {
        return fail("off", "这台电脑现在没开，或还没登录同一个账号。");
    };
    let nearby: Vec<(String, String)> = list_nearby()
        .into_iter()
        .map(|peer| (peer.id, peer.account))
        .collect();
    let nearby_refs: Vec<(&str, &str)> = nearby
        .iter()
        .map(|(lan_id, account_id)| (lan_id.as_str(), account_id.as_str()))
        .collect();
    match choose_account_route(
        &computer.id,
        computer.wan,
        &computer.public_ip,
        &computer.ssh_user,
        &nearby_refs,
    ) {
        AccountRoute::SameLan { .. } => fail("lan", "这台电脑就在旁边。用上面的列表连接。"),
        AccountRoute::Unavailable => {
            fail("off", "这台电脑还没分享到外网。在那边打开「分享到外网」。")
        }
        AccountRoute::PublicSsh { host, user } => match public_ssh_entry(&host, &user) {
            Ok(entry) => state.connect_direct(&host_id, &entry),
            Err(_) => fail(
                "ssh",
                "连不上。请确认两边都登录了同一个账号，并在那台电脑打开系统的远程登录，再在路由器把外网的 22 转到那台电脑。",
            ),
        },
    }
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
