//! Clicks, typing, and screenshots on Windows.
//!
//! The desktop app listens on [`agent_doctor_core::COMPUTER_HAND_PIPE`]. The computer
//! tool sends one JSON line and reads one JSON line back. An app opened as
//! administrator sits above this process, so clicks to it are refused.

use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use windows::core::{IUnknown, BSTR, HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    GetMonitorInfoW, MonitorFromWindow, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY, SRCCOPY,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::Storage::FileSystem::{
    ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTreeWalker,
    UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId, UIA_DocumentControlTypeId,
    UIA_EditControlTypeId, UIA_MenuBarControlTypeId, UIA_MenuControlTypeId,
    UIA_MenuItemControlTypeId, UIA_RadioButtonControlTypeId, UIA_TextControlTypeId,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, MOUSE_EVENT_FLAGS,
    VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetSystemMetrics, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible, WindowFromPoint, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
};

use crate::computer_geom::{
    absolute_axis, control_line, parse_key, pixel_of, point_of, scale_of, KeyPress,
};

const ADMIN_BLOCK: &str = "这个软件是用管理员打开的，点不了。用普通方式打开它再试。";
const CONTROLS_MISSING: &str = "控件名单没有读到。按图上的位置点。\n";

static LAST_SCREEN: Mutex<Option<Desktop>> = Mutex::new(None);

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

fn serve() -> Result<()> {
    let _com = ComInit::new();
    let name = HSTRING::from(agent_doctor_core::COMPUTER_HAND_PIPE);
    loop {
        let pipe = unsafe {
            CreateNamedPipeW(
                &name,
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0),
                // 8 is PIPE_REJECT_REMOTE_CLIENTS. It belongs here.
                // On the open mode, some Windows builds reject the pipe, so the
                // tool hears "file not found". Builds that ignore the bit still work.
                PIPE_TYPE_BYTE | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                None,
            )
        };
        if pipe.is_invalid() {
            thread::sleep(Duration::from_secs(1));
            continue;
        }
        let connected = unsafe { ConnectNamedPipe(pipe, None) };
        if let Err(err) = connected {
            if (err.code().0 as u32 & 0xFFFF) != 535 {
                unsafe {
                    let _ = CloseHandle(pipe);
                }
                thread::sleep(Duration::from_millis(200));
                continue;
            }
        }
        let reply = match read_line(pipe) {
            Ok(line) => match handle_line(&line) {
                Ok(result) => json!({ "ok": true, "result": result }),
                Err(err) => json!({ "ok": false, "error": format!("{err:#}") }),
            },
            Err(err) => json!({ "ok": false, "error": format!("{err:#}") }),
        };
        let body = format!("{reply}\n");
        let mut written = 0u32;
        unsafe {
            let _ = WriteFile(pipe, Some(body.as_bytes()), Some(&mut written), None);
            let _ = DisconnectNamedPipe(pipe);
            let _ = CloseHandle(pipe);
        }
    }
}

struct ComInit {
    uninit: bool,
}

impl ComInit {
    fn new() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Self { uninit: hr.0 == 0 }
    }
}

impl Drop for ComInit {
    fn drop(&mut self) {
        if self.uninit {
            unsafe { CoUninitialize() };
        }
    }
}

fn read_line(pipe: HANDLE) -> Result<String> {
    let mut line = Vec::new();
    let mut buf = [0u8; 4096];
    while !line.contains(&b'\n') && line.len() < 256 * 1024 {
        let mut read = 0u32;
        unsafe { ReadFile(pipe, Some(&mut buf), Some(&mut read), None) }
            .context("could not read the computer request")?;
        if read == 0 {
            break;
        }
        line.extend_from_slice(&buf[..read as usize]);
    }
    let text = String::from_utf8_lossy(&line);
    let text = text.trim();
    if text.is_empty() {
        bail!("empty computer request");
    }
    Ok(text.to_string())
}

fn handle_line(line: &str) -> Result<Value> {
    let req: Value = serde_json::from_str(line).context("bad request")?;
    let tool = req.get("tool").and_then(Value::as_str).unwrap_or("");
    let args = req.get("args").cloned().unwrap_or(json!({}));
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

#[derive(Clone)]
struct Desktop {
    origin_x: i32,
    origin_y: i32,
    width: i32,
    height: i32,
    scale: f64,
}

fn screen_look(app: &str) -> Result<Value> {
    let hwnd = window_for(app);
    let desk = hwnd.map(monitor_of).unwrap_or_else(primary_monitor);
    let shot = capture_png(&desk)?;
    if let Ok(mut saved) = LAST_SCREEN.lock() {
        *saved = Some(desk.clone());
    }
    let mut text = window_header(hwnd);
    if let Some(hwnd) = hwnd {
        if confirmed_elevated(hwnd) {
            text.push_str(ADMIN_BLOCK);
            text.push('\n');
        } else if let Some(body) = read_controls(hwnd, &desk) {
            text.push_str(&body);
        } else {
            text.push_str(CONTROLS_MISSING);
        }
    } else {
        text.push_str(CONTROLS_MISSING);
    }
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

struct Shot {
    path: String,
    pixels: (i64, i64),
    points: (i64, i64),
    scale: f64,
}

fn capture_png(desk: &Desktop) -> Result<Shot> {
    let hdc = unsafe { GetDC(HWND::default()) };
    if hdc.is_invalid() {
        bail!("could not capture the screen");
    }
    let shot = (|| {
        if desk.width <= 0 || desk.height <= 0 {
            bail!("could not capture the screen");
        }
        let mem = unsafe { CreateCompatibleDC(hdc) };
        if mem.is_invalid() {
            bail!("could not capture the screen");
        }
        let bitmap = unsafe { CreateCompatibleBitmap(hdc, desk.width, desk.height) };
        if bitmap.is_invalid() {
            unsafe {
                let _ = DeleteDC(mem);
            }
            bail!("could not capture the screen");
        }
        let previous = unsafe { SelectObject(mem, HGDIOBJ::from(bitmap)) };
        let copied = unsafe {
            BitBlt(
                mem,
                0,
                0,
                desk.width,
                desk.height,
                hdc,
                desk.origin_x,
                desk.origin_y,
                SRCCOPY,
            )
        };
        if !previous.is_invalid() {
            unsafe { SelectObject(mem, previous) };
        }
        let pixels = if copied.is_ok() {
            read_bgra(hdc, bitmap, desk.width, desk.height)
        } else {
            None
        };
        unsafe {
            let _ = DeleteObject(HGDIOBJ::from(bitmap));
            let _ = DeleteDC(mem);
        }
        let Some(bgra) = pixels else {
            bail!("could not capture the screen");
        };
        let mut rgba = bgra;
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("agent-doctor-screen-{stamp}.png"));
        image::save_buffer(
            &path,
            &rgba,
            desk.width as u32,
            desk.height as u32,
            image::ColorType::Rgba8,
        )
        .context("could not capture the screen")?;
        let point_w = point_of(desk.origin_x + desk.width, desk.origin_x, desk.scale).max(1);
        let point_h = point_of(desk.origin_y + desk.height, desk.origin_y, desk.scale).max(1);
        Ok(Shot {
            path: path.display().to_string(),
            pixels: (desk.width as i64, desk.height as i64),
            points: (point_w, point_h),
            scale: desk.scale,
        })
    })();
    unsafe {
        ReleaseDC(HWND::default(), hdc);
    }
    shot
}

fn read_bgra(hdc: HDC, bitmap: HBITMAP, width: i32, height: i32) -> Option<Vec<u8>> {
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];
    let rows = unsafe {
        GetDIBits(
            hdc,
            bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    if rows <= 0 {
        None
    } else {
        Some(pixels)
    }
}

fn window_header(hwnd: Option<HWND>) -> String {
    let Some(hwnd) = hwnd else {
        return String::new();
    };
    let mut out = String::new();
    if let Some(name) = process_name(hwnd) {
        out.push_str(&format!("front app: {name}\n"));
    }
    let title = window_title(hwnd);
    if !title.is_empty() {
        out.push_str(&format!("window: {title}\n"));
    }
    out
}

fn read_controls(hwnd: HWND, desk: &Desktop) -> Option<String> {
    let auto = automation().ok()?;
    let element = unsafe { auto.ElementFromHandle(hwnd) }.ok()?;
    let walker = unsafe { auto.ControlViewWalker() }.ok()?;
    let mut out = String::new();
    let deadline = Instant::now() + Duration::from_millis(1200);
    let mut seen = 0;
    append_controls(&walker, &element, desk, 0, &mut seen, deadline, &mut out);
    if let Ok(menus) = menu_from(&walker, &element, desk, deadline) {
        if !menus.is_empty() {
            out.push_str(&menus);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn menu_text() -> Result<String> {
    let hwnd = window_for("").context("could not see the front app")?;
    if confirmed_elevated(hwnd) {
        return Ok(ADMIN_BLOCK.to_string());
    }
    let desk = monitor_of(hwnd);
    read_controls(hwnd, &desk).context(CONTROLS_MISSING.trim())
}

fn monitor_of(hwnd: HWND) -> Desktop {
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        monitor_desk(monitor).unwrap_or_else(primary_monitor)
    }
}

fn primary_monitor() -> Desktop {
    unsafe {
        let monitor = MonitorFromWindow(HWND::default(), MONITOR_DEFAULTTOPRIMARY);
        monitor_desk(monitor).unwrap_or_else(fallback_desktop)
    }
}

fn monitor_desk(monitor: windows::Win32::Graphics::Gdi::HMONITOR) -> Option<Desktop> {
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.0 == 0 {
        return None;
    }
    let origin_x = info.rcMonitor.left;
    let origin_y = info.rcMonitor.top;
    let width = info.rcMonitor.right.saturating_sub(origin_x).max(1);
    let height = info.rcMonitor.bottom.saturating_sub(origin_y).max(1);
    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    Some(Desktop {
        origin_x,
        origin_y,
        width,
        height,
        scale: scale_of(dpi_x as i32),
    })
}

fn fallback_desktop() -> Desktop {
    let (origin_x, origin_y, width, height) = virtual_screen();
    Desktop {
        origin_x,
        origin_y,
        width,
        height,
        scale: 1.0,
    }
}

fn virtual_screen() -> (i32, i32, i32, i32) {
    let origin_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let origin_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) }.max(1);
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) }.max(1);
    (origin_x, origin_y, width, height)
}

fn automation() -> Result<IUIAutomation> {
    unsafe { CoCreateInstance(&CUIAutomation, None::<&IUnknown>, CLSCTX_INPROC_SERVER) }
        .context("could not see the front app")
}

fn append_controls(
    walker: &IUIAutomationTreeWalker,
    element: &IUIAutomationElement,
    desk: &Desktop,
    depth: u8,
    seen: &mut i32,
    deadline: Instant,
    out: &mut String,
) {
    if depth > 4 || *seen >= 40 || Instant::now() >= deadline {
        return;
    }
    let Ok(mut child) = (unsafe { walker.GetFirstChildElement(element) }) else {
        return;
    };
    loop {
        if *seen >= 40 || Instant::now() >= deadline {
            return;
        }
        let kind = control_type(&child);
        if let Some(role) = control_role(kind) {
            if let Some(label) = element_name(&child) {
                if let Some((x, y)) = element_center(&child, desk) {
                    *seen += 1;
                    out.push_str(&control_line(role, &label, x, y));
                }
            }
        }
        if kind != UIA_MenuBarControlTypeId.0 && kind != UIA_MenuControlTypeId.0 {
            append_controls(walker, &child, desk, depth + 1, seen, deadline, out);
        }
        match unsafe { walker.GetNextSiblingElement(&child) } {
            Ok(next) => child = next,
            Err(_) => return,
        }
    }
}

fn menu_from(
    walker: &IUIAutomationTreeWalker,
    root: &IUIAutomationElement,
    desk: &Desktop,
    deadline: Instant,
) -> Result<String> {
    let mut out = String::new();
    let mut seen = 0;
    walk_menus(walker, root, desk, 0, &mut seen, deadline, &mut out);
    Ok(out)
}

fn walk_menus(
    walker: &IUIAutomationTreeWalker,
    element: &IUIAutomationElement,
    desk: &Desktop,
    depth: u8,
    seen: &mut i32,
    deadline: Instant,
    out: &mut String,
) {
    if depth > 3 || *seen >= 20 || Instant::now() >= deadline {
        return;
    }
    let Ok(mut child) = (unsafe { walker.GetFirstChildElement(element) }) else {
        return;
    };
    loop {
        if *seen >= 20 || Instant::now() >= deadline {
            return;
        }
        let kind = control_type(&child);
        if kind == UIA_MenuItemControlTypeId.0 {
            if let Some(label) = element_name(&child) {
                if let Some((x, y)) = element_center(&child, desk) {
                    *seen += 1;
                    out.push_str(&control_line("button", &label, x, y));
                }
            }
        }
        if kind == UIA_MenuBarControlTypeId.0 || kind == UIA_MenuControlTypeId.0 || depth == 0 {
            walk_menus(walker, &child, desk, depth + 1, seen, deadline, out);
        }
        match unsafe { walker.GetNextSiblingElement(&child) } {
            Ok(next) => child = next,
            Err(_) => return,
        }
    }
}

fn control_type(element: &IUIAutomationElement) -> i32 {
    unsafe { element.CurrentControlType() }
        .map(|id| id.0)
        .unwrap_or(0)
}

fn control_role(kind: i32) -> Option<&'static str> {
    if kind == UIA_ButtonControlTypeId.0
        || kind == UIA_CheckBoxControlTypeId.0
        || kind == UIA_RadioButtonControlTypeId.0
    {
        Some("button")
    } else if kind == UIA_EditControlTypeId.0 || kind == UIA_DocumentControlTypeId.0 {
        Some("text field")
    } else if kind == UIA_TextControlTypeId.0 {
        Some("text")
    } else {
        None
    }
}

fn element_name(element: &IUIAutomationElement) -> Option<String> {
    let name: BSTR = unsafe { element.CurrentName() }.ok()?;
    let text = name.to_string();
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn element_center(element: &IUIAutomationElement, desk: &Desktop) -> Option<(i64, i64)> {
    let rect: RECT = unsafe { element.CurrentBoundingRectangle() }.ok()?;
    let width = rect.right.saturating_sub(rect.left);
    let height = rect.bottom.saturating_sub(rect.top);
    if width <= 0 || height <= 0 {
        return None;
    }
    let x = rect.left.saturating_add(width / 2);
    let y = rect.top.saturating_add(height / 2);
    Some((
        point_of(x, desk.origin_x, desk.scale),
        point_of(y, desk.origin_y, desk.scale),
    ))
}

fn window_for(app: &str) -> Option<HWND> {
    if app.trim().is_empty() {
        let hwnd = unsafe { GetForegroundWindow() };
        return (!hwnd.is_invalid()).then_some(hwnd);
    }
    let hwnds = top_windows();
    let want = app.trim();
    let mut title_hit = None;
    for hwnd in hwnds {
        if process_name(hwnd).is_some_and(|name| name.eq_ignore_ascii_case(want)) {
            return Some(hwnd);
        }
        if title_hit.is_none()
            && window_title(hwnd)
                .to_ascii_lowercase()
                .contains(&want.to_ascii_lowercase())
        {
            title_hit = Some(hwnd);
        }
    }
    title_hit
}

fn app_names() -> Result<Vec<String>> {
    let mut names = Vec::new();
    for hwnd in top_windows() {
        if names.len() >= 40 {
            break;
        }
        let Some(name) = process_name(hwnd) else {
            continue;
        };
        if names
            .iter()
            .any(|have: &String| have.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        names.push(name);
    }
    if names.is_empty() {
        bail!("could not see the open apps");
    }
    Ok(names)
}

fn top_windows() -> Vec<HWND> {
    let mut hwnds = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(push_hwnd),
            LPARAM(&mut hwnds as *mut Vec<HWND> as isize),
        );
    }
    hwnds
        .into_iter()
        .filter(|hwnd| unsafe { IsWindowVisible(*hwnd) }.0 != 0)
        .filter(|hwnd| !window_title(*hwnd).is_empty())
        .collect()
}

unsafe extern "system" fn push_hwnd(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if lparam.0 != 0 {
        let list = &mut *(lparam.0 as *mut Vec<HWND>);
        list.push(hwnd);
    }
    BOOL(1)
}

fn window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize])
}

fn process_name(hwnd: HWND) -> Option<String> {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }
    process_stem(pid)
}

fn process_stem(pid: u32) -> Option<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL(0), pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    };
    unsafe {
        let _ = CloseHandle(process);
    }
    ok.ok()?;
    let n = (len as usize).min(buf.len());
    let path = String::from_utf16_lossy(&buf[..n]);
    let file = path.rsplit(['\\', '/']).next().unwrap_or(&path);
    let stem = if file.len() >= 4 && file[file.len() - 4..].eq_ignore_ascii_case(".exe") {
        &file[..file.len() - 4]
    } else {
        file
    };
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_string())
    }
}

fn confirmed_elevated(hwnd: HWND) -> bool {
    if self_elevated() {
        return false;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid != 0 && process_elevated(pid) == Some(true)
}

fn refuse_if_higher(hwnd: HWND) -> Result<()> {
    if confirmed_elevated(hwnd) {
        bail!(ADMIN_BLOCK);
    }
    Ok(())
}

fn self_elevated() -> bool {
    let process = unsafe { GetCurrentProcess() };
    token_elevated(process).unwrap_or(false)
}

fn process_elevated(pid: u32) -> Option<bool> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL(0), pid) }.ok()?;
    let elevated = token_elevated(process);
    unsafe {
        let _ = CloseHandle(process);
    }
    elevated
}

fn token_elevated(process: HANDLE) -> Option<bool> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.ok()?;
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    unsafe {
        let _ = CloseHandle(token);
    }
    ok.ok()?;
    Some(elevation.TokenIsElevated != 0)
}

fn click_at(x: f64, y: f64) -> Result<()> {
    let desk = click_screen();
    let (virt_x, virt_y, virt_w, virt_h) = virtual_screen();
    let px = pixel_of(x, desk.origin_x, desk.scale);
    let py = pixel_of(y, desk.origin_y, desk.scale);
    let ax = absolute_axis(px, virt_x, virt_w);
    let ay = absolute_axis(py, virt_y, virt_h);
    let hwnd = unsafe { WindowFromPoint(POINT { x: px, y: py }) };
    if !hwnd.is_invalid() {
        refuse_if_higher(hwnd)?;
    }
    let flags = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    send_mouse(ax, ay, MOUSEEVENTF_MOVE | flags)?;
    thread::sleep(Duration::from_millis(20));
    send_mouse(ax, ay, MOUSEEVENTF_LEFTDOWN | flags)?;
    thread::sleep(Duration::from_millis(80));
    send_mouse(ax, ay, MOUSEEVENTF_LEFTUP | flags)?;
    Ok(())
}

fn click_screen() -> Desktop {
    if let Ok(saved) = LAST_SCREEN.lock() {
        if let Some(desk) = saved.clone() {
            return desk;
        }
    }
    window_for("")
        .map(monitor_of)
        .unwrap_or_else(primary_monitor)
}

fn send_mouse(dx: i32, dy: i32, flags: MOUSE_EVENT_FLAGS) -> Result<()> {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        bail!("the click did not go through");
    }
    Ok(())
}

fn press_key(key: &str) -> Result<()> {
    let press = parse_key(key).map_err(anyhow::Error::msg)?;
    send_key(&press)
}

fn send_key(press: &KeyPress) -> Result<()> {
    let mut mods = Vec::new();
    if press.control {
        mods.push(VK_CONTROL);
    }
    if press.shift {
        mods.push(VK_SHIFT);
    }
    if press.alt {
        mods.push(VK_MENU);
    }
    if press.win {
        mods.push(VK_LWIN);
    }
    for vk in &mods {
        send_vk(*vk, false)?;
    }
    send_vk(VIRTUAL_KEY(press.vk), false)?;
    thread::sleep(Duration::from_millis(20));
    send_vk(VIRTUAL_KEY(press.vk), true)?;
    for vk in mods.iter().rev() {
        send_vk(*vk, true)?;
    }
    Ok(())
}

fn send_vk(vk: VIRTUAL_KEY, up: bool) -> Result<()> {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        bail!("keys did not go through");
    }
    Ok(())
}

fn type_text(text: &str) -> Result<()> {
    for unit in text.encode_utf16() {
        send_unicode(unit, false)?;
        send_unicode(unit, true)?;
        thread::sleep(Duration::from_millis(8));
    }
    Ok(())
}

fn send_unicode(unit: u16, up: bool) -> Result<()> {
    let mut flags = KEYEVENTF_UNICODE;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: unit,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        bail!("typing did not go through");
    }
    Ok(())
}
