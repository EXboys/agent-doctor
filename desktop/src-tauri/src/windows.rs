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
/// Designed height of the agent home. Taller screens stay here; shorter
/// logical work areas (150% on 1080p) use the work area itself.
const MAIN_WINDOW_HEIGHT: f64 = 800.0;

fn main_inner_height(work_h: f64) -> f64 {
    // macOS stretches with the screen. Windows keeps the designed height so a
    // large monitor does not turn the home into a full-height column.
    fit_main_inner_height(work_h, cfg!(target_os = "windows"))
}

fn fit_main_inner_height(work_h: f64, limit_to_design: bool) -> f64 {
    let available = (work_h - MAIN_WINDOW_MARGIN * 2.0).max(MAIN_WINDOW_MIN_HEIGHT);
    if limit_to_design {
        // Do not apply a second fraction. 72% of a scaled 1080p work area is
        // shorter than the agent home, so the main page had to scroll.
        available.min(MAIN_WINDOW_HEIGHT)
    } else {
        available
    }
}

/// Outer height shared by the home and the window beside it.
fn docked_outer_height(work_h: f64) -> f64 {
    (work_h - MAIN_WINDOW_MARGIN * 2.0).max(MAIN_WINDOW_MIN_HEIGHT)
}

/// `set_size` is the webview. Subtract only this window's title bar so both
/// outer edges land on `docked_outer_height`.
fn docked_inner_height(work_h: f64, decoration_h: f64, min_inner: f64) -> f64 {
    (docked_outer_height(work_h) - decoration_h.max(0.0)).max(min_inner)
}

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
    // Alone, the home stays at the designed height. Beside Ask / Resources /
    // Diagnose it grows to that window so the two bottoms line up.

    let (current_main_w, _) = main_window_logical_size(&main).unwrap_or((420.0, 720.0));
    let main_outer_w = current_main_w.clamp(
        MAIN_WINDOW_MIN_WIDTH,
        (work_w * 0.38).clamp(MAIN_WINDOW_MIN_WIDTH, 480.0),
    );

    let secondary_deco_w = window_decoration_width(&secondary, scale);
    let secondary_deco_h = window_decoration_height(&secondary, scale);
    let main_deco_h = window_decoration_height(&main, scale);
    let main_h = docked_inner_height(work_h, main_deco_h, MAIN_WINDOW_MIN_HEIGHT);

    let room = work_w - MAIN_WINDOW_MARGIN * 2.0 - gap - main_outer_w;
    let max_outer_w = (work_w - MAIN_WINDOW_MARGIN * 2.0).max(360.0);
    // A 720-wide window beside the main window runs off a narrow Windows screen
    // and looks like the click did nothing. Shrink to the work area instead.
    let secondary_outer_w = if room >= ASK_WINDOW_MIN_WIDTH {
        room.min(max_outer_w)
    } else {
        room.max(360.0).min(max_outer_w)
    };
    let secondary_inner_w = (secondary_outer_w - secondary_deco_w).max(360.0);
    let secondary_inner_h = docked_inner_height(work_h, secondary_deco_h, ASK_WINDOW_MIN_HEIGHT);

    let main_x = work_x + MAIN_WINDOW_MARGIN;
    let mut secondary_x = main_x + main_outer_w + gap;
    let max_x = work_x + work_w - MAIN_WINDOW_MARGIN - secondary_outer_w;
    if secondary_x > max_x {
        secondary_x = max_x.max(work_x + MAIN_WINDOW_MARGIN);
    }
    if secondary_inner_w < ASK_WINDOW_MIN_WIDTH {
        let _ = secondary.set_min_size(Some(LogicalSize::new(
            secondary_inner_w,
            ASK_WINDOW_MIN_HEIGHT,
        )));
    }

    let _ = main.set_size(LogicalSize::new(main_outer_w, main_h));
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

fn secondary_is_visible(app: &AppHandle, label: &str) -> bool {
    app.get_webview_window(label)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

fn another_secondary_is_visible(app: &AppHandle, except: &str) -> bool {
    [
        ASK_WINDOW_LABEL,
        RESOURCES_WINDOW_LABEL,
        DIAGNOSE_WINDOW_LABEL,
    ]
    .into_iter()
    .any(|label| label != except && secondary_is_visible(app, label))
}

/// When Ask / Resources / Diagnose already share the screen, open the next one
/// slightly offset so it stays visible instead of replacing the others.
fn place_secondary_cascaded(app: &AppHandle, window: &tauri::WebviewWindow) {
    let Some(main) = app.get_webview_window("main") else {
        return;
    };
    let Some((work_x, work_y, work_w, work_h, scale)) = monitor_work_area(&main) else {
        return;
    };
    let deco_w = window_decoration_width(window, scale);
    let deco_h = window_decoration_height(window, scale);
    let (main_w, _) = main_window_logical_size(&main).unwrap_or((420.0, 720.0));
    let gap = ASK_WINDOW_MARGIN;
    let open_count = [
        ASK_WINDOW_LABEL,
        RESOURCES_WINDOW_LABEL,
        DIAGNOSE_WINDOW_LABEL,
    ]
    .into_iter()
    .filter(|label| secondary_is_visible(app, label))
    .count()
    .saturating_sub(1);
    let step = 36.0 * open_count as f64;
    let outer_w = ((work_w - MAIN_WINDOW_MARGIN * 2.0 - gap - main_w) * 0.92)
        .max(ASK_WINDOW_MIN_WIDTH)
        .min(work_w - MAIN_WINDOW_MARGIN * 2.0);
    let outer_h = ((work_h - MAIN_WINDOW_MARGIN * 2.0) * 0.88).max(ASK_WINDOW_MIN_HEIGHT);
    let inner_w = (outer_w - deco_w).max(ASK_WINDOW_MIN_WIDTH);
    let inner_h = (outer_h - deco_h).max(ASK_WINDOW_MIN_HEIGHT);
    let x = (work_x + MAIN_WINDOW_MARGIN + main_w + gap + step)
        .min(work_x + work_w - MAIN_WINDOW_MARGIN - outer_w);
    let y =
        (work_y + MAIN_WINDOW_MARGIN + step).min(work_y + work_h - MAIN_WINDOW_MARGIN - outer_h);
    let _ = window.set_size(LogicalSize::new(inner_w, inner_h));
    let _ = window.set_position(LogicalPosition::new(x, y));
}

fn show_secondary_window(
    app: &AppHandle,
    label: &str,
    window: &tauri::WebviewWindow,
    dock: fn(&AppHandle),
) {
    let was_visible = window.is_visible().unwrap_or(false);
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.unminimize();
        let _ = main.show();
    }
    let _ = window.set_skip_taskbar(false);
    let _ = window.unminimize();
    let _ = window.show();
    if !was_visible {
        if another_secondary_is_visible(app, label) {
            place_secondary_cascaded(app, window);
        } else {
            dock(app);
            // Second pass after chrome metrics are valid.
            dock(app);
        }
    }
    let _ = window.set_focus();
    // A new WebView2 window often stays behind the main window, so the click
    // looks like it did nothing.
    #[cfg(target_os = "windows")]
    {
        let _ = window.set_always_on_top(true);
        let _ = window.set_always_on_top(false);
    }
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
    let height = main_inner_height(work_h);
    let x = work_x + MAIN_WINDOW_MARGIN;
    let y = work_y + MAIN_WINDOW_MARGIN;
    let _ = window.set_size(LogicalSize::new(width, height));
    let _ = window.set_position(LogicalPosition::new(x, y));
}

pub(crate) fn show_main_window(app: &tauri::AppHandle) {
    let Some(window) = ensure_main_window(app) else {
        return;
    };
    // Show first. A hidden window often has no monitor, so sizing it before
    // show left the restored window short and the home had to scroll.
    let _ = window.unminimize();
    let _ = window.show();
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
        .inner_size(420.0, 800.0)
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

/// Leave fullscreen before dismissing. Otherwise macOS keeps an empty Space
/// in Mission Control (a black "Agent Doctor — Ask" thumbnail) after maximize.
fn dismiss_secondary_window(window: &tauri::WebviewWindow, destroy: bool) {
    let _ = window.set_fullscreen(false);
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
    }
    // On macOS, hide-after-maximize still leaves a Mission Control Space.
    // Destroy so the independent window is really gone; the next open recreates it.
    if destroy || cfg!(target_os = "macos") {
        let _ = window.destroy();
        return;
    }
    // Windows: keep the WebView2 shell so the next open does not rebuild it.
    let _ = window.set_skip_taskbar(true);
    let _ = window.hide();
}

fn attach_ask_window_close_behavior(window: &tauri::WebviewWindow) {
    let win = window.clone();
    let app = window.app_handle().clone();
    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            dismiss_secondary_window(&win, false);
        }
        WindowEvent::Destroyed => crate::island::on_ask_closed(&app),
        _ => {}
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

fn ask_still_at_create_size(window: &tauri::WebviewWindow) -> bool {
    let Ok((width, height)) = main_window_logical_size(window) else {
        return true;
    };
    if width < 1.0 || height < 1.0 {
        return true;
    }
    (width - ASK_WINDOW_WIDTH).abs() < 8.0 && (height - ASK_WINDOW_HEIGHT).abs() < 8.0
}

/// Show Ask beside the main window without reloading the page.
/// The first open uses the usual size. A size the person dragged is kept.
pub(crate) fn present_ask_window(app: &AppHandle) {
    let Ok(window) = ensure_ask_window(app, "claude-code") else {
        return;
    };
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.unminimize();
        let _ = main.show();
    }
    let _ = window.set_skip_taskbar(false);
    let _ = window.unminimize();
    let place = ask_still_at_create_size(&window);
    let _ = window.show();
    if place {
        layout_main_and_ask_side_by_side(app);
        // Second pass after the title bar height is known.
        layout_main_and_ask_side_by_side(app);
    } else if let Ok((width, height)) = main_window_logical_size(&window) {
        // The page keeps the width it had while hidden and its right edge gets cut off.
        // A real size change makes it lay out again at the window width.
        let _ = window.set_size(LogicalSize::new(width + 1.0, height));
        let _ = window.set_size(LogicalSize::new(width, height));
    }
    let _ = window.set_focus();
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
    // Ask / Resources / Diagnose can stay open together.
    show_secondary_window(
        app,
        ASK_WINDOW_LABEL,
        &window,
        layout_main_and_ask_side_by_side,
    );
    // Recover crashed Ask (blank/titled, never booted) with one soft reload. A healthy
    // page must not reload on every Agents click — dev builds load from Vite
    // (127.0.0.1:1420) and reload whitescreens when `npm run dev` is not running.
    if already_exists {
        let runtime_json = serde_json::Value::String(runtime.to_string()).to_string();
        let recover_or_switch = format!(
            "(function(){{\
                window.__AD_ASK_RUNTIME__ = {runtime};\
                try {{ localStorage.setItem('ad.ask.pendingRuntime', {runtime}); }} catch (e) {{}}\
                var shell = document.getElementById('chat-shell');\
                if (window.__AD_ASK_BOOTED__ && shell) {{\
                    if (typeof window.__AD_ASK_APPLY_RUNTIME__ === 'function') {{\
                        window.__AD_ASK_APPLY_RUNTIME__({runtime});\
                    }}\
                    return;\
                }}\
                location.reload();\
            }})();",
            runtime = runtime_json
        );
        let _ = window.eval(&recover_or_switch);
        let payload = serde_json::json!({ "runtime": runtime });
        let _ = window.emit("ask-window-focus", &payload);
        let _ = app.emit("ask-window-focus", &payload);
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
    dismiss_secondary_window(&window, destroy);
    crate::island::on_ask_closed(app);
    Ok(())
}

fn attach_resources_window_close_behavior(window: &tauri::WebviewWindow) {
    let win = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            dismiss_secondary_window(&win, false);
        }
    });
}

fn resources_section_token(section: Option<&str>) -> String {
    match section.map(str::trim) {
        Some(
            value @ ("agents" | "skills" | "mall" | "tools" | "browser" | "catalog" | "store"
            | "mcp"),
        ) => value.to_string(),
        _ => "catalog".to_string(),
    }
}

fn resources_section_bootstrap_script(section: &str, defer_boot_scan: bool) -> String {
    let section_json = serde_json::Value::String(section.to_string()).to_string();
    let defer = if defer_boot_scan { "true" } else { "false" };
    // A later open writes the real section into sessionStorage before reload.
    // Do not let a startup fallback overwrite that stored section.
    // Hidden pre-create sets defer so the page does not probe CLIs until shown.
    format!(
        "(function(){{\
            window.__AD_DEFER_BOOT_SCAN__ = {defer};\
            var fallback = {section};\
            var stored = null;\
            try {{ stored = sessionStorage.getItem('ad.resources.pendingSection'); }} catch (e) {{}}\
            if (stored && stored.trim()) {{\
                window.__AD_RESOURCES_SECTION__ = stored.trim();\
                try {{ sessionStorage.removeItem('ad.resources.pendingSection'); }} catch (e) {{}}\
            }} else {{\
                window.__AD_RESOURCES_SECTION__ = fallback;\
            }}\
        }})();",
        defer = defer,
        section = section_json
    )
}

fn create_resources_window(
    app: &AppHandle,
    visible: bool,
    section: &str,
    defer_boot_scan: bool,
) -> Result<tauri::WebviewWindow, String> {
    let window = WebviewWindowBuilder::new(
        app,
        RESOURCES_WINDOW_LABEL,
        WebviewUrl::App("resources.html".into()),
    )
    .initialization_script(resources_section_bootstrap_script(section, defer_boot_scan))
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

pub(crate) fn ensure_resources_window(
    app: &AppHandle,
    section: &str,
    defer_boot_scan: bool,
) -> Result<tauri::WebviewWindow, String> {
    if let Some(existing) = app.get_webview_window(RESOURCES_WINDOW_LABEL) {
        return Ok(existing);
    }
    create_resources_window(app, false, section, defer_boot_scan)
}

fn attach_diagnose_window_close_behavior(window: &tauri::WebviewWindow) {
    let win = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            dismiss_secondary_window(&win, false);
        }
    });
}

fn diagnose_runtime_bootstrap_script(runtime: &str, defer_boot_scan: bool) -> String {
    let runtime_json = serde_json::Value::String(runtime.to_string()).to_string();
    let defer = if defer_boot_scan { "true" } else { "false" };
    // Do not overwrite sessionStorage: the window is pre-created at startup with a
    // default runtime, and location.reload() re-runs this script. A later open
    // writes the real runtime into sessionStorage before reload.
    format!(
        "(function(){{\
            window.__AD_DEFER_BOOT_SCAN__ = {defer};\
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
        defer = defer,
        runtime = runtime_json
    )
}

fn create_diagnose_window(
    app: &AppHandle,
    runtime: &str,
    visible: bool,
    defer_boot_scan: bool,
) -> Result<tauri::WebviewWindow, String> {
    let init_script = diagnose_runtime_bootstrap_script(runtime, defer_boot_scan);
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
    defer_boot_scan: bool,
) -> Result<tauri::WebviewWindow, String> {
    if let Some(existing) = app.get_webview_window(DIAGNOSE_WINDOW_LABEL) {
        return Ok(existing);
    }
    create_diagnose_window(app, runtime, false, defer_boot_scan)
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
    let window = ensure_diagnose_window(app, runtime, false)?;
    // Ask / Resources / Diagnose can stay open together.
    show_secondary_window(
        app,
        DIAGNOSE_WINDOW_LABEL,
        &window,
        layout_main_and_diagnose_side_by_side,
    );
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
    // Pre-created hidden windows skip the boot scan. Ask them to scan once shown.
    let _ = window
        .eval("window.__AD_DIAGNOSE_ENSURE_LOADED__ && window.__AD_DIAGNOSE_ENSURE_LOADED__()");

    Ok(())
}

pub(crate) fn close_diagnose_window(app: &AppHandle, destroy: bool) -> Result<(), String> {
    let Some(window) = app.get_webview_window(DIAGNOSE_WINDOW_LABEL) else {
        return Ok(());
    };
    dismiss_secondary_window(&window, destroy);
    Ok(())
}

pub(crate) fn open_or_focus_resources_window(
    app: &AppHandle,
    section: Option<&str>,
) -> Result<(), String> {
    let section = resources_section_token(section);
    let already_exists = app.get_webview_window(RESOURCES_WINDOW_LABEL).is_some();
    let window = ensure_resources_window(app, &section, false)?;
    // Ask / Resources / Diagnose can stay open together.
    show_secondary_window(
        app,
        RESOURCES_WINDOW_LABEL,
        &window,
        layout_main_and_resources_side_by_side,
    );
    if already_exists {
        let section_json = serde_json::Value::String(section.clone()).to_string();
        let script = format!(
            "(function(){{\
                try {{ sessionStorage.setItem('ad.resources.pendingSection', {section}); }} catch (e) {{}}\
                window.__AD_RESOURCES_SECTION__ = {section};\
                if (typeof window.__AD_RESOURCES_APPLY_SECTION__ === 'function') {{\
                    window.__AD_RESOURCES_APPLY_SECTION__({section});\
                }} else {{\
                    location.reload();\
                }}\
            }})();",
            section = section_json
        );
        let _ = window.eval(&script);
    }
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
    dismiss_secondary_window(&window, destroy);
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

#[cfg(test)]
mod tests {
    use super::{docked_inner_height, fit_main_inner_height};

    #[test]
    fn windows_home_keeps_design_height_and_does_not_shrink_further() {
        // Tall screen: stay at the designed height instead of filling the monitor.
        assert_eq!(fit_main_inner_height(1040.0, true), 800.0);
        // 125% on 1080p still has room for the designed height.
        assert_eq!(fit_main_inner_height(832.0, true), 800.0);
        // 150% on 1080p: logical work area is under the design height. Use it all.
        // The old 72% cap landed near 495 and forced the home to scroll.
        assert_eq!(fit_main_inner_height(688.0, true), 656.0);
    }

    #[test]
    fn docked_windows_share_one_outer_height() {
        let work_h = 1040.0;
        let main = docked_inner_height(work_h, 32.0, 480.0);
        let ask = docked_inner_height(work_h, 32.0, 480.0);
        assert_eq!(main, ask);
        assert_eq!(main + 32.0, 1040.0 - 32.0);

        let main_title = docked_inner_height(work_h, 40.0, 480.0);
        let ask_title = docked_inner_height(work_h, 28.0, 480.0);
        assert_eq!(main_title + 40.0, ask_title + 28.0);
    }
}
