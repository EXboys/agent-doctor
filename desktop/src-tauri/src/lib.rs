use agent_doctor_core::{
    apply_profile_model, ensure_default_workspace, evotown_status, execute_evotown_onboarding,
    execute_register, execute_skills_sync, list_mcp_inventory, load_doctor_node_config,
    load_profiles, load_workspaces, open_interactive_session, run_doctor,
    run_prompt_session_with_cancel, runtime_catalog, set_runtime_model, use_profile, ApplyReport,
    DoctorReport, EvotownStatus, HermesAdapter, HermesProfilePreset, HermesSettings,
    OnboardingOptions, OnboardingReport, OpenSessionOptions, OpenSessionReport, ProfilesDocument,
    PromptSessionCancel, PromptSessionControl, PromptSessionEvent, PromptSessionOptions,
    PromptSessionReport, RegisterOptions, RegisterReport, RuntimeCatalogEntry, RuntimeModelPreset,
    SkillsSyncOptions, SyncReport, UseProfileReport,
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
mod island;
mod speech;
mod state;
mod tray;
mod windows;

pub(crate) use tray::{rebuild_tray_menu, remember_tray_health, update_tray_tooltip};

use commands::*;
use island::{
    current_island_view_command, island_claim_keyboard_command, island_hide_when_idle_command,
    island_open_session_command, island_pin_command, island_restore_command,
    island_send_text_command, island_set_content_height_command, island_set_hide_when_idle_command,
    island_set_hover_command, island_set_reading_command, publish_island_snapshot_command,
    IslandHost,
};
use state::{ActivePromptRun, PromptSessionState};
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
async fn run_doctor_command(app: tauri::AppHandle) -> Result<DoctorReport, String> {
    let report = tauri::async_runtime::spawn_blocking(run_doctor)
        .await
        .map_err(|error| error.to_string())?;
    tray::remember_tray_health(&app, &report);
    tray::update_tray_tooltip(&app);
    // The resources window scans on its own. Push that result to the home so a
    // newly installed agent shows up without another click on 扫描.
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("doctor-report", &report);
    }
    Ok(report)
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
fn runtime_catalog_command() -> Vec<RuntimeCatalogEntry> {
    runtime_catalog()
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

fn event_session_id(event: &PromptSessionEvent) -> Option<&str> {
    Some(match event {
        PromptSessionEvent::Started { session_id, .. }
        | PromptSessionEvent::Status { session_id, .. }
        | PromptSessionEvent::Delta { session_id, .. }
        | PromptSessionEvent::Thinking { session_id, .. }
        | PromptSessionEvent::StdoutLine { session_id, .. }
        | PromptSessionEvent::StderrLine { session_id, .. }
        | PromptSessionEvent::PermissionRequest { session_id, .. }
        | PromptSessionEvent::PermissionResolved { session_id, .. }
        | PromptSessionEvent::Plan { session_id, .. }
        | PromptSessionEvent::Completed { session_id, .. } => session_id.as_str(),
    })
}

fn stamp_client_run(event: PromptSessionEvent, client_run_id: &str) -> PromptSessionEvent {
    match event {
        PromptSessionEvent::Started {
            session_id,
            runtime,
            cwd,
            command,
            ..
        } => PromptSessionEvent::Started {
            session_id,
            runtime,
            cwd,
            command,
            client_run_id: client_run_id.to_string(),
        },
        other => other,
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn start_prompt_session_command(
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, PromptSessionState>,
    runtime: String,
    prompt: String,
    cwd: Option<String>,
    timeout_sec: Option<u64>,
    dangerously_skip_permissions: Option<bool>,
    full_auto: Option<bool>,
    resume_thread_id: Option<String>,
    selected_mcps: Option<Vec<String>>,
    image_paths: Option<Vec<String>>,
    workspace_name: Option<String>,
    client_run_id: Option<String>,
) -> Result<PromptSessionReport, String> {
    let owner_label = window.label().to_string();
    // Chat sessions pass their own id so several can run together. Callers
    // that omit it keep one slot for that window.
    let client_key = client_run_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| owner_label.clone());

    let cancel = PromptSessionCancel::new();
    let control = PromptSessionControl::new();
    state.try_insert(
        client_key.clone(),
        ActivePromptRun {
            cancel: cancel.clone(),
            control: control.clone(),
            owner: owner_label.clone(),
        },
    )?;

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
        image_paths: image_paths
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect(),
        workspace_name: workspace_name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    };

    // The window's Allow button writes through this control. DeepSeek always
    // needs it: auto-approve can be turned on after the reply has started.
    let control_for_run = {
        let runtime = options.runtime.as_str();
        let claude_ask = runtime == "claude-code" && !options.dangerously_skip_permissions;
        let codex_ask = runtime == "codex" && !options.full_auto;
        let deepseek_ask = runtime == "deepseek-harness";
        if claude_ask || codex_ask || deepseek_ask {
            Some(control)
        } else {
            None
        }
    };

    let app_for_emit = app.clone();
    let emit_target = owner_label.clone();
    let client_for_events = client_key.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        run_prompt_session_with_cancel(
            &options,
            cancel,
            control_for_run,
            |event: PromptSessionEvent| {
                let event = stamp_client_run(event, &client_for_events);
                if let Some(backend_id) = event_session_id(&event) {
                    if let Some(sessions) = app_for_emit.try_state::<PromptSessionState>() {
                        sessions.note_backend(backend_id, &client_for_events);
                    }
                }
                // Only the window that started this run: Ask and Diagnose share the
                // event name, and a broadcast made each one render the other's reply.
                let _ = app_for_emit.emit_to(emit_target.as_str(), "prompt-session-event", &event);
            },
        )
    })
    .await;

    state.release(&client_key);

    let report = result
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))?;
    Ok(report)
}

#[tauri::command]
fn cancel_prompt_session_command(
    window: tauri::Window,
    state: State<'_, PromptSessionState>,
    client_run_id: Option<String>,
) -> Result<bool, String> {
    state.cancel_owned(window.label(), client_run_id)
}

#[tauri::command]
fn resolve_permission_session_command(
    app: AppHandle,
    state: State<'_, PromptSessionState>,
    session_id: String,
    request_id: String,
    allow: bool,
    text: Option<String>,
) -> Result<bool, String> {
    let (control, owner) = state.control_for_backend(&session_id)?;
    let sent_text = text.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(text) = sent_text {
        control
            .respond_line(&request_id, text)
            .map_err(|e| format!("{e:#}"))?;
    } else {
        control
            .respond_permission(&request_id, allow)
            .map_err(|e| format!("{e:#}"))?;
    }
    let allowed = sent_text.is_some() || allow;
    let _ = app.emit_to(
        owner.as_str(),
        "prompt-session-event",
        &PromptSessionEvent::PermissionResolved {
            session_id,
            request_id,
            allowed,
        },
    );
    Ok(true)
}

pub fn run() {
    agent_doctor_mcp::register_browser_mcp_wire_backend();
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
            agent_doctor_core::enable_warm_sessions();
            app.manage(DeepDiagnoseState::default());
            app.manage(IslandHost::default());
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
            // Windows: creating Resources or Diagnose on the first click can hang
            // WebView2, so those buttons look dead. Build the hidden windows now.
            // macOS still waits for the click; those pages scan folders and would
            // raise the Files-and-Folders prompt during startup.
            #[cfg(target_os = "windows")]
            {
                let _ = windows::ensure_resources_window(app.handle(), "skills", true);
                let _ = windows::ensure_diagnose_window(app.handle(), "openclaw", true);
            }
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
            remove_workspace_command,
            workspace_status_command,
            workspace_doctor_command,
            workspace_fix_command,
            list_workspace_dir_command,
            read_workspace_file_command,
            write_workspace_file_command,
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
            deep_diagnose_chat_command,
            cancel_deep_diagnose_command,
            deep_repair_command,
            run_browser_smoke_command,
            run_repair_rollback_command,
            install_runtime_command,
            submit_install_input_command,
            uninstall_runtime_command,
            open_path_command,
            runtime_catalog_command,
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
            voice_turn_end_command,
            read_image_texts_command,
            ask_image_support_command,
            publish_island_snapshot_command,
            island_set_hover_command,
            island_restore_command,
            island_pin_command,
            island_open_session_command,
            island_send_text_command,
            island_claim_keyboard_command,
            island_set_reading_command,
            island_set_content_height_command,
            island_set_hide_when_idle_command,
            island_hide_when_idle_command,
            current_island_view_command
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                agent_doctor_core::shutdown_warm_sessions();
            }
        });
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
    use std::fs;
    use std::path::Path;

    use crate::tray::format_tray_tooltip;

    fn command_names(text: &str) -> Vec<String> {
        let mut names = Vec::new();
        let bytes = text.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == b'"' {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end] != b'"' {
                    end += 1;
                }
                let token = &text[start..end];
                if token.ends_with("_command")
                    && token.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                {
                    names.push(token.to_string());
                }
                i = end + 1;
                continue;
            }
            i += 1;
        }
        names.sort();
        names.dedup();
        names
    }

    fn handler_idents(lib_rs: &str) -> Vec<String> {
        let start = lib_rs
            .find("tauri::generate_handler!")
            .expect("generate_handler");
        let rest = &lib_rs[start..];
        let end = rest.find("])").expect("handler list end");
        let mut names: Vec<String> = rest[..end]
            .split(|c: char| !c.is_ascii_lowercase() && c != '_')
            .filter(|token| token.ends_with("_command"))
            .map(str::to_string)
            .collect();
        names.sort();
        names.dedup();
        names
    }

    #[test]
    fn ipc_contract_matches_registered_commands() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let lib_rs = fs::read_to_string(manifest.join("src/lib.rs")).expect("lib.rs");
        let frontend = manifest.join("../src");
        let ipc = fs::read_to_string(frontend.join("ipc.ts")).expect("ipc.ts");
        let registered = handler_idents(&lib_rs);
        let declared = command_names(&ipc);
        assert_eq!(
            declared, registered,
            "desktop/src/ipc.ts must name every Tauri command and no others"
        );

        let mut stray = Vec::new();
        visit_ts(&frontend, &mut stray);
        assert!(
            stray.is_empty(),
            "command names belong in ipc.ts, found {stray:?}"
        );
    }

    fn visit_ts(dir: &Path, stray: &mut Vec<String>) {
        let entries = fs::read_dir(dir).expect("read frontend");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit_ts(&path, stray);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("ts") {
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) == Some("ipc.ts") {
                continue;
            }
            let text = fs::read_to_string(&path).expect("read ts");
            let found = command_names(&text);
            if !found.is_empty() {
                stray.push(format!("{}: {}", path.display(), found.join(", ")));
            }
        }
    }

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
