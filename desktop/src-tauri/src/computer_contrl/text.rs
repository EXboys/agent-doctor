//! Type into the focused place, or write one control's value.
//!
//! Reference: trycua/cua `platform-macos` `tools/type_text.rs` and `set_value`.

pub fn type_text(text: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        super::mac::type_text(text)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = text;
        Err("目前只在 Mac 上。".into())
    }
}

pub fn set_value(app: &str, label: &str, text: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        super::mac::set_value(app, label, text)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, label, text);
        Err("目前只在 Mac 上。".into())
    }
}
