//! Chromium-family browser discovery, launch, and CDP port helpers.

mod cdp;
mod discover;
mod launch;
mod paths;
mod types;

pub use cdp::{
    cdp_automation_markers, cdp_port_is_headless, cdp_user_data_dir, chrome_http_json,
    connect_chrome, kill_chrome_on_port, profile_locked_by_other_chrome, stop_chrome,
};
pub use discover::{discover_browser, discover_chrome};
pub use launch::launch_chrome;
pub use paths::{
    isolated_chrome_user_data_dir, resolve_profile_directory, resolve_user_data_dir,
    system_chrome_user_data_dir,
};
pub use types::{BrowserDiscovery, BrowserFamily, ChromeInstance};

#[cfg(test)]
mod tests {
    use super::cdp::automation_markers_from_command;
    use super::discover::{browser_paths_for_windows_roots, default_user_data_dir};
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_default_user_data_dir_is_valid_path() {
        let dir = default_user_data_dir();
        let s = dir.to_string_lossy();
        assert!(
            s.contains("Chrome")
                || s.contains("google-chrome")
                || s.contains("Chromium")
                || s.contains("Brave")
                || s.contains("Edge")
                || s.contains("chrome-cdp"),
            "unexpected user-data-dir: {s}"
        );
    }

    #[test]
    fn test_system_profile_is_not_isolated_by_default() {
        let system = system_chrome_user_data_dir(None);
        let isolated = isolated_chrome_user_data_dir();
        assert_ne!(system, isolated);
        assert!(isolated.to_string_lossy().contains("chrome-cdp"));
        assert_eq!(resolve_user_data_dir(None, None), isolated);
    }

    #[test]
    fn test_discover_chrome_finds_binary_when_installed() {
        if let Ok(discovery) = discover_chrome() {
            assert!(discovery.binary_path.exists(), "Chrome binary should exist");
            println!(
                "Found Chrome: {:?} version={:?}",
                discovery.binary_path, discovery.version
            );
        }
    }

    #[test]
    fn detects_strong_chromedriver_process_markers() {
        assert_eq!(
            automation_markers_from_command(
                "/tmp/chrome --remote-debugging-port=9222 --enable-automation",
                false
            ),
            vec!["--enable-automation"]
        );
        assert_eq!(
            automation_markers_from_command("/opt/bin/chromedriver --port=9515", true),
            vec!["ChromeDriver ancestor process"]
        );
    }

    #[test]
    fn accepts_direct_cdp_chrome_command() {
        assert!(automation_markers_from_command(
            "/Applications/Google Chrome --remote-debugging-port=9222 --headless=new",
            false
        )
        .is_empty());
    }

    #[test]
    fn windows_install_roots_include_chrome_and_edge_exe() {
        let root = PathBuf::from(r"C:\Program Files");
        let auto =
            browser_paths_for_windows_roots(BrowserFamily::Auto, std::slice::from_ref(&root));
        assert!(auto.iter().any(|p| p.ends_with("chrome.exe")));
        assert!(auto.iter().any(|p| p.ends_with("msedge.exe")));
        let chrome_only = browser_paths_for_windows_roots(BrowserFamily::Chrome, &[root]);
        assert!(chrome_only.iter().any(|p| p.ends_with("chrome.exe")));
        assert!(!chrome_only.iter().any(|p| p.ends_with("msedge.exe")));
    }
}
