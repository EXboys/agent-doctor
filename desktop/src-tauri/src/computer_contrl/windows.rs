//! List an app's windows and read the front window's controls.
//!
//! Reference: trycua/cua `platform-macos` `list_windows.rs`, `get_window_state.rs`,
//! and `ax/tree.rs`.

/// One window title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub title: String,
    pub app: String,
}

/// A button or text field inside the window. x and y are screen points.
#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    pub role: String,
    pub label: String,
    pub x: f64,
    pub y: f64,
}

/// The front window and the controls in it.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowState {
    pub app: String,
    pub title: String,
    pub controls: Vec<Control>,
}

pub fn list_windows(app: &str) -> Result<Vec<Window>, String> {
    #[cfg(target_os = "macos")]
    {
        super::mac::list_windows(app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("目前只在 Mac 上。".into())
    }
}

pub fn window_state(app: &str) -> Result<WindowState, String> {
    #[cfg(target_os = "macos")]
    {
        super::mac::window_state(app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("目前只在 Mac 上。".into())
    }
}
