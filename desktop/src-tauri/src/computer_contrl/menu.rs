//! Open a menu by its names, from the menu bar down.
//!
//! Example: 文件, then 新建. A missing name stops. This does not guess a point.
//! Reference: trycua/cua `platform-macos` `tools/invoke_menu.rs`.

pub fn invoke(app: &str, names: &[String]) -> Result<(), String> {
    if names.is_empty() || names.iter().all(|name| name.trim().is_empty()) {
        return Err("说一下菜单的名字。".into());
    }
    #[cfg(target_os = "macos")]
    {
        super::mac::invoke_menu(app, names)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("目前只在 Mac 上。".into())
    }
}

#[cfg(test)]
mod tests {
    use super::invoke;

    #[test]
    fn a_menu_needs_a_name() {
        assert_eq!(invoke("", &[]).unwrap_err(), "说一下菜单的名字。");
        assert_eq!(
            invoke("微信", &[String::new()]).unwrap_err(),
            "说一下菜单的名字。"
        );
    }
}
