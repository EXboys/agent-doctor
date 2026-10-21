//! Clicks, typing, and screenshots on Windows.
//!
//! The desktop app listens on [`agent_doctor_core::COMPUTER_HAND_PIPE`]. The computer
//! tool sends one JSON line and reads one JSON line back. The reply stays in the
//! pipe until that tool closes it. Disconnecting sooner throws the reply away, and
//! the tool hears "管道的另一端上无任何进程". An app opened as administrator sits
//! above this process, so clicks to it are refused.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use windows::core::{IUnknown, BSTR, HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM, POINT, RECT, WPARAM};
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
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PeekNamedPipe, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcess, GetCurrentThreadId, OpenProcess, OpenProcessToken,
    QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTreeWalker,
    IUIAutomationValuePattern, UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId,
    UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_MenuBarControlTypeId,
    UIA_MenuControlTypeId, UIA_MenuItemControlTypeId, UIA_RadioButtonControlTypeId,
    UIA_TextControlTypeId, UIA_ValuePatternId,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
    VK_CONTROL, VK_LWIN, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetSystemMetrics, GetWindowRect, GetWindowThreadProcessId,
    IsWindowVisible, SendMessageTimeoutW, SetForegroundWindow, ShowWindow, WindowFromPoint,
    SEND_MESSAGE_TIMEOUT_FLAGS, SMTO_ABORTIFHUNG, SMTO_ERRORONEXIT, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_RESTORE, SW_SHOWNORMAL,
    WM_GETTEXT,
};

use crate::computer_geom::{
    crop_pixels, picture_changed, point_inside, window_has_control, window_has_field,
};

use crate::computer_geom::{
    absolute_axis, control_line, parse_key, pixel_of, point_of, scale_of, KeyPress,
};

const ADMIN_BLOCK: &str = "这个软件是用管理员打开的，点不了。用普通方式打开它再试。";
const CONTROLS_MISSING: &str = "控件名单没有读到。按图上的位置点。\n";

static LAST_SCREEN: Mutex<Option<Desktop>> = Mutex::new(None);
static LAST_PRINT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static LAST_FIELD: AtomicBool = AtomicBool::new(false);

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
    let name = HSTRING::from(agent_doctor_core::COMPUTER_HAND_PIPE);
    loop {
        let pipe = unsafe {
            CreateNamedPipeW(
                &name,
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0),
                // Do not set PIPE_REJECT_REMOTE_CLIENTS. On some Windows PCs the
                // tool is treated as a remote client, and then the pipe is hidden:
                // the tool hears "系统找不到指定的文件".
                PIPE_TYPE_BYTE,
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
        // Answer on another thread. A slow open must not stop the next listen,
        // or the following screenshot reports that the computer is not connected.
        let raw = pipe.0 as usize;
        let started = thread::Builder::new()
            .name("computer-hand-job".into())
            .spawn(move || answer_pipe(HANDLE(raw as *mut core::ffi::c_void)));
        if started.is_err() {
            unsafe {
                let _ = DisconnectNamedPipe(pipe);
                let _ = CloseHandle(pipe);
            }
        }
    }
}

fn answer_pipe(pipe: HANDLE) {
    let _com = ComInit::new();
    let reply =
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match read_line(pipe) {
            Ok(line) => match handle_line(&line) {
                Ok(result) => json!({ "ok": true, "result": result }),
                Err(err) => json!({ "ok": false, "error": format!("{err:#}") }),
            },
            Err(err) => json!({ "ok": false, "error": format!("{err:#}") }),
        })) {
            Ok(reply) => reply,
            Err(_) => json!({ "ok": false, "error": "看屏幕这一步中断了。请再试一次。" }),
        };
    let body = format!("{reply}\n");
    // DisconnectNamedPipe discards bytes still sitting in the pipe. Wait
    // until the tool has closed its end. It does that only after reading.
    if let Err(err) = write_pipe(pipe, body.as_bytes()) {
        eprintln!("computer hand reply was not written: {err:#}");
    } else if let Err(err) = wait_until_tool_closes(pipe) {
        eprintln!("computer hand reply was not read: {err:#}");
    }
    unsafe {
        let _ = DisconnectNamedPipe(pipe);
        let _ = CloseHandle(pipe);
    }
}

fn write_pipe(pipe: HANDLE, bytes: &[u8]) -> Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        let mut written = 0u32;
        unsafe { WriteFile(pipe, Some(&bytes[offset..]), Some(&mut written), None) }
            .context("could not answer the computer tool")?;
        if written == 0 {
            bail!("could not answer the computer tool");
        }
        offset += written as usize;
    }
    Ok(())
}

fn wait_until_tool_closes(pipe: HANDLE) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut buf = [0u8; 32];
    loop {
        let mut available = 0u32;
        match unsafe { PeekNamedPipe(pipe, None, 0, None, Some(&mut available), None) } {
            Ok(()) => {
                if available > 0 {
                    let mut read = 0u32;
                    unsafe { ReadFile(pipe, Some(&mut buf), Some(&mut read), None) }
                        .context("could not finish the computer reply")?;
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    bail!("computer tool left the reply unread");
                }
                thread::sleep(Duration::from_millis(15));
            }
            Err(err) => {
                // The tool closed this end. 109 broken pipe, 232 no data,
                // 233 not connected — the same 0x800700E9 the tool used to see
                // when we disconnected first.
                let code = err.code().0 as u32 & 0xFFFF;
                if matches!(code, 109 | 232 | 233) {
                    return Ok(());
                }
                return Err(err).context("could not finish the computer reply");
            }
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

fn try_cua(tool: &str, args: &Value) -> Option<Value> {
    match tool {
        "screen_apps" => {
            let apps = crate::computer_contrl::cua::running_apps()?;
            Some(json!({ "apps": apps }))
        }
        "screen_open" => {
            let name = args.get("app").and_then(Value::as_str).unwrap_or("").trim();
            if name.is_empty() || !crate::computer_contrl::cua::launch(None, name) {
                return None;
            }
            Some(json!({ "opened": name }))
        }
        "screen_menu" => {
            let names = args
                .get("path")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let app = args.get("app").and_then(Value::as_str).unwrap_or("");
            if names.is_empty() || !crate::computer_contrl::cua::menu(app, &names) {
                return None;
            }
            Some(json!({ "menu": names }))
        }
        "screen_windows" => {
            let app = args.get("app").and_then(Value::as_str).unwrap_or("");
            let windows = crate::computer_contrl::cua::windows(app)?;
            Some(json!({ "windows": windows }))
        }
        _ => None,
    }
}

fn handle_line(line: &str) -> Result<Value> {
    let req: Value = serde_json::from_str(line).context("bad request")?;
    let tool = req.get("tool").and_then(Value::as_str).unwrap_or("");
    let args = req.get("args").cloned().unwrap_or(json!({}));
    if let Some(done) = try_cua(tool, &args) {
        return Ok(done);
    }
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
        "screen_open" => open_named(args.get("app").and_then(Value::as_str).unwrap_or("")),
        "screen_windows" => {
            list_named_windows(args.get("app").and_then(Value::as_str).unwrap_or(""))
        }
        "screen_state" => window_controls(args.get("app").and_then(Value::as_str).unwrap_or("")),
        "screen_click_label" => click_label(
            args.get("app").and_then(Value::as_str).unwrap_or(""),
            args.get("label").and_then(Value::as_str).unwrap_or(""),
        ),
        "screen_set" => set_labeled(
            args.get("app").and_then(Value::as_str).unwrap_or(""),
            args.get("label").and_then(Value::as_str).unwrap_or(""),
            args.get("text").and_then(Value::as_str).unwrap_or(""),
        ),
        "screen_menu" => click_menu(
            args.get("app").and_then(Value::as_str).unwrap_or(""),
            string_list(args.get("path")),
        ),
        "screen_scroll" => scroll_view(&args),
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
    let mut desk = hwnd.map(monitor_of).unwrap_or_else(primary_monitor);
    let mut shot = capture_png(&desk)?;
    let mut body = String::new();
    let mut cropped = false;
    if let Some(hwnd) = hwnd {
        if confirmed_elevated(hwnd) {
            body.push_str(ADMIN_BLOCK);
            body.push('\n');
        } else if let Some(controls) = read_controls(hwnd, &desk) {
            body = controls;
        } else {
            body = CONTROLS_MISSING.to_string();
        }
        if !window_has_control(&body) {
            if let Some(frame) = window_frame(hwnd) {
                if let Some((next_desk, next_shot)) = crop_to_window(&shot, &desk, frame) {
                    desk = next_desk;
                    shot = next_shot;
                    cropped = true;
                }
            }
        }
    } else {
        body = CONTROLS_MISSING.to_string();
    }
    remember_field(window_has_field(&body));
    if let Some(print) = shot_fingerprint(&shot.path) {
        remember_print(print);
    }
    *LAST_SCREEN.lock().unwrap_or_else(|err| err.into_inner()) = Some(desk);
    let mut text =
        "Rows are @eN name x y. Click that @eN. If a row is not listed, call screen_scroll on the list.\n"
            .to_string();
    if cropped {
        text.push_str("The picture is this app's window.\n");
    }
    text.push_str(&window_header(hwnd));
    text.push_str(&body);
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
    // GetWindowText waits on the other app's thread. One stuck window used to
    // freeze this hand for the whole timeout, and then nothing could connect.
    let mut buf = [0u16; 512];
    let mut copied = 0usize;
    let answered = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETTEXT,
            WPARAM(buf.len()),
            LPARAM(buf.as_mut_ptr() as isize),
            SEND_MESSAGE_TIMEOUT_FLAGS(SMTO_ABORTIFHUNG.0 | SMTO_ERRORONEXIT.0),
            80,
            Some(&mut copied),
        )
    };
    if answered.0 == 0 || copied == 0 {
        return String::new();
    }
    let n = copied.min(buf.len());
    String::from_utf16_lossy(&buf[..n])
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
    let desk = saved_screen()?;
    let (point_w, point_h) = point_size(&desk);
    if !point_inside(x, y, point_w, point_h) {
        bail!("这个位置不在刚才的画面里。先看一眼屏幕，再用画面里的点。");
    }
    click_desk(&desk, x, y)?;
    confirm_picture(&desk)
}

fn click_desk(desk: &Desktop, x: f64, y: f64) -> Result<()> {
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

fn saved_screen() -> Result<Desktop> {
    LAST_SCREEN
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
        .context("先看一下屏幕，再说要点哪里。")
}

fn point_size(desk: &Desktop) -> (f64, f64) {
    (
        point_of(desk.origin_x + desk.width, desk.origin_x, desk.scale).max(1) as f64,
        point_of(desk.origin_y + desk.height, desk.origin_y, desk.scale).max(1) as f64,
    )
}

fn remember_print(print: Vec<u8>) {
    *LAST_PRINT.lock().unwrap_or_else(|err| err.into_inner()) = print;
}

fn last_print() -> Vec<u8> {
    LAST_PRINT
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

fn remember_field(yes: bool) {
    LAST_FIELD.store(yes, Ordering::Relaxed);
}

fn last_has_field() -> bool {
    LAST_FIELD.load(Ordering::Relaxed)
}

fn shot_fingerprint(path: &str) -> Option<Vec<u8>> {
    let image = image::open(path).ok()?;
    let small = image.resize_exact(48, 48, image::imageops::FilterType::Nearest);
    Some(small.to_luma8().into_raw())
}

fn confirm_picture(desk: &Desktop) -> Result<()> {
    let before = last_print();
    if before.is_empty() {
        return Ok(());
    }
    thread::sleep(Duration::from_millis(280));
    let shot = match capture_png(desk) {
        Ok(shot) => shot,
        Err(_) => return Ok(()),
    };
    let after = shot_fingerprint(&shot.path).unwrap_or_default();
    let _ = std::fs::remove_file(&shot.path);
    if picture_changed(&before, &after) {
        remember_print(after);
        Ok(())
    } else {
        bail!("这一下没有点到。画面没有变化。先看一眼屏幕，再点一次。");
    }
}

fn window_frame(hwnd: HWND) -> Option<(i32, i32, i32, i32)> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    let width = rect.right.saturating_sub(rect.left);
    let height = rect.bottom.saturating_sub(rect.top);
    if width < 40 || height < 40 {
        return None;
    }
    Some((rect.left, rect.top, width, height))
}

fn crop_to_window(
    shot: &Shot,
    desk: &Desktop,
    frame: (i32, i32, i32, i32),
) -> Option<(Desktop, Shot)> {
    let (x, y, width, height) = crop_pixels(
        crate::computer_geom::Picture {
            origin_x: desk.origin_x as f64,
            origin_y: desk.origin_y as f64,
            scale: 1.0,
            pixel_w: shot.pixels.0,
            pixel_h: shot.pixels.1,
        },
        frame.0 as f64,
        frame.1 as f64,
        frame.2 as f64,
        frame.3 as f64,
    )?;
    let image = image::open(&shot.path).ok()?;
    let cropped = image.crop_imm(x, y, width, height);
    cropped.save(&shot.path).ok()?;
    let desk = Desktop {
        origin_x: desk.origin_x.saturating_add(x as i32),
        origin_y: desk.origin_y.saturating_add(y as i32),
        width: width as i32,
        height: height as i32,
        scale: desk.scale,
    };
    let (point_w, point_h) = point_size(&desk);
    Some((
        desk,
        Shot {
            path: shot.path.clone(),
            pixels: (i64::from(width), i64::from(height)),
            points: (point_w.round() as i64, point_h.round() as i64),
            scale: shot.scale,
        },
    ))
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn open_named(app: &str) -> Result<Value> {
    let app = app.trim();
    if app.is_empty() {
        bail!("说一下要打开的应用名字。");
    }
    if let Some(hwnd) = window_for(app) {
        bring_forward(hwnd)?;
        return Ok(json!({ "opened": app }));
    }
    let (file, params) = launch_target(app);
    shell_open(&file, &params)?;
    Ok(json!({ "opened": app }))
}

fn launch_target(app: &str) -> (String, String) {
    match app.trim().to_lowercase().as_str() {
        "计算器" | "calculator" | "calc" | "calc.exe" => ("calc.exe".into(), String::new()),
        "计算机" | "此电脑" | "我的电脑" | "this pc" | "my computer" => {
            ("explorer.exe".into(), "shell:MyComputerFolder".into())
        }
        "记事本" | "notepad" | "notepad.exe" => ("notepad.exe".into(), String::new()),
        "画图" | "paint" | "mspaint" | "mspaint.exe" => ("mspaint.exe".into(), String::new()),
        "文件资源管理器" | "资源管理器" | "explorer" | "explorer.exe" => {
            ("explorer.exe".into(), String::new())
        }
        "设置" | "settings" => ("ms-settings:".into(), String::new()),
        _ => (app.trim().to_string(), String::new()),
    }
}

fn shell_open(file: &str, params: &str) -> Result<()> {
    let file = HSTRING::from(file);
    let params = HSTRING::from(params);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        // NO_UI: a missing name must not pop a dialog that blocks this hand.
        // NOASYNC: return when the launch has started, not when the app exits.
        fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
        lpFile: windows::core::PCWSTR(file.as_ptr()),
        lpParameters: if params.is_empty() {
            windows::core::PCWSTR::null()
        } else {
            windows::core::PCWSTR(params.as_ptr())
        },
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.context("没有找到这个应用。看一下它的名字再试。")
}

fn bring_forward(hwnd: HWND) -> Result<()> {
    refuse_if_higher(hwnd)?;
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let foreground = GetForegroundWindow();
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let target_thread = GetWindowThreadProcessId(hwnd, None);
        let current = GetCurrentThreadId();
        let _ = AttachThreadInput(current, foreground_thread, true);
        let _ = AttachThreadInput(current, target_thread, true);
        let _ = SetForegroundWindow(hwnd);
        let _ = AttachThreadInput(current, target_thread, false);
        let _ = AttachThreadInput(current, foreground_thread, false);
    }
    Ok(())
}

fn list_named_windows(app: &str) -> Result<Value> {
    let want = app.trim();
    let mut windows = Vec::new();
    for hwnd in top_windows() {
        if !want.is_empty() {
            match process_name(hwnd) {
                Some(name) if name.eq_ignore_ascii_case(want) => {}
                _ => continue,
            }
        }
        let title = window_title(hwnd);
        if title.is_empty() {
            continue;
        }
        windows.push(json!({
            "app": process_name(hwnd).unwrap_or_default(),
            "title": title,
        }));
    }
    Ok(json!({ "windows": windows }))
}

fn window_controls(app: &str) -> Result<Value> {
    let hwnd = window_for(app).context("没有看到窗口。")?;
    let desk = monitor_of(hwnd);
    let body = read_controls(hwnd, &desk).unwrap_or_default();
    remember_field(window_has_field(&body));
    let mut controls = Vec::new();
    for line in body.lines() {
        let mut parts = line.split('\t');
        let role = parts.next().unwrap_or("").trim();
        if !matches!(role, "button" | "text field" | "text") {
            continue;
        }
        let label = parts.next().unwrap_or("").trim();
        let x: f64 = parts
            .next()
            .unwrap_or("")
            .trim()
            .parse()
            .unwrap_or(f64::NAN);
        let y: f64 = parts
            .next()
            .unwrap_or("")
            .trim()
            .parse()
            .unwrap_or(f64::NAN);
        if label.is_empty() || !x.is_finite() || !y.is_finite() {
            continue;
        }
        controls.push(json!({ "role": role, "label": label, "x": x, "y": y }));
    }
    Ok(json!({
        "app": process_name(hwnd).unwrap_or_default(),
        "window": window_title(hwnd),
        "controls": controls,
    }))
}

fn click_label(app: &str, label: &str) -> Result<Value> {
    let label = label.trim();
    if label.is_empty() {
        bail!("说一下要点的名字。");
    }
    let hwnd = window_for(app).context("没有看到窗口。")?;
    refuse_if_higher(hwnd)?;
    let desk = saved_screen()?;
    let (x, y) =
        named_center(hwnd, &desk, label).ok_or_else(|| anyhow::anyhow!("没有找到「{label}」。"))?;
    click_at(x, y)?;
    Ok(json!({ "clicked": label }))
}

fn set_labeled(app: &str, label: &str, text: &str) -> Result<Value> {
    let label = label.trim();
    if label.is_empty() {
        bail!("说一下要填的名字。");
    }
    if !last_has_field() {
        bail!("这个窗口没有输入框。先点一下要填的地方，再用打字。");
    }
    let hwnd = window_for(app).context("没有看到窗口。")?;
    refuse_if_higher(hwnd)?;
    let Some(element) = named_element(hwnd, label, true) else {
        bail!("没有这个输入框。先点一下要填的地方，再用打字。");
    };
    let pattern =
        unsafe { element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) }
            .map_err(|_| anyhow::anyhow!("没有这个输入框。先点一下要填的地方，再用打字。"))?;
    unsafe { pattern.SetValue(&BSTR::from(text)) }
        .map_err(|_| anyhow::anyhow!("没有填进「{label}」。"))?;
    Ok(json!({ "set": label }))
}

fn click_menu(app: &str, names: Vec<String>) -> Result<Value> {
    if names.is_empty() {
        bail!("说一下菜单的名字。");
    }
    let hwnd = window_for(app).context("没有看到窗口。")?;
    refuse_if_higher(hwnd)?;
    bring_forward(hwnd)?;
    for name in &names {
        let desk = saved_screen().unwrap_or_else(|_| monitor_of(hwnd));
        let Some((x, y)) = named_center(hwnd, &desk, name) else {
            bail!("没有找到菜单「{name}」。");
        };
        click_desk(&desk, x, y)?;
        thread::sleep(Duration::from_millis(220));
    }
    Ok(json!({ "menu": names }))
}

fn scroll_view(args: &Value) -> Result<Value> {
    let direction = args
        .get("direction")
        .and_then(Value::as_str)
        .unwrap_or("down");
    let amount = args
        .get("amount")
        .and_then(Value::as_i64)
        .unwrap_or(4)
        .clamp(1, 12);
    let desk = saved_screen()?;
    let (point_w, point_h) = point_size(&desk);
    let (x, y) = match (
        args.get("x").and_then(Value::as_f64),
        args.get("y").and_then(Value::as_f64),
    ) {
        (Some(x), Some(y)) => (x, y),
        _ => (point_w / 2.0, point_h / 2.0),
    };
    if !point_inside(x, y, point_w, point_h) {
        bail!("这个位置不在刚才的画面里。先看一眼屏幕，再用画面里的点。");
    }
    let delta = if direction.eq_ignore_ascii_case("up") {
        amount * 120
    } else {
        -amount * 120
    };
    let (virt_x, virt_y, virt_w, virt_h) = virtual_screen();
    let px = pixel_of(x, desk.origin_x, desk.scale);
    let py = pixel_of(y, desk.origin_y, desk.scale);
    let ax = absolute_axis(px, virt_x, virt_w);
    let ay = absolute_axis(py, virt_y, virt_h);
    let move_flags = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK | MOUSEEVENTF_MOVE;
    send_mouse(ax, ay, move_flags)?;
    send_wheel(ax, ay, delta as i32)?;
    Ok(json!({ "scrolled": direction, "x": x, "y": y, "amount": amount }))
}

fn named_center(hwnd: HWND, desk: &Desktop, label: &str) -> Option<(f64, f64)> {
    let element = named_element(hwnd, label, false)?;
    let (x, y) = element_center(&element, desk)?;
    Some((x as f64, y as f64))
}

fn named_element(hwnd: HWND, label: &str, field_only: bool) -> Option<IUIAutomationElement> {
    let auto = automation().ok()?;
    let root = unsafe { auto.ElementFromHandle(hwnd) }.ok()?;
    let walker = unsafe { auto.ControlViewWalker() }.ok()?;
    let deadline = Instant::now() + Duration::from_millis(1200);
    find_named(&walker, &root, label, field_only, 0, deadline)
}

fn find_named(
    walker: &IUIAutomationTreeWalker,
    element: &IUIAutomationElement,
    label: &str,
    field_only: bool,
    depth: u8,
    deadline: Instant,
) -> Option<IUIAutomationElement> {
    if depth > 6 || Instant::now() >= deadline {
        return None;
    }
    let mut child = unsafe { walker.GetFirstChildElement(element) }.ok()?;
    loop {
        if Instant::now() >= deadline {
            return None;
        }
        let kind = control_type(&child);
        let role_ok = !field_only || control_role(kind) == Some("text field");
        if role_ok {
            if let Some(name) = element_name(&child) {
                if same_label(&name, label) {
                    return Some(child);
                }
            }
        }
        if let Some(found) = find_named(walker, &child, label, field_only, depth + 1, deadline) {
            return Some(found);
        }
        match unsafe { walker.GetNextSiblingElement(&child) } {
            Ok(next) => child = next,
            Err(_) => return None,
        }
    }
}

fn same_label(have: &str, want: &str) -> bool {
    let want = want.trim();
    if want.is_empty() {
        return false;
    }
    have.eq_ignore_ascii_case(want)
        || have
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case(want)
}

fn send_mouse(dx: i32, dy: i32, flags: MOUSE_EVENT_FLAGS) -> Result<()> {
    send_input(dx, dy, 0, flags)
}

fn send_wheel(dx: i32, dy: i32, delta: i32) -> Result<()> {
    let flags = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK | MOUSEEVENTF_WHEEL;
    send_input(dx, dy, delta, flags)
}

fn send_input(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> Result<()> {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data as u32,
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
    let mut down = Vec::with_capacity(mods.len() + 1);
    for vk in &mods {
        down.push(key_input(*vk, false, false));
    }
    down.push(key_input(VIRTUAL_KEY(press.vk), false, press.extended));
    send_keys(&down)?;
    thread::sleep(Duration::from_millis(20));
    let mut up = Vec::with_capacity(mods.len() + 1);
    up.push(key_input(VIRTUAL_KEY(press.vk), true, press.extended));
    for vk in mods.iter().rev() {
        up.push(key_input(*vk, true, false));
    }
    send_keys(&up)
}

fn key_input(vk: VIRTUAL_KEY, up: bool, extended: bool) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(u32::from(vk.0), MAPVK_VK_TO_VSC) } as u16;
    let mut flags = if up {
        KEYEVENTF_KEYUP
    } else {
        KEYBD_EVENT_FLAGS(0)
    };
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_keys(inputs: &[INPUT]) -> Result<()> {
    if inputs.is_empty() {
        return Ok(());
    }
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        bail!("keys did not go through");
    }
    Ok(())
}

fn type_text(text: &str) -> Result<()> {
    for ch in text.chars() {
        match ch {
            // A Unicode packet for these two is ignored by a text field.
            '\u{8}' => send_key(&parse_key("backspace").map_err(anyhow::Error::msg)?)?,
            '\u{7f}' => send_key(&parse_key("delete").map_err(anyhow::Error::msg)?)?,
            other => {
                let mut units = [0u16; 2];
                let n = other.encode_utf16(&mut units).len();
                for &unit in &units[..n] {
                    send_unicode(unit, false)?;
                    send_unicode(unit, true)?;
                }
            }
        }
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
