//! macOS-only notch: a small window at the top of every screen.
//!
//! The Ask page publishes what the session is doing. The normal windows stay
//! put until a browser tool is running; then this module hides them so the page
//! can use the screen. The notch itself stays up on each screen.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
};

const ISLAND_LABEL: &str = "island";
const SCREEN_FOLLOW: Duration = Duration::from_millis(700);
const CHIP_WIDTH: f64 = 156.0;
const CHIP_HEIGHT: f64 = 28.0;
const PEEK_WIDTH: f64 = 640.0;
const PEEK_HEIGHT: f64 = 460.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chrome {
    Hidden,
    Pill,
    Peek,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandPending {
    pub kind: String,
    pub request_id: String,
    pub session_id: String,
    pub title: String,
    pub detail: String,
    /// Structured question payload. Empty for a plain reply.
    #[serde(default)]
    pub input_json: String,
    #[serde(default)]
    pub tool: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandSnapshotInput {
    pub active: bool,
    pub browser: bool,
    pub composing: bool,
    pub title: String,
    pub detail: String,
    #[serde(default = "one_row")]
    pub rows: u32,
    #[serde(default)]
    pub pending: Option<IslandPending>,
}

fn one_row() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandView {
    shown: bool,
    expanded: bool,
    title: String,
    detail: String,
    attention: bool,
    pending: Option<IslandPending>,
}

#[derive(Debug, Clone)]
struct Snapshot {
    active: bool,
    browser: bool,
    composing: bool,
    title: String,
    detail: String,
    rows: u32,
    pending: Option<IslandPending>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            active: false,
            browser: false,
            composing: false,
            title: String::new(),
            detail: String::new(),
            rows: 1,
            pending: None,
        }
    }
}

impl From<IslandSnapshotInput> for Snapshot {
    fn from(input: IslandSnapshotInput) -> Self {
        Self {
            active: input.active,
            browser: input.browser,
            composing: input.composing,
            title: input.title,
            detail: input.detail,
            rows: input.rows.max(1),
            pending: input.pending,
        }
    }
}

struct Inner {
    snapshot: Snapshot,
    hovering: bool,
    /// The person clicked outside, so a question stays a pill until they come back.
    folded: bool,
    /// They clicked the bar, so the card stays open until the next outside click.
    opened: bool,
    /// One conversation is open inside the card, so it takes the full fixed height.
    reading: bool,
    /// User asked to see the full conversation during this browser turn.
    hold_open: bool,
    /// User asked to keep the conversation window open until the turn ends.
    pinned: bool,
    /// Ask was hidden so a browser tool could use the screen.
    parked_ask: bool,
    /// Main window was hidden for the same reason.
    parked_main: bool,
    applying: bool,
    dirty: bool,
    /// Last placed size and screen origin, so status text can update without resizing.
    placed: HashMap<String, (i32, i32, i32, i32)>,
    /// Monitor work-area origins, so a new screen gets its own notch.
    screen_sig: Vec<(i32, i32)>,
    last_view: IslandView,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            snapshot: Snapshot::default(),
            hovering: false,
            folded: false,
            opened: false,
            reading: false,
            hold_open: false,
            pinned: false,
            parked_ask: false,
            parked_main: false,
            applying: false,
            dirty: false,
            placed: HashMap::new(),
            screen_sig: Vec::new(),
            last_view: IslandView {
                shown: false,
                expanded: false,
                title: String::new(),
                detail: String::new(),
                attention: false,
                pending: None,
            },
        }
    }
}

pub struct IslandHost {
    inner: Mutex<Inner>,
}

impl Default for IslandHost {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }
}

/// The island stays up on macOS. Hover peeks. A click on the bar keeps it open.
/// A question opens it unless the person is typing or they just clicked somewhere else.
pub(crate) fn island_chrome(
    macos: bool,
    hovering: bool,
    attention: bool,
    composing: bool,
    folded: bool,
    opened: bool,
) -> Chrome {
    if !macos {
        return Chrome::Hidden;
    }
    if hovering || opened || (attention && !composing && !folded) {
        Chrome::Peek
    } else {
        Chrome::Pill
    }
}

/// Only a running browser tool hides the normal windows. The island does not.
pub(crate) fn should_park_windows(
    active: bool,
    browser: bool,
    composing: bool,
    hold_open: bool,
    pinned: bool,
) -> bool {
    if pinned || hold_open || composing {
        return false;
    }
    active && browser
}

/// Tall enough for the conversations on screen, and no taller.
pub(crate) fn peek_height(rows: u32, pending: bool) -> f64 {
    let count = rows.clamp(1, 6) as f64;
    // Room for the reply field or allow / deny under the row that is waiting.
    let actions = if pending { 72.0 } else { 0.0 };
    (28.0 + count * 84.0 + actions + 12.0).clamp(120.0, PEEK_HEIGHT)
}

fn chrome_size(chrome: Chrome, rows: u32, pending: bool) -> (f64, f64) {
    match chrome {
        Chrome::Hidden | Chrome::Pill => (CHIP_WIDTH, CHIP_HEIGHT),
        Chrome::Peek => (PEEK_WIDTH, peek_height(rows, pending)),
    }
}

fn view_for(chrome: Chrome, snapshot: &Snapshot) -> IslandView {
    let shown = chrome != Chrome::Hidden;
    IslandView {
        shown,
        expanded: chrome == Chrome::Peek,
        title: snapshot.title.clone(),
        detail: snapshot.detail.clone(),
        attention: snapshot.pending.is_some(),
        pending: if shown {
            snapshot.pending.clone()
        } else {
            None
        },
    }
}

pub(crate) fn on_ask_closed(app: &AppHandle) {
    {
        let host = app.state::<IslandHost>();
        let mut guard = host.inner.lock().expect("island");
        guard.snapshot = Snapshot::default();
        guard.pinned = false;
        guard.hold_open = false;
        guard.hovering = false;
        guard.parked_ask = false;
    }
    apply(app);
}

fn apply(app: &AppHandle) {
    if !cfg!(target_os = "macos") {
        return;
    }
    {
        let host = app.state::<IslandHost>();
        let mut guard = host.inner.lock().expect("island");
        if guard.applying {
            guard.dirty = true;
            return;
        }
        guard.applying = true;
        guard.dirty = false;
    }
    loop {
        apply_once(app);
        let host = app.state::<IslandHost>();
        let mut guard = host.inner.lock().expect("island");
        if !guard.dirty {
            guard.applying = false;
            break;
        }
        guard.dirty = false;
    }
}

fn apply_once(app: &AppHandle) {
    let (chrome, snapshot, park, parked_ask, parked_main) = {
        let host = app.state::<IslandHost>();
        let guard = host.inner.lock().expect("island");
        let chrome = island_chrome(
            cfg!(target_os = "macos"),
            guard.hovering,
            guard.snapshot.pending.is_some(),
            guard.snapshot.composing,
            guard.folded,
            guard.opened,
        );
        let park = should_park_windows(
            guard.snapshot.active,
            guard.snapshot.browser,
            guard.snapshot.composing,
            guard.hold_open,
            guard.pinned,
        );
        (
            chrome,
            guard.snapshot.clone(),
            park,
            guard.parked_ask,
            guard.parked_main,
        )
    };

    if chrome == Chrome::Hidden {
        hide_island(app);
    } else {
        show_island(app, chrome);
    }
    if park {
        park_windows(app);
    } else if parked_ask || parked_main {
        reveal_parked(app, false);
    }

    let view = view_for(chrome, &snapshot);
    let changed = {
        let host = app.state::<IslandHost>();
        let mut guard = host.inner.lock().expect("island");
        if guard.last_view == view {
            false
        } else {
            guard.last_view = view.clone();
            true
        }
    };
    if changed {
        for window in island_windows(app) {
            let _ = window.emit("island-view", &view);
        }
    }
}

fn park_windows(app: &AppHandle) {
    park_one(app, "ask", true);
    park_one(app, "main", false);
}

fn park_one(app: &AppHandle, label: &str, ask: bool) {
    let Some(window) = app.get_webview_window(label) else {
        return;
    };
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let _ = window.hide();
    let host = app.state::<IslandHost>();
    let mut guard = host.inner.lock().expect("island");
    if ask {
        guard.parked_ask = true;
    } else {
        guard.parked_main = true;
    }
}

fn reveal_parked(app: &AppHandle, focus_ask: bool) {
    let (parked_ask, parked_main) = {
        let host = app.state::<IslandHost>();
        let guard = host.inner.lock().expect("island");
        (guard.parked_ask, guard.parked_main)
    };
    if parked_main {
        reveal_window(app, "main", false);
    }
    if parked_ask {
        reveal_window(app, "ask", focus_ask);
    }
    let host = app.state::<IslandHost>();
    let mut guard = host.inner.lock().expect("island");
    guard.parked_ask = false;
    guard.parked_main = false;
}

fn reveal_window(app: &AppHandle, label: &str, focus: bool) {
    let Some(window) = app.get_webview_window(label) else {
        return;
    };
    let _ = window.unminimize();
    let _ = window.set_skip_taskbar(false);
    let _ = window.show();
    if focus {
        let _ = window.set_focus();
    }
}

fn hide_island(app: &AppHandle) {
    for window in island_windows(app) {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        }
    }
    let host = app.state::<IslandHost>();
    let mut guard = host.inner.lock().expect("island");
    guard.placed.clear();
}

fn show_island(app: &AppHandle, chrome: Chrome) {
    #[cfg(target_os = "macos")]
    watch_outside_clicks(app);
    let monitors = monitors_for_island(app);
    if monitors.is_empty() {
        return;
    }
    let (rows, pending, reading) = {
        let host = app.state::<IslandHost>();
        let guard = host.inner.lock().expect("island");
        (
            guard.snapshot.rows,
            guard.snapshot.pending.is_some(),
            guard.reading,
        )
    };
    let (_, mut peek_height) = chrome_size(chrome, rows, pending);
    if reading || pending {
        peek_height = PEEK_HEIGHT;
    }
    let mut keep = Vec::new();
    for (index, monitor) in monitors.iter().enumerate() {
        let label = island_label(index);
        let Some(window) = ensure_island_window(app, &label) else {
            continue;
        };
        keep.push(label.clone());
        let Some((screen_x, screen_y, screen_w)) = screen_top(monitor) else {
            continue;
        };
        let anchor = top_anchor(monitor);
        let (width, height, x, y) = if chrome == Chrome::Pill {
            if let Some(anchor) = anchor {
                (
                    anchor.notch_w,
                    anchor.menu_h - 8.0 + CHIP_HEIGHT,
                    screen_x + anchor.notch_x,
                    screen_y,
                )
            } else {
                (
                    CHIP_WIDTH,
                    CHIP_HEIGHT,
                    screen_x + (screen_w - CHIP_WIDTH) / 2.0,
                    screen_y + 22.0,
                )
            }
        } else {
            let width = PEEK_WIDTH.min((screen_w - 32.0).max(320.0));
            (
                width,
                peek_height,
                screen_x + (screen_w - width) / 2.0,
                screen_y,
            )
        };
        let place_key = (
            x.round() as i32,
            y.round() as i32,
            width.round() as i32,
            height.round() as i32,
        );
        let needs_place = {
            let host = app.state::<IslandHost>();
            let guard = host.inner.lock().expect("island");
            guard.placed.get(&label) != Some(&place_key)
        };
        let was_visible = window.is_visible().unwrap_or(false);
        if needs_place || !was_visible {
            present_island(&window);
        }
        if needs_place {
            place_island_frame(&window, x, y, width, height);
            let host = app.state::<IslandHost>();
            let mut guard = host.inner.lock().expect("island");
            guard.placed.insert(label, place_key);
        }
    }
    for window in island_windows(app) {
        if !keep.iter().any(|label| label == window.label()) {
            let _ = window.hide();
        }
    }
    ensure_screen_follow(app);
}

fn island_label(index: usize) -> String {
    if index == 0 {
        ISLAND_LABEL.to_string()
    } else {
        format!("{ISLAND_LABEL}-{index}")
    }
}

fn is_island_label(label: &str) -> bool {
    label == ISLAND_LABEL || label.starts_with(&format!("{ISLAND_LABEL}-"))
}

fn island_windows(app: &AppHandle) -> Vec<tauri::WebviewWindow> {
    app.webview_windows()
        .into_iter()
        .filter(|(label, _)| is_island_label(label))
        .map(|(_, window)| window)
        .collect()
}

fn monitors_for_island(app: &AppHandle) -> Vec<tauri::Monitor> {
    if let Ok(list) = app.available_monitors() {
        if !list.is_empty() {
            return list;
        }
    }
    let anchor = app
        .get_webview_window("ask")
        .or_else(|| app.get_webview_window("main"));
    let Some(window) = anchor else {
        return Vec::new();
    };
    window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())
        .into_iter()
        .collect()
}

struct TopAnchor {
    notch_x: f64,
    notch_w: f64,
    menu_h: f64,
}

fn top_anchor(monitor: &tauri::Monitor) -> Option<TopAnchor> {
    #[cfg(target_os = "macos")]
    {
        return menu_anchor(monitor);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = monitor;
        None
    }
}

#[cfg(target_os = "macos")]
fn menu_anchor(monitor: &tauri::Monitor) -> Option<TopAnchor> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;

    let mtm = MainThreadMarker::new()?;
    let scale = monitor.scale_factor();
    if scale <= 0.0 {
        return None;
    }
    let want_w = monitor.size().width as f64 / scale;
    let want_h = monitor.size().height as f64 / scale;
    let screens = NSScreen::screens(mtm);
    for screen in screens.iter() {
        let frame = screen.frame();
        if (frame.size.width - want_w).abs() > 2.0 || (frame.size.height - want_h).abs() > 2.0 {
            continue;
        }
        let left_area = screen.auxiliaryTopLeftArea();
        let right_area = screen.auxiliaryTopRightArea();
        let left = left_area.size.width;
        let right = right_area.size.width;
        if left < 1.0 || right < 1.0 {
            return None;
        }
        let notch = frame.size.width - left - right;
        if !(80.0..400.0).contains(&notch) {
            return None;
        }
        let visible = screen.visibleFrame();
        let menu_h =
            (frame.origin.y + frame.size.height) - (visible.origin.y + visible.size.height);
        if !(20.0..80.0).contains(&menu_h) {
            return None;
        }
        return Some(TopAnchor {
            notch_x: left,
            notch_w: notch,
            menu_h,
        });
    }
    None
}

fn screen_top(monitor: &tauri::Monitor) -> Option<(f64, f64, f64)> {
    let scale = monitor.scale_factor();
    if scale <= 0.0 {
        return None;
    }
    let pos = monitor.position();
    let size = monitor.size();
    Some((
        pos.x as f64 / scale,
        pos.y as f64 / scale,
        size.width as f64 / scale,
    ))
}

fn screen_signature(app: &AppHandle) -> Vec<(i32, i32)> {
    monitors_for_island(app)
        .iter()
        .filter_map(|monitor| {
            let work = monitor.work_area();
            Some((work.position.x, work.position.y))
        })
        .collect()
}

fn ensure_screen_follow(app: &AppHandle) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(SCREEN_FOLLOW);
        let app = app.clone();
        let _ = app
            .clone()
            .run_on_main_thread(move || refresh_screens(&app));
    });
}

fn refresh_screens(app: &AppHandle) {
    let sig = screen_signature(app);
    let (changed, applying) = {
        let host = app.state::<IslandHost>();
        let mut guard = host.inner.lock().expect("island");
        let changed = guard.screen_sig != sig;
        if changed {
            guard.screen_sig = sig;
            guard.placed.clear();
        }
        (changed, guard.applying)
    };
    if changed {
        if applying {
            let host = app.state::<IslandHost>();
            let mut guard = host.inner.lock().expect("island");
            guard.dirty = true;
        } else {
            apply(app);
        }
        return;
    }
    // Another app's fullscreen space can order the notch out. Raise it again.
    let shown = {
        let host = app.state::<IslandHost>();
        let guard = host.inner.lock().expect("island");
        guard.last_view.shown
    };
    if !shown {
        return;
    }
    for window in island_windows(app) {
        present_island(&window);
    }
}

fn ensure_island_window(app: &AppHandle, label: &str) -> Option<tauri::WebviewWindow> {
    if let Some(existing) = app.get_webview_window(label) {
        return Some(existing);
    }
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App("island.html".into()))
        .title("Agent Doctor")
        .inner_size(CHIP_WIDTH, CHIP_HEIGHT)
        .resizable(false)
        .closable(false)
        .minimizable(false)
        .maximizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .visible_on_all_workspaces(true)
        .accept_first_mouse(true)
        .background_color(tauri::webview::Color(0, 0, 0, 0))
        .build()
        .map_err(|err| {
            eprintln!("failed to open island window: {err}");
            err
        })
        .ok()?;
    Some(window)
}

fn place_island_frame(window: &tauri::WebviewWindow, x: f64, y: f64, width: f64, height: f64) {
    let _ = window.set_size(LogicalSize::new(width, height));
    let _ = window.set_position(LogicalPosition::new(x, y));
    #[cfg(target_os = "macos")]
    pin_frame_to_screen_top(window, x, y, width, height);
}

#[cfg(target_os = "macos")]
fn pin_frame_to_screen_top(window: &tauri::WebviewWindow, x: f64, y: f64, width: f64, height: f64) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Ok(ptr) = window.ns_window() else {
        return;
    };
    if ptr.is_null() {
        return;
    }
    let screens = NSScreen::screens(mtm);
    let mut cocoa_top = 0.0f64;
    for screen in screens.iter() {
        let frame = screen.frame();
        cocoa_top = cocoa_top.max(frame.origin.y + frame.size.height);
    }
    unsafe {
        use objc2_app_kit::NSWindow;
        let host: &NSWindow = &*ptr.cast();
        let ns = overlay_target(host);
        ns.setFrame_display(
            NSRect {
                origin: NSPoint {
                    x,
                    y: cocoa_top - y - height,
                },
                size: NSSize { width, height },
            },
            false,
        );
    }
}

fn present_island(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    if order_front_without_activating(window) {
        return;
    }
    let _ = window.set_always_on_top(true);
    let _ = window.set_visible_on_all_workspaces(true);
    let _ = window.show();
}

#[cfg(target_os = "macos")]
struct OverlayPanel {
    host: usize,
    panel: *mut objc2_app_kit::NSPanel,
}

#[cfg(target_os = "macos")]
unsafe impl Send for OverlayPanel {}

#[cfg(target_os = "macos")]
static OVERLAYS: Mutex<Vec<OverlayPanel>> = Mutex::new(Vec::new());

/// The notch is a real panel. A normal window is dropped when another app
/// takes a fullscreen space, even at a high window level.
#[cfg(target_os = "macos")]
fn overlay_target(host: &objc2_app_kit::NSWindow) -> &objc2_app_kit::NSWindow {
    let key = host as *const objc2_app_kit::NSWindow as usize;
    let guard = OVERLAYS.lock().expect("overlays");
    if let Some(found) = guard.iter().find(|item| item.host == key) {
        if !found.panel.is_null() {
            return unsafe { &*found.panel.cast() };
        }
    }
    host
}

// A borderless panel refuses the keyboard by default. This one takes it
// without bringing the rest of the app forward.
#[cfg(target_os = "macos")]
objc2::define_class!(
    #[unsafe(super(objc2_app_kit::NSPanel, objc2_app_kit::NSWindow, objc2_app_kit::NSResponder, objc2::runtime::NSObject))]
    #[thread_kind = objc2::MainThreadOnly]
    #[name = "AgentDoctorIslandPanel"]
    struct IslandPanel;

    impl IslandPanel {
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
fn ensure_overlay_panel(host: &objc2_app_kit::NSWindow) -> *mut objc2_app_kit::NSPanel {
    use objc2::rc::Retained;
    use objc2::runtime::AnyClass;
    use objc2::{msg_send, MainThreadMarker};
    use objc2_app_kit::{NSPanel, NSView, NSWindow, NSWindowStyleMask};

    let key = host as *const NSWindow as usize;
    {
        let guard = OVERLAYS.lock().expect("overlays");
        if let Some(found) = guard.iter().find(|item| item.host == key) {
            return found.panel;
        }
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return std::ptr::null_mut();
    };
    let custom: Retained<IslandPanel> =
        unsafe { msg_send![objc2::MainThreadOnly::alloc(mtm), init] };
    let panel: Retained<NSPanel> = Retained::into_super(custom);
    let panel_win: &NSWindow = unsafe { &*(&*panel as *const NSPanel as *const NSWindow) };
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(false);
    panel.setWorksWhenModal(true);
    panel_win.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
    panel_win.setHidesOnDeactivate(false);
    panel_win.setCanHide(false);
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
    if let Some(view) = host.contentView() {
        panel_win.setContentView(Some(&view));
        // Tao still asks the original window for a view. Leave it an empty one
        // so a later frame update does not abort.
        host.setContentView(Some(&NSView::new(mtm)));
    }
    host.orderOut(None);
    let ptr = Retained::into_raw(panel);
    OVERLAYS.lock().expect("overlays").push(OverlayPanel {
        host: key,
        panel: ptr,
    });
    ptr
}

#[cfg(target_os = "macos")]
fn order_front_without_activating(window: &tauri::WebviewWindow) -> bool {
    use objc2_app_kit::{NSScreenSaverWindowLevel, NSWindow, NSWindowCollectionBehavior};
    let Ok(ptr) = window.ns_window() else {
        return false;
    };
    if ptr.is_null() {
        return false;
    }
    unsafe {
        let host: &NSWindow = &*ptr.cast();
        let panel = ensure_overlay_panel(host);
        let ns: &NSWindow = if panel.is_null() {
            host
        } else {
            &*panel.cast()
        };
        ns.setHidesOnDeactivate(false);
        ns.setCanHide(false);
        ns.setLevel(NSScreenSaverWindowLevel);
        let behavior = NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::CanJoinAllApplications;
        ns.setCollectionBehavior(behavior);
        ns.orderFrontRegardless();
    }
    true
}

#[cfg(target_os = "macos")]
fn watch_outside_clicks(app: &AppHandle) {
    use std::sync::Once;

    static START: Once = Once::new();
    let app = app.clone();
    START.call_once(move || {
        use std::ptr::NonNull;

        use block2::RcBlock;
        use objc2_app_kit::{NSEvent, NSEventMask};

        let mask = NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown;
        let global_app = app.clone();
        let global = RcBlock::new(move |_event: NonNull<NSEvent>| {
            fold_if_click_is_outside(&global_app);
        });
        let _monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &global);
        std::mem::forget(global);
        std::mem::forget(_monitor);

        let local_app = app;
        let local = RcBlock::new(move |event: NonNull<NSEvent>| {
            fold_if_click_is_outside(&local_app);
            event.as_ptr()
        });
        let _local = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &local) };
        std::mem::forget(local);
        std::mem::forget(_local);
    });
}

#[cfg(target_os = "macos")]
fn fold_if_click_is_outside(app: &AppHandle) {
    use objc2_app_kit::NSEvent;

    let point = NSEvent::mouseLocation();
    let on_island = click_lands_on_island(point);
    let refresh = {
        let host = app.state::<IslandHost>();
        let mut guard = host.inner.lock().expect("island");
        if on_island {
            let reopen = !guard.last_view.expanded;
            guard.folded = false;
            guard.opened = true;
            if reopen {
                guard.hovering = true;
            }
            reopen
        } else if guard.last_view.expanded {
            guard.hovering = false;
            guard.opened = false;
            guard.reading = false;
            guard.folded = true;
            true
        } else {
            false
        }
    };
    if !on_island && refresh {
        release_island_keyboard(app);
    }
    if refresh {
        apply(app);
    }
}

#[cfg(target_os = "macos")]
static ISLAND_KEY_PANEL: AtomicUsize = AtomicUsize::new(0);

/// The panel takes the keyboard on its own. The app is never brought forward,
/// so the chat windows stay where they are.
#[cfg(target_os = "macos")]
fn claim_island_keyboard(_app: &AppHandle) {
    use objc2_app_kit::NSEvent;

    let Some(panel) = panel_at(NSEvent::mouseLocation()) else {
        return;
    };
    ISLAND_KEY_PANEL.store(panel as usize, Ordering::SeqCst);
    let ns = unsafe { &*panel };
    if !ns.isKeyWindow() {
        ns.makeKeyWindow();
    }
}

#[cfg(target_os = "macos")]
fn release_island_keyboard(_app: &AppHandle) {
    use objc2_app_kit::NSWindow;

    let ptr = ISLAND_KEY_PANEL.swap(0, Ordering::SeqCst);
    if ptr == 0 {
        return;
    }
    let ns = unsafe { &*(ptr as *mut NSWindow) };
    if ns.isKeyWindow() {
        ns.resignKeyWindow();
    }
}

#[cfg(not(target_os = "macos"))]
fn release_island_keyboard(_app: &AppHandle) {}

#[cfg(target_os = "macos")]
fn panel_at(point: objc2_foundation::NSPoint) -> Option<*mut objc2_app_kit::NSWindow> {
    use objc2_app_kit::NSWindow;

    let guard = OVERLAYS.lock().expect("overlays");
    for item in guard.iter() {
        if item.panel.is_null() {
            continue;
        }
        let ns = unsafe { &*item.panel.cast::<NSWindow>() };
        let frame = ns.frame();
        let inside = point.x >= frame.origin.x
            && point.x < frame.origin.x + frame.size.width
            && point.y >= frame.origin.y
            && point.y < frame.origin.y + frame.size.height;
        if inside {
            return Some(item.panel.cast());
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn click_lands_on_island(point: objc2_foundation::NSPoint) -> bool {
    use objc2_app_kit::NSWindow;

    let guard = OVERLAYS.lock().expect("overlays");
    for item in guard.iter() {
        if item.panel.is_null() {
            continue;
        }
        let frame = unsafe { &*item.panel.cast::<NSWindow>() }.frame();
        let origin = frame.origin;
        let size = frame.size;
        let inside = point.x >= origin.x
            && point.x < origin.x + size.width
            && point.y >= origin.y
            && point.y < origin.y + size.height;
        if inside {
            return true;
        }
    }
    false
}

fn hop(app: &AppHandle, f: impl FnOnce(&AppHandle) + Send + 'static) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || f(&app));
}

fn store_snapshot(app: &AppHandle, input: IslandSnapshotInput) {
    let host = app.state::<IslandHost>();
    let mut guard = host.inner.lock().expect("island");
    let next = Snapshot::from(input);
    if !next.active {
        guard.pinned = false;
        guard.hold_open = false;
    }
    let prev_request = guard
        .snapshot
        .pending
        .as_ref()
        .map(|item| item.request_id.clone());
    let next_request = next.pending.as_ref().map(|item| item.request_id.clone());
    if prev_request != next_request {
        guard.folded = false;
    }
    guard.snapshot = next;
}

fn restore_conversation(app: &AppHandle, pin: bool) {
    release_island_keyboard(app);
    {
        let host = app.state::<IslandHost>();
        let mut guard = host.inner.lock().expect("island");
        if pin {
            guard.pinned = true;
        } else {
            guard.hold_open = true;
        }
        guard.hovering = false;
        guard.opened = false;
        guard.reading = false;
        guard.folded = true;
        guard.parked_ask = false;
        guard.parked_main = false;
    }
    // The notch sits above every window and does not activate the app, so the
    // conversation has to be brought forward on purpose. Use the same placement
    // as a normal open; a plain show keeps the smaller size from first create.
    activate_app();
    crate::windows::present_ask_window(app);
    apply(app);
}

fn activate_app() {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, AnyObject};

        let Some(cls) = AnyClass::get(c"NSApplication") else {
            return;
        };
        unsafe {
            let app: *mut AnyObject = msg_send![cls, sharedApplication];
            if !app.is_null() {
                let _: () = msg_send![app, activateIgnoringOtherApps: true];
            }
        }
    }
}

#[tauri::command]
pub fn publish_island_snapshot_command(
    app: AppHandle,
    snapshot: IslandSnapshotInput,
) -> Result<(), String> {
    hop(&app, move |app| {
        store_snapshot(app, snapshot);
        apply(app);
    });
    Ok(())
}

#[tauri::command]
pub fn island_set_hover_command(
    app: AppHandle,
    hovering: bool,
    sticky: Option<bool>,
) -> Result<(), String> {
    hop(&app, move |app| {
        let left = !hovering && !sticky.unwrap_or(false);
        {
            let host = app.state::<IslandHost>();
            let mut guard = host.inner.lock().expect("island");
            if hovering {
                guard.folded = false;
                if sticky.unwrap_or(false) {
                    guard.opened = true;
                }
            }
            guard.hovering = hovering;
            if left {
                guard.opened = false;
                guard.reading = false;
                guard.folded = guard.last_view.expanded;
            }
        }
        if left {
            release_island_keyboard(app);
        }
        apply(app);
    });
    Ok(())
}

#[tauri::command]
pub fn island_set_reading_command(app: AppHandle, reading: bool) -> Result<(), String> {
    hop(&app, move |app| {
        {
            let host = app.state::<IslandHost>();
            let mut guard = host.inner.lock().expect("island");
            guard.reading = reading;
            if reading {
                guard.folded = false;
            }
        }
        apply(app);
    });
    Ok(())
}

#[tauri::command]
pub fn island_restore_command(app: AppHandle) -> Result<(), String> {
    hop(&app, move |app| restore_conversation(app, false));
    Ok(())
}

#[tauri::command]
pub fn island_pin_command(app: AppHandle) -> Result<(), String> {
    hop(&app, move |app| restore_conversation(app, true));
    Ok(())
}

#[tauri::command]
pub fn island_send_text_command(
    app: AppHandle,
    session_id: String,
    text: String,
) -> Result<(), String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Ok(());
    }
    let window = app
        .get_webview_window("ask")
        .ok_or_else(|| "ask window is closed".to_string())?;
    window
        .emit(
            "island-send-text",
            serde_json::json!({ "sessionId": session_id, "text": text }),
        )
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn island_claim_keyboard_command(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if objc2::MainThreadMarker::new().is_some() {
            claim_island_keyboard(&app);
        } else {
            let (tx, rx) = std::sync::mpsc::channel();
            let app = app.clone();
            let _ = app.clone().run_on_main_thread(move || {
                claim_island_keyboard(&app);
                let _ = tx.send(());
            });
            let _ = rx.recv_timeout(Duration::from_millis(400));
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    Ok(())
}

#[tauri::command]
pub fn island_open_session_command(app: AppHandle, session_id: String) -> Result<(), String> {
    hop(&app, move |app| {
        // Also brings back windows hidden for a browser turn, and keeps them up for it.
        restore_conversation(app, false);
        if let Some(window) = app.get_webview_window("ask") {
            let _ = window.emit("island-open-session", session_id);
        }
    });
    Ok(())
}

#[tauri::command]
pub fn current_island_view_command(app: AppHandle) -> IslandView {
    app.state::<IslandHost>()
        .inner
        .lock()
        .expect("island")
        .last_view
        .clone()
}

#[cfg(test)]
mod tests {
    use super::{island_chrome, peek_height, should_park_windows, Chrome};

    #[test]
    fn one_conversation_does_not_leave_a_tall_gap() {
        let one = peek_height(1, false);
        let many = peek_height(6, false);
        assert!(one < 220.0);
        assert!(many > one);
        assert!(many <= 460.0);
    }

    #[test]
    fn the_island_stays_up_while_idle() {
        assert_eq!(
            island_chrome(true, false, false, false, false, false),
            Chrome::Pill
        );
        assert_eq!(
            island_chrome(true, true, false, false, false, false),
            Chrome::Peek
        );
    }

    #[test]
    fn a_question_opens_the_island_unless_typing() {
        assert_eq!(
            island_chrome(true, false, true, false, false, false),
            Chrome::Peek
        );
        assert_eq!(
            island_chrome(true, false, true, true, false, false),
            Chrome::Pill
        );
        assert_eq!(
            island_chrome(true, true, true, true, true, false),
            Chrome::Peek
        );
    }

    #[test]
    fn a_click_outside_folds_the_card() {
        assert_eq!(
            island_chrome(true, false, true, false, true, false),
            Chrome::Pill
        );
        assert_eq!(
            island_chrome(true, true, true, false, true, false),
            Chrome::Peek
        );
    }

    #[test]
    fn a_click_on_the_bar_opens_it_again() {
        assert_eq!(
            island_chrome(true, false, true, false, true, true),
            Chrome::Peek
        );
        assert_eq!(
            island_chrome(true, false, false, false, true, true),
            Chrome::Peek
        );
    }

    #[test]
    fn other_platforms_have_no_island() {
        assert_eq!(
            island_chrome(false, true, false, false, false, false),
            Chrome::Hidden
        );
    }

    #[test]
    fn browser_hides_the_windows() {
        assert!(should_park_windows(true, true, false, false, false));
    }

    #[test]
    fn staying_here_keeps_the_windows() {
        assert!(!should_park_windows(true, false, false, false, false));
        assert!(!should_park_windows(false, false, false, false, false));
    }

    #[test]
    fn typing_keeps_the_windows() {
        assert!(!should_park_windows(true, true, true, false, false));
    }

    #[test]
    fn hold_open_suppresses_the_browser_reason() {
        assert!(!should_park_windows(true, true, false, true, false));
    }

    #[test]
    fn pin_keeps_the_windows() {
        assert!(!should_park_windows(true, true, false, false, true));
    }
}
