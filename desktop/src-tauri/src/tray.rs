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
    zh: bool,
) -> String {
    if let Some(action) = busy {
        return if zh {
            format!("Agent Doctor · 忙碌 · {action}")
        } else {
            format!("Agent Doctor · Busy · {action}")
        };
    }

    let health_label = match health {
        None if zh => "还没检查".to_string(),
        None => "Health —".to_string(),
        Some((installed, total)) if zh && (total == 0 || installed == 0) => "需要处理".to_string(),
        Some((installed, total)) if total == 0 || installed == 0 => "Attention".to_string(),
        Some((installed, total)) if zh && installed == total => format!("正常 {installed}/{total}"),
        Some((installed, total)) if installed == total => format!("OK {installed}/{total}"),
        Some((installed, total)) if zh => format!("部分 {installed}/{total}"),
        Some((installed, total)) => format!("Partial {installed}/{total}"),
    };
    let ws = workspace.unwrap_or("—");
    let mode_label = match (mode, zh) {
        ("personal", true) => "个人",
        ("personal", false) => "personal",
        ("team", true) => "团队",
        ("team", false) => "team",
        (_, true) => "未设置",
        (_, false) => "unset",
    };
    let project = if zh { "项目" } else { "ws" };
    format!("Agent Doctor · {health_label} · {project}:{ws} · {mode_label}")
}

#[derive(Clone, Copy)]
struct TrayMenuLabels {
    show: &'static str,
    switch: &'static str,
    check_project: &'static str,
    diagnose: &'static str,
    updates: &'static str,
    quit: &'static str,
    no_projects: &'static str,
    busy_switch: &'static str,
    busy_check: &'static str,
    busy_diagnose: &'static str,
}

fn tray_menu_labels(zh: bool) -> TrayMenuLabels {
    if zh {
        TrayMenuLabels {
            show: "显示 Agent Doctor",
            switch: "切换项目",
            check_project: "检查这个项目",
            diagnose: "诊断",
            updates: "检查更新",
            quit: "退出",
            no_projects: "还没有项目",
            busy_switch: "正在切换…",
            busy_check: "正在检查这个项目…",
            busy_diagnose: "正在诊断…",
        }
    } else {
        TrayMenuLabels {
            show: "Show Agent Doctor",
            switch: "Switch workspace",
            check_project: "Workspace check",
            diagnose: "Run doctor",
            updates: "Check for updates",
            quit: "Quit",
            no_projects: "(no workspaces)",
            busy_switch: "Switching…",
            busy_check: "Workspace check…",
            busy_diagnose: "Doctor…",
        }
    }
}

fn menu_labels(app: &tauri::AppHandle) -> TrayMenuLabels {
    tray_menu_labels(crate::commands::app_locale(app).is_zh())
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
        crate::commands::app_locale(app).is_zh(),
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
    let labels = menu_labels(app);
    let show = MenuItem::with_id(app, "show", labels.show, true, None::<&str>)?;
    let ws_doctor = MenuItem::with_id(
        app,
        "workspace_doctor",
        labels.check_project,
        true,
        None::<&str>,
    )?;
    let doctor = MenuItem::with_id(app, "doctor", labels.diagnose, true, None::<&str>)?;
    let check_update = MenuItem::with_id(app, "check_update", labels.updates, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", labels.quit, true, None::<&str>)?;

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
        labels.no_projects,
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
    let switch_sub = Submenu::with_id(app, "switch", labels.switch, true)?;
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
    set_tray_busy(app, Some(menu_labels(app).busy_switch));
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
                set_tray_busy(app, Some(menu_labels(app).busy_check));
                let app_handle = app.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let result = workspace_doctor();
                    set_tray_busy(&app_handle, None);
                    if let Ok(report) = result {
                        publish_workspace_doctor_report(&app_handle, &report);
                    }
                });
            }
            "doctor" => {
                set_tray_busy(app, Some(menu_labels(app).busy_diagnose));
                let app_handle = app.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    // Doctor spawns version/npm probe children; menu events run on
                    // the main thread, so this must stay off it.
                    let report = run_doctor();
                    remember_tray_health(&app_handle, &report);
                    set_tray_busy(&app_handle, None);
                    publish_doctor_report(&app_handle, &report);
                });
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

#[cfg(test)]
mod tests {
    use super::tray_menu_labels;

    #[test]
    fn tray_menu_follows_language() {
        let en = tray_menu_labels(false);
        assert_eq!(en.show, "Show Agent Doctor");
        assert_eq!(en.switch, "Switch workspace");
        assert_eq!(en.check_project, "Workspace check");
        assert_eq!(en.diagnose, "Run doctor");
        assert_eq!(en.updates, "Check for updates");
        assert_eq!(en.quit, "Quit");
        assert_eq!(en.no_projects, "(no workspaces)");
        assert_eq!(en.busy_switch, "Switching…");
        assert_eq!(en.busy_check, "Workspace check…");
        assert_eq!(en.busy_diagnose, "Doctor…");

        let zh = tray_menu_labels(true);
        assert_eq!(zh.show, "显示 Agent Doctor");
        assert_eq!(zh.switch, "切换项目");
        assert_eq!(zh.check_project, "检查这个项目");
        assert_eq!(zh.diagnose, "诊断");
        assert_eq!(zh.updates, "检查更新");
        assert_eq!(zh.quit, "退出");
        assert_eq!(zh.no_projects, "还没有项目");
        assert_eq!(zh.busy_switch, "正在切换…");
        assert_eq!(zh.busy_check, "正在检查这个项目…");
        assert_eq!(zh.busy_diagnose, "正在诊断…");
    }
}
