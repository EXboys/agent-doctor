//! Desktop turns browser repair off while the "操作网页" switch is off.
//! Other callers leave the flag at its default, so CLI repair still wires the browser.

use std::cell::Cell;

thread_local! {
    static ALLOW: Cell<bool> = const { Cell::new(true) };
}

pub fn browser_tools_allowed() -> bool {
    ALLOW.with(|cell| cell.get())
}

/// Holds browser wiring off on this thread until dropped.
pub struct HoldBrowserToolsOff;

impl Default for HoldBrowserToolsOff {
    fn default() -> Self {
        Self::new()
    }
}

impl HoldBrowserToolsOff {
    pub fn new() -> Self {
        ALLOW.with(|cell| cell.set(false));
        Self
    }
}

impl Drop for HoldBrowserToolsOff {
    fn drop(&mut self) {
        ALLOW.with(|cell| cell.set(true));
    }
}
