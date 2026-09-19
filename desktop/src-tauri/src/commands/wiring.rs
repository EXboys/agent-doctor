use agent_doctor_core::{
    activate_personal_provider, delete_personal_provider, execute_personal_provider_setup,
    list_personal_providers, load_mode_status, load_personal_provider_status, load_workspaces,
    resolve_agent_doctor_binary, switch_to_personal_mode, switch_to_team_mode,
    upsert_personal_provider, verify_personal_provider_with_protocol, ModeStatus, ModeSwitchReport,
    PersonalProviderOptions, PersonalProviderSetupReport, PersonalProviderStatus,
    PersonalProviderVerifyReport, PersonalProvidersDocument, UpsertPersonalProviderOptions,
};
use agent_doctor_mcp::{
    discover_chrome, wire_browser_mcp, BrowserMcpWireReport, WireBrowserMcpOptions,
};
use serde::Serialize;

use crate::update_tray_tooltip;

#[derive(Debug, Clone, Serialize)]
pub struct ModeSwitchDesktopReport {
    #[serde(flatten)]
    pub switch: ModeSwitchReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_mcp: Option<BrowserMcpWireReport>,
}

fn wire_browser_mcp_for_desktop() -> Result<BrowserMcpWireReport, String> {
    let discovery = discover_chrome().map_err(|error| error.to_string())?;
    let binary = resolve_agent_doctor_binary().map_err(|error| error.to_string())?;
    let workspaces = load_workspaces().unwrap_or_default();
    let active_entry = workspaces
        .active
        .as_ref()
        .and_then(|name| workspaces.workspaces.get(name));
    let mut options = WireBrowserMcpOptions::with_binary(binary);
    options.project_path = active_entry.map(|entry| entry.path.clone());
    options.codex_home = active_entry.map(|entry| entry.codex_home.clone());
    options.hermes_home = active_entry.map(|entry| {
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(std::path::PathBuf::from)
            .unwrap_or_default()
            .join(".hermes/profiles")
            .join(&entry.hermes_profile)
    });
    options.openclaw_workspace = active_entry.map(|entry| entry.openclaw_workspace.clone());
    Ok(wire_browser_mcp(&discovery, &options))
}

#[tauri::command]
pub fn get_personal_provider_status_command() -> PersonalProviderStatus {
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
pub fn list_personal_providers_command() -> PersonalProvidersDocument {
    list_personal_providers().unwrap_or(PersonalProvidersDocument {
        active_id: None,
        providers: Vec::new(),
        store_path: String::new(),
    })
}

#[tauri::command]
pub fn upsert_personal_provider_command(
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
pub fn delete_personal_provider_command(id: String) -> Result<PersonalProvidersDocument, String> {
    delete_personal_provider(&id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn activate_personal_provider_command(
    id: String,
) -> Result<PersonalProviderSetupReport, String> {
    activate_personal_provider(&id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn verify_personal_provider_command(
    url: String,
    key: String,
    protocol: String,
) -> Result<PersonalProviderVerifyReport, String> {
    verify_personal_provider_with_protocol(&url, &key, &protocol).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn apply_personal_provider_command(
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
pub fn get_mode_status_command() -> ModeStatus {
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
pub async fn switch_to_personal_mode_command(
    app: tauri::AppHandle,
    provider_id: Option<String>,
    with_browser_mcp: Option<bool>,
) -> Result<ModeSwitchDesktopReport, String> {
    let report = tauri::async_runtime::spawn_blocking(move || {
        let switch =
            switch_to_personal_mode(provider_id.as_deref()).map_err(|error| error.to_string())?;
        let browser_mcp = if with_browser_mcp.unwrap_or(false) {
            Some(wire_browser_mcp_for_desktop()?)
        } else {
            None
        };
        Ok::<_, String>(ModeSwitchDesktopReport {
            switch,
            browser_mcp,
        })
    })
    .await
    .map_err(|error| error.to_string())??;
    update_tray_tooltip(&app);
    Ok(report)
}

#[tauri::command]
pub async fn switch_to_team_mode_command(
    app: tauri::AppHandle,
    with_browser_mcp: Option<bool>,
) -> Result<ModeSwitchDesktopReport, String> {
    let report = tauri::async_runtime::spawn_blocking(move || {
        let switch = switch_to_team_mode().map_err(|error| error.to_string())?;
        let browser_mcp = if with_browser_mcp.unwrap_or(false) {
            Some(wire_browser_mcp_for_desktop()?)
        } else {
            None
        };
        Ok::<_, String>(ModeSwitchDesktopReport {
            switch,
            browser_mcp,
        })
    })
    .await
    .map_err(|error| error.to_string())??;
    update_tray_tooltip(&app);
    Ok(report)
}

#[tauri::command]
pub fn wire_browser_mcp_command() -> Result<BrowserMcpWireReport, String> {
    wire_browser_mcp_for_desktop()
}

#[tauri::command]
pub async fn rewire_current_mode_command(
    app: tauri::AppHandle,
    with_browser_mcp: Option<bool>,
) -> Result<ModeSwitchDesktopReport, String> {
    let status = load_mode_status().map_err(|error| error.to_string())?;
    match status.mode.as_str() {
        "personal" => {
            switch_to_personal_mode_command(app, status.personal_active_id, with_browser_mcp).await
        }
        "team" => switch_to_team_mode_command(app, with_browser_mcp).await,
        _ => Err(
            "No active mode yet. Configure a Personal Provider or connect Evotown, then switch mode."
                .into(),
        ),
    }
}
