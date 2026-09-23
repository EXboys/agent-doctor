use std::time::Duration;

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};

const ASK_WINDOW_LABEL: &str = "ask";
/// Soft defaults for first create only; live size follows the monitor work area.
const ASK_WINDOW_WIDTH: f64 = 980.0;
const ASK_WINDOW_HEIGHT: f64 = 640.0;
const ASK_WINDOW_MARGIN: f64 = 16.0;
const ASK_WINDOW_MIN_WIDTH: f64 = 720.0;
const ASK_WINDOW_MIN_HEIGHT: f64 = 480.0;
const RESOURCES_WINDOW_LABEL: &str = "resources";
const DIAGNOSE_WINDOW_LABEL: &str = "diagnose";
const MAIN_WINDOW_MARGIN: f64 = 16.0;
const MAIN_WINDOW_MIN_WIDTH: f64 = 360.0;
const MAIN_WINDOW_MIN_HEIGHT: f64 = 480.0;

fn monitor_work_area(window: &tauri::WebviewWindow) -> Option<(f64, f64, f64, f64, f64)> {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())?;
    let scale = monitor.scale_factor();
    let work = monitor.work_area();
    let work_x = work.position.x as f64 / scale;
    let work_y = work.position.y as f64 / scale;
    let work_w = work.size.width as f64 / scale;
    let work_h = work.size.height as f64 / scale;
    Some((work_x, work_y, work_w, work_h, scale))
}

fn window_decoration_height(window: &tauri::WebviewWindow, scale: f64) -> f64 {
    let Ok(outer) = window.outer_size() else {
        return 0.0;
    };
    let Ok(inner) = window.inner_size() else {
        return 0.0;
    };
    ((outer.height as f64 - inner.height as f64) / scale).max(0.0)
}

fn window_decoration_width(window: &tauri::WebviewWindow, scale: f64) -> f64 {
    let Ok(outer) = window.outer_size() else {
        return 0.0;
    };
    let Ok(inner) = window.inner_size() else {
        return 0.0;
    };
    ((outer.width as f64 - inner.width as f64) / scale).max(0.0)
}

/// Dock main (left) + secondary (right: Ask / Resources / Diagnose): same top/bottom, side by side.
fn layout_main_and_secondary_side_by_side(app: &AppHandle, secondary_label: &str) {
    let Some(main) = app.get_webview_window("main") else {
        return;
    };
    let Some(secondary) = app.get_webview_window(secondary_label) else {
        return;
    };
    let Some((work_x, work_y, work_w, work_h, scale)) = monitor_work_area(&main) else {
        return;
    };

    let gap = ASK_WINDOW_MARGIN;
    let y = work_y + MAIN_WINDOW_MARGIN;
    let outer_h = (work_h - MAIN_WINDOW_MARGIN * 2.0).max(MAIN_WINDOW_MIN_HEIGHT);

    let (current_main_w, _) = main_window_logical_size(&main).unwrap_or((420.0, 720.0));
    let main_outer_w = current_main_w.clamp(
        MAIN_WINDOW_MIN_WIDTH,
        (work_w * 0.38).clamp(MAIN_WINDOW_MIN_WIDTH, 480.0),
    );

    let secondary_deco_w = window_decoration_width(&secondary, scale);
    let secondary_deco_h = window_decoration_height(&secondary, scale);
    let main_deco_h = window_decoration_height(&main, scale);

    let secondary_outer_w =
        (work_w - MAIN_WINDOW_MARGIN * 2.0 - gap - main_outer_w).max(ASK_WINDOW_MIN_WIDTH);
    let secondary_inner_w = (secondary_outer_w - secondary_deco_w).max(ASK_WINDOW_MIN_WIDTH);
    let main_inner_h = (outer_h - main_deco_h).max(MAIN_WINDOW_MIN_HEIGHT);
    let secondary_inner_h = (outer_h - secondary_deco_h).max(ASK_WINDOW_MIN_HEIGHT);

    let main_x = work_x + MAIN_WINDOW_MARGIN;
    let secondary_x = main_x + main_outer_w + gap;

    let _ = main.set_size(LogicalSize::new(main_outer_w, main_inner_h));
    let _ = main.set_position(LogicalPosition::new(main_x, y));
    let _ = secondary.set_size(LogicalSize::new(secondary_inner_w, secondary_inner_h));
    let _ = secondary.set_position(LogicalPosition::new(secondary_x, y));
}

pub(crate) fn layout_main_and_ask_side_by_side(app: &AppHandle) {
    layout_main_and_secondary_side_by_side(app, ASK_WINDOW_LABEL);
}

pub(crate) fn layout_main_and_resources_side_by_side(app: &AppHandle) {
    layout_main_and_secondary_side_by_side(app, RESOURCES_WINDOW_LABEL);
}

pub(crate) fn layout_main_and_diagnose_side_by_side(app: &AppHandle) {
    layout_main_and_secondary_side_by_side(app, DIAGNOSE_WINDOW_LABEL);
}

fn hide_secondary_window(app: &AppHandle, label: &str) {
    let Some(window) = app.get_webview_window(label) else {
        return;
    };
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let _ = window.set_skip_taskbar(true);
    let _ = window.hide();
}

fn position_main_window_left(window: &tauri::WebviewWindow) {
    let Some((work_x, work_y, work_w, work_h, _)) = monitor_work_area(window) else {
        return;
    };
    let (current_w, _) = main_window_logical_size(window).unwrap_or((420.0, 720.0));
    let width = current_w.clamp(
        MAIN_WINDOW_MIN_WIDTH,
        (work_w * 0.38).clamp(MAIN_WINDOW_MIN_WIDTH, 480.0),
    );
    let height = (work_h - MAIN_WINDOW_MARGIN * 2.0).max(MAIN_WINDOW_MIN_HEIGHT);
    let x = work_x + MAIN_WINDOW_MARGIN;
    let y = work_y + MAIN_WINDOW_MARGIN;
    let _ = window.set_size(LogicalSize::new(width, height));
    let _ = window.set_position(LogicalPosition::new(x, y));
}

pub(crate) fn show_main_window(app: &tauri::AppHandle) {
    let Some(window) = ensure_main_window(app) else {
        return;
    };
    let ask_visible = app
        .get_webview_window(ASK_WINDOW_LABEL)
        .and_then(|ask| ask.is_visible().ok())
        .unwrap_or(false);
    let resources_visible = app
        .get_webview_window(RESOURCES_WINDOW_LABEL)
        .and_then(|win| win.is_visible().ok())
        .unwrap_or(false);
    let diagnose_visible = app
        .get_webview_window(DIAGNOSE_WINDOW_LABEL)
        .and_then(|win| win.is_visible().ok())
        .unwrap_or(false);
    if ask_visible {
        layout_main_and_ask_side_by_side(app);
    } else if resources_visible {
        layout_main_and_resources_side_by_side(app);
    } else if diagnose_visible {
        layout_main_and_diagnose_side_by_side(app);
    } else {
        position_main_window_left(&window);
    }
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    // WebView2 / undecorated windows on Windows sometimes stay behind after
    // restore; a brief always-on-top pulse pulls them to the foreground.
    #[cfg(target_os = "windows")]
    {
        let _ = window.set_always_on_top(true);
        let _ = window.set_always_on_top(false);
    }
}

pub(crate) fn attach_main_window_close_behavior(window: &tauri::WebviewWindow) {
    let hide = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            // Keep the process alive for tray + Ask; recreate is expensive on WebView2.
            api.prevent_close();
            let _ = hide.hide();
        }
    });
}

fn ensure_main_window(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(window) = app.get_webview_window("main") {
        return Some(window);
    }

    let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Agent Doctor")
        .inner_size(420.0, 720.0)
        .min_inner_size(360.0, 520.0)
        .decorations(true)
        .transparent(false)
        .shadow(true)
        .resizable(true)
        .visible(false);

    match builder.build() {
        Ok(window) => {
            attach_main_window_close_behavior(&window);
            Some(window)
        }
        Err(err) => {
            eprintln!("failed to recreate main window: {err}");
            None
        }
    }
}

#[derive(Debug, Serialize)]
pub struct WindowSizeReport {
    width: f64,
    height: f64,
}

fn main_window_logical_size(window: &tauri::WebviewWindow) -> Result<(f64, f64), String> {
    let scale = window.scale_factor().map_err(|error| error.to_string())?;
    let inner = window.inner_size().map_err(|error| error.to_string())?;
    Ok((inner.width as f64 / scale, inner.height as f64 / scale))
}

/// Window create/resize must run on the UI thread. WebView2 on Windows
/// otherwise yields a titled shell with a permanently blank page.
fn run_on_main_thread<T, F>(app: &AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|err| format!("failed to run on UI thread: {err}"))?;
    rx.recv_timeout(Duration::from_secs(12))
        .map_err(|_| {
            "UI thread did not finish the window operation in time. If a blank Ask window is stuck, end Agent Doctor.exe in Task Manager, then reopen.".to_string()
        })?
}

/// Resize the main window; omit width/height to only report the current size.
/// Clamps to the monitor work area and shifts left if expanding would overflow.
#[tauri::command]
pub fn resize_main_window_command(
    app: AppHandle,
    width: Option<f64>,
    height: Option<f64>,
) -> Result<WindowSizeReport, String> {
    let app_for_ui = app.clone();
    run_on_main_thread(&app, move || resize_main_window(&app_for_ui, width, height))
}

fn resize_main_window(
    app: &AppHandle,
    width: Option<f64>,
    height: Option<f64>,
) -> Result<WindowSizeReport, String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window missing".to_string())?;
    let (current_w, current_h) = main_window_logical_size(&window)?;
    if width.is_none() && height.is_none() {
        return Ok(WindowSizeReport {
            width: current_w,
            height: current_h,
        });
    }

    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let (max_w, max_h, work_x, work_y, work_w, work_h, scale) = if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let work = monitor.work_area();
        let work_x = work.position.x as f64 / scale;
        let work_y = work.position.y as f64 / scale;
        let work_w = work.size.width as f64 / scale;
        let work_h = work.size.height as f64 / scale;
        (
            (work_w - MAIN_WINDOW_MARGIN * 2.0).max(MAIN_WINDOW_MIN_WIDTH),
            (work_h - MAIN_WINDOW_MARGIN * 2.0).max(MAIN_WINDOW_MIN_HEIGHT),
            work_x,
            work_y,
            work_w,
            work_h,
            scale,
        )
    } else {
        (
            1600.0,
            1200.0,
            0.0,
            0.0,
            1600.0,
            1200.0,
            window.scale_factor().unwrap_or(1.0),
        )
    };

    let new_w = width
        .unwrap_or(current_w)
        .clamp(MAIN_WINDOW_MIN_WIDTH, max_w.min(900.0));
    let new_h = height
        .unwrap_or(current_h)
        .clamp(MAIN_WINDOW_MIN_HEIGHT, max_h);

    window
        .set_size(LogicalSize::new(new_w, new_h))
        .map_err(|error| error.to_string())?;

    if let Ok(pos) = window.outer_position() {
        let x = pos.x as f64 / scale;
        let y = pos.y as f64 / scale;
        let mut nx = x;
        let mut ny = y;
        if x + new_w > work_x + work_w - MAIN_WINDOW_MARGIN {
            nx = (work_x + work_w - MAIN_WINDOW_MARGIN - new_w).max(work_x + MAIN_WINDOW_MARGIN);
        }
        if y + new_h > work_y + work_h - MAIN_WINDOW_MARGIN {
            ny = (work_y + work_h - MAIN_WINDOW_MARGIN - new_h).max(work_y + MAIN_WINDOW_MARGIN);
        }
        if (nx - x).abs() > 0.5 || (ny - y).abs() > 0.5 {
            let _ = window.set_position(LogicalPosition::new(nx, ny));
        }
    }

    let (final_w, final_h) = main_window_logical_size(&window).unwrap_or((new_w, new_h));
    Ok(WindowSizeReport {
        width: final_w,
        height: final_h,
    })
}

fn attach_ask_window_close_behavior(window: &tauri::WebviewWindow) {
    let hide = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            // Hide instead of destroy so WebView2 is not rebuilt on the next Ask
            // (rebuild on Windows can hang the UI thread and leave a blank shell).
            api.prevent_close();
            let _ = hide.set_skip_taskbar(true);
            let _ = hide.hide();
        }
    });
}

fn create_ask_window(
    app: &AppHandle,
    runtime: &str,
    visible: bool,
) -> Result<tauri::WebviewWindow, String> {
    let init_script = format!(
        "window.__AD_ASK_RUNTIME__ = {};",
        serde_json::Value::String(runtime.to_string())
    );
    // Load chat.html with no query string. WebView2 custom-protocol + `?query`
    // often produces a titled, permanently blank Ask window.
    let window = WebviewWindowBuilder::new(app, ASK_WINDOW_LABEL, WebviewUrl::App("chat.html".into()))
            .title("Agent Doctor — Ask")
            .inner_size(ASK_WINDOW_WIDTH, ASK_WINDOW_HEIGHT)
            .min_inner_size(ASK_WINDOW_MIN_WIDTH, ASK_WINDOW_MIN_HEIGHT)
            .resizable(true)
            .closable(true)
            .minimizable(true)
            .decorations(true)
            .visible(visible)
            // Hidden Ask must not steal taskbar restore from the main window.
            .skip_taskbar(!visible)
            .initialization_script(&init_script)
            .build()
            .map_err(|err| format!("failed to open ask window: {err}"))?;
    attach_ask_window_close_behavior(&window);
    Ok(window)
}

pub(crate) fn ensure_ask_window(
    app: &AppHandle,
    runtime: &str,
) -> Result<tauri::WebviewWindow, String> {
    if let Some(existing) = app.get_webview_window(ASK_WINDOW_LABEL) {
        return Ok(existing);
    }
    create_ask_window(app, runtime, false)
}

fn apply_ask_runtime_in_webview(window: &tauri::WebviewWindow, runtime: &str) {
    // Ask is pre-created at startup (often as claude-code) and then reused.
    // Events alone can be missed; eval updates the injected runtime and calls
    // the chat page apply hook so the active session matches the Agents entry.
    // Also persist to localStorage so a soft-reload cannot fall back to the
    // create-time initialization_script (which is stuck on the first runtime).
    let runtime_json = serde_json::Value::String(runtime.to_string()).to_string();
    let script = format!(
        "(function(){{\
            window.__AD_ASK_RUNTIME__ = {runtime};\
            try {{ localStorage.setItem('ad.ask.pendingRuntime', {runtime}); }} catch (e) {{}}\
            if (typeof window.__AD_ASK_APPLY_RUNTIME__ === 'function') {{\
                window.__AD_ASK_APPLY_RUNTIME__({runtime});\
            }}\
        }})();",
        runtime = runtime_json
    );
    let _ = window.eval(&script);
}

pub(crate) fn open_or_focus_ask_window(
    app: &AppHandle,
    runtime: Option<&str>,
) -> Result<(), String> {
    let runtime = runtime
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("claude-code");

    let already_exists = app.get_webview_window(ASK_WINDOW_LABEL).is_some();
    let window = ensure_ask_window(app, runtime)?;
    apply_ask_runtime_in_webview(&window, runtime);
    // Right-dock slot is shared with Resources — only one secondary on the right.
    hide_secondary_window(app, RESOURCES_WINDOW_LABEL);
    hide_secondary_window(app, DIAGNOSE_WINDOW_LABEL);
    // Pair with main: left/right side-by-side, top and bottom aligned.
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.unminimize();
        let _ = main.show();
    }
    let _ = window.set_skip_taskbar(false);
    let _ = window.unminimize();
    let _ = window.show();
    layout_main_and_ask_side_by_side(app);
    // Second pass after Ask chrome metrics are valid.
    layout_main_and_ask_side_by_side(app);
    let _ = window.set_focus();

    // A previously crashed Ask webview can stay titled+blank forever while we only
    // hide/show it. Always soft-reload existing Ask windows so history paints again.
    // Persist runtime before reload: create-time init script still injects the first
    // runtime (usually claude-code) and would otherwise win over `__AD_ASK_RUNTIME__`.
    if already_exists {
        let runtime_json = serde_json::Value::String(runtime.to_string()).to_string();
        let reload = format!(
            "(function(){{\
                window.__AD_ASK_RUNTIME__ = {runtime};\
                try {{ localStorage.setItem('ad.ask.pendingRuntime', {runtime}); }} catch (e) {{}}\
                location.reload();\
            }})();",
            runtime = runtime_json
        );
        let _ = window.eval(&reload);
    } else {
        // Fresh window: still push runtime + focus event after show.
        apply_ask_runtime_in_webview(&window, runtime);
        let payload = serde_json::json!({ "runtime": runtime });
        let _ = window.emit("ask-window-focus", &payload);
        let _ = app.emit("ask-window-focus", &payload);
    }

    Ok(())
}

pub(crate) fn close_ask_window(app: &AppHandle, destroy: bool) -> Result<(), String> {
    let Some(window) = app.get_webview_window(ASK_WINDOW_LABEL) else {
        return Ok(());
    };
    if destroy {
        window
            .destroy()
            .map_err(|err| format!("failed to close ask window: {err}"))?;
    } else {
        let _ = window.set_skip_taskbar(true);
        let _ = window.hide();
    }
    Ok(())
}

fn attach_resources_window_close_behavior(window: &tauri::WebviewWindow) {
    let hide = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = hide.set_skip_taskbar(true);
            let _ = hide.hide();
        }
    });
}

fn create_resources_window(app: &AppHandle, visible: bool) -> Result<tauri::WebviewWindow, String> {
    let window = WebviewWindowBuilder::new(
        app,
        RESOURCES_WINDOW_LABEL,
        WebviewUrl::App("resources.html".into()),
    )
    .title("Agent Doctor — Resources")
    .inner_size(ASK_WINDOW_WIDTH, ASK_WINDOW_HEIGHT)
    .min_inner_size(ASK_WINDOW_MIN_WIDTH, ASK_WINDOW_MIN_HEIGHT)
    .resizable(true)
    .closable(true)
    .minimizable(true)
    .decorations(true)
    .visible(visible)
    .skip_taskbar(!visible)
    .build()
    .map_err(|err| format!("failed to open resources window: {err}"))?;
    attach_resources_window_close_behavior(&window);
    Ok(window)
}

pub(crate) fn ensure_resources_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(existing) = app.get_webview_window(RESOURCES_WINDOW_LABEL) {
        return Ok(existing);
    }
    create_resources_window(app, false)
}

fn attach_diagnose_window_close_behavior(window: &tauri::WebviewWindow) {
    let hide = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = hide.set_skip_taskbar(true);
            let _ = hide.hide();
        }
    });
}

fn diagnose_runtime_bootstrap_script(runtime: &str) -> String {
    let runtime_json = serde_json::Value::String(runtime.to_string()).to_string();
    // Do not overwrite sessionStorage: the window is pre-created at startup with a
    // default runtime, and location.reload() re-runs this script. A later open
    // writes the real runtime into sessionStorage before reload.
    format!(
        "(function(){{\
            var fallback = {runtime};\
            var stored = null;\
            try {{ stored = sessionStorage.getItem('ad-diagnose-runtime'); }} catch (e) {{}}\
            if (stored && stored.trim()) {{\
                window.__AD_DIAGNOSE_RUNTIME__ = stored.trim();\
            }} else {{\
                window.__AD_DIAGNOSE_RUNTIME__ = fallback;\
                try {{ sessionStorage.setItem('ad-diagnose-runtime', fallback); }} catch (e) {{}}\
            }}\
        }})();",
        runtime = runtime_json
    )
}

fn create_diagnose_window(
    app: &AppHandle,
    runtime: &str,
    visible: bool,
) -> Result<tauri::WebviewWindow, String> {
    let init_script = diagnose_runtime_bootstrap_script(runtime);
    // Same soft defaults as Ask / Resources; live size follows right-dock layout.
    let window = WebviewWindowBuilder::new(
        app,
        DIAGNOSE_WINDOW_LABEL,
        WebviewUrl::App("diagnose.html".into()),
    )
    .title("Agent Doctor — Diagnose")
    .inner_size(ASK_WINDOW_WIDTH, ASK_WINDOW_HEIGHT)
    .min_inner_size(ASK_WINDOW_MIN_WIDTH, ASK_WINDOW_MIN_HEIGHT)
    .resizable(true)
    .closable(true)
    .minimizable(true)
    .decorations(true)
    .visible(visible)
    .skip_taskbar(!visible)
    .initialization_script(&init_script)
    .build()
    .map_err(|err| format!("failed to open diagnose window: {err}"))?;
    attach_diagnose_window_close_behavior(&window);
    Ok(window)
}

pub(crate) fn ensure_diagnose_window(
    app: &AppHandle,
    runtime: &str,
) -> Result<tauri::WebviewWindow, String> {
    if let Some(existing) = app.get_webview_window(DIAGNOSE_WINDOW_LABEL) {
        return Ok(existing);
    }
    create_diagnose_window(app, runtime, false)
}

fn apply_diagnose_runtime_in_webview(window: &tauri::WebviewWindow, runtime: &str) {
    let runtime_json = serde_json::Value::String(runtime.to_string()).to_string();
    let script = format!(
        "(function(){{\
            try {{ sessionStorage.setItem('ad-diagnose-runtime', {runtime}); }} catch (e) {{}}\
            window.__AD_DIAGNOSE_RUNTIME__ = {runtime};\
            if (typeof window.__AD_DIAGNOSE_APPLY_RUNTIME__ === 'function') {{\
                window.__AD_DIAGNOSE_APPLY_RUNTIME__({runtime});\
            }}\
        }})();",
        runtime = runtime_json
    );
    let _ = window.eval(&script);
}

pub(crate) fn open_or_focus_diagnose_window(
    app: &AppHandle,
    runtime: Option<&str>,
) -> Result<(), String> {
    let runtime = runtime
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("openclaw");

    let already_exists = app.get_webview_window(DIAGNOSE_WINDOW_LABEL).is_some();
    let window = ensure_diagnose_window(app, runtime)?;
    // Same right-dock slot as Ask / Resources.
    hide_secondary_window(app, ASK_WINDOW_LABEL);
    hide_secondary_window(app, RESOURCES_WINDOW_LABEL);
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.unminimize();
        let _ = main.show();
    }
    let _ = window.set_skip_taskbar(false);
    let _ = window.unminimize();
    let _ = window.show();
    layout_main_and_diagnose_side_by_side(app);
    layout_main_and_diagnose_side_by_side(app);
    let _ = window.set_focus();

    if already_exists {
        // Prefer soft switch; reload only if the page has not registered yet.
        // Persist to sessionStorage first so a reload cannot fall back to the
        // startup init script (always baked as the first create runtime).
        let runtime_json = serde_json::Value::String(runtime.to_string()).to_string();
        let script = format!(
            "(function(){{\
                try {{ sessionStorage.setItem('ad-diagnose-runtime', {runtime}); }} catch (e) {{}}\
                window.__AD_DIAGNOSE_RUNTIME__ = {runtime};\
                if (typeof window.__AD_DIAGNOSE_APPLY_RUNTIME__ === 'function') {{\
                    window.__AD_DIAGNOSE_APPLY_RUNTIME__({runtime});\
                }} else {{\
                    location.reload();\
                }}\
            }})();",
            runtime = runtime_json
        );
        let _ = window.eval(&script);
    } else {
        apply_diagnose_runtime_in_webview(&window, runtime);
        let payload = serde_json::json!({ "runtime": runtime });
        let _ = window.emit("diagnose-window-focus", &payload);
        let _ = app.emit("diagnose-window-focus", &payload);
    }

    Ok(())
}

pub(crate) fn close_diagnose_window(app: &AppHandle, destroy: bool) -> Result<(), String> {
    let Some(window) = app.get_webview_window(DIAGNOSE_WINDOW_LABEL) else {
        return Ok(());
    };
    if destroy {
        window
            .destroy()
            .map_err(|err| format!("failed to close diagnose window: {err}"))?;
    } else {
        let _ = window.set_skip_taskbar(true);
        let _ = window.hide();
    }
    Ok(())
}

pub(crate) fn open_or_focus_resources_window(
    app: &AppHandle,
    section: Option<&str>,
) -> Result<(), String> {
    let window = ensure_resources_window(app)?;
    // Same right-dock as Ask — hide Ask so Resources does not stack over main.
    hide_secondary_window(app, ASK_WINDOW_LABEL);
    hide_secondary_window(app, DIAGNOSE_WINDOW_LABEL);
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.unminimize();
        let _ = main.show();
    }
    let _ = window.set_skip_taskbar(false);
    let _ = window.unminimize();
    let _ = window.show();
    layout_main_and_resources_side_by_side(app);
    layout_main_and_resources_side_by_side(app);
    let _ = window.set_focus();
    let section = section
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("catalog");
    let _ = window.emit(
        "resources-window-focus",
        serde_json::json!({ "section": section }),
    );
    Ok(())
}

pub(crate) fn close_resources_window(app: &AppHandle, destroy: bool) -> Result<(), String> {
    let Some(window) = app.get_webview_window(RESOURCES_WINDOW_LABEL) else {
        return Ok(());
    };
    if destroy {
        window
            .destroy()
            .map_err(|err| format!("failed to close resources window: {err}"))?;
    } else {
        let _ = window.set_skip_taskbar(true);
        let _ = window.hide();
    }
    Ok(())
}

#[tauri::command]
pub fn open_ask_window_command(app: AppHandle, runtime: Option<String>) -> Result<(), String> {
    let app_for_ui = app.clone();
    run_on_main_thread(&app, move || {
        open_or_focus_ask_window(&app_for_ui, runtime.as_deref())
    })
}

#[tauri::command]
pub fn close_ask_window_command(app: AppHandle, destroy: Option<bool>) -> Result<(), String> {
    let app_for_ui = app.clone();
    run_on_main_thread(&app, move || {
        close_ask_window(&app_for_ui, destroy.unwrap_or(false))
    })
}

#[tauri::command]
pub fn open_resources_window_command(
    app: AppHandle,
    section: Option<String>,
) -> Result<(), String> {
    let app_for_ui = app.clone();
    run_on_main_thread(&app, move || {
        open_or_focus_resources_window(&app_for_ui, section.as_deref())
    })
}

#[tauri::command]
pub fn close_resources_window_command(app: AppHandle, destroy: Option<bool>) -> Result<(), String> {
    let app_for_ui = app.clone();
    run_on_main_thread(&app, move || {
        close_resources_window(&app_for_ui, destroy.unwrap_or(false))
    })
}

#[tauri::command]
pub fn open_diagnose_window_command(app: AppHandle, runtime: Option<String>) -> Result<(), String> {
    let app_for_ui = app.clone();
    run_on_main_thread(&app, move || {
        open_or_focus_diagnose_window(&app_for_ui, runtime.as_deref())
    })
}

#[tauri::command]
pub fn close_diagnose_window_command(app: AppHandle, destroy: Option<bool>) -> Result<(), String> {
    let app_for_ui = app.clone();
    run_on_main_thread(&app, move || {
        close_diagnose_window(&app_for_ui, destroy.unwrap_or(false))
    })
}

/// Focus the main window and ask it to open a tab (e.g. `resources`).
#[tauri::command]
pub fn focus_main_tab_command(app: AppHandle, tab: Option<String>) -> Result<(), String> {
    show_main_window(&app);
    if let Some(window) = app.get_webview_window("main") {
        let tab = tab
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("resources");
        let _ = window.emit("main-navigate", serde_json::json!({ "tab": tab }));
    }
    Ok(())
}
