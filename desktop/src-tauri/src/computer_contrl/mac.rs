//! macOS Accessibility and event posting for this folder.
//!
//! Reference: trycua/cua `platform-macos` (MIT). Menu actions use the control,
//! not a guessed point.

use std::cell::Cell;
use std::ffi::{c_char, c_void, CStr, CString};
use std::time::{Duration, Instant};

use super::windows::{Control, Window, WindowState};

const UTF8: u32 = 0x0800_0100;
const TIMEOUT: f32 = 0.2;

thread_local! {
    static BUDGET: Cell<Option<Instant>> = const { Cell::new(None) };
}

struct El(*const c_void);

impl El {
    unsafe fn own(ptr: *const c_void) -> Option<Self> {
        if ptr.is_null() {
            None
        } else {
            Some(Self(ptr))
        }
    }

    unsafe fn retain(ptr: *const c_void) -> Option<Self> {
        if ptr.is_null() {
            None
        } else {
            CFRetain(ptr);
            Some(Self(ptr))
        }
    }

    fn ptr(&self) -> *const c_void {
        self.0
    }

    unsafe fn copy(&self, name: &str) -> Option<El> {
        if spent() {
            return None;
        }
        AXUIElementSetMessagingTimeout(self.0 as *mut c_void, TIMEOUT);
        let attr = cf_string(name)?;
        let mut value: *const c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(self.0 as *mut c_void, attr.ptr(), &mut value);
        if err != 0 || value.is_null() {
            if !value.is_null() {
                CFRelease(value);
            }
            return None;
        }
        El::own(value)
    }

    unsafe fn string(&self, name: &str) -> Option<String> {
        let value = self.copy(name)?;
        if CFGetTypeID(value.ptr()) != CFStringGetTypeID() {
            return None;
        }
        cf_string_value(value.ptr())
    }

    unsafe fn press(&self) -> bool {
        let Some(action) = cf_string("AXPress") else {
            return false;
        };
        AXUIElementPerformAction(self.0 as *mut c_void, action.ptr()) == 0
    }

    unsafe fn set_string(&self, name: &str, text: &str) -> bool {
        let Some(attr) = cf_string(name) else {
            return false;
        };
        let Some(value) = cf_string(text) else {
            return false;
        };
        AXUIElementSetAttributeValue(self.0 as *mut c_void, attr.ptr(), value.ptr()) == 0
    }
}

impl Drop for El {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

fn spent() -> bool {
    BUDGET.with(|cell| {
        cell.get()
            .is_some_and(|deadline| Instant::now() >= deadline)
    })
}

fn with_budget<T>(read: impl FnOnce() -> T) -> T {
    BUDGET.with(|cell| cell.set(Some(Instant::now() + Duration::from_millis(1200))));
    let value = read();
    BUDGET.with(|cell| cell.set(None));
    value
}

pub fn list_apps() -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let mut pids = vec![0i32; 1024];
    let bytes = unsafe {
        proc_listpids(
            1,
            0,
            pids.as_mut_ptr() as *mut c_void,
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
        let Some(name) = app_stem(pid) else {
            continue;
        };
        if names.iter().any(|have| have == &name) {
            continue;
        }
        names.push(name);
    }
    if names.is_empty() {
        return Err("could not see the open apps".into());
    }
    Ok(names)
}

pub fn list_windows(app: &str) -> Result<Vec<Window>, String> {
    with_budget(|| unsafe { list_windows_inner(app) })
}

pub fn window_state(app: &str) -> Result<WindowState, String> {
    with_budget(|| unsafe { window_state_inner(app) })
}

pub fn scroll_point(x: f64, y: f64, lines: i32) -> Result<(), String> {
    unsafe {
        let source = CGEventSourceCreate(1);
        if !source.is_null() {
            CGEventSourceSetLocalEventsSuppressionInterval(source, 0.0);
        }
        let point = CGPoint { x, y };
        post_mouse(5, point, source);
        let event = CGEventCreateScrollWheelEvent(source, 1, 1, lines);
        if !event.is_null() {
            CGEventSetLocation(event, point);
            CGEventPost(0, event);
            CFRelease(event as *const c_void);
        }
        if !source.is_null() {
            CFRelease(source as *const c_void);
        }
    }
    Ok(())
}

pub fn click_point(x: f64, y: f64) -> Result<(), String> {
    unsafe {
        let source = CGEventSourceCreate(1);
        if !source.is_null() {
            CGEventSourceSetLocalEventsSuppressionInterval(source, 0.0);
        }
        let point = CGPoint { x, y };
        post_mouse(5, point, source);
        std::thread::sleep(Duration::from_millis(20));
        post_mouse(1, point, source);
        std::thread::sleep(Duration::from_millis(80));
        post_mouse(2, point, source);
        if !source.is_null() {
            CFRelease(source as *const c_void);
        }
    }
    Ok(())
}

pub fn click_control(app: &str, label: &str) -> Result<(), String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("说一下要点的名字。".into());
    }
    with_budget(|| unsafe { click_control_inner(app, label) })
}

pub fn type_text(text: &str) -> Result<(), String> {
    for unit in text.encode_utf16() {
        unsafe { post_unicode(unit) };
        std::thread::sleep(Duration::from_millis(8));
    }
    Ok(())
}

pub fn set_value(app: &str, label: &str, text: &str) -> Result<(), String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("说一下要填的名字。".into());
    }
    with_budget(|| unsafe { set_value_inner(app, label, text) })
}

pub fn post_key(code: u16, flags: u64) -> Result<(), String> {
    unsafe {
        let down = CGEventCreateKeyboardEvent(std::ptr::null_mut(), code, true);
        let up = CGEventCreateKeyboardEvent(std::ptr::null_mut(), code, false);
        if !down.is_null() {
            CGEventSetFlags(down, flags);
            CGEventPost(0, down);
            CFRelease(down as *const c_void);
        }
        std::thread::sleep(Duration::from_millis(20));
        if !up.is_null() {
            CGEventSetFlags(up, flags);
            CGEventPost(0, up);
            CFRelease(up as *const c_void);
        }
    }
    Ok(())
}

pub fn invoke_menu(app: &str, names: &[String]) -> Result<(), String> {
    if names.is_empty() || names.iter().all(|name| name.trim().is_empty()) {
        return Err("说一下菜单的名字。".into());
    }
    with_budget(|| unsafe { invoke_menu_inner(app, names) })
}

unsafe fn list_windows_inner(app: &str) -> Result<Vec<Window>, String> {
    let element = app_element(app)?;
    let app_name = element
        .string("AXTitle")
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| app.to_string());
    let Some(windows) = element.copy("AXWindows") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let count = CFArrayGetCount(windows.ptr()).min(8);
    for i in 0..count {
        let Some(window) = array_at(&windows, i) else {
            continue;
        };
        let title = window.string("AXTitle").unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        out.push(Window {
            title,
            app: app_name.clone(),
        });
    }
    Ok(out)
}

unsafe fn window_state_inner(app: &str) -> Result<WindowState, String> {
    let element = app_element(app)?;
    let app_name = element
        .string("AXTitle")
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| app.to_string());
    let Some(windows) = element.copy("AXWindows") else {
        return Err("没有看到窗口。".into());
    };
    if CFArrayGetCount(windows.ptr()) <= 0 {
        return Err("没有看到窗口。".into());
    }
    let Some(window) = array_at(&windows, 0) else {
        return Err("没有看到窗口。".into());
    };
    let title = window.string("AXTitle").unwrap_or_default();
    let mut controls = Vec::new();
    collect_controls(&window, 0, &mut controls);
    Ok(WindowState {
        app: app_name,
        title,
        controls,
    })
}

unsafe fn click_control_inner(app: &str, label: &str) -> Result<(), String> {
    let element = app_element(app)?;
    let Some(control) = find_labeled(&element, label, 0) else {
        return Err(format!("没有找到「{label}」。"));
    };
    if let Some((x, y)) = center(&control) {
        return click_point(x, y);
    }
    if control.press() {
        return Ok(());
    }
    Err(format!("没有点到「{label}」。"))
}

unsafe fn set_value_inner(app: &str, label: &str, text: &str) -> Result<(), String> {
    let element = app_element(app)?;
    let Some(control) = find_labeled(&element, label, 0) else {
        return Err(format!("没有找到「{label}」。"));
    };
    if control.set_string("AXValue", text) {
        Ok(())
    } else {
        Err(format!("没有填进「{label}」。"))
    }
}

unsafe fn invoke_menu_inner(app: &str, names: &[String]) -> Result<(), String> {
    let element = app_element(app)?;
    let Some(bar) = element.copy("AXMenuBar") else {
        return Err("没有看到菜单。".into());
    };
    let mut parent = bar;
    for (index, name) in names.iter().enumerate() {
        let name = name.trim();
        let Some(item) = find_menu_item(&parent, name, index == 0) else {
            return Err(format!("没有找到菜单「{name}」。"));
        };
        if !item.press() {
            return Err(format!("没有点到菜单「{name}」。"));
        }
        if index + 1 != names.len() {
            std::thread::sleep(Duration::from_millis(180));
            parent = item;
        }
    }
    Ok(())
}

unsafe fn app_element(name: &str) -> Result<El, String> {
    if let Some(app) = find_named_app(name) {
        return Ok(app);
    }
    if let Some(app) = front_app() {
        let title = app.string("AXTitle").unwrap_or_default();
        if name.trim().is_empty() || same_name(&title, name) {
            return Ok(app);
        }
    }
    Err(if name.trim().is_empty() {
        "没有看到前面的应用。".into()
    } else {
        format!("没有找到叫「{name}」的应用。")
    })
}

unsafe fn find_named_app(name: &str) -> Option<El> {
    let want = name.trim();
    if want.is_empty() {
        return None;
    }
    let mut pids = vec![0i32; 1024];
    let bytes = proc_listpids(
        1,
        0,
        pids.as_mut_ptr() as *mut c_void,
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
        let stem = app_stem(pid).unwrap_or_default();
        let proc = proc_name_of(pid).unwrap_or_default();
        if !same_name(&stem, want) && !same_name(&proc, want) {
            continue;
        }
        let created = AXUIElementCreateApplication(pid);
        if let Some(app) = El::own(created as *const c_void) {
            AXUIElementSetMessagingTimeout(app.ptr() as *mut c_void, TIMEOUT);
            return Some(app);
        }
    }
    None
}

unsafe fn front_app() -> Option<El> {
    let mut psn = ProcessSerialNumber { high: 0, low: 0 };
    if GetFrontProcess(&mut psn) == 0 {
        let mut pid = 0i32;
        if GetProcessPID(&psn, &mut pid) == 0 && pid > 0 {
            let created = AXUIElementCreateApplication(pid);
            if let Some(app) = El::own(created as *const c_void) {
                AXUIElementSetMessagingTimeout(app.ptr() as *mut c_void, TIMEOUT);
                return Some(app);
            }
        }
    }
    let system = El::own(AXUIElementCreateSystemWide() as *const c_void)?;
    system.copy("AXFocusedApplication")
}

unsafe fn collect_controls(el: &El, depth: u8, out: &mut Vec<Control>) {
    if depth > 6 || out.len() >= 60 || spent() {
        return;
    }
    let Some(children) = el.copy("AXChildren") else {
        return;
    };
    let count = CFArrayGetCount(children.ptr());
    for i in 0..count {
        if out.len() >= 60 || spent() {
            break;
        }
        let Some(child) = array_at(&children, i) else {
            continue;
        };
        let role = child.string("AXRole").unwrap_or_default();
        if let Some(kind) = control_kind(&role) {
            if let Some(label) = label_of(&child) {
                if let Some((x, y)) = center(&child) {
                    let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
                    out.push(Control {
                        role: kind,
                        label,
                        x,
                        y,
                    });
                }
            }
        }
        if role != "AXMenu" && role != "AXMenuBar" {
            collect_controls(&child, depth + 1, out);
        }
    }
}

unsafe fn find_labeled(el: &El, want: &str, depth: u8) -> Option<El> {
    if depth > 6 || spent() {
        return None;
    }
    let children = el.copy("AXChildren")?;
    let count = CFArrayGetCount(children.ptr());
    for i in 0..count {
        let Some(child) = array_at(&children, i) else {
            continue;
        };
        let role = child.string("AXRole").unwrap_or_default();
        if control_kind(&role).is_some() {
            if let Some(label) = label_of(&child) {
                if same_name(&label, want) {
                    return Some(child);
                }
            }
        }
        if role != "AXMenu" && role != "AXMenuBar" {
            if let Some(found) = find_labeled(&child, want, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

unsafe fn find_menu_item(parent: &El, want: &str, top: bool) -> Option<El> {
    let children = parent.copy("AXChildren")?;
    let count = CFArrayGetCount(children.ptr());
    for i in 0..count {
        let Some(child) = array_at(&children, i) else {
            continue;
        };
        let role = child.string("AXRole").unwrap_or_default();
        if top {
            if same_name(&child.string("AXTitle").unwrap_or_default(), want) {
                return Some(child);
            }
            continue;
        }
        if role == "AXMenu" {
            if let Some(found) = find_menu_item(&child, want, true) {
                return Some(found);
            }
        }
        if role == "AXMenuItem" && same_name(&child.string("AXTitle").unwrap_or_default(), want) {
            return Some(child);
        }
    }
    None
}

unsafe fn label_of(el: &El) -> Option<String> {
    el.string("AXTitle")
        .filter(|label| !label.is_empty())
        .or_else(|| el.string("AXDescription"))
        .filter(|label| !label.is_empty())
        .or_else(|| el.string("AXValue"))
        .filter(|label| !label.is_empty())
}

fn control_kind(role: &str) -> Option<String> {
    Some(
        match role {
            "AXButton" | "AXCheckBox" => "button",
            "AXTextField" | "AXTextArea" => "text field",
            "AXStaticText" => "text",
            _ => return None,
        }
        .into(),
    )
}

unsafe fn center(el: &El) -> Option<(f64, f64)> {
    let pos = el.copy("AXPosition")?;
    let size = el.copy("AXSize")?;
    let mut point = CGPoint { x: 0.0, y: 0.0 };
    let mut cg_size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    let ok_pos = AXValueGetValue(pos.ptr(), 1, &mut point as *mut CGPoint as *mut c_void);
    let ok_size = AXValueGetValue(size.ptr(), 2, &mut cg_size as *mut CGSize as *mut c_void);
    if ok_pos == 0 || ok_size == 0 {
        return None;
    }
    Some((
        point.x + cg_size.width / 2.0,
        point.y + cg_size.height / 2.0,
    ))
}

unsafe fn array_at(array: &El, index: isize) -> Option<El> {
    El::retain(CFArrayGetValueAtIndex(array.ptr(), index))
}

fn same_name(have: &str, want: &str) -> bool {
    let want = want.trim();
    if want.is_empty() {
        return false;
    }
    let have = have.trim();
    have.eq_ignore_ascii_case(want)
        || have
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case(want)
}

fn app_stem(pid: i32) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    let n = unsafe { proc_pidpath(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    let path = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char).to_string_lossy() };
    let bundle = path.split(".app/").next()?;
    let name = bundle.rsplit('/').next()?.trim_end_matches(".app");
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn proc_name_of(pid: i32) -> Option<String> {
    let mut buf = [0u8; 256];
    let n = unsafe { proc_name(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    let name = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char).to_string_lossy() };
    if name.is_empty() {
        None
    } else {
        Some(name.into_owned())
    }
}

unsafe fn cf_string(text: &str) -> Option<El> {
    let c = CString::new(text).ok()?;
    El::own(CFStringCreateWithCString(
        std::ptr::null(),
        c.as_ptr(),
        UTF8,
    ))
}

unsafe fn cf_string_value(value: *const c_void) -> Option<String> {
    let len = CFStringGetLength(value) as usize;
    let mut buf = vec![0u8; len * 4 + 1];
    let ok = CFStringGetCString(
        value,
        buf.as_mut_ptr() as *mut c_char,
        buf.len() as isize,
        UTF8,
    );
    if ok == 0 {
        return None;
    }
    Some(
        CStr::from_ptr(buf.as_ptr() as *const c_char)
            .to_string_lossy()
            .into_owned(),
    )
}

unsafe fn post_mouse(kind: u32, point: CGPoint, source: *mut c_void) {
    let event = CGEventCreateMouseEvent(source, kind, point, 0);
    if !event.is_null() {
        if kind == 1 || kind == 2 {
            CGEventSetIntegerValueField(event, 1, 1);
        }
        CGEventPost(0, event);
        CFRelease(event as *const c_void);
    }
}

unsafe fn post_unicode(ch: u16) {
    let down = CGEventCreateKeyboardEvent(std::ptr::null_mut(), 0, true);
    let up = CGEventCreateKeyboardEvent(std::ptr::null_mut(), 0, false);
    if !down.is_null() {
        CGEventKeyboardSetUnicodeString(down, 1, &ch);
        CGEventPost(0, down);
        CFRelease(down as *const c_void);
    }
    if !up.is_null() {
        CGEventKeyboardSetUnicodeString(up, 1, &ch);
        CGEventPost(0, up);
        CFRelease(up as *const c_void);
    }
}

#[repr(C)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
struct CGSize {
    width: f64,
    height: f64,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> *mut c_void;
    fn AXUIElementCreateApplication(pid: i32) -> *mut c_void;
    fn AXUIElementSetMessagingTimeout(element: *mut c_void, timeout: f32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: *mut c_void,
        attribute: *const c_void,
        value: *mut *const c_void,
    ) -> i32;
    fn AXUIElementPerformAction(element: *mut c_void, action: *const c_void) -> i32;
    fn AXUIElementSetAttributeValue(
        element: *mut c_void,
        attribute: *const c_void,
        value: *const c_void,
    ) -> i32;
    fn AXValueGetValue(value: *const c_void, value_type: i32, dest: *mut c_void) -> u8;
    fn GetFrontProcess(psn: *mut ProcessSerialNumber) -> i32;
    fn GetProcessPID(psn: *const ProcessSerialNumber, pid: *mut i32) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: *const c_void);
    fn CFRetain(value: *const c_void) -> *const c_void;
    fn CFStringCreateWithCString(
        alloc: *const c_void,
        cstr: *const c_char,
        encoding: u32,
    ) -> *const c_void;
    fn CFStringGetLength(value: *const c_void) -> isize;
    fn CFStringGetCString(
        value: *const c_void,
        buffer: *mut c_char,
        size: isize,
        encoding: u32,
    ) -> u8;
    fn CFStringGetTypeID() -> usize;
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFGetTypeID(value: *const c_void) -> usize;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceCreate(state_id: u32) -> *mut c_void;
    fn CGEventSourceSetLocalEventsSuppressionInterval(source: *mut c_void, seconds: f64);
    fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
    fn CGEventCreateMouseEvent(
        source: *mut c_void,
        mouse_type: u32,
        position: CGPoint,
        button: u32,
    ) -> *mut c_void;
    fn CGEventCreateKeyboardEvent(source: *mut c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventKeyboardSetUnicodeString(event: *mut c_void, length: usize, chars: *const u16);
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
    fn CGEventSetLocation(event: *mut c_void, point: CGPoint);
    fn CGEventCreateScrollWheelEvent(
        source: *mut c_void,
        units: u32,
        wheel_count: u32,
        wheel1: i32,
    ) -> *mut c_void;
}

#[link(name = "proc")]
extern "C" {
    fn proc_listpids(kind: u32, typeinfo: u32, buffer: *mut c_void, buffersize: i32) -> i32;
    fn proc_name(pid: i32, buffer: *mut c_void, buffersize: u32) -> i32;
    fn proc_pidpath(pid: i32, buffer: *mut c_void, buffersize: u32) -> i32;
}
