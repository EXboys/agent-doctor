//! Find an app, open it, and bring its window forward.
//!
//! Reference: trycua/cua `platform-macos` `tools/launch_app.rs`, `list_apps.rs`,
//! `bring_to_front.rs`, and `apps/nsworkspace.rs`.

use std::process::{Command, ExitStatus};

/// Running apps, by the name on the .app bundle.
pub fn list_apps() -> Result<Vec<String>, String> {
    #[cfg(target_os = "macos")]
    {
        super::mac::list_apps()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("目前只在 Mac 上。".into())
    }
}

/// How to ask Launch Services to open one exact app.
/// A bundle id wins, so 微信 cannot open 微信开发者工具.
pub fn launch_args(bundle_id: Option<&str>, app_path: &str) -> Vec<String> {
    match bundle_id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => vec!["-b".into(), id.into()],
        None => vec!["-a".into(), app_path.into()],
    }
}

/// Open that app and bring it forward. The caller has already picked one exact app.
pub fn open_app(bundle_id: Option<&str>, app_path: &str) -> std::io::Result<ExitStatus> {
    Command::new("/usr/bin/open")
        .args(launch_args(bundle_id, app_path))
        .status()
}

/// Bring an already chosen app forward. Launch Services activates it.
pub fn bring_to_front(bundle_id: Option<&str>, app_path: &str) -> std::io::Result<ExitStatus> {
    open_app(bundle_id, app_path)
}

#[cfg(test)]
mod tests {
    use super::launch_args;

    #[test]
    fn a_bundle_id_opens_that_app_only() {
        assert_eq!(
            launch_args(Some("com.tencent.xinWeChat"), "/Applications/WeChat.app"),
            vec!["-b", "com.tencent.xinWeChat"]
        );
        assert_eq!(
            launch_args(None, "/Applications/WeChat.app"),
            vec!["-a", "/Applications/WeChat.app"]
        );
        assert_eq!(
            launch_args(Some("  "), "/Applications/WeChat.app"),
            vec!["-a", "/Applications/WeChat.app"]
        );
    }
}
