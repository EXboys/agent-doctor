use agent_doctor_core::{
    apply_profile_model, ensure_default_workspace, evotown_status, execute_evotown_onboarding,
    execute_register, execute_skills_sync, list_mcp_inventory, load_doctor_node_config,
    load_profiles, load_workspaces, open_interactive_session, run_doctor,
    run_prompt_session_with_cancel, set_runtime_model, use_profile, ApplyReport, DoctorReport,
    EvotownStatus, HermesAdapter, HermesProfilePreset, HermesSettings, OnboardingOptions,
    OnboardingReport, OpenSessionOptions, OpenSessionReport, ProfilesDocument, PromptSessionCancel,
    PromptSessionControl, PromptSessionEvent, PromptSessionOptions, PromptSessionReport,
    RegisterOptions, RegisterReport, RuntimeModelPreset, SkillsSyncOptions, SyncReport,
    UseProfileReport,
};

use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

// Rebuild this crate (and re-embed Info.plist via generate_context!) whenever
// desktop/src-tauri/Info.plist changes. Without this, `tauri:dev` can keep a
// stale binary that lacks NSSpeechRecognitionUsageDescription and TCC-aborts.
#[cfg(target_os = "macos")]
const _: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/info_plist.stamp"));

mod commands;
mod speech;
mod state;
mod tray;
mod windows;

pub(crate) use tray::{rebuild_tray_menu, remember_tray_health, update_tray_tooltip};

use commands::*;
use state::PromptSessionState;
use windows::{
    close_ask_window_command, close_diagnose_window_command, close_resources_window_command,
    focus_main_tab_command, open_ask_window_command, open_diagnose_window_command,
    open_resources_window_command, resize_main_window_command,
};

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
    execute_skills_sync(&SkillsSyncOptions {
        dry_run: false,
        only_skills: Vec::new(),
        runtime_target: None,
        pack_or_bundle_id: None,
        source_override: None,
    })
    .map_err(|error| error.to_string())
}

#[derive(Debug, Clone, serde::Serialize)]
struct EngineRegisterStatus {
    registered: bool,
    engine_id: Option<String>,
    env_path: Option<String>,
}

#[tauri::command]
fn get_engine_register_status_command() -> EngineRegisterStatus {
    let env_path = agent_doctor_core::evotown_agent_env_path().map(|p| p.display().to_string());
    match load_doctor_node_config() {
        Ok(config) => EngineRegisterStatus {
            registered: true,
            engine_id: Some(config.engine_id),
            env_path: Some(config.config_source),
        },
        Err(_) => EngineRegisterStatus {
            registered: false,
            engine_id: None,
            env_path,
        },
    }
}

#[tauri::command]
fn run_engine_register_command(
    bootstrap_token: String,
    engine_id: Option<String>,
    rotate: bool,
) -> Result<RegisterReport, String> {
    let token = bootstrap_token.trim();
    if token.is_empty() {
        return Err("bootstrap token is required".into());
    }
    execute_register(&RegisterOptions {
        bootstrap_token: Some(token.to_string()),
        engine_id: engine_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        engine_type: None,
        runtime: None,
        display_name: None,
        owner_team: None,
        deployment_kind: None,
        engine_version: None,
        rotate,
        save_token: true,
    })
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn run_doctor_command(app: tauri::AppHandle) -> DoctorReport {
    let report = run_doctor();
    tray::remember_tray_health(&app, &report);
    tray::update_tray_tooltip(&app);
    report
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
fn open_path_command(path: String, app: tauri::AppHandle) -> Result<(), String> {
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn open_session_command(
    runtime: String,
    cwd: Option<String>,
    prompt: Option<String>,
    terminal: Option<bool>,
) -> Result<OpenSessionReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        open_interactive_session(&OpenSessionOptions {
            runtime,
            cwd: cwd.map(std::path::PathBuf::from),
            prompt,
            prefer_deep_link: !terminal.unwrap_or(false),
        })
        .map_err(|err| format!("{err:#}"))
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn start_prompt_session_command(
    app: AppHandle,
    state: State<'_, PromptSessionState>,
    runtime: String,
    prompt: String,
    cwd: Option<String>,
    timeout_sec: Option<u64>,
    dangerously_skip_permissions: Option<bool>,
    full_auto: Option<bool>,
    resume_thread_id: Option<String>,
    selected_mcps: Option<Vec<String>>,
) -> Result<PromptSessionReport, String> {
    {
        let guard = state.cancel.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Err("another ask session is already running".into());
        }
    }

    let cancel = PromptSessionCancel::new();
    let control = PromptSessionControl::new();
    {
        let mut guard = state.cancel.lock().map_err(|e| e.to_string())?;
        *guard = Some(cancel.clone());
    }
    {
        let mut guard = state.control.lock().map_err(|e| e.to_string())?;
        *guard = Some(control.clone());
    }

    let options = PromptSessionOptions {
        runtime,
        prompt,
        cwd: cwd.map(PathBuf::from),
        timeout_sec: timeout_sec.unwrap_or(600),
        dangerously_skip_permissions: dangerously_skip_permissions.unwrap_or(false),
        full_auto: full_auto.unwrap_or(false),
        resume_thread_id: resume_thread_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        selected_mcps: selected_mcps
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    };

    // Interactive Allow/Deny when Claude skip is off, or Codex full-auto is off.
    let control_for_run = {
        let runtime = options.runtime.as_str();
        let claude_ask = runtime == "claude-code" && !options.dangerously_skip_permissions;
        let codex_ask = runtime == "codex" && !options.full_auto;
        if claude_ask || codex_ask {
            Some(control)
        } else {
            None
        }
    };

    let app_for_emit = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        run_prompt_session_with_cancel(
            &options,
            cancel,
            control_for_run,
            |event: PromptSessionEvent| {
                // Emit once. `AppHandle::emit` already broadcasts to every webview;
                // also targeting the ask window duplicated every chat event.
                let _ = app_for_emit.emit("prompt-session-event", &event);
            },
        )
    })
    .await;

    if let Ok(mut guard) = state.cancel.lock() {
        *guard = None;
    }
    if let Ok(mut guard) = state.control.lock() {
        *guard = None;
    }

    let report = result
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))?;
    Ok(report)
}

#[tauri::command]
fn cancel_prompt_session_command(state: State<'_, PromptSessionState>) -> Result<bool, String> {
    let guard = state.cancel.lock().map_err(|e| e.to_string())?;
    if let Some(cancel) = guard.as_ref() {
        cancel.request();
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
fn resolve_permission_session_command(
    app: AppHandle,
    state: State<'_, PromptSessionState>,
    session_id: String,
    request_id: String,
    allow: bool,
) -> Result<bool, String> {
    let guard = state.control.lock().map_err(|e| e.to_string())?;
    let Some(control) = guard.as_ref() else {
        return Err("no active ask session for permission reply".into());
    };
    control
        .respond_permission(&request_id, allow)
        .map_err(|e| format!("{e:#}"))?;
    let _ = app.emit(
        "prompt-session-event",
        &PromptSessionEvent::PermissionResolved {
            session_id,
            request_id,
            allowed: allow,
        },
    );
    Ok(true)
}

pub fn run() {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // Second launch (Start Menu / desktop shortcut) while minimized or
            // tray-hidden: bring the existing main window back instead of
            // starting a stuck second WebView2 process.
            windows::show_main_window(app);
        }));

    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }

    builder
        .setup(|app| {
            app.manage(Mutex::new(tray::TrayCompactState::default()));
            app.manage(PromptSessionState::default());
            // Paint the main window first. Seeding a workspace can hit macOS
            // Files-and-Folders prompts (Documents / Desktop) and must not
            // block the first frame on a blank chrome.
            if let Some(window) = app.get_webview_window("main") {
                windows::attach_main_window_close_behavior(&window);
            }
            windows::show_main_window(app.handle());
            tray::setup_tray(app);
            seed_default_workspace_in_background();
            // Pre-create Ask on the UI thread at startup. Creating it on first
            // click can hang WebView2 on Windows (blank titled window, Close
            // and Task Manager "End task" appear to do nothing).
            let _ = windows::ensure_ask_window(app.handle(), "claude-code");
            // Do not pre-create Resources / Diagnose: their pages scan Chrome and
            // the project folder and would re-raise the same macOS Files prompt.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_evotown_status_command,
            run_evotown_onboarding_command,
            run_sync_command,
            get_engine_register_status_command,
            run_engine_register_command,
            list_skills_inventory_command,
            skill_mount_runtime_ids_command,
            list_teamups_mall_catalog_command,
            start_teamups_login_command,
            poll_teamups_login_command,
            teamups_account_status_command,
            sign_out_teamups_command,
            install_teamups_mall_item_command,
            list_mcp_inventory_command,
            mcp_status_command,
            mcp_configure_command,
            mcp_diagnose_wire_command,
            mount_synced_skills_command,
            unmount_synced_skills_command,
            get_personal_provider_status_command,
            list_personal_providers_command,
            upsert_personal_provider_command,
            delete_personal_provider_command,
            activate_personal_provider_command,
            verify_personal_provider_command,
            apply_personal_provider_command,
            get_mode_status_command,
            get_product_edition_command,
            switch_to_personal_mode_command,
            switch_to_team_mode_command,
            wire_browser_mcp_command,
            rewire_current_mode_command,
            run_doctor_command,
            check_runtime_versions_command,
            list_profiles_command,
            list_workspaces_command,
            init_workspace_command,
            use_workspace_command,
            workspace_status_command,
            workspace_doctor_command,
            workspace_fix_command,
            list_remote_hosts_command,
            list_remote_host_rows_command,
            list_remote_projects_command,
            bootstrap_remote_host_command,
            add_remote_host_command,
            add_remote_project_command,
            remove_remote_host_command,
            remove_remote_project_command,
            probe_remote_host_command,
            run_remote_doctor_command,
            use_profile_command,
            get_hermes_model_command,
            set_hermes_model_command,
            apply_profile_model_command,
            run_repair_preview_command,
            run_repair_execute_command,
            run_browser_smoke_command,
            run_repair_rollback_command,
            install_runtime_command,
            uninstall_runtime_command,
            open_path_command,
            open_session_command,
            open_ask_window_command,
            close_ask_window_command,
            open_resources_window_command,
            close_resources_window_command,
            open_diagnose_window_command,
            close_diagnose_window_command,
            focus_main_tab_command,
            resize_main_window_command,
            start_prompt_session_command,
            cancel_prompt_session_command,
            resolve_permission_session_command,
            speech_capability_command,
            speech_dictate_command,
            speech_cancel_dictation_command,
            voice_listen_start_command,
            voice_listen_stop_command,
            voice_speak_command,
            voice_speak_stop_command,
            voice_hosted_reduce_command,
            read_image_texts_command
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn seed_default_workspace_in_background() {
    let _ = std::thread::Builder::new()
        .name("ad-seed-workspace".into())
        .spawn(|| {
            let _ = ensure_default_workspace();
            if let Ok(doc) = load_workspaces() {
                if let Some(active) = doc.active.as_ref() {
                    if let Some(entry) = doc.workspaces.get(active) {
                        let _ = agent_doctor_core::workspace::backends::bind_codex_for_project(
                            &entry.codex_home,
                            Some(&entry.path),
                        );
                    }
                }
            }
            // Warm inventory in this thread so later “Resources” clicks reuse it
            // instead of statting Documents / Chrome again.
            let _ = list_mcp_inventory();
        });
}

#[cfg(test)]
mod tests {
    use crate::tray::format_tray_tooltip;

    #[test]
    fn tooltip_shows_busy_over_status() {
        assert_eq!(
            format_tray_tooltip(Some((2, 4)), Some("demo"), "personal", Some("Doctor…")),
            "Agent Doctor · Busy · Doctor…"
        );
    }

    #[test]
    fn tooltip_compact_status_ok_partial_attention() {
        assert_eq!(
            format_tray_tooltip(Some((4, 4)), Some("demo"), "team", None),
            "Agent Doctor · OK 4/4 · ws:demo · team"
        );
        assert_eq!(
            format_tray_tooltip(Some((1, 4)), None, "personal", None),
            "Agent Doctor · Partial 1/4 · ws:— · personal"
        );
        assert_eq!(
            format_tray_tooltip(Some((0, 4)), Some("x"), "unset", None),
            "Agent Doctor · Attention · ws:x · unset"
        );
        assert_eq!(
            format_tray_tooltip(None, None, "personal", None),
            "Agent Doctor · Health — · ws:— · personal"
        );
    }
}
