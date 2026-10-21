//! Clicks and keystrokes run in this app. macOS blocks `osascript` from sending
//! them, even when this app is allowed to control the computer.
//!
//! Tool names, argument checks and reply shapes live in
//! [`crate::computer_hand_common`]; this file is the macOS way of doing them.

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

use crate::computer_geom::{crop_pixels, point_inside, window_has_control, window_has_field};
use crate::computer_hand_common as proto;

pub fn spawn() {
    proto::spawn(serve);
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
            let reply = proto::envelope(proto::handle_line(&MacHand, &line));
            let _ = writeln!(write_half, "{reply}");
        });
    }
    Ok(())
}

struct MacHand;

impl proto::Hand for MacHand {
    fn cua_open(&self, args: &Value) -> Option<Value> {
        try_cua_open(args)
    }

    fn ensure_trusted(&self) -> Result<()> {
        ensure_trusted()
    }

    fn click_at(&self, x: f64, y: f64) -> Result<()> {
        click_at(x, y)
    }

    fn press_key(&self, key: &str) -> Result<()> {
        press_key(key)
    }

    fn scroll_view(&self, args: &Value) -> Result<Value> {
        let direction = args
            .get("direction")
            .and_then(Value::as_str)
            .unwrap_or("down");
        let amount = args
            .get("amount")
            .and_then(Value::as_i64)
            .unwrap_or(4)
            .clamp(1, 12) as i32;
        let lines = if direction.eq_ignore_ascii_case("up") {
            amount
        } else {
            -amount
        };
        let (x, y) = scroll_point(args)?;
        require_point(x, y)?;
        scroll_at(x, y, lines)?;
        Ok(json!({ "scrolled": direction, "x": x, "y": y, "amount": amount }))
    }

    fn type_text(&self, text: &str) -> Result<()> {
        type_text(text)
    }

    fn menu_text(&self) -> Result<String> {
        menu_text()
    }

    fn app_names(&self) -> Result<Vec<String>> {
        app_names()
    }

    fn open_named(&self, app: &str) -> Result<Value> {
        open_named_app(app)
    }

    fn list_windows(&self, app: &str) -> Result<Value> {
        let windows = crate::computer_contrl::list_windows(app).map_err(anyhow::Error::msg)?;
        Ok(json!({
            "windows": windows
                .into_iter()
                .map(|window| json!({ "app": window.app, "title": window.title }))
                .collect::<Vec<_>>()
        }))
    }

    fn window_state(&self, app: &str) -> Result<Value> {
        let state = crate::computer_contrl::window_state(app).map_err(anyhow::Error::msg)?;
        proto::remember_field(
            state
                .controls
                .iter()
                .any(|control| control.role == "text field"),
        );
        Ok(json!({
            "app": state.app,
            "window": state.title,
            "controls": state
                .controls
                .into_iter()
                .map(|control| json!({
                    "role": control.role,
                    "label": control.label,
                    "x": control.x,
                    "y": control.y
                }))
                .collect::<Vec<_>>()
        }))
    }

    fn click_label(&self, app: &str, label: &str) -> Result<Value> {
        crate::computer_contrl::click_control(app, label).map_err(anyhow::Error::msg)?;
        confirm_click()?;
        Ok(json!({ "clicked": label }))
    }

    fn set_label(&self, app: &str, label: &str, text: &str) -> Result<Value> {
        crate::computer_contrl::set_value(app, label, text).map_err(|err| {
            if err.contains("没有找到") {
                anyhow::anyhow!("没有这个输入框。先点一下要填的地方，再用打字。")
            } else {
                anyhow::anyhow!(err)
            }
        })?;
        Ok(json!({ "set": label }))
    }

    fn click_menu(&self, app: &str, names: Vec<String>) -> Result<Value> {
        crate::computer_contrl::invoke_menu(app, &names).map_err(anyhow::Error::msg)?;
        Ok(json!({ "menu": names }))
    }

    fn look(&self, app: &str) -> Result<Value> {
        screen_look(app)
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

fn one_line(label: &str) -> String {
    label.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Look {
    text: String,
    frame: Option<(f64, f64, f64, f64)>,
}

fn screen_look(app: &str) -> Result<Value> {
    // A missing picture must fail the tool. Returning the menu names alone
    // makes the assistant click without seeing the screen.
    let shot = capture_png(app)?;
    let look = look_text(app)?;
    proto::remember_field(window_has_field(&look.text));
    let (shot, cropped) = fit_window(shot, &look);
    if let Some(print) = proto::shot_fingerprint(&shot.path) {
        proto::remember_print(print);
    }
    let text = proto::look_text(&shot, cropped, &look.text);
    Ok(proto::look_value(text, &shot))
}

fn look_text(app: &str) -> Result<Look> {
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
unsafe fn ax_look(want: &str) -> Result<Look> {
    let app = if want.is_empty() {
        front_app_element()
    } else {
        app_element_named(want).or_else(|| front_app_element())
    };
    let Some(app) = app else {
        bail!("could not see the front app");
    };
    let mut out = String::new();
    let mut frame = None;
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
                frame = attr_box(window);
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
    Ok(Look { text: out, frame })
}

#[cfg(target_os = "macos")]
unsafe fn append_controls(
    out: &mut String,
    el: *const std::ffi::c_void,
    depth: u8,
    seen: &mut i32,
) {
    if depth > 6 || *seen >= 60 {
        return;
    }
    let Some(children) = copy_attr(el, "AXChildren") else {
        return;
    };
    let count = CFArrayGetCount(children);
    for i in 0..count {
        if *seen >= 60 {
            break;
        }
        let child = CFArrayGetValueAtIndex(children, i);
        if child.is_null() {
            continue;
        }
        let role = attr_string(child, "AXRole").unwrap_or_default();
        if matches!(
            role.as_str(),
            "AXButton" | "AXTextField" | "AXTextArea" | "AXStaticText" | "AXCheckBox"
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
                        "AXTextField" | "AXTextArea" => "text field",
                        _ => "text",
                    };
                    let label = one_line(&label);
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

fn try_cua_open(args: &Value) -> Option<Value> {
    let name = args.get("app").and_then(Value::as_str).unwrap_or("").trim();
    if name.is_empty() || name.contains(['"', '\n', '\r']) {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        let apps = installed_apps(name);
        let app = pick_named_app(name, &apps)?;
        if !crate::computer_contrl::cua::launch(app.bundle_id.as_deref(), name) {
            return None;
        }
        Some(json!({ "opened": name, "path": app.path, "bundleId": app.bundle_id }))
    }
    #[cfg(not(target_os = "macos"))]
    {
        if !crate::computer_contrl::cua::launch(None, name) {
            return None;
        }
        Some(json!({ "opened": name }))
    }
}

fn app_names() -> Result<Vec<String>> {
    #[cfg(target_os = "macos")]
    {
        crate::computer_contrl::list_apps().map_err(anyhow::Error::msg)
    }
    #[cfg(not(target_os = "macos"))]
    {
        bail!("apps are listed on macOS");
    }
}

struct NamedApp {
    names: Vec<String>,
    path: String,
    bundle_id: Option<String>,
}

/// Exact name only. 微信 must not open 微信开发者工具.
fn pick_named_app<'a>(query: &str, apps: &'a [NamedApp]) -> Option<&'a NamedApp> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return None;
    }
    let mut matches: Vec<&NamedApp> = apps
        .iter()
        .filter(|app| {
            app.names
                .iter()
                .any(|name| name.trim().to_lowercase() == query)
        })
        .collect();
    matches.sort_by_key(|app| {
        let in_applications = if app.path.starts_with("/Applications/") {
            0
        } else {
            1
        };
        (in_applications, app.path.len())
    });
    matches.into_iter().next()
}

fn open_named_app(name: &str) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        let name = name.trim();
        if name.is_empty() {
            bail!("说一下要打开的应用名字。");
        }
        if name.contains(['"', '\n', '\r']) {
            bail!("应用名字里不要带引号。");
        }
        let apps = installed_apps(name);
        let Some(app) = pick_named_app(name, &apps) else {
            bail!("没有找到叫「{name}」的应用。");
        };
        let status = crate::computer_contrl::bring_to_front(app.bundle_id.as_deref(), &app.path)
            .context("打不开这个应用")?;
        if !status.success() {
            bail!("没有打开「{name}」。");
        }
        Ok(json!({ "opened": name, "path": app.path, "bundleId": app.bundle_id }))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = name;
        bail!("按名字打开应用目前只在 Mac 上。");
    }
}

#[cfg(target_os = "macos")]
fn installed_apps(query: &str) -> Vec<NamedApp> {
    let mut apps = Vec::new();
    let mut push_path = |path: String| {
        let path = path.trim().trim_end_matches('/').to_string();
        if !path.ends_with(".app") || apps.iter().any(|app: &NamedApp| app.path == path) {
            return;
        }
        let stem = path
            .rsplit('/')
            .next()
            .unwrap_or("")
            .trim_end_matches(".app")
            .to_string();
        let mut names = vec![stem];
        if let Some(display) = spotlight_value(&path, "kMDItemDisplayName") {
            if !names.iter().any(|name| name == &display) {
                names.push(display);
            }
        }
        let bundle_id = spotlight_value(&path, "kMDItemCFBundleIdentifier");
        apps.push(NamedApp {
            names,
            path,
            bundle_id,
        });
    };
    for path in spotlight_apps(query) {
        push_path(path);
    }
    for path in running_app_paths() {
        push_path(path);
    }
    apps
}

#[cfg(target_os = "macos")]
fn spotlight_apps(query: &str) -> Vec<String> {
    let mut found = Vec::new();
    let queries = [
        format!(
            "kMDItemContentTypeTree == 'com.apple.application-bundle' && kMDItemDisplayName == '{query}'cd"
        ),
        format!(
            "kMDItemContentTypeTree == 'com.apple.application-bundle' && kMDItemFSName == '{query}.app'cd"
        ),
    ];
    for query in queries {
        let output = std::process::Command::new("/usr/bin/mdfind")
            .arg(&query)
            .output();
        let Ok(output) = output else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let line = line.trim();
            if line.ends_with(".app") {
                found.push(line.to_string());
            }
        }
    }
    found
}

#[cfg(target_os = "macos")]
fn spotlight_value(path: &str, field: &str) -> Option<String> {
    let output = std::process::Command::new("/usr/bin/mdls")
        .args(["-name", field, "-raw", path])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() || text == "(null)" {
        None
    } else {
        Some(text)
    }
}

#[cfg(target_os = "macos")]
fn running_app_paths() -> Vec<String> {
    let mut paths = Vec::new();
    let mut pids = vec![0i32; 1024];
    let bytes = unsafe {
        proc_listpids(
            1,
            0,
            pids.as_mut_ptr() as *mut std::ffi::c_void,
            (pids.len() * 4) as i32,
        )
    };
    if bytes <= 0 {
        return paths;
    }
    let count = (bytes as usize / 4).min(pids.len());
    for pid in pids.into_iter().take(count) {
        if pid <= 0 || paths.len() >= 80 {
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
        let app = format!("{bundle}.app");
        if !paths.iter().any(|have| have == &app) {
            paths.push(app);
        }
    }
    paths
}

fn capture_png(app: &str) -> Result<proto::Shot> {
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
        let shot = capture_rect(
            display.origin_x,
            display.origin_y,
            display.width,
            display.height,
            Some(display.id),
        )?;
        remember_origin(display.origin_x, display.origin_y);
        remember_size(display.width, display.height);
        Ok(shot)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        bail!("the screen is captured on macOS");
    }
}

#[cfg(target_os = "macos")]
fn capture_rect(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    display_id: Option<u32>,
) -> Result<proto::Shot> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = format!("/tmp/agent-doctor-screen-{stamp}.png");
    let pt_w = width.round().max(1.0);
    let pt_h = height.round().max(1.0);
    let rect = format!(
        "{},{},{},{}",
        x.round() as i64,
        y.round() as i64,
        pt_w as i64,
        pt_h as i64
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
    let display_scale = display_id.map(display_scale).unwrap_or(1.0);
    let scale = if px_w > 0 && pt_w > 0.0 {
        px_w as f64 / pt_w
    } else {
        display_scale
    };
    Ok(proto::Shot {
        path,
        pixels: (px_w, px_h),
        points: (pt_w as i64, pt_h as i64),
        scale,
    })
}

fn fit_window(shot: proto::Shot, look: &Look) -> (proto::Shot, bool) {
    #[cfg(target_os = "macos")]
    {
        if window_has_control(&look.text) {
            return (shot, false);
        }
        let Some(frame) = look.frame else {
            return (shot, false);
        };
        crop_shot_to_window(shot, frame)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = look;
        (shot, false)
    }
}

#[cfg(target_os = "macos")]
fn crop_shot_to_window(shot: proto::Shot, frame: (f64, f64, f64, f64)) -> (proto::Shot, bool) {
    let (origin_x, origin_y) = last_origin();
    let Some((x, y, width, height)) = crop_pixels(
        crate::computer_geom::Picture {
            origin_x,
            origin_y,
            scale: shot.scale,
            pixel_w: shot.pixels.0,
            pixel_h: shot.pixels.1,
        },
        frame.0,
        frame.1,
        frame.2,
        frame.3,
    ) else {
        return (shot, false);
    };
    let Ok(image) = image::open(&shot.path) else {
        return (shot, false);
    };
    let cropped = image.crop_imm(x, y, width, height);
    if cropped.save(&shot.path).is_err() {
        return (shot, false);
    }
    let scale = if shot.scale > 0.0 { shot.scale } else { 1.0 };
    let vis_x = origin_x + f64::from(x) / scale;
    let vis_y = origin_y + f64::from(y) / scale;
    let vis_w = (f64::from(width) / scale).round().max(1.0);
    let vis_h = (f64::from(height) / scale).round().max(1.0);
    remember_origin(vis_x, vis_y);
    remember_size(vis_w, vis_h);
    let pixels = png_size(&shot.path).unwrap_or((i64::from(width), i64::from(height)));
    (
        proto::Shot {
            path: shot.path,
            pixels,
            points: (vis_w as i64, vis_h as i64),
            scale,
        },
        true,
    )
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
static LAST_SIZE: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));

#[cfg(target_os = "macos")]
fn remember_origin(x: f64, y: f64) {
    *LAST_ORIGIN.lock().unwrap_or_else(|err| err.into_inner()) = (x, y);
}

#[cfg(target_os = "macos")]
fn last_origin() -> (f64, f64) {
    *LAST_ORIGIN.lock().unwrap_or_else(|err| err.into_inner())
}

fn remember_size(width: f64, height: f64) {
    *LAST_SIZE.lock().unwrap_or_else(|err| err.into_inner()) = (width, height);
}

fn require_point(x: f64, y: f64) -> Result<()> {
    let (width, height) = last_size();
    if width <= 0.0 || height <= 0.0 {
        bail!("先看一下屏幕，再说要点哪里。");
    }
    if !point_inside(x, y, width, height) {
        bail!("这个位置不在刚才的画面里。先看一眼屏幕，再用画面里的点。");
    }
    Ok(())
}

fn last_size() -> (f64, f64) {
    *LAST_SIZE.lock().unwrap_or_else(|err| err.into_inner())
}

/// The verification shot after a click: whatever the last picture showed.
fn confirm_click() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        proto::confirm_picture(|| {
            let (origin_x, origin_y) = last_origin();
            let (width, height) = last_size();
            capture_rect(origin_x, origin_y, width, height, None)
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
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
    use super::{pick_named_app, place_menu_rows, MenuRow, NamedApp};

    #[test]
    fn an_exact_name_does_not_open_a_longer_one() {
        let apps = vec![
            NamedApp {
                names: vec!["微信开发者工具".into(), "wechatwebdevtools".into()],
                path: "/Applications/wechatwebdevtools.app".into(),
                bundle_id: Some("com.tencent.wechatwebdevtools".into()),
            },
            NamedApp {
                names: vec!["WeChat".into(), "微信".into()],
                path: "/Applications/WeChat.app".into(),
                bundle_id: Some("com.tencent.xinWeChat".into()),
            },
            NamedApp {
                names: vec!["微信".into()],
                path: "/Users/airlu/Applications/微信.app".into(),
                bundle_id: Some("com.tencent.xinWeChat".into()),
            },
        ];
        let picked = pick_named_app("微信", &apps).unwrap();
        assert_eq!(picked.path, "/Applications/WeChat.app");
        assert!(pick_named_app("微信开发者工具", &apps)
            .unwrap()
            .path
            .contains("wechatwebdevtools"));
        assert!(pick_named_app("扫描全能王", &apps).is_none());
    }

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

fn scroll_point(args: &Value) -> Result<(f64, f64)> {
    if let (Some(x), Some(y)) = (
        args.get("x").and_then(Value::as_f64),
        args.get("y").and_then(Value::as_f64),
    ) {
        return Ok((x, y));
    }
    let (width, height) = last_size();
    if width <= 0.0 || height <= 0.0 {
        bail!("先看一下屏幕，再说在哪里滑动。");
    }
    Ok((width / 2.0, height / 2.0))
}

fn scroll_at(x: f64, y: f64, lines: i32) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let (origin_x, origin_y) = last_origin();
        crate::computer_contrl::scroll_point(x + origin_x, y + origin_y, lines)
            .map_err(anyhow::Error::msg)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (x, y, lines);
        bail!("目前只在 Mac 上。");
    }
}

fn click_at(x: f64, y: f64) -> Result<()> {
    require_point(x, y)?;
    #[cfg(target_os = "macos")]
    {
        let (origin_x, origin_y) = last_origin();
        crate::computer_contrl::click_point(x + origin_x, y + origin_y)
            .map_err(anyhow::Error::msg)?;
        confirm_click()
    }
    #[cfg(not(target_os = "macos"))]
    {
        bail!("clicking runs on macOS");
    }
}

fn press_key(key: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let key = key.trim();
        if key.contains('+') {
            crate::computer_contrl::hotkey(key).map_err(anyhow::Error::msg)
        } else {
            crate::computer_contrl::press(key).map_err(anyhow::Error::msg)
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = key;
        bail!("keys run on macOS");
    }
}

fn type_text(text: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        crate::computer_contrl::type_text(text).map_err(anyhow::Error::msg)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = text;
        bail!("typing runs on macOS");
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
