//! Computer control that runs inside this app.
//!
//! [`LINE`] is the whole path, in the order a turn uses it. Each name is one
//! file in this folder. Cursor themes, recording, perception, and a separate
//! driver app stay out.
//!
//! Reference: trycua/cua `platform-macos` (MIT). Re-read that core when this
//! folder is refreshed.

mod apps;
pub mod cua;
mod keys;
#[cfg(target_os = "macos")]
mod mac;
mod menu;
mod pointer;
mod text;
mod windows;

pub use apps::{bring_to_front, list_apps};
pub use keys::{hotkey, press};
pub use menu::invoke as invoke_menu;
pub use pointer::{click_control, click_point, scroll_point};
pub use text::{set_value, type_text};
pub use windows::{list_windows, window_state};

/// The full path. Refresh this folder against Cua in this order.
pub const LINE: [&str; 11] = [
    "list_apps",
    "launch_app",
    "bring_to_front",
    "list_windows",
    "get_window_state",
    "click",
    "type_text",
    "set_value",
    "press_key",
    "hotkey",
    "invoke_menu",
];

/// Whether this folder already performs the step.
pub fn ready(step: &str) -> bool {
    LINE.contains(&step)
}

/// What this folder would do for one step, without doing it.
pub fn plan(step: &str) -> Plan {
    if ready(step) {
        Plan::Ready
    } else {
        Plan::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Ready,
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::{plan, ready, Plan, LINE};

    #[test]
    fn the_line_is_ready() {
        assert_eq!(
            LINE,
            [
                "list_apps",
                "launch_app",
                "bring_to_front",
                "list_windows",
                "get_window_state",
                "click",
                "type_text",
                "set_value",
                "press_key",
                "hotkey",
                "invoke_menu",
            ]
        );
        assert!(LINE.iter().copied().all(ready));
        assert_eq!(plan("invoke_menu"), Plan::Ready);
        assert_eq!(plan("record"), Plan::Unknown);
    }
}
