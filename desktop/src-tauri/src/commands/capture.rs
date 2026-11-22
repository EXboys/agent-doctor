use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::computer_hand_common::DisplayBox;

#[derive(Serialize, Clone)]
pub struct ScreenCapture {
    pub path: String,
}

/// One overlay: which display it sits on. The picker floats over every
/// space; the capture happens live when the drag finishes, so the space the
/// person landed on is what they picked on.
#[derive(Clone)]
struct Overlay {
    label: String,
    display: DisplayBox,
}

struct RegionSession {
    initiator: String,
    overlays: Vec<Overlay>,
}

static REGION: Mutex<Option<RegionSession>> = Mutex::new(None);

fn take_region() -> Option<RegionSession> {
    REGION.lock().ok()?.take()
}

/// Freeze every display, then open a picker on each. Sync command so the
/// windows are built on the main thread, where Tauri needs it.
#[tauri::command]
pub fn start_region_capture_command(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    close_overlays(&app, take_region());
    let displays = crate::computer_hand::capture_screen_displays();
    if displays.is_empty() {
        return Err("no display found".to_string());
    }
    let initiator = window.label().to_string();
    let mut overlays = Vec::new();
    for (idx, display) in displays.iter().enumerate() {
        // The stderr line lands in the app log, so a person testing with a
        // fresh monitor leaves the exact frames behind.
        let label = format!("screenshot-{idx}");
        let (x, y, width, height) = logical_frame(display);
        eprintln!(
            "[screenshot] display {idx}: capture=({},{} {}x{}) scale={} place=({},{} {width}x{height})",
            display.origin_x, display.origin_y, display.width, display.height, display.scale, x, y,
        );
        let url = WebviewUrl::App("screenshot.html".into());
        let built = WebviewWindowBuilder::new(&app, label.clone(), url)
            .title("Agent Doctor")
            .position(x, y)
            .inner_size(width, height)
            .decorations(false)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .always_on_top(true)
            .visible_on_all_workspaces(true)
            .skip_taskbar(true)
            .shadow(false)
            .background_color(tauri::webview::Color(0, 0, 0, 0))
            .accept_first_mouse(true)
            .visible(false)
            .build();
        #[cfg_attr(not(windows), allow(unused_variables))]
        let Ok(win) = built
        else {
            eprintln!("[screenshot] display {idx}: picker window failed");
            continue;
        };
        // Windows: the builder converts logical through whichever scale the
        // window starts on, so re-pin with raw pixels once it exists.
        #[cfg(windows)]
        {
            let _ = win.set_position(tauri::PhysicalPosition::new(
                display.origin_x.round() as i32,
                display.origin_y.round() as i32,
            ));
        }
        #[cfg(target_os = "macos")]
        float_above_every_space(&win);
        overlays.push(Overlay {
            label,
            display: *display,
        });
    }
    if overlays.is_empty() {
        return Err("could not capture any display".to_string());
    }
    *REGION.lock().unwrap_or_else(|err| err.into_inner()) = Some(RegionSession {
        initiator,
        overlays,
    });
    Ok(())
}

/// The overlay finished a drag: crop the frozen shot and report back to the
/// chat that asked. The dim layer is never captured — it only ever covered a
/// picture of the past.
#[tauri::command]
pub async fn complete_region_capture_command(
    app: AppHandle,
    window: WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let session = take_region().ok_or_else(|| "no region picker is open".to_string())?;
    let display = session
        .overlays
        .iter()
        .find(|overlay| overlay.label == window.label())
        .map(|overlay| overlay.display)
        .ok_or_else(|| "unknown screenshot overlay".to_string())?;
    let initiator = session.initiator.clone();
    // Drop the dim layer first so it never lands in the shot, then grab the
    // live screen — the space the person landed on is the one they picked on.
    hide_overlays(&app, &session.overlays);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let captured = tauri::async_runtime::spawn_blocking(move || {
        crate::computer_hand::capture_screen_region(display, x, y, width, height)
    })
    .await
    .map_err(|err| err.to_string());
    close_overlays(&app, Some(session));
    let _ = match captured {
        Ok(Ok(path)) => app.emit_to(
            initiator.clone(),
            "screenshot-captured",
            ScreenCapture { path },
        ),
        Ok(Err(message)) => app.emit_to(initiator.clone(), "screenshot-failed", message),
        Err(message) => app.emit_to(initiator.clone(), "screenshot-failed", message),
    };
    if let Some(chat) = app.get_webview_window(&initiator) {
        let _ = chat.set_focus();
    }
    Ok(())
}

/// Drop the picker without a shot.
#[tauri::command]
pub fn cancel_region_capture_command(app: AppHandle) -> Result<(), String> {
    if let Some(session) = take_region() {
        let initiator = session.initiator.clone();
        close_overlays(&app, Some(session));
        let _ = app.emit_to(initiator.clone(), "screenshot-cancelled", ());
        if let Some(chat) = app.get_webview_window(&initiator) {
            let _ = chat.set_focus();
        }
    }
    Ok(())
}

/// Drop the dim layer without a shot, before the live capture runs.
fn hide_overlays(app: &AppHandle, overlays: &[Overlay]) {
    for overlay in overlays {
        if let Some(win) = app.get_webview_window(&overlay.label) {
            let _ = win.hide();
        }
    }
}

/// Overlay frames in logical points; Windows displays carry pixels, so the
/// scale converts. macOS points are already logical.
fn logical_frame(display: &DisplayBox) -> (f64, f64, f64, f64) {
    #[cfg(windows)]
    {
        let scale = if display.scale > 0.0 {
            display.scale
        } else {
            1.0
        };
        (
            display.place_x,
            display.place_y,
            display.width / scale,
            display.height / scale,
        )
    }
    #[cfg(not(windows))]
    {
        (
            display.place_x,
            display.place_y,
            display.width,
            display.height,
        )
    }
}

fn close_overlays(app: &AppHandle, session: Option<RegionSession>) {
    let Some(session) = session else {
        return;
    };
    for overlay in &session.overlays {
        if let Some(win) = app.get_webview_window(&overlay.label) {
            let _ = win.close();
        }
    }
}

/// The picker must survive Space switches and full-screen apps, so give its
/// window the same native treatment the island chip gets: screen-saver level
/// plus a join-every-space collection behavior.
#[cfg(target_os = "macos")]
fn float_above_every_space(window: &tauri::WebviewWindow) {
    use objc2_app_kit::{NSScreenSaverWindowLevel, NSWindow, NSWindowCollectionBehavior};

    let Ok(ptr) = window.ns_window() else {
        return;
    };
    if ptr.is_null() {
        return;
    }
    unsafe {
        let ns: &NSWindow = &*ptr.cast();
        ns.setLevel(NSScreenSaverWindowLevel);
        ns.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::CanJoinAllApplications,
        );
        ns.orderFrontRegardless();
    }
}
