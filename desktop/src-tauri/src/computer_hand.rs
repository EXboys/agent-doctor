//! Clicks and keystrokes run in this app. macOS blocks `osascript` from sending
//! them, even when this app is allowed to control the computer.

use std::cell::Cell;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

pub fn spawn() {
    thread::Builder::new()
        .name("computer-hand".into())
        .spawn(|| {
            if let Err(err) = serve() {
                eprintln!("computer hand stopped: {err:#}");
            }
        })
        .ok();
}

fn socket_path() -> Result<PathBuf> {
    let dir = agent_doctor_core::store::agent_doctor_config_dir()
        .context("could not find the app config folder")?;
    Ok(dir.join("computer.sock"))
}

fn serve() -> Result<()> {
    let path = socket_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    for conn in listener.incoming() {
        let stream = match conn {
            Ok(stream) => stream,
            Err(_) => continue,
        };
        let mut write_half = match stream.try_clone() {
            Ok(half) => half,
            Err(_) => continue,
        };
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            continue;
        }
        thread::spawn(move || {
            let reply = match handle_line(&line) {
                Ok(result) => json!({ "ok": true, "result": result }),
                Err(err) => json!({ "ok": false, "error": format!("{err:#}") }),
            };
            let _ = writeln!(write_half, "{reply}");
        });
    }
    Ok(())
}

fn handle_line(line: &str) -> Result<Value> {
    let req: Value = serde_json::from_str(line).context("bad request")?;
    let tool = req.get("tool").and_then(Value::as_str).unwrap_or("");
    let args = req.get("args").cloned().unwrap_or(json!({}));
    ensure_trusted()?;
    match tool {
        "screen_click" => {
            let x = args.get("x").and_then(Value::as_f64).context("missing x")?;
            let y = args.get("y").and_then(Value::as_f64).context("missing y")?;
            click_at(x, y)?;
            Ok(json!({ "clicked": "point", "x": x, "y": y }))
        }
        "screen_press" => {
            let key = args.get("key").and_then(Value::as_str).unwrap_or("");
            press_key(key)?;
            Ok(json!({ "pressed": key }))
        }
        "screen_type" => {
            let text = args.get("text").and_then(Value::as_str).unwrap_or("");
            if text.is_empty() {
                bail!("text is empty");
            }
            type_text(text)?;
            Ok(json!({ "typed": text.chars().count() }))
        }
        "screen_menus" => Ok(json!({ "text": menu_text()? })),
        "screen_apps" => Ok(json!({ "apps": app_names()? })),
        "screen_look" => {
            let app = args
                .get("app")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            screen_look(&app)
        }
        other => bail!("unknown hand tool: {other}"),
    }
}

static ASKED_SYSTEM: AtomicBool = AtomicBool::new(false);

fn ensure_trusted() -> Result<()> {
    if trusted() {
        return Ok(());
    }
    let first = ASKED_SYSTEM
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok();
    if first {
        // The system dialog only appears when this check runs on the main thread.
        prompt_trust_on_main();
    }
    if trusted() {
        return Ok(());
    }
    if first {
        #[cfg(target_os = "macos")]
        crate::commands::reveal_accessibility_settings();
        bail!("系统设置已经打开。把这个应用的开关打开，然后回到这里再说一次。这一步不要重复点击。");
    }
    bail!("还没有允许。开关打开之前不要再点。");
}

fn prompt_trust_on_main() {
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        let tx = Box::new(tx);
        unsafe {
            dispatch_async_f(
                &raw const _dispatch_main_q as *mut std::ffi::c_void,
                Box::into_raw(tx) as *mut std::ffi::c_void,
                prompt_on_main,
            );
        }
        let _ = rx.recv_timeout(Duration::from_secs(3));
    }
}

#[cfg(target_os = "macos")]
extern "C" fn prompt_on_main(context: *mut std::ffi::c_void) {
    let trusted = crate::commands::prompt_accessibility();
    let tx = unsafe { Box::from_raw(context as *mut std::sync::mpsc::Sender<bool>) };
    let _ = tx.send(trusted);
}

#[cfg(target_os = "macos")]
extern "C" {
    static _dispatch_main_q: std::ffi::c_void;
    fn dispatch_async_f(
        queue: *mut std::ffi::c_void,
        context: *mut std::ffi::c_void,
        work: extern "C" fn(*mut std::ffi::c_void),
    );
}

fn screen_look(app: &str) -> Result<Value> {
    // A missing picture must fail the tool. Returning the menu names alone
    // makes the assistant click without seeing the screen.
    let shot = capture_png(app)?;
    let mut text = look_text(app)?;
    if text.chars().count() > 12000 {
        text.truncate(12000);
        text.push_str("\n…");
    }
    text.push_str(&format!(
        "\n\nScreenshot file: {}\nImage pixels: {}x{}. Scale: {}.\nClick x,y are points ({}x{}), not image pixels. Divide a pixel position by the scale.",
        shot.path, shot.pixels.0, shot.pixels.1, shot.scale, shot.points.0, shot.points.1
    ));
    Ok(json!({
        "text": text,
        "image": shot.path,
        "pixel_width": shot.pixels.0,
        "pixel_height": shot.pixels.1,
        "scale": shot.scale,
        "point_width": shot.points.0,
        "point_height": shot.points.1,
    }))
}

fn look_text(app: &str) -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        // The other app's main thread owns an open menu and stops answering.
        // Stop asking after a short budget so this read cannot hold the channel.
        with_look_deadline(|| unsafe { ax_look(app) })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        bail!("the screen is read on macOS");
    }
}

#[cfg(target_os = "macos")]
unsafe fn ax_look(want: &str) -> Result<String> {
    let app = if want.is_empty() {
        front_app_element()
    } else {
        app_element_named(want).or_else(|| front_app_element())
    };
    let Some(app) = app else {
        bail!("could not see the front app");
    };
    let mut out = String::new();
    if let Some(name) = attr_string(app, "AXTitle").filter(|name| !name.is_empty()) {
        out.push_str(&format!("front app: {name}\n"));
    }
    if let Some(windows) = copy_attr(app, "AXWindows") {
        let count = CFArrayGetCount(windows).min(3);
        for i in 0..count {
            let window = CFArrayGetValueAtIndex(windows, i);
            if window.is_null() {
                continue;
            }
            let title = attr_string(window, "AXTitle").unwrap_or_default();
            if i == 0 {
                out.push_str(&format!("window: {title}\n"));
                let mut seen = 0;
                append_controls(&mut out, window, 0, &mut seen);
            }
        }
        CFRelease(windows);
    }
    CFRelease(app);
    if let Ok(menus) = ax_menu_text() {
        if !menus.is_empty() {
            out.push_str(&menus);
        }
    }
    Ok(out)
}

#[cfg(target_os = "macos")]
unsafe fn append_controls(
    out: &mut String,
    el: *const std::ffi::c_void,
    depth: u8,
    seen: &mut i32,
) {
    if depth > 4 || *seen >= 40 {
        return;
    }
    let Some(children) = copy_attr(el, "AXChildren") else {
        return;
    };
    let count = CFArrayGetCount(children);
    for i in 0..count {
        if *seen >= 40 {
            break;
        }
        let child = CFArrayGetValueAtIndex(children, i);
        if child.is_null() {
            continue;
        }
        let role = attr_string(child, "AXRole").unwrap_or_default();
        if matches!(
            role.as_str(),
            "AXButton" | "AXTextField" | "AXStaticText" | "AXCheckBox"
        ) {
            let label = attr_string(child, "AXTitle")
                .filter(|label| !label.is_empty())
                .or_else(|| attr_string(child, "AXDescription"))
                .filter(|label| !label.is_empty())
                .or_else(|| attr_string(child, "AXValue"))
                .unwrap_or_default();
            if !label.is_empty() {
                if let Some((x, y)) = attr_center(child) {
                    let kind = match role.as_str() {
                        "AXButton" | "AXCheckBox" => "button",
                        "AXTextField" => "text field",
                        _ => "text",
                    };
                    *seen += 1;
                    let (x, y) = shot_point(x, y);
                    out.push_str(&format!("{kind}\t{label}\t{x}\t{y}\n"));
                }
            }
        }
        if role != "AXMenu" && role != "AXMenuBar" {
            append_controls(out, child, depth + 1, seen);
        }
    }
    CFRelease(children);
}

#[cfg(target_os = "macos")]
unsafe fn app_element_named(name: &str) -> Option<*const std::ffi::c_void> {
    let mut pids = vec![0i32; 1024];
    let bytes = proc_listpids(
        1,
        0,
        pids.as_mut_ptr() as *mut std::ffi::c_void,
        (pids.len() * 4) as i32,
    );
    if bytes <= 0 {
        return None;
    }
    let count = (bytes as usize / 4).min(pids.len());
    for pid in pids.into_iter().take(count) {
        if pid <= 0 {
            continue;
        }
        let mut buf = [0u8; 256];
        if proc_name(
            pid,
            buf.as_mut_ptr() as *mut std::ffi::c_void,
            buf.len() as u32,
        ) <= 0
        {
            continue;
        }
        let proc =
            std::ffi::CStr::from_ptr(buf.as_ptr() as *const std::ffi::c_char).to_string_lossy();
        if proc.eq_ignore_ascii_case(name) {
            let app = AXUIElementCreateApplication(pid);
            if let Some(app) = limit_element(app) {
                return Some(app);
            }
        }
    }
    None
}

fn app_names() -> Result<Vec<String>> {
    #[cfg(target_os = "macos")]
    {
        // Names come from the program path. Asking every app for its windows
        // stalls the channel when one app is not responding.
        let mut names = Vec::new();
        let mut pids = vec![0i32; 1024];
        let bytes = unsafe {
            proc_listpids(
                1,
                0,
                pids.as_mut_ptr() as *mut std::ffi::c_void,
                (pids.len() * 4) as i32,
            )
        };
        let count = if bytes > 0 {
            (bytes as usize / 4).min(pids.len())
        } else {
            0
        };
        for pid in pids.into_iter().take(count) {
            if pid <= 0 || names.len() >= 40 {
                continue;
            }
            let mut buf = vec![0u8; 4096];
            let n = unsafe {
                proc_pidpath(
                    pid,
                    buf.as_mut_ptr() as *mut std::ffi::c_void,
                    buf.len() as u32,
                )
            };
            if n <= 0 {
                continue;
            }
            let path = unsafe {
                std::ffi::CStr::from_ptr(buf.as_ptr() as *const std::ffi::c_char).to_string_lossy()
            };
            let Some(bundle) = path.split(".app/").next() else {
                continue;
            };
            let Some(name) = bundle.rsplit('/').next() else {
                continue;
            };
            if name.is_empty() || names.iter().any(|have| have == name) {
                continue;
            }
            names.push(name.to_string());
        }
        if names.is_empty() {
            bail!("could not see the open apps");
        }
        Ok(names)
    }
    #[cfg(not(target_os = "macos"))]
    {
        bail!("apps are listed on macOS");
    }
}

struct Shot {
    path: String,
    pixels: (i64, i64),
    points: (i64, i64),
    scale: f64,
}

fn capture_png(app: &str) -> Result<Shot> {
    #[cfg(target_os = "macos")]
    {
        if !unsafe { CGPreflightScreenCaptureAccess() } {
            static ASKED_SCREEN: AtomicBool = AtomicBool::new(false);
            if ASKED_SCREEN
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                unsafe { CGRequestScreenCaptureAccess() };
            }
            bail!(
                "还没有允许录制屏幕。打开系统设置里的「录屏与系统录音」，打开这个应用的开关，然后完全退出再打开。开关打开之前不要再点屏幕。"
            );
        }
        let display = display_for_app(app);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = format!("/tmp/agent-doctor-screen-{stamp}.png");
        let rect = format!(
            "{},{},{},{}",
            display.origin_x.round() as i64,
            display.origin_y.round() as i64,
            display.width.round().max(1.0) as i64,
            display.height.round().max(1.0) as i64
        );
        let mut child = std::process::Command::new("/usr/sbin/screencapture")
            .args(["-x", "-R", &rect, &path])
            .spawn()
            .context("could not capture the screen")?;
        let started = std::time::Instant::now();
        loop {
            if let Some(status) = child.try_wait().context("could not capture the screen")? {
                if !status.success() {
                    bail!("could not capture the screen");
                }
                break;
            }
            if started.elapsed() > Duration::from_secs(3) {
                let _ = child.kill();
                let _ = child.wait();
                bail!("screenshot took too long");
            }
            thread::sleep(Duration::from_millis(40));
        }
        let (px_w, px_h) = png_size(&path).unwrap_or((0, 0));
        let pt_w = display.width.round().max(1.0) as i64;
        let pt_h = display.height.round().max(1.0) as i64;
        let display_scale = display_scale(display.id);
        // The file is the truth when the capture succeeded. Points are this
        // display, not the main display.
        let scale = if px_w > 0 && pt_w > 0 {
            px_w as f64 / pt_w as f64
        } else {
            display_scale
        };
        remember_origin(display.origin_x, display.origin_y);
        Ok(Shot {
            path,
            pixels: (px_w, px_h),
            points: (pt_w, pt_h),
            scale,
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        bail!("the screen is captured on macOS");
    }
}

#[cfg(target_os = "macos")]
fn png_size(path: &str) -> Option<(i64, i64)> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() < 24 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let width = i64::from(u32::from_be_bytes(bytes[16..20].try_into().ok()?));
    let height = i64::from(u32::from_be_bytes(bytes[20..24].try_into().ok()?));
    Some((width, height))
}

#[cfg(target_os = "macos")]
fn display_scale(id: u32) -> f64 {
    unsafe {
        let bounds = CGDisplayBounds(id);
        let pixels = CGDisplayPixelsWide(id) as f64;
        let width = bounds.size.width;
        if width > 0.0 {
            pixels / width
        } else {
            1.0
        }
    }
}

#[cfg(target_os = "macos")]
struct MacDisplay {
    id: u32,
    origin_x: f64,
    origin_y: f64,
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
fn display_for_app(app: &str) -> MacDisplay {
    let anchor = unsafe { window_anchor(app) };
    let displays = active_displays();
    if let Some((x, y)) = anchor {
        if let Some(display) = displays.iter().find(|display| display.contains(x, y)) {
            return display.clone();
        }
    }
    displays
        .into_iter()
        .find(|display| display.id == unsafe { CGMainDisplayID() })
        .unwrap_or_else(main_display)
}

#[cfg(target_os = "macos")]
fn active_displays() -> Vec<MacDisplay> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    let err = unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut count) };
    if err != 0 || count == 0 {
        return vec![main_display()];
    }
    ids.iter()
        .take(count as usize)
        .map(|id| display_from(*id))
        .collect()
}

#[cfg(target_os = "macos")]
fn main_display() -> MacDisplay {
    display_from(unsafe { CGMainDisplayID() })
}

#[cfg(target_os = "macos")]
fn display_from(id: u32) -> MacDisplay {
    let bounds = unsafe { CGDisplayBounds(id) };
    MacDisplay {
        id,
        origin_x: bounds.origin.x,
        origin_y: bounds.origin.y,
        width: bounds.size.width,
        height: bounds.size.height,
    }
}

#[cfg(target_os = "macos")]
impl MacDisplay {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.origin_x
            && y >= self.origin_y
            && x < self.origin_x + self.width
            && y < self.origin_y + self.height
    }
}

#[cfg(target_os = "macos")]
impl Clone for MacDisplay {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            origin_x: self.origin_x,
            origin_y: self.origin_y,
            width: self.width,
            height: self.height,
        }
    }
}

/// Top-left of the display captured by the latest screenshot. Clicks from the
/// model are points inside that picture, so they are shifted back to the desktop.
#[cfg(target_os = "macos")]
static LAST_ORIGIN: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));

#[cfg(target_os = "macos")]
fn remember_origin(x: f64, y: f64) {
    *LAST_ORIGIN.lock().unwrap_or_else(|err| err.into_inner()) = (x, y);
}

#[cfg(target_os = "macos")]
fn last_origin() -> (f64, f64) {
    *LAST_ORIGIN.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(target_os = "macos")]
fn shot_point(x: f64, y: f64) -> (i64, i64) {
    let (origin_x, origin_y) = last_origin();
    ((x - origin_x).round() as i64, (y - origin_y).round() as i64)
}

#[cfg(target_os = "macos")]
unsafe fn window_anchor(want: &str) -> Option<(f64, f64)> {
    let app = if want.is_empty() {
        front_app_element()
    } else {
        app_element_named(want).or_else(|| front_app_element())
    }?;
    let windows = copy_attr(app, "AXWindows");
    let center = windows.and_then(|windows| {
        let center = if CFArrayGetCount(windows) <= 0 {
            None
        } else {
            let window = CFArrayGetValueAtIndex(windows, 0);
            if window.is_null() {
                None
            } else {
                attr_center(window)
            }
        };
        CFRelease(windows);
        center
    });
    CFRelease(app);
    center
}

fn menu_text() -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        with_look_deadline(|| unsafe { ax_menu_text() })
    }
    #[cfg(not(target_os = "macos"))]
    {
        bail!("menus are read on macOS");
    }
}

#[cfg(target_os = "macos")]
unsafe fn ax_menu_text() -> Result<String> {
    let app = front_app_element();
    let Some(app) = app else {
        bail!("could not see the front app");
    };
    let bar = copy_attr(app, "AXMenuBar");
    CFRelease(app);
    let Some(bar) = bar else {
        bail!("could not see the menu bar");
    };
    let mut out = String::new();
    if let Some(items) = copy_attr(bar, "AXChildren") {
        let count = CFArrayGetCount(items);
        for i in 0..count {
            let item = CFArrayGetValueAtIndex(items, i);
            if item.is_null() {
                continue;
            }
            let Some(name) = attr_string(item, "AXTitle") else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let Some((x, y)) = attr_center(item) else {
                continue;
            };
            let (x, y) = shot_point(x, y);
            out.push_str(&format!("menubar\t{name}\t{x}\t{y}\n"));
            if attr_bool(item, "AXSelected") {
                append_open_menu(&mut out, item, &name);
            }
        }
        CFRelease(items);
    }
    CFRelease(bar);
    Ok(out)
}

#[cfg(target_os = "macos")]
unsafe fn front_app_element() -> Option<*const std::ffi::c_void> {
    let mut psn = ProcessSerialNumber { high: 0, low: 0 };
    if GetFrontProcess(&mut psn) == 0 {
        let mut pid = 0i32;
        if GetProcessPID(&psn, &mut pid) == 0 && pid > 0 {
            let app = AXUIElementCreateApplication(pid);
            if let Some(app) = limit_element(app) {
                return Some(app);
            }
        }
    }
    let system = AXUIElementCreateSystemWide();
    if system.is_null() {
        return None;
    }
    let app = copy_attr(system, "AXFocusedApplication");
    CFRelease(system);
    app
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}

#[cfg(target_os = "macos")]
unsafe fn append_open_menu(out: &mut String, item: *const std::ffi::c_void, bar_name: &str) {
    let Some(children) = copy_attr(item, "AXChildren") else {
        return;
    };
    let count = CFArrayGetCount(children);
    for i in 0..count {
        let child = CFArrayGetValueAtIndex(children, i);
        if attr_string(child, "AXRole").as_deref() != Some("AXMenu") {
            continue;
        }
        let Some(entries) = copy_attr(child, "AXChildren") else {
            continue;
        };
        let n = CFArrayGetCount(entries);
        let mut rows = Vec::new();
        for j in 0..n {
            let entry = CFArrayGetValueAtIndex(entries, j);
            if attr_string(entry, "AXRole").as_deref() != Some("AXMenuItem") {
                continue;
            }
            let Some(label) = attr_string(entry, "AXTitle") else {
                continue;
            };
            let Some((left, top, width, height)) = attr_box(entry) else {
                continue;
            };
            rows.push(MenuRow {
                label,
                left,
                top,
                width,
                height,
            });
        }
        for (label, x, y) in place_menu_rows(&rows) {
            let (x, y) = shot_point(x as f64, y as f64);
            out.push_str(&format!("menu\t{bar_name}\t{label}\t{x}\t{y}\n"));
        }
        CFRelease(entries);
    }
    CFRelease(children);
}

struct MenuRow {
    label: String,
    left: f64,
    top: f64,
    width: f64,
    height: f64,
}

/// macOS reports every open menu item at the first item's position. Keep a
/// position that already moved down, and stack the copies under the previous row.
fn place_menu_rows(rows: &[MenuRow]) -> Vec<(String, i64, i64)> {
    let mut out = Vec::new();
    let mut prev: Option<(f64, f64)> = None;
    for row in rows {
        let height = if (10.0..40.0).contains(&row.height) {
            row.height
        } else {
            22.0
        };
        let mut top = row.top;
        if let Some((prev_top, prev_height)) = prev {
            if (top - prev_top).abs() < 4.0 {
                top = prev_top + prev_height;
            }
        }
        if row.label.is_empty() {
            prev = Some((top, height));
            continue;
        }
        prev = Some((top, height));
        let x = (row.left + row.width / 2.0).round() as i64;
        let y = (top + height / 2.0).round() as i64;
        out.push((row.label.clone(), x, y));
    }
    out
}

#[cfg(target_os = "macos")]
unsafe fn attr_box(el: *const std::ffi::c_void) -> Option<(f64, f64, f64, f64)> {
    let pos = copy_attr(el, "AXPosition")?;
    let size = copy_attr(el, "AXSize")?;
    let mut point = CGPoint { x: 0.0, y: 0.0 };
    let mut cg_size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    let ok_pos = AXValueGetValue(pos, 1, &mut point as *mut CGPoint as *mut std::ffi::c_void);
    let ok_size = AXValueGetValue(
        size,
        2,
        &mut cg_size as *mut CGSize as *mut std::ffi::c_void,
    );
    CFRelease(pos);
    CFRelease(size);
    if ok_pos == 0 || ok_size == 0 {
        return None;
    }
    Some((point.x, point.y, cg_size.width, cg_size.height))
}

#[cfg(target_os = "macos")]
unsafe fn attr_center(el: *const std::ffi::c_void) -> Option<(f64, f64)> {
    let (left, top, width, height) = attr_box(el)?;
    Some((left + width / 2.0, top + height / 2.0))
}

#[cfg(target_os = "macos")]
unsafe fn attr_string(el: *const std::ffi::c_void, name: &str) -> Option<String> {
    let value = copy_attr(el, name)?;
    let text = cf_string_value(value);
    CFRelease(value);
    text
}

#[cfg(target_os = "macos")]
unsafe fn attr_bool(el: *const std::ffi::c_void, name: &str) -> bool {
    let Some(value) = copy_attr(el, name) else {
        return false;
    };
    let yes = CFGetTypeID(value) == CFBooleanGetTypeID() && CFBooleanGetValue(value) != 0;
    CFRelease(value);
    yes
}

#[cfg(target_os = "macos")]
thread_local! {
    static LOOK_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

#[cfg(target_os = "macos")]
fn with_look_deadline<T>(read: impl FnOnce() -> T) -> T {
    LOOK_DEADLINE.with(|cell| cell.set(Some(Instant::now() + Duration::from_millis(1200))));
    let value = read();
    LOOK_DEADLINE.with(|cell| cell.set(None));
    value
}

#[cfg(target_os = "macos")]
fn look_budget_spent() -> bool {
    LOOK_DEADLINE.with(|cell| {
        cell.get()
            .is_some_and(|deadline| Instant::now() >= deadline)
    })
}

#[cfg(target_os = "macos")]
const AX_MESSAGE_TIMEOUT: f32 = 0.2;

#[cfg(target_os = "macos")]
unsafe fn limit_element(el: *mut std::ffi::c_void) -> Option<*const std::ffi::c_void> {
    if el.is_null() {
        return None;
    }
    AXUIElementSetMessagingTimeout(el, AX_MESSAGE_TIMEOUT);
    Some(el)
}

#[cfg(target_os = "macos")]
unsafe fn copy_attr(el: *const std::ffi::c_void, name: &str) -> Option<*const std::ffi::c_void> {
    if look_budget_spent() {
        return None;
    }
    if !el.is_null() {
        AXUIElementSetMessagingTimeout(el as *mut std::ffi::c_void, AX_MESSAGE_TIMEOUT);
    }
    let attr = cf_string(name)?;
    let mut value: *const std::ffi::c_void = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(el as *mut std::ffi::c_void, attr, &mut value);
    CFRelease(attr);
    if err != 0 || value.is_null() {
        None
    } else {
        Some(value)
    }
}

#[cfg(target_os = "macos")]
unsafe fn cf_string(text: &str) -> Option<*const std::ffi::c_void> {
    let c = std::ffi::CString::new(text).ok()?;
    let s = CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), 0x0800_0100);
    if s.is_null() {
        None
    } else {
        Some(s)
    }
}

#[cfg(target_os = "macos")]
unsafe fn cf_string_value(value: *const std::ffi::c_void) -> Option<String> {
    if CFGetTypeID(value) != CFStringGetTypeID() {
        return None;
    }
    let len = CFStringGetLength(value) as usize;
    let mut buf = vec![0u8; len * 4 + 1];
    let ok = CFStringGetCString(
        value,
        buf.as_mut_ptr() as *mut std::ffi::c_char,
        buf.len() as isize,
        0x0800_0100,
    );
    if ok == 0 {
        return None;
    }
    Some(
        std::ffi::CStr::from_ptr(buf.as_ptr() as *const std::ffi::c_char)
            .to_string_lossy()
            .into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::{place_menu_rows, MenuRow};

    #[test]
    fn stacked_menu_items_do_not_share_a_row() {
        let rows = vec![
            MenuRow {
                label: "Close Window".into(),
                left: 180.0,
                top: 43.0,
                width: 152.0,
                height: 22.0,
            },
            MenuRow {
                label: "Close All".into(),
                left: 180.0,
                top: 43.0,
                width: 152.0,
                height: 22.0,
            },
        ];
        let placed = place_menu_rows(&rows);
        assert_eq!(placed[0].2, 54);
        assert_eq!(placed[1].2, 76);
        assert!(placed[1].2 > placed[0].2);
    }
}

fn trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        unsafe { AXIsProcessTrusted() != 0 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

fn click_at(x: f64, y: f64) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let (origin_x, origin_y) = last_origin();
        let point = CGPoint {
            x: x + origin_x,
            y: y + origin_y,
        };
        unsafe {
            // A null source is treated as a private event. The menu bar only
            // highlights and never tracks the dropdown.
            let source = CGEventSourceCreate(1);
            if !source.is_null() {
                CGEventSourceSetLocalEventsSuppressionInterval(source, 0.0);
            }
            post_mouse(5, point, source);
            thread::sleep(Duration::from_millis(20));
            post_mouse(1, point, source);
            thread::sleep(Duration::from_millis(80));
            post_mouse(2, point, source);
            if !source.is_null() {
                CFRelease(source as *const std::ffi::c_void);
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (x, y);
        bail!("clicking runs on macOS");
    }
}

#[cfg(target_os = "macos")]
unsafe fn post_mouse(kind: u32, point: CGPoint, source: *mut std::ffi::c_void) {
    let event = CGEventCreateMouseEvent(source, kind, point, 0);
    if !event.is_null() {
        if kind == 1 || kind == 2 {
            // kCGMouseEventClickState
            CGEventSetIntegerValueField(event, 1, 1);
        }
        CGEventPost(0, event);
        CFRelease(event as *const std::ffi::c_void);
    }
}

fn press_key(key: &str) -> Result<()> {
    let key = key.trim().to_ascii_lowercase();
    if key.is_empty() {
        bail!("key is empty");
    }
    let (flags, code) = parse_key(&key)?;
    #[cfg(target_os = "macos")]
    {
        unsafe { post_keycode(code, flags) };
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (flags, code);
        bail!("keys run on macOS");
    }
}

fn parse_key(key: &str) -> Result<(u64, u16)> {
    let mut flags = 0u64;
    let mut token = String::new();
    for part in key.split('+') {
        match part.trim() {
            "command" | "cmd" | "meta" => flags |= 0x0010_0000,
            "shift" => flags |= 0x0002_0000,
            "option" | "alt" => flags |= 0x0008_0000,
            "control" | "ctrl" => flags |= 0x0004_0000,
            other if !other.is_empty() => token = other.to_string(),
            _ => {}
        }
    }
    let code = match token.as_str() {
        "return" | "enter" => 36,
        "escape" | "esc" => 53,
        "tab" => 48,
        "space" => 49,
        "up" => 126,
        "down" => 125,
        "left" => 123,
        "right" => 124,
        "a" => 0,
        "b" => 11,
        "c" => 8,
        "d" => 2,
        "e" => 14,
        "f" => 3,
        "g" => 5,
        "h" => 4,
        "i" => 34,
        "j" => 38,
        "k" => 40,
        "l" => 37,
        "m" => 46,
        "n" => 45,
        "o" => 31,
        "p" => 35,
        "q" => 12,
        "r" => 15,
        "s" => 1,
        "t" => 17,
        "u" => 32,
        "v" => 9,
        "w" => 13,
        "x" => 7,
        "y" => 16,
        "z" => 6,
        other => bail!("unknown key: {other}"),
    };
    Ok((flags, code))
}

#[cfg(target_os = "macos")]
unsafe fn post_keycode(code: u16, flags: u64) {
    let down = CGEventCreateKeyboardEvent(std::ptr::null_mut(), code, true);
    let up = CGEventCreateKeyboardEvent(std::ptr::null_mut(), code, false);
    if !down.is_null() {
        CGEventSetFlags(down, flags);
        CGEventPost(0, down);
        CFRelease(down as *const std::ffi::c_void);
    }
    thread::sleep(Duration::from_millis(20));
    if !up.is_null() {
        CGEventSetFlags(up, flags);
        CGEventPost(0, up);
        CFRelease(up as *const std::ffi::c_void);
    }
}

fn type_text(text: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        for ch in text.encode_utf16() {
            unsafe { post_unicode(ch) };
            thread::sleep(Duration::from_millis(8));
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = text;
        bail!("typing runs on macOS");
    }
}

#[cfg(target_os = "macos")]
unsafe fn post_unicode(ch: u16) {
    let down = CGEventCreateKeyboardEvent(std::ptr::null_mut(), 0, true);
    let up = CGEventCreateKeyboardEvent(std::ptr::null_mut(), 0, false);
    if !down.is_null() {
        CGEventKeyboardSetUnicodeString(down, 1, &ch);
        CGEventPost(0, down);
        CFRelease(down as *const std::ffi::c_void);
    }
    if !up.is_null() {
        CGEventKeyboardSetUnicodeString(up, 1, &ch);
        CGEventPost(0, up);
        CFRelease(up as *const std::ffi::c_void);
    }
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateSystemWide() -> *mut std::ffi::c_void;
    fn AXUIElementCreateApplication(pid: i32) -> *mut std::ffi::c_void;
    fn AXUIElementSetMessagingTimeout(element: *mut std::ffi::c_void, timeout: f32) -> i32;
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGMainDisplayID() -> u32;
    fn CGGetActiveDisplayList(max_displays: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayPixelsWide(display: u32) -> usize;
    fn GetFrontProcess(psn: *mut ProcessSerialNumber) -> i32;
    fn GetProcessPID(psn: *const ProcessSerialNumber, pid: *mut i32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: *mut std::ffi::c_void,
        attribute: *const std::ffi::c_void,
        value: *mut *const std::ffi::c_void,
    ) -> i32;
    fn AXValueGetValue(
        value: *const std::ffi::c_void,
        value_type: i32,
        dest: *mut std::ffi::c_void,
    ) -> u8;
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct CGSize {
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceCreate(state_id: u32) -> *mut std::ffi::c_void;
    fn CGEventSourceSetLocalEventsSuppressionInterval(source: *mut std::ffi::c_void, seconds: f64);
    fn CGEventSetIntegerValueField(event: *mut std::ffi::c_void, field: u32, value: i64);
    fn CGEventCreateMouseEvent(
        source: *mut std::ffi::c_void,
        mouse_type: u32,
        position: CGPoint,
        button: u32,
    ) -> *mut std::ffi::c_void;
    fn CGEventCreateKeyboardEvent(
        source: *mut std::ffi::c_void,
        key: u16,
        down: bool,
    ) -> *mut std::ffi::c_void;
    fn CGEventKeyboardSetUnicodeString(
        event: *mut std::ffi::c_void,
        length: usize,
        chars: *const u16,
    );
    fn CGEventSetFlags(event: *mut std::ffi::c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut std::ffi::c_void);
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: *const std::ffi::c_void);
    fn CFStringCreateWithCString(
        alloc: *const std::ffi::c_void,
        cstr: *const std::ffi::c_char,
        encoding: u32,
    ) -> *const std::ffi::c_void;
    fn CFStringGetLength(value: *const std::ffi::c_void) -> isize;
    fn CFStringGetCString(
        value: *const std::ffi::c_void,
        buffer: *mut std::ffi::c_char,
        size: isize,
        encoding: u32,
    ) -> u8;
    fn CFStringGetTypeID() -> usize;
    fn CFArrayGetCount(array: *const std::ffi::c_void) -> isize;
    fn CFArrayGetValueAtIndex(
        array: *const std::ffi::c_void,
        index: isize,
    ) -> *const std::ffi::c_void;
    fn CFGetTypeID(value: *const std::ffi::c_void) -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: *const std::ffi::c_void) -> u8;
}

#[cfg(target_os = "macos")]
#[link(name = "proc")]
extern "C" {
    fn proc_listpids(
        kind: u32,
        typeinfo: u32,
        buffer: *mut std::ffi::c_void,
        buffersize: i32,
    ) -> i32;
    fn proc_name(pid: i32, buffer: *mut std::ffi::c_void, buffersize: u32) -> i32;
    fn proc_pidpath(pid: i32, buffer: *mut std::ffi::c_void, buffersize: u32) -> i32;
}
