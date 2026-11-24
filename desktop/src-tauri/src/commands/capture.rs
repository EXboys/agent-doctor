#[cfg(target_os = "macos")]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
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
    from_external_space: bool,
    /// Mac: the shot stays on the top bar, and the conversation window must not
    /// pull the user onto its desktop.
    stay_on_island: bool,
    overlays: Vec<Overlay>,
}

static REGION: Mutex<Option<RegionSession>> = Mutex::new(None);
/// True between hiding the dim layer and closing the picker, so the space
/// follower never orders the dim layer back in front of `screencapture`.
static CAPTURING: AtomicBool = AtomicBool::new(false);
/// Bumped whenever a stay-on-this-desktop capture starts or is cancelled early,
/// so a delayed restore cannot run in the middle of the next capture.
#[cfg(target_os = "macos")]
static STAY_GEN: AtomicU64 = AtomicU64::new(0);
/// Holds a shot taken while the Ask window was closed until `chat.html` boots.
static PENDING_SHOT: Mutex<Option<String>> = Mutex::new(None);

fn take_region() -> Option<RegionSession> {
    REGION.lock().ok()?.take()
}

pub(crate) fn is_region_capture_open() -> bool {
    REGION.lock().ok().is_some_and(|guard| guard.is_some())
}

fn normalize_initiator(label: &str) -> (String, bool) {
    if label.is_empty()
        || label == "tray"
        || label == "island"
        || label.starts_with("island-")
        || label.starts_with("screenshot-")
    {
        ("ask".to_string(), true)
    } else {
        (label.to_string(), false)
    }
}

/// Open a live see-through region picker on every display. Safe to call from
/// the chat window, the top island on another Space, or the menu-bar tray.
pub(crate) fn start_region_capture(app: &AppHandle, requester: &str) -> Result<(), String> {
    eprintln!("[screenshot] picker requested by {requester}");
    CAPTURING.store(false, Ordering::SeqCst);
    let (initiator, from_external_space) = normalize_initiator(requester);
    let stay_on_island = stay_with_island(app, requester, from_external_space);
    // Before any new page is created. Creating one activates the app, and
    // hiding the top bar then hands the keyboard to the conversation window.
    // Either of those swipes to that window's desktop.
    if stay_on_island {
        begin_stay(app);
    } else {
        release_stay(app);
    }
    close_overlays(app, take_region());
    let displays = crate::computer_hand::capture_screen_displays();
    if displays.is_empty() {
        if stay_on_island {
            end_stay(app);
        }
        return Err("no display found".to_string());
    }
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
        let built = WebviewWindowBuilder::new(app, label.clone(), url)
            .title("Agent Doctor")
            .position(x, y)
            .inner_size(width, height)
            .decorations(false)
            .transparent(true)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .always_on_top(true)
            .visible_on_all_workspaces(true)
            .skip_taskbar(true)
            .shadow(false)
            .background_color(tauri::webview::Color(0, 0, 0, 0))
            .accept_first_mouse(true)
            .focused(false)
            .visible(false)
            .build();
        #[cfg_attr(not(any(windows, target_os = "macos")), allow(unused_variables))]
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
        attach_picker_panel(app, &win, &label, display);
        overlays.push(Overlay {
            label,
            display: *display,
        });
    }
    if overlays.is_empty() {
        if stay_on_island {
            end_stay(app);
        }
        return Err("could not capture any display".to_string());
    }
    *REGION.lock().unwrap_or_else(|err| err.into_inner()) = Some(RegionSession {
        initiator,
        from_external_space,
        stay_on_island,
        overlays,
    });
    crate::island::on_capture_state_changed(app);
    #[cfg(target_os = "macos")]
    ensure_picker_space_follow(app);
    Ok(())
}

/// Async command that hops onto the UI thread after WebView2's IPC callback
/// returns. Building `WebviewWindow` synchronously inside `WebResourceRequested`
/// / `WebMessageReceived` deadlocks WebView2 on Windows.
#[tauri::command]
pub async fn start_region_capture_command(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<(), String> {
    let requester = window.label().to_string();
    let app_for_ui = app.clone();
    crate::windows::run_on_main_thread(&app, move || start_region_capture(&app_for_ui, &requester))
        .await
}

/// The overlay finished a drag: grab the live screen on the Space the person
/// landed on and deliver the image back to the conversation.
#[tauri::command]
pub async fn complete_region_capture_command(
    app: AppHandle,
    window: WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    CAPTURING.store(true, Ordering::SeqCst);
    let session = match take_region() {
        Some(session) => session,
        None => {
            CAPTURING.store(false, Ordering::SeqCst);
            return Err("no region picker is open".to_string());
        }
    };
    let Some(display) = session
        .overlays
        .iter()
        .find(|overlay| overlay.label == window.label())
        .map(|overlay| overlay.display)
    else {
        let stay = session.stay_on_island;
        close_overlays(&app, Some(session));
        CAPTURING.store(false, Ordering::SeqCst);
        crate::island::on_capture_state_changed(&app);
        if stay {
            end_stay(&app);
        }
        return Err("unknown screenshot overlay".to_string());
    };
    let initiator = session.initiator.clone();
    let from_external = session.from_external_space;
    let stay = session.stay_on_island;
    eprintln!("[screenshot] deliver stay={stay} external={from_external}");
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
    CAPTURING.store(false, Ordering::SeqCst);
    crate::island::on_capture_state_changed(&app);
    match &captured {
        Ok(Ok(path)) => eprintln!("[screenshot] captured {path}"),
        Ok(Err(message)) => eprintln!("[screenshot] capture failed: {message}"),
        Err(message) => eprintln!("[screenshot] capture panicked: {message}"),
    }
    match captured {
        Ok(Ok(path)) if stay => crate::island::present_island_shot(&app, path),
        Ok(Ok(path)) => deliver_captured_shot(&app, &initiator, path),
        Ok(Err(message)) if stay => crate::island::present_island_shot_failure(&app, message),
        Ok(Err(message)) => deliver_failed_shot(&app, &initiator, message),
        Err(message) if stay => crate::island::present_island_shot_failure(&app, message),
        Err(message) => deliver_failed_shot(&app, &initiator, message),
    }
    if stay {
        end_stay(&app);
    }
    Ok(())
}

/// A shot started on the island stays on the island card. On macOS the tray
/// does too, and a shot started in the conversation stays when that window is
/// on another desktop. A shot started in the conversation on this desktop
/// comes back to the conversation. On Windows the tray still opens it there.
fn stay_with_island(app: &AppHandle, requester: &str, from_external: bool) -> bool {
    if !crate::island::island_bar_enabled(app) {
        return false;
    }
    let outside_chat = if cfg!(target_os = "macos") {
        from_external
    } else {
        requester == "island" || requester.starts_with("island-")
    };
    shot_stays_on_island(outside_chat, chat_is_on_another_desktop(app))
}

/// `from_external` is an island or tray shot. `chat_elsewhere` is a macOS
/// conversation window sitting on another desktop.
fn shot_stays_on_island(from_external: bool, chat_elsewhere: bool) -> bool {
    from_external || chat_elsewhere
}

fn chat_is_on_another_desktop(app: &AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    {
        !ask_on_active_space(app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        false
    }
}

#[cfg(target_os = "macos")]
fn ask_on_active_space(app: &AppHandle) -> bool {
    if objc2::MainThreadMarker::new().is_some() {
        return ask_on_active_space_main(app);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        let _ = tx.send(ask_on_active_space_main(&app));
    });
    rx.recv_timeout(Duration::from_millis(400)).unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn ask_on_active_space_main(app: &AppHandle) -> bool {
    use objc2_app_kit::NSWindow;

    let Some(window) = app.get_webview_window("ask") else {
        return false;
    };
    if window.is_minimized().unwrap_or(false) || !window.is_visible().unwrap_or(false) {
        return false;
    }
    let Ok(ptr) = window.ns_window() else {
        return false;
    };
    if ptr.is_null() {
        return false;
    }
    let ns: &NSWindow = unsafe { &*ptr.cast() };
    ns.isOnActiveSpace()
}

/// Stop the conversation windows from becoming key, and stop the app from
/// activating, before the top bar is ordered out. Ordering out the key bar
/// otherwise makes macOS activate whichever window is on the other desktop.
#[cfg(target_os = "macos")]
fn begin_stay(app: &AppHandle) {
    STAY_GEN.fetch_add(1, Ordering::SeqCst);
    crate::macos_activation::set_suppressed(true);
    let app = app.clone();
    run_main_sync(&app.clone(), move || {
        set_regular_windows_focusable(&app, false);
        deactivate_app();
    });
    eprintln!("[screenshot] stay on this desktop");
}

#[cfg(not(target_os = "macos"))]
fn begin_stay(app: &AppHandle) {
    let _ = app;
}

/// Give the conversation windows their keyboard back once the desktop has
/// settled. A restore that belongs to an older capture does nothing.
#[cfg(target_os = "macos")]
fn end_stay(app: &AppHandle) {
    let gen = STAY_GEN.load(Ordering::SeqCst);
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        if STAY_GEN.load(Ordering::SeqCst) != gen || is_region_capture_open() {
            return;
        }
        let app = app.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = app.clone().run_on_main_thread(move || {
            if STAY_GEN.load(Ordering::SeqCst) == gen && !is_region_capture_open() {
                set_regular_windows_focusable(&app, true);
                crate::macos_activation::set_suppressed(false);
            }
            let _ = tx.send(());
        });
        let _ = rx.recv_timeout(Duration::from_millis(500));
    });
}

#[cfg(not(target_os = "macos"))]
fn end_stay(app: &AppHandle) {
    let _ = app;
}

/// A capture that should come back to the conversation cancels any pending
/// "stay here" restore and lets activation through again.
#[cfg(target_os = "macos")]
fn release_stay(app: &AppHandle) {
    STAY_GEN.fetch_add(1, Ordering::SeqCst);
    crate::macos_activation::set_suppressed(false);
    let app = app.clone();
    run_main_sync(&app.clone(), move || {
        set_regular_windows_focusable(&app, true)
    });
}

#[cfg(not(target_os = "macos"))]
fn release_stay(app: &AppHandle) {
    let _ = app;
}

#[cfg(target_os = "macos")]
fn set_regular_windows_focusable(app: &AppHandle, focusable: bool) {
    use objc2_app_kit::NSWindow;

    let windows: Vec<_> = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| {
            label != "island" && !label.starts_with("island-") && !label.starts_with("screenshot-")
        })
        .map(|(_, window)| window)
        .collect();
    for window in &windows {
        let _ = window.set_focusable(focusable);
    }
    if focusable {
        return;
    }
    // Resign only after every conversation window has refused the keyboard.
    // Resigning the first one early would hand the key to the next one, and
    // that handoff is what changes desktops.
    for window in &windows {
        let Ok(ptr) = window.ns_window() else {
            continue;
        };
        if ptr.is_null() {
            continue;
        }
        let ns: &NSWindow = unsafe { &*ptr.cast() };
        if ns.isKeyWindow() {
            ns.resignKeyWindow();
        }
    }
}

#[cfg(target_os = "macos")]
fn deactivate_app() {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    let Some(cls) = AnyClass::get(c"NSApplication") else {
        return;
    };
    unsafe {
        let app: *mut AnyObject = msg_send![cls, sharedApplication];
        if !app.is_null() {
            let _: () = msg_send![app, deactivate];
        }
    }
}

fn deliver_captured_shot(app: &AppHandle, initiator: &str, path: String) {
    let existed = app.get_webview_window(initiator).is_some();
    if !existed && initiator == "ask" {
        *PENDING_SHOT.lock().unwrap_or_else(|err| err.into_inner()) = Some(path.clone());
    }
    if initiator == "ask" {
        crate::island::bring_ask_forward(app);
    } else if let Some(chat) = app.get_webview_window(initiator) {
        let _ = chat.set_focus();
    }
    if existed {
        let _ = app.emit_to(initiator, "screenshot-captured", ScreenCapture { path });
    }
}

fn deliver_failed_shot(app: &AppHandle, initiator: &str, message: String) {
    if initiator == "ask" && app.get_webview_window("ask").is_some() {
        crate::island::bring_ask_forward(app);
    } else if let Some(chat) = app.get_webview_window(initiator) {
        let _ = chat.set_focus();
    }
    let _ = app.emit_to(initiator, "screenshot-failed", message);
}

/// Drain a screenshot captured before `chat.html` finished booting.
#[tauri::command]
pub fn take_pending_screenshot_command() -> Option<String> {
    PENDING_SHOT.lock().ok()?.take()
}

/// Drop the picker without a shot. Runs on the UI thread outside WebView2's
/// IPC callback so an overlay never destroys itself mid-callback.
#[tauri::command]
pub async fn cancel_region_capture_command(app: AppHandle) -> Result<(), String> {
    let app_for_ui = app.clone();
    crate::windows::run_on_main_thread(&app, move || {
        CAPTURING.store(false, Ordering::SeqCst);
        if let Some(session) = take_region() {
            eprintln!("[screenshot] cancelled");
            let initiator = session.initiator.clone();
            let from_external_space = session.from_external_space;
            let stay = session.stay_on_island;
            close_overlays(&app_for_ui, Some(session));
            crate::island::on_capture_state_changed(&app_for_ui);
            if stay {
                end_stay(&app_for_ui);
            }
            let _ = app_for_ui.emit_to(initiator.clone(), "screenshot-cancelled", ());
            // If they started from the island/tray or swiped to another Space on
            // macOS, Esc should leave them on the Space they are looking at rather
            // than yanking them back to the chat window.
            if !from_external_space && !cfg!(target_os = "macos") {
                if let Some(chat) = app_for_ui.get_webview_window(&initiator) {
                    let _ = chat.set_focus();
                }
            }
        }
        Ok(())
    })
    .await
}

/// Drop the dim layer without a shot, before the live capture runs.
fn hide_overlays(app: &AppHandle, overlays: &[Overlay]) {
    #[cfg(target_os = "macos")]
    hide_picker_panels_sync(app, overlays);
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
    #[cfg(target_os = "macos")]
    teardown_picker_panels_sync(app, &session.overlays);
    for overlay in &session.overlays {
        if let Some(win) = app.get_webview_window(&overlay.label) {
            let _ = win.close();
        }
    }
}

/// The picker page calls this right after its stylesheet loads so the overlay
/// appears without a white flash and asserts cross-Space focus.
#[tauri::command]
pub fn pin_picker_command(app: AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    reveal_picker_panel_sync(&app, window.label());
    #[cfg(not(target_os = "macos"))]
    let _ = (app, window);
    Ok(())
}

// ---------------------------------------------------------------------------
// macOS non-activating NSPanel host
//
// macOS WindowServer drops regular NSWindows whenever another app owns a
// full-screen Space, even at NSScreenSaverWindowLevel. Hosting the picker
// webview inside a Borderless | NonactivatingPanel NSPanel (the same mechanism
// the top Island uses) lets the overlay float over every Desktop and every
// full-screen app without activating Agent Doctor or yanking Spaces back.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
objc2::define_class!(
    #[unsafe(super(
        objc2_app_kit::NSPanel,
        objc2_app_kit::NSWindow,
        objc2_app_kit::NSResponder,
        objc2::runtime::NSObject
    ))]
    #[thread_kind = objc2::MainThreadOnly]
    #[name = "AgentDoctorPickerPanel"]
    struct PickerPanel;

    impl PickerPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            false
        }
    }
);

#[cfg(target_os = "macos")]
struct PickerPanelEntry {
    label: String,
    display: DisplayBox,
    panel: *mut objc2_app_kit::NSPanel,
    revealed: bool,
}

#[cfg(target_os = "macos")]
unsafe impl Send for PickerPanelEntry {}

#[cfg(target_os = "macos")]
static PICKER_PANELS: Mutex<Vec<PickerPanelEntry>> = Mutex::new(Vec::new());

#[cfg(target_os = "macos")]
fn run_main_sync(app: &AppHandle, f: impl FnOnce() + Send + 'static) {
    if objc2::MainThreadMarker::new().is_some() {
        f();
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = app.run_on_main_thread(move || {
        f();
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(Duration::from_millis(500));
}

#[cfg(target_os = "macos")]
fn cocoa_panel_rect(
    display: &DisplayBox,
    mtm: objc2::MainThreadMarker,
) -> objc2_foundation::NSRect {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let screens = NSScreen::screens(mtm);
    let mut cocoa_top = 0.0f64;
    for screen in screens.iter() {
        let frame = screen.frame();
        cocoa_top = cocoa_top.max(frame.origin.y + frame.size.height);
    }
    NSRect {
        origin: NSPoint {
            x: display.place_x,
            y: cocoa_top - display.place_y - display.height,
        },
        size: NSSize {
            width: display.width,
            height: display.height,
        },
    }
}

#[cfg(target_os = "macos")]
fn apply_panel_space_behavior(panel_win: &objc2_app_kit::NSWindow) {
    use objc2_app_kit::{NSScreenSaverWindowLevel, NSWindowCollectionBehavior};

    panel_win.setHidesOnDeactivate(false);
    panel_win.setCanHide(false);
    panel_win.setLevel(NSScreenSaverWindowLevel);
    panel_win.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::CanJoinAllApplications,
    );
}

#[cfg(target_os = "macos")]
fn attach_picker_panel(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    label: &str,
    display: &DisplayBox,
) {
    let win = window.clone();
    let label = label.to_string();
    let display = *display;
    run_main_sync(app, move || {
        use objc2::rc::Retained;
        use objc2::runtime::AnyClass;
        use objc2::{msg_send, MainThreadMarker};
        use objc2_app_kit::{NSPanel, NSView, NSWindow, NSWindowStyleMask};

        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Ok(ptr) = win.ns_window() else {
            return;
        };
        if ptr.is_null() {
            return;
        }
        let host: &NSWindow = unsafe { &*ptr.cast() };
        let custom: Retained<PickerPanel> =
            unsafe { msg_send![objc2::MainThreadOnly::alloc(mtm), init] };
        let panel: Retained<NSPanel> = Retained::into_super(custom);
        let panel_win: &NSWindow = unsafe { &*(&*panel as *const NSPanel as *const NSWindow) };
        unsafe {
            panel_win.setReleasedWhenClosed(false);
        }
        panel.setFloatingPanel(true);
        panel.setBecomesKeyOnlyIfNeeded(false);
        panel.setWorksWhenModal(true);
        panel_win
            .setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
        panel_win.setOpaque(false);
        panel_win.setHasShadow(false);
        panel_win.setMovable(false);
        panel_win.setAcceptsMouseMovedEvents(true);
        panel_win.setIgnoresMouseEvents(false);
        unsafe {
            if let Some(color_cls) = AnyClass::get(c"NSColor") {
                let color: *mut objc2::runtime::AnyObject = msg_send![color_cls, clearColor];
                let _: () = msg_send![panel_win, setBackgroundColor: color];
            }
        }
        apply_panel_space_behavior(panel_win);
        panel_win.setFrame_display(cocoa_panel_rect(&display, mtm), false);
        if let Some(view) = host.contentView() {
            panel_win.setContentView(Some(&view));
            host.setContentView(Some(&NSView::new(mtm)));
        }
        host.orderOut(None);
        panel_win.orderOut(None);

        let raw = Retained::into_raw(panel);
        let mut guard = PICKER_PANELS.lock().unwrap_or_else(|err| err.into_inner());
        guard.retain(|entry| entry.label != label);
        guard.push(PickerPanelEntry {
            label,
            display,
            panel: raw,
            revealed: false,
        });
    });
}

#[cfg(target_os = "macos")]
fn reveal_picker_panel_sync(app: &AppHandle, label: &str) {
    let label = label.to_string();
    run_main_sync(app, move || {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSWindow;

        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let mut guard = PICKER_PANELS.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(entry) = guard.iter_mut().find(|item| item.label == label) {
            if entry.panel.is_null() {
                return;
            }
            entry.revealed = true;
            let panel_win: &NSWindow = unsafe { &*entry.panel.cast() };
            apply_panel_space_behavior(panel_win);
            panel_win.setFrame_display(cocoa_panel_rect(&entry.display, mtm), true);
            panel_win.orderFrontRegardless();
        }
        drop(guard);
        focus_panel_under_cursor();
    });
}

/// Keep every revealed picker panel above fullscreen Spaces while a three-finger
/// swipe moves between desktops, and keep keyboard focus (`Esc` / `Enter`) on
/// whichever monitor holds the pointer.
#[cfg(target_os = "macos")]
fn refresh_picker_panels_on_main() {
    use objc2_app_kit::NSWindow;

    if CAPTURING.load(Ordering::SeqCst) || !is_region_capture_open() {
        return;
    }
    let guard = PICKER_PANELS.lock().unwrap_or_else(|err| err.into_inner());
    let mut any_revealed = false;
    for entry in guard.iter() {
        if !entry.revealed || entry.panel.is_null() {
            continue;
        }
        any_revealed = true;
        let panel_win: &NSWindow = unsafe { &*entry.panel.cast() };
        apply_panel_space_behavior(panel_win);
        panel_win.orderFrontRegardless();
    }
    drop(guard);
    if any_revealed {
        focus_panel_under_cursor();
    }
}

#[cfg(target_os = "macos")]
fn focus_panel_under_cursor() {
    use objc2_app_kit::{NSEvent, NSWindow};

    let point = NSEvent::mouseLocation();
    let guard = PICKER_PANELS.lock().unwrap_or_else(|err| err.into_inner());
    let mut target: Option<*mut NSWindow> = None;
    for entry in guard.iter() {
        if !entry.revealed || entry.panel.is_null() {
            continue;
        }
        let panel_win: &NSWindow = unsafe { &*entry.panel.cast() };
        if target.is_none() {
            target = Some(entry.panel.cast());
        }
        let frame = panel_win.frame();
        if point.x >= frame.origin.x
            && point.x < frame.origin.x + frame.size.width
            && point.y >= frame.origin.y
            && point.y < frame.origin.y + frame.size.height
        {
            target = Some(entry.panel.cast());
            break;
        }
    }
    drop(guard);
    if let Some(ptr) = target {
        let panel_win = unsafe { &*ptr };
        if !panel_win.isKeyWindow() {
            panel_win.makeKeyWindow();
        }
    }
}

#[cfg(target_os = "macos")]
fn ensure_picker_space_follow(app: &AppHandle) {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        while is_region_capture_open() {
            std::thread::sleep(Duration::from_millis(200));
            if CAPTURING.load(Ordering::SeqCst) || !is_region_capture_open() {
                continue;
            }
            let _ = app.run_on_main_thread(refresh_picker_panels_on_main);
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}

#[cfg(target_os = "macos")]
fn hide_picker_panels_sync(app: &AppHandle, overlays: &[Overlay]) {
    let labels: Vec<String> = overlays.iter().map(|o| o.label.clone()).collect();
    run_main_sync(app, move || {
        use objc2_app_kit::NSWindow;

        let mut guard = PICKER_PANELS.lock().unwrap_or_else(|err| err.into_inner());
        for entry in guard.iter_mut() {
            if !labels.iter().any(|label| label == &entry.label) || entry.panel.is_null() {
                continue;
            }
            entry.revealed = false;
            let panel_win: &NSWindow = unsafe { &*entry.panel.cast() };
            if panel_win.isKeyWindow() {
                panel_win.resignKeyWindow();
            }
            panel_win.orderOut(None);
        }
    });
}

#[cfg(target_os = "macos")]
fn teardown_picker_panels_sync(app: &AppHandle, overlays: &[Overlay]) {
    let labels: Vec<String> = overlays.iter().map(|o| o.label.clone()).collect();
    let handle = app.clone();
    run_main_sync(app, move || {
        use objc2::rc::Retained;
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSView, NSWindow};

        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let mut removed = Vec::new();
        {
            let mut guard = PICKER_PANELS.lock().unwrap_or_else(|err| err.into_inner());
            let mut i = 0;
            while i < guard.len() {
                if labels.iter().any(|label| label == &guard[i].label) {
                    removed.push(guard.remove(i));
                } else {
                    i += 1;
                }
            }
        }
        for entry in removed {
            if entry.panel.is_null() {
                continue;
            }
            let panel_win: &NSWindow = unsafe { &*entry.panel.cast() };
            if panel_win.isKeyWindow() {
                panel_win.resignKeyWindow();
            }
            panel_win.orderOut(None);
            if let Some(win) = handle.get_webview_window(&entry.label) {
                if let Ok(ptr) = win.ns_window() {
                    if !ptr.is_null() {
                        let host: &NSWindow = unsafe { &*ptr.cast() };
                        if let Some(view) = panel_win.contentView() {
                            host.setContentView(Some(&view));
                            panel_win.setContentView(Some(&NSView::new(mtm)));
                        }
                        // Putting the view back can order the host forward on the
                        // Space where the chat lives. Keep it out.
                        host.orderOut(None);
                    }
                }
            }
            if let Some(panel) = unsafe { Retained::from_raw(entry.panel) } {
                panel.close();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::normalize_initiator;

    #[test]
    fn island_and_tray_route_captures_to_ask_window() {
        assert_eq!(normalize_initiator("ask"), ("ask".to_string(), false));
        assert_eq!(normalize_initiator("island"), ("ask".to_string(), true));
        assert_eq!(normalize_initiator("island-1"), ("ask".to_string(), true));
        assert_eq!(normalize_initiator("tray"), ("ask".to_string(), true));
    }

    #[test]
    fn shot_stays_on_the_island_when_started_there_or_chat_is_on_another_desktop() {
        assert!(super::shot_stays_on_island(true, false));
        assert!(super::shot_stays_on_island(true, true));
        assert!(super::shot_stays_on_island(false, true));
        assert!(!super::shot_stays_on_island(false, false));
    }
}
