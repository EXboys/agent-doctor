//! A click on the top bar, and creating the screenshot page, both ask macOS
//! to activate this app. Activation orders the conversation window forward,
//! and if that window is on another desktop, the user is swiped there.
//!
//! While a screenshot should stay on the current desktop, those calls are
//! ignored. A click whose window is the top bar or the picker is ignored too,
//! because that click happens before the screenshot command runs. Bringing
//! the conversation forward on purpose still activates.

use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use std::sync::Once;

use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{msg_send, sel};

type ActivateImp = unsafe extern "C-unwind" fn(*mut AnyObject, Sel);
type IgnoringImp = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, Bool);
type OptionsImp = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, usize);

static ORIG_ACTIVATE: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
static ORIG_IGNORING: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
static ORIG_OPTIONS: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
static ALLOW_DEPTH: AtomicU32 = AtomicU32::new(0);
static SUPPRESS: AtomicU32 = AtomicU32::new(0);

pub(crate) fn install() {
    static START: Once = Once::new();
    START.call_once(|| unsafe {
        let Some(class) = AnyClass::get(c"NSApplication") else {
            return;
        };
        hook(
            class,
            sel!(activate),
            std::mem::transmute::<ActivateImp, Imp>(guarded_activate),
            &ORIG_ACTIVATE,
        );
        hook(
            class,
            sel!(activateIgnoringOtherApps:),
            std::mem::transmute::<IgnoringImp, Imp>(guarded_ignoring),
            &ORIG_IGNORING,
        );
        hook(
            class,
            sel!(activateWithOptions:),
            std::mem::transmute::<OptionsImp, Imp>(guarded_options),
            &ORIG_OPTIONS,
        );
    });
}

pub(crate) fn set_suppressed(on: bool) {
    SUPPRESS.store(u32::from(on), Ordering::SeqCst);
}

pub(crate) fn allow<T>(body: impl FnOnce() -> T) -> T {
    ALLOW_DEPTH.fetch_add(1, Ordering::SeqCst);
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            ALLOW_DEPTH.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _guard = Guard;
    body()
}

pub(crate) fn activate_ignoring_other_apps() {
    allow(|| {
        let Some(cls) = AnyClass::get(c"NSApplication") else {
            return;
        };
        unsafe {
            let app: *mut AnyObject = msg_send![cls, sharedApplication];
            if app.is_null() {
                return;
            }
            let _: () = msg_send![app, activateIgnoringOtherApps: true];
        }
    });
}

fn block_reason() -> Option<&'static str> {
    if ALLOW_DEPTH.load(Ordering::SeqCst) > 0 {
        return None;
    }
    if SUPPRESS.load(Ordering::SeqCst) != 0 {
        return Some("capture");
    }
    if event_on_floating_panel() {
        return Some("panel");
    }
    None
}

fn event_on_floating_panel() -> bool {
    let Some(cls) = AnyClass::get(c"NSApplication") else {
        return false;
    };
    unsafe {
        let app: *mut AnyObject = msg_send![cls, sharedApplication];
        if app.is_null() {
            return false;
        }
        let event: *mut AnyObject = msg_send![app, currentEvent];
        if event.is_null() {
            return false;
        }
        let event_window: *mut AnyObject = msg_send![event, window];
        is_floating_panel(event_window)
    }
}

unsafe fn is_floating_panel(win: *mut AnyObject) -> bool {
    if win.is_null() {
        return false;
    }
    let name = (*win).class().name();
    if name == c"AgentDoctorIslandPanel" || name == c"AgentDoctorPickerPanel" {
        return true;
    }
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask};

    let ns: &NSWindow = &*win.cast();
    ns.styleMask()
        .contains(NSWindowStyleMask::NonactivatingPanel)
        && ns
            .collectionBehavior()
            .contains(NSWindowCollectionBehavior::CanJoinAllSpaces)
}

unsafe fn hook(class: &AnyClass, sel: Sel, replacement: Imp, slot: &AtomicPtr<()>) {
    let Some(method) = class.instance_method(sel) else {
        return;
    };
    let previous = method.implementation();
    slot.store(previous as *mut (), Ordering::SeqCst);
    let _ = method.set_implementation(replacement);
}

unsafe extern "C-unwind" fn guarded_activate(this: *mut AnyObject, cmd: Sel) {
    if let Some(reason) = block_reason() {
        eprintln!("[screenshot] blocked activate ({reason})");
        return;
    }
    let orig = ORIG_ACTIVATE.load(Ordering::SeqCst);
    if orig.is_null() {
        return;
    }
    let call: ActivateImp = std::mem::transmute(orig);
    call(this, cmd);
}

unsafe extern "C-unwind" fn guarded_ignoring(this: *mut AnyObject, cmd: Sel, flag: Bool) {
    if let Some(reason) = block_reason() {
        eprintln!("[screenshot] blocked activateIgnoringOtherApps ({reason})");
        return;
    }
    let orig = ORIG_IGNORING.load(Ordering::SeqCst);
    if orig.is_null() {
        return;
    }
    let call: IgnoringImp = std::mem::transmute(orig);
    call(this, cmd, flag);
}

unsafe extern "C-unwind" fn guarded_options(this: *mut AnyObject, cmd: Sel, options: usize) {
    if let Some(reason) = block_reason() {
        eprintln!("[screenshot] blocked activateWithOptions ({reason})");
        return;
    }
    let orig = ORIG_OPTIONS.load(Ordering::SeqCst);
    if orig.is_null() {
        return;
    }
    let call: OptionsImp = std::mem::transmute(orig);
    call(this, cmd, options);
}
