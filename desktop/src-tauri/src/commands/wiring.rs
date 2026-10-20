use agent_doctor_core::{
    activate_personal_provider, browser_mcp_wire_options_for_active_workspace,
    delete_personal_provider, execute_personal_provider_setup, list_personal_providers,
    load_mode_status, load_personal_provider_status, native_account_status, refresh_account_models,
    resign_native_subscription, resolve_agent_doctor_binary, switch_to_personal_mode,
    switch_to_team_mode, upsert_personal_provider, use_current_service, use_native_subscription,
    verify_personal_provider_with_protocol, wire_browser_mcp_installed, ModeStatus,
    ModeSwitchReport, NativeAccountStatus, NativeAccountSwitch, PersonalProviderOptions,
    PersonalProviderSetupReport, PersonalProviderStatus, PersonalProviderVerifyReport,
    PersonalProvidersDocument, UpsertPersonalProviderOptions,
};
use agent_doctor_mcp::BrowserMcpWireReport;
use serde::{Deserialize, Serialize};

use crate::update_tray_tooltip;
use tauri::Emitter;

fn notify_provider_changed(app: &tauri::AppHandle) {
    agent_doctor_prompt::retire_warm_sessions();
    let _ = app.emit("personal-provider-changed", ());
}

#[derive(Debug, Clone, Serialize)]
pub struct ModeSwitchDesktopReport {
    #[serde(flatten)]
    pub switch: ModeSwitchReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_mcp: Option<BrowserMcpWireReport>,
}

fn wire_browser_mcp_for_desktop() -> Result<BrowserMcpWireReport, String> {
    let binary = resolve_agent_doctor_binary().map_err(|error| error.to_string())?;
    let options = browser_mcp_wire_options_for_active_workspace(binary);
    wire_browser_mcp_installed(&options)
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

#[derive(Debug, Deserialize)]
pub struct UpsertPersonalProviderArgs {
    id: Option<String>,
    name: String,
    url: String,
    key: String,
    model: String,
    protocol: String,
    activate: bool,
}

#[tauri::command]
pub fn upsert_personal_provider_command(
    app: tauri::AppHandle,
    args: UpsertPersonalProviderArgs,
) -> Result<PersonalProvidersDocument, String> {
    let doc = upsert_personal_provider(&UpsertPersonalProviderOptions {
        id: args.id,
        name: args.name,
        url: args.url,
        api_key: args.key,
        model: args.model,
        protocol: args.protocol,
        activate: args.activate,
    })
    .map_err(|error| error.to_string())?;
    notify_provider_changed(&app);
    Ok(doc)
}

#[tauri::command]
pub fn delete_personal_provider_command(id: String) -> Result<PersonalProvidersDocument, String> {
    delete_personal_provider(&id).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn activate_personal_provider_command(
    app: tauri::AppHandle,
    id: String,
) -> Result<PersonalProviderSetupReport, String> {
    let report = tauri::async_runtime::spawn_blocking(move || {
        activate_personal_provider(&id).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    update_tray_tooltip(&app);
    notify_provider_changed(&app);
    Ok(report)
}

#[tauri::command]
pub async fn token_usage_by_day_command(
    since_ts: i64,
    tz_offset_sec: i64,
) -> Result<Vec<agent_doctor_core::usage::UsageDayRow>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        agent_doctor_core::usage::usage_by_day(since_ts, tz_offset_sec)
            .map_err(|error| format!("{error:#}"))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn verify_personal_provider_command(
    url: String,
    key: String,
    protocol: String,
) -> Result<PersonalProviderVerifyReport, String> {
    // A plain command runs on the window's thread: the whole app froze while
    // the provider (and its other region hosts) took their time to answer.
    tauri::async_runtime::spawn_blocking(move || {
        verify_personal_provider_with_protocol(&url, &key, &protocol)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn apply_personal_provider_command(
    app: tauri::AppHandle,
    url: String,
    key: String,
    model: String,
    protocol: String,
) -> Result<PersonalProviderSetupReport, String> {
    let report = tauri::async_runtime::spawn_blocking(move || {
        execute_personal_provider_setup(&PersonalProviderOptions {
            url,
            api_key: key,
            model,
            protocol,
        })
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    update_tray_tooltip(&app);
    notify_provider_changed(&app);
    Ok(report)
}

#[tauri::command]
pub fn get_mode_status_command() -> ModeStatus {
    load_mode_status().unwrap_or(ModeStatus {
        mode: "unset".to_string(),
        edition: agent_doctor_core::product_edition().as_str().to_string(),
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
pub fn get_product_edition_command() -> String {
    agent_doctor_core::product_edition().as_str().to_string()
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
    notify_provider_changed(&app);
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
    notify_provider_changed(&app);
    Ok(report)
}

#[tauri::command]
pub async fn wire_browser_mcp_command() -> Result<BrowserMcpWireReport, String> {
    tauri::async_runtime::spawn_blocking(wire_browser_mcp_for_desktop)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn rewire_current_mode_command(
    app: tauri::AppHandle,
    with_browser_mcp: Option<bool>,
) -> Result<ModeSwitchDesktopReport, String> {
    let edition = agent_doctor_core::product_edition();
    let status = load_mode_status().map_err(|error| error.to_string())?;
    match (edition, status.mode.as_str()) {
        (agent_doctor_core::ProductEdition::Personal, "personal") => {
            switch_to_personal_mode_command(app, status.personal_active_id, with_browser_mcp).await
        }
        (agent_doctor_core::ProductEdition::Team, "team") => {
            switch_to_team_mode_command(app, with_browser_mcp).await
        }
        (agent_doctor_core::ProductEdition::Personal, _) => {
            Err("Personal edition: configure a personal provider first, then rewire.".into())
        }
        (agent_doctor_core::ProductEdition::Team, _) => {
            Err("Team edition: connect Evotown first, then rewire.".into())
        }
    }
}

#[tauri::command]
pub async fn native_account_status_command(probe: bool) -> NativeAccountStatus {
    tauri::async_runtime::spawn_blocking(move || native_account_status(probe))
        .await
        .unwrap_or(NativeAccountStatus { rows: Vec::new() })
}

#[tauri::command]
pub async fn refresh_subscription_models_command(
    runtime_id: String,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        refresh_account_models(&runtime_id)
            .map(|list| list.models)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn use_native_subscription_command(
    app: tauri::AppHandle,
    runtime_id: String,
) -> Result<NativeAccountSwitch, String> {
    let switched = tauri::async_runtime::spawn_blocking(move || {
        use_native_subscription(&runtime_id).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    if switched.outcome == "using" {
        notify_provider_changed(&app);
    }
    Ok(switched)
}

#[tauri::command]
pub async fn resign_native_subscription_command(
    app: tauri::AppHandle,
    runtime_id: String,
) -> Result<NativeAccountSwitch, String> {
    let switched = tauri::async_runtime::spawn_blocking(move || {
        resign_native_subscription(&runtime_id).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    // The parked chat still holds the expired account. The next send starts clean.
    agent_doctor_prompt::retire_warm_sessions();
    if switched.outcome == "using" {
        notify_provider_changed(&app);
    }
    Ok(switched)
}

#[tauri::command]
pub async fn use_current_service_command(
    runtime_id: String,
) -> Result<NativeAccountSwitch, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use_current_service(&runtime_id).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}
