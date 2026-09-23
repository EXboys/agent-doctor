use agent_doctor_core::{
    load_mode_status, load_workspaces, run_doctor, use_workspace_with_options, workspace_doctor,
    DoctorReport, UseWorkspaceOptions, WorkspaceDoctorReport,
};
use std::sync::Mutex;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::{Emitter, Manager};

use crate::windows::show_main_window;

#[derive(Debug, Default)]
pub(crate) struct TrayCompactState {
    /// Last doctor installed/total. `None` until the first doctor run.
    health: Option<(usize, usize)>,
    /// Brief busy label while a tray action runs (e.g. `Doctor…`).
    busy: Option<String>,
}

pub(crate) fn format_tray_tooltip(
    health: Option<(usize, usize)>,
    workspace: Option<&str>,
    mode: &str,
    busy: Option<&str>,
) -> String {
    if let Some(action) = busy {
        return format!("Agent Doctor · Busy · {action}");
    }

    let health_label = match health {
        None => "Health —".to_string(),
        Some((installed, total)) if total == 0 || installed == 0 => "Attention".to_string(),
        Some((installed, total)) if installed == total => format!("OK {installed}/{total}"),
        Some((installed, total)) => format!("Partial {installed}/{total}"),
    };
    let ws = workspace.unwrap_or("—");
    let mode_label = match mode {
        "personal" => "personal",
        "team" => "team",
        _ => "unset",
    };
    format!("Agent Doctor · {health_label} · ws:{ws} · {mode_label}")
}

fn tray_mode_label() -> String {
    load_mode_status()
        .map(|status| status.mode)
        .unwrap_or_else(|_| "unset".to_string())
}

fn with_tray_state<R>(
    app: &tauri::AppHandle,
    f: impl FnOnce(&mut TrayCompactState) -> R,
) -> Option<R> {
    let state = app.try_state::<Mutex<TrayCompactState>>()?;
    let mut guard = state.lock().ok()?;
    Some(f(&mut guard))
}

pub(crate) fn remember_tray_health(app: &tauri::AppHandle, report: &DoctorReport) {
    let installed = report
        .runtimes
        .iter()
        .filter(|runtime| runtime.installed)
        .count();
    let total = report.runtimes.len();
    let _ = with_tray_state(app, |state| {
        state.health = Some((installed, total));
    });
}

fn set_tray_busy(app: &tauri::AppHandle, action: Option<&str>) {
    let _ = with_tray_state(app, |state| {
        state.busy = action.map(str::to_string);
    });
    update_tray_tooltip(app);
}

pub(crate) fn update_tray_tooltip(app: &tauri::AppHandle) {
    let doc = load_workspaces().unwrap_or_default();
    let (health, busy) =
        with_tray_state(app, |state| (state.health, state.busy.clone())).unwrap_or((None, None));
    let label = format_tray_tooltip(
        health,
        doc.active.as_deref(),
        &tray_mode_label(),
        busy.as_deref(),
    );
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(&label));
    }
}

fn publish_doctor_report(app: &tauri::AppHandle, report: &DoctorReport) {
    show_main_window(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("doctor-report", report);
    }
}

fn build_tray_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{IsMenuItem, Menu, MenuItem, Submenu};

    let doc = load_workspaces().unwrap_or_default();
    let show = MenuItem::with_id(app, "show", "Show Agent Doctor", true, None::<&str>)?;
    let ws_doctor = MenuItem::with_id(
        app,
        "workspace_doctor",
        "Workspace check",
        true,
        None::<&str>,
    )?;
    let doctor = MenuItem::with_id(app, "doctor", "Run doctor", true, None::<&str>)?;
    let check_update =
        MenuItem::with_id(app, "check_update", "Check for updates", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let mut switch_items: Vec<MenuItem<tauri::Wry>> = Vec::new();
    for name in doc.workspaces.keys() {
        let label = if doc.active.as_deref() == Some(name.as_str()) {
            format!("✓ {name}")
        } else {
            name.clone()
        };
        switch_items.push(MenuItem::with_id(
            app,
            format!("workspace:{name}"),
            &label,
            true,
            None::<&str>,
        )?);
    }

    let none_item = MenuItem::with_id(
        app,
        "workspace:none",
        "(no workspaces)",
        false,
        None::<&str>,
    )?;
    let switch_refs: Vec<&dyn IsMenuItem<tauri::Wry>> = if switch_items.is_empty() {
        vec![&none_item as &dyn IsMenuItem<tauri::Wry>]
    } else {
        switch_items
            .iter()
            .map(|item| item as &dyn IsMenuItem<tauri::Wry>)
            .collect()
    };
    let switch_sub = Submenu::with_id(app, "switch", "Switch workspace", true)?;
    if switch_items.is_empty() {
        switch_sub.append(&none_item)?;
    } else {
        switch_sub.append_items(&switch_refs)?;
    }
    Menu::with_items(
        app,
        &[
            &show,
            &switch_sub,
            &ws_doctor,
            &doctor,
            &check_update,
            &quit,
        ],
    )
}

pub(crate) fn rebuild_tray_menu(app: &tauri::AppHandle) {
    if let Ok(menu) = build_tray_menu(app) {
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn switch_workspace_from_tray(app: &tauri::AppHandle, name: &str) {
    set_tray_busy(app, Some("Switching…"));
    let ok = use_workspace_with_options(
        name,
        &UseWorkspaceOptions {
            backup: true,
            restart_gateways: false,
        },
    )
    .is_ok();
    set_tray_busy(app, None);
    if ok {
        rebuild_tray_menu(app);
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.emit("workspace-changed", name);
        }
    }
}

fn publish_workspace_doctor_report(app: &tauri::AppHandle, report: &WorkspaceDoctorReport) {
    show_main_window(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("workspace-doctor-report", report);
    }
}

pub(crate) fn setup_tray(app: &tauri::App) {
    use tauri::tray::TrayIconBuilder;

    let Ok(menu) = build_tray_menu(app.handle()) else {
        return;
    };

    let tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Agent Doctor")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "workspace_doctor" => {
                set_tray_busy(app, Some("Workspace check…"));
                let result = workspace_doctor();
                set_tray_busy(app, None);
                if let Ok(report) = result {
                    publish_workspace_doctor_report(app, &report);
                }
            }
            "doctor" => {
                set_tray_busy(app, Some("Doctor…"));
                let report = run_doctor();
                remember_tray_health(app, &report);
                set_tray_busy(app, None);
                publish_doctor_report(app, &report);
            }
            "check_update" => {
                show_main_window(app);
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("check-for-updates", ());
                }
            }
            "quit" => {
                app.exit(0);
            }
            id if id.starts_with("workspace:") => {
                if let Some(name) = id.strip_prefix("workspace:") {
                    if name != "none" {
                        switch_workspace_from_tray(app, name);
                    }
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    let tray = if let Some(icon) = app.default_window_icon() {
        tray.icon(icon.clone())
    } else {
        tray
    };

    if tray.build(app).is_ok() {
        update_tray_tooltip(app.handle());
    }
}
