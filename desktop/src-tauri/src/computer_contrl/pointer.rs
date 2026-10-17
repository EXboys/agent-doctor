//! Press a control by its name, or a point when it has no name.
//!
//! A named control is clicked at its center. A point is a screen point.
//! Reference: trycua/cua `platform-macos` `tools/click.rs`.

pub fn click_point(x: f64, y: f64) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        super::mac::click_point(x, y)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (x, y);
        Err("目前只在 Mac 上。".into())
    }
}

/// `lines` above zero moves the list toward earlier rows. Below zero reveals later rows.
pub fn scroll_point(x: f64, y: f64, lines: i32) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        super::mac::scroll_point(x, y, lines)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (x, y, lines);
        Err("目前只在 Mac 上。".into())
    }
}

pub fn click_control(app: &str, label: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        super::mac::click_control(app, label)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, label);
        Err("目前只在 Mac 上。".into())
    }
}
