use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::computer_hand_common::DisplayBox;

#[derive(Serialize, Clone)]
pub struct ScreenCapture {
    pub path: String,
}

/// One overlay: which display it sits on and the frozen full-screen shot it
/// shows. The picker drags on the frozen picture, so the dim layer can never
/// land in the result and what you saw is what you get.
#[derive(Clone)]
struct Overlay {
    label: String,
    display: DisplayBox,
    shot: String,
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
        // One bad display must not take the whole picker down; skip it and
        // keep the rest. The stderr line lands in the app log, so a person
        // testing with a fresh monitor leaves the exact frames behind.
        let Ok(shot) = crate::computer_hand::capture_display_full(*display) else {
            eprintln!("[screenshot] display {idx}: freeze failed");
            continue;
        };
        let label = format!("screenshot-{idx}");
        let (x, y, width, height) = logical_frame(display);
        eprintln!(
            "[screenshot] display {idx}: capture=({},{} {}x{}) scale={} place=({},{} {width}x{height})",
            display.origin_x, display.origin_y, display.width, display.height, display.scale, x, y,
        );
        let url = WebviewUrl::App(format!("screenshot.html#shot={shot}").into());
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
            shot,
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
    let overlay = session
        .overlays
        .iter()
        .find(|overlay| overlay.label == window.label())
        .cloned()
        .ok_or_else(|| "unknown screenshot overlay".to_string())?;
    let initiator = session.initiator.clone();
    let cropped =
        tauri::async_runtime::spawn_blocking(move || crop_shot(&overlay, x, y, width, height))
            .await
            .map_err(|err| err.to_string());
    close_overlays(&app, Some(session));
    let _ = match cropped {
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

/// Cut the picked rect out of the frozen shot. The rect arrives in logical
/// points from the picker; the shot is stored in this platform's capture
/// unit. The scale comes from the shot itself — the reported display scale
/// has been wrong on modern macOS, and points against a 2x image would crop
/// only the top-left quarter of the selection.
#[cfg(any(target_os = "macos", windows))]
fn crop_shot(overlay: &Overlay, x: f64, y: f64, width: f64, height: f64) -> Result<String, String> {
    let image = image::open(&overlay.shot)
        .map_err(|err| format!("could not read the frozen shot: {err}"))?;
    let scale = if overlay.display.width > 0.0 {
        (image.width() as f64 / overlay.display.width).max(1.0)
    } else {
        1.0
    };
    let pixel = |value: f64| (value * scale).round() as i64;
    let left = pixel(x).clamp(0, i64::from(image.width()) - 1);
    let top = pixel(y).clamp(0, i64::from(image.height()) - 1);
    let crop_width = pixel(width).clamp(1, i64::from(image.width()) - left);
    let crop_height = pixel(height).clamp(1, i64::from(image.height()) - top);
    let cropped = image.crop_imm(
        left as u32,
        top as u32,
        crop_width as u32,
        crop_height as u32,
    );
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // Same folder and name pattern as the platform's own shots, so the asset
    // scope already lets the chat show the thumbnail.
    #[cfg(target_os = "macos")]
    let out = format!("/tmp/agent-doctor-screen-{stamp}.png");
    #[cfg(windows)]
    let out = std::env::temp_dir()
        .join(format!("agent-doctor-screen-{stamp}.png"))
        .display()
        .to_string();
    cropped
        .save(&out)
        .map_err(|err| format!("could not save the shot: {err}"))?;
    Ok(out)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn crop_shot(overlay: &Overlay, x: f64, y: f64, width: f64, height: f64) -> Result<String, String> {
    let _ = (overlay, x, y, width, height);
    Err("the screen is captured on macOS".to_string())
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
