use std::path::PathBuf;

use agent_doctor_core::{
    browser_configured_runtimes, browser_mcp_wire_options_for_active_workspace,
    diagnose_and_wire_browser_mcp, install_teamups_mall_item, list_browser_mcp_targets,
    list_mcp_inventory, list_skills_inventory_with_options, list_teamups_mall_catalog,
    load_workspaces, mount_synced_skills, poll_teamups_login, resolve_agent_doctor_binary,
    sign_out_teamups, start_teamups_login, teamups_account_status, unmount_synced_skills,
    BrowserMcpDiagnoseWireReport, BrowserMcpTargetStatus, McpInventoryReport, SkillMountOptions,
    SkillMountReport, SkillsInventoryOptions, SkillsInventoryReport, SyncReport,
    TeamupsAccountStatus, TeamupsLoginPoll, TeamupsLoginStart, TeamupsMallCatalog,
};
use agent_doctor_mcp::{
    browser_mcp_status_with_probe, configure_for, discover_chrome, generate_config_snippet,
    resolve_profile_directory, resolve_user_data_dir, BrowserMcpStatus, McpConfigureOptions,
    DEFAULT_BROWSER_MCP_PORT,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize)]
pub struct McpProgressEvent {
    pub stage: String,
    pub message: String,
    pub done: bool,
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpModuleStatus {
    pub browser: BrowserMcpStatus,
    pub inventory: McpInventoryReport,
    pub configured_runtimes: Vec<String>,
    pub targets: Vec<BrowserMcpTargetStatus>,
    pub binary: String,
    pub config_snippet: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpConfigureReport {
    pub runtime: String,
    pub port: u16,
    pub config_path: String,
    pub binary: String,
}

#[tauri::command]
pub async fn list_skills_inventory_command(
    remote_stats: Option<bool>,
) -> Result<SkillsInventoryReport, String> {
    let remote_stats = remote_stats.unwrap_or(true);
    tauri::async_runtime::spawn_blocking(move || {
        list_skills_inventory_with_options(&SkillsInventoryOptions { remote_stats })
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn list_mcp_inventory_command() -> Result<McpInventoryReport, String> {
    list_mcp_inventory().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn mcp_status_command(
    port: Option<u16>,
    probe_chrome: Option<bool>,
) -> Result<McpModuleStatus, String> {
    let port = port.unwrap_or(DEFAULT_BROWSER_MCP_PORT);
    let inventory = list_mcp_inventory().map_err(|error| error.to_string())?;
    let configured_runtimes = browser_configured_runtimes(&inventory);
    let targets = list_browser_mcp_targets();
    let binary_result = resolve_agent_doctor_binary();
    let binary = binary_result
        .as_ref()
        .ok()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let probe_live = probe_chrome.unwrap_or(false);
    let browser = browser_mcp_status_with_probe(port, probe_live);
    let user_data = browser
        .user_data_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&browser.system_user_data_dir));
    let profile = browser.profile_directory.clone();
    let config_snippet = match binary_result.as_ref() {
        Ok(path) => generate_config_snippet(
            path,
            port,
            false,
            Some(user_data.as_path()),
            Some(profile.as_str()),
        ),
        Err(error) => serde_json::json!({
            "error": error.to_string(),
            "hint": "cli_unresolved",
        }),
    };
    Ok(McpModuleStatus {
        browser,
        inventory,
        configured_runtimes,
        targets,
        binary,
        config_snippet,
    })
}

#[tauri::command]
pub fn mcp_configure_command(
    app: AppHandle,
    runtime: String,
    port: Option<u16>,
    headless: Option<bool>,
    user_data_dir: Option<String>,
    profile_directory: Option<String>,
) -> Result<McpConfigureReport, String> {
    let port = port.unwrap_or(DEFAULT_BROWSER_MCP_PORT);
    // Default: show browser UI (headed). Pass headless=true to hide the window.
    let headless = headless.unwrap_or(false);
    let emit = |stage: &str, message: &str, done: bool, ok: bool| {
        let _ = app.emit(
            "mcp-progress",
            &McpProgressEvent {
                stage: stage.to_string(),
                message: message.to_string(),
                done,
                ok,
            },
        );
    };

    emit("discover", "Looking for Chrome…", false, true);
    let discovery = discover_chrome().map_err(|error| {
        emit("discover", &error.to_string(), true, false);
        error.to_string()
    })?;

    emit(
        "binary",
        &format!("Resolving agent-doctor binary for {runtime}…"),
        false,
        true,
    );
    let binary = resolve_agent_doctor_binary().map_err(|error| {
        emit("binary", &error.to_string(), true, false);
        error.to_string()
    })?;

    let workspaces = load_workspaces().unwrap_or_default();
    let active_entry = workspaces
        .active
        .as_ref()
        .and_then(|name| workspaces.workspaces.get(name));
    let project_path = active_entry.map(|entry| entry.path.clone());
    let codex_home = active_entry.map(|entry| entry.codex_home.clone());
    let explicit_dir = user_data_dir
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let resolved_dir = resolve_user_data_dir(explicit_dir.as_ref(), Some(&discovery.binary_path));
    let resolved_profile = resolve_profile_directory(profile_directory.as_deref());

    emit(
        "write",
        &format!("Writing MCP config for {runtime}…"),
        false,
        true,
    );
    let options = McpConfigureOptions {
        runtime: runtime.clone(),
        port,
        headless,
        user_data_dir: Some(resolved_dir),
        profile_directory: Some(resolved_profile),
        binary: binary.clone(),
        project_path,
        codex_home,
        hermes_home: active_entry.map(|entry| {
            std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .map(std::path::PathBuf::from)
                .unwrap_or_default()
                .join(".hermes/profiles")
                .join(&entry.hermes_profile)
        }),
        openclaw_workspace: active_entry.map(|entry| entry.openclaw_workspace.clone()),
    };
    configure_for(&discovery, &options).map_err(|error| {
        emit("write", &error.to_string(), true, false);
        error.to_string()
    })?;

    let config_path = agent_doctor_mcp::mcp_servers_path_with_openclaw(
        &runtime,
        options.project_path.as_deref(),
        options.codex_home.as_deref(),
        options.hermes_home.as_deref(),
        options.openclaw_workspace.as_deref(),
    )
    .map_err(|error| error.to_string())?;

    emit(
        "done",
        &format!("Browser MCP configured for {runtime}. Restart the runtime to apply."),
        true,
        true,
    );

    Ok(McpConfigureReport {
        runtime,
        port,
        config_path: config_path.display().to_string(),
        binary: binary.display().to_string(),
    })
}

/// Diagnose Chrome / CLI / installed agents, then write Browser MCP into each installed target.
#[tauri::command]
pub fn mcp_diagnose_wire_command(
    app: AppHandle,
    port: Option<u16>,
    headless: Option<bool>,
    user_data_dir: Option<String>,
    profile_directory: Option<String>,
) -> Result<BrowserMcpDiagnoseWireReport, String> {
    let port = port.unwrap_or(DEFAULT_BROWSER_MCP_PORT);
    let headless = headless.unwrap_or(false);
    let emit = |stage: &str, message: &str, done: bool, ok: bool| {
        let _ = app.emit(
            "mcp-progress",
            &McpProgressEvent {
                stage: stage.to_string(),
                message: message.to_string(),
                done,
                ok,
            },
        );
    };

    emit(
        "diagnose",
        "Diagnosing Chrome, CLI, and installed agents…",
        false,
        true,
    );

    let binary = resolve_agent_doctor_binary().unwrap_or_default();
    let mut options = browser_mcp_wire_options_for_active_workspace(binary);
    options.port = port;
    options.headless = headless;
    let explicit_dir = user_data_dir
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    options.user_data_dir = explicit_dir;
    options.profile_directory = profile_directory
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let report = diagnose_and_wire_browser_mcp(options);
    let ok = report.failed == 0 && report.chrome_ok && report.cli_ok;
    let summary = if report.wrote > 0 {
        format!(
            "Wrote Browser MCP into {} agent(s); skipped {}; failed {}.",
            report.wrote, report.skipped, report.failed
        )
    } else if !report.issues.is_empty() {
        report.issues[0].message.clone()
    } else {
        "No installed agents to write.".into()
    };
    emit("done", &summary, true, ok);
    Ok(report)
}

#[tauri::command]
pub fn mount_synced_skills_command(
    skill_ids: Option<Vec<String>>,
    runtimes: Option<Vec<String>>,
) -> Result<SkillMountReport, String> {
    mount_synced_skills(&SkillMountOptions {
        skill_ids: skill_ids.unwrap_or_default(),
        runtimes: runtimes.unwrap_or_default(),
        include_active_workspace: true,
    })
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn unmount_synced_skills_command(
    skill_ids: Option<Vec<String>>,
    runtimes: Option<Vec<String>>,
) -> Result<SkillMountReport, String> {
    unmount_synced_skills(&SkillMountOptions {
        skill_ids: skill_ids.unwrap_or_default(),
        runtimes: runtimes.unwrap_or_default(),
        include_active_workspace: true,
    })
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_teamups_mall_catalog_command() -> Result<TeamupsMallCatalog, String> {
    tauri::async_runtime::spawn_blocking(list_teamups_mall_catalog)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn start_teamups_login_command() -> Result<TeamupsLoginStart, String> {
    tauri::async_runtime::spawn_blocking(start_teamups_login)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn poll_teamups_login_command(device_code: String) -> Result<TeamupsLoginPoll, String> {
    tauri::async_runtime::spawn_blocking(move || poll_teamups_login(&device_code))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn teamups_account_status_command() -> Result<TeamupsAccountStatus, String> {
    tauri::async_runtime::spawn_blocking(teamups_account_status)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn sign_out_teamups_command() -> Result<TeamupsAccountStatus, String> {
    tauri::async_runtime::spawn_blocking(sign_out_teamups)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn install_teamups_mall_item_command(
    kind: String,
    id: String,
    pack_slug: Option<String>,
) -> Result<SyncReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        install_teamups_mall_item(&kind, &id, pack_slug.as_deref())
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}
