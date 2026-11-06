//! Lazy browser session — MCP handshake first, Chrome on first tool use.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::browser::{
    cdp_automation_markers, cdp_port_is_headless, cdp_user_data_dir, connect_chrome,
    discover_chrome, kill_chrome_on_port, launch_chrome, profile_locked_by_other_chrome,
    stop_chrome, ChromeInstance,
};
use crate::tools::BrowserContext;

pub struct LazyBrowser {
    port: u16,
    headless: bool,
    user_data_dir: Option<std::path::PathBuf>,
    profile_directory: Option<String>,
    chrome: Option<ChromeInstance>,
    ctx: Option<BrowserContext>,
}

impl LazyBrowser {
    pub fn new(port: u16, headless: bool) -> Self {
        Self::with_options(port, headless, None, None)
    }

    pub fn with_user_data_dir(
        port: u16,
        headless: bool,
        user_data_dir: Option<std::path::PathBuf>,
    ) -> Self {
        Self::with_options(port, headless, user_data_dir, None)
    }

    pub fn with_options(
        port: u16,
        headless: bool,
        user_data_dir: Option<std::path::PathBuf>,
        profile_directory: Option<String>,
    ) -> Self {
        Self {
            port,
            headless,
            user_data_dir,
            profile_directory,
            chrome: None,
            ctx: None,
        }
    }

    pub fn ensure(&mut self) -> Result<&mut BrowserContext> {
        // The page under the session can close (window shut, tab closed) and
        // Chrome can quit; probing before each use turns that into a reconnect
        // instead of a session that stays dead until the process restarts.
        if self.ctx.as_mut().is_some_and(|ctx| !ctx.is_alive()) {
            eprintln!(
                "Chrome CDP connection on port {} was lost; reconnecting…",
                self.port
            );
            self.reset();
        }
        if self.ctx.is_none() {
            let mut discovery = discover_chrome().context("Chrome not found")?;
            if let Some(dir) = &self.user_data_dir {
                discovery.user_data_dir = dir.clone();
            } else {
                discovery.user_data_dir =
                    crate::browser::resolve_user_data_dir(None, Some(&discovery.binary_path));
            }
            discovery.profile_directory =
                crate::browser::resolve_profile_directory(self.profile_directory.as_deref());
            let mut instance = match connect_chrome(self.port) {
                Ok(existing) => {
                    let automation_markers = cdp_automation_markers(self.port);
                    if !automation_markers.is_empty() {
                        anyhow::bail!(
                            "Refusing Chrome CDP on port {} because it appears to be owned by \
                             ChromeDriver ({}). Stop Selenium/ChromeDriver and retry so Agent \
                             Doctor can launch a clean Chrome instance.",
                            self.port,
                            automation_markers.join(", ")
                        );
                    }
                    // Prefer reusing CDP, but never keep a headless Chrome when the
                    // user asked for a visible window (and vice versa).
                    let existing_headless = cdp_port_is_headless(self.port);
                    let mode_mismatch = matches!(
                        (self.headless, existing_headless),
                        (false, Some(true)) | (true, Some(false))
                    );
                    let profile_mismatch = cdp_user_data_dir(self.port)
                        .map(|dir| dir != discovery.user_data_dir)
                        .unwrap_or(false);
                    if mode_mismatch || profile_mismatch {
                        eprintln!(
                            "Existing Chrome on port {} is wrong mode/profile (headless_mismatch={}, profile_mismatch={}); restarting…",
                            self.port, mode_mismatch, profile_mismatch
                        );
                        kill_chrome_on_port(self.port)?;
                        if profile_held_by_other_chrome(&discovery.user_data_dir, self.port) {
                            anyhow::bail!(
                                "Chrome profile `{}` is already open without remote debugging. \
                                 Quit that Chrome completely, then retry — or start it with \
                                 --remote-debugging-port={}.",
                                discovery.user_data_dir.display(),
                                self.port
                            );
                        }
                        eprintln!(
                            "Starting Chrome on port {} (profile: {} / {}, ui={})",
                            self.port,
                            discovery.user_data_dir.display(),
                            discovery.profile_directory,
                            if self.headless { "hidden" } else { "visible" }
                        );
                        launch_chrome(&discovery, self.port, self.headless)?
                    } else {
                        eprintln!("Connected to existing Chrome CDP on port {}", self.port);
                        existing
                    }
                }
                Err(_) => {
                    if profile_held_by_other_chrome(&discovery.user_data_dir, self.port) {
                        anyhow::bail!(
                            "Chrome profile `{}` is already open without remote debugging. \
                             Quit that Chrome completely, then retry — or start it with \
                             --remote-debugging-port={}. Opening a second copy would either \
                             fail or spawn an empty browser.",
                            discovery.user_data_dir.display(),
                            self.port
                        );
                    }
                    eprintln!(
                        "Starting Chrome on port {} (profile: {} / {}, ui={})",
                        self.port,
                        discovery.user_data_dir.display(),
                        discovery.profile_directory,
                        if self.headless { "hidden" } else { "visible" }
                    );
                    launch_chrome(&discovery, self.port, self.headless)?
                }
            };

            let ws_endpoint = instance
                .ws_endpoint
                .clone()
                .or_else(|| {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    connect_chrome(self.port).ok().and_then(|c| c.ws_endpoint)
                })
                .with_context(|| {
                    format!(
                        "Failed to connect to Chrome CDP on port {}. \
                         Quit other Chrome instances using this profile, or start Chrome with \
                         --remote-debugging-port={}.",
                        self.port, self.port
                    )
                })?;

            let mut ctx = BrowserContext::connect(&ws_endpoint)?;
            let chrome_driver_artifacts = ctx.chrome_driver_artifacts().unwrap_or_default();
            if !chrome_driver_artifacts.is_empty() {
                // Only stop processes launched by this session. An externally owned
                // browser must be stopped by its owner (usually ChromeDriver).
                let _ = stop_chrome(&mut instance);
                anyhow::bail!(
                    "Refusing Chrome CDP on port {} because the page contains ChromeDriver \
                     globals ({}). Stop Selenium/ChromeDriver and retry so Agent Doctor can \
                     launch a clean Chrome instance.",
                    self.port,
                    chrome_driver_artifacts.join(", ")
                );
            }
            eprintln!("Chrome CDP ready: {ws_endpoint}");
            self.adopt(instance);
            self.ctx = Some(ctx);
        }

        self.ctx
            .as_mut()
            .context("browser context missing after ensure")
    }

    /// Forget the CDP connection; the next [`ensure`](Self::ensure) reconnects,
    /// reusing Chrome while it still runs and relaunching it once it is gone.
    /// Never stops Chrome: the pages the person opened stay up.
    pub fn reset(&mut self) {
        self.ctx = None;
        let exited = self.chrome.as_mut().is_some_and(|instance| {
            instance
                .process
                .as_mut()
                .is_some_and(|child| !matches!(child.try_wait(), Ok(None)))
        });
        if exited {
            self.chrome = None;
        }
    }

    /// Record the Chrome the session now talks to. Reattaching to a Chrome
    /// this session launched keeps its process handle, so Drop still stops it.
    fn adopt(&mut self, mut instance: ChromeInstance) {
        if let Some(mut child) = self.chrome.take().and_then(|previous| previous.process) {
            if instance.process.is_none() && matches!(child.try_wait(), Ok(None)) {
                instance.process = Some(child);
            } else {
                // The restart path killed it; reap it so it doesn't linger.
                let _ = child.try_wait();
            }
        }
        self.chrome = Some(instance);
    }
}

/// A Chrome that just quit can hold its profile for a moment after its CDP
/// port closes; only a lock that outlasts that belongs to another Chrome.
fn profile_held_by_other_chrome(user_data_dir: &Path, port: u16) -> bool {
    let user_data_dir = user_data_dir.to_path_buf();
    for _ in 0..15 {
        if !profile_locked_by_other_chrome(&user_data_dir, port) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    true
}

impl Drop for LazyBrowser {
    fn drop(&mut self) {
        if let Some(mut chrome) = self.chrome.take() {
            let _ = stop_chrome(&mut chrome);
        }
    }
}

pub type SharedBrowser = Mutex<LazyBrowser>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::chrome_http_json;

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map(|addr| addr.port())
            .expect("free port")
    }

    fn page(title: &str) -> String {
        format!("data:text/html,<title>{title}</title>")
    }

    /// Profile path with a space in it, like macOS "Application Support".
    fn spaced_profile(root: &tempfile::TempDir) -> std::path::PathBuf {
        root.path().join("chrome cdp")
    }

    fn close_all_pages(port: u16) {
        let list = chrome_http_json(port, "/json/list").expect("list targets");
        for target in list.as_array().into_iter().flatten() {
            if target["type"] == "page" {
                if let Some(id) = target["id"].as_str() {
                    // Chrome answers in plain text, so the JSON parse fails
                    // after the close already went through.
                    let _ = chrome_http_json(port, &format!("/json/close/{id}"));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    #[test]
    #[ignore = "launches a real headless Chrome"]
    fn reconnects_after_its_page_closes() {
        let root = tempfile::tempdir().unwrap();
        let port = free_port();
        let mut browser = LazyBrowser::with_user_data_dir(port, true, Some(spaced_profile(&root)));
        browser.ensure().unwrap().navigate(&page("one")).unwrap();

        close_all_pages(port);

        let ctx = browser.ensure().unwrap();
        ctx.navigate(&page("two")).unwrap();
        assert!(ctx.get_url().unwrap().contains("two"));
    }

    #[test]
    #[ignore = "launches a real headless Chrome"]
    fn relaunches_after_chrome_quits() {
        let root = tempfile::tempdir().unwrap();
        let port = free_port();
        let mut browser = LazyBrowser::with_user_data_dir(port, true, Some(spaced_profile(&root)));
        browser.ensure().unwrap().navigate(&page("one")).unwrap();

        let chrome = browser
            .chrome
            .as_mut()
            .and_then(|instance| instance.process.as_mut())
            .expect("session launched Chrome");
        chrome.kill().unwrap();
        chrome.wait().unwrap();

        browser.ensure().unwrap().navigate(&page("two")).unwrap();
        assert!(
            browser
                .chrome
                .as_ref()
                .is_some_and(|instance| instance.process.is_some()),
            "the relaunched Chrome belongs to the session"
        );
    }

    #[test]
    #[ignore = "launches a real headless Chrome"]
    fn second_session_attaches_instead_of_restarting_chrome() {
        let root = tempfile::tempdir().unwrap();
        let port = free_port();
        let profile = spaced_profile(&root);
        let mut first = LazyBrowser::with_user_data_dir(port, true, Some(profile.clone()));
        first.ensure().unwrap().navigate(&page("one")).unwrap();
        assert_eq!(cdp_user_data_dir(port), Some(profile.clone()));

        let mut second = LazyBrowser::with_user_data_dir(port, true, Some(profile));
        second.ensure().unwrap().navigate(&page("two")).unwrap();

        let first_chrome = first
            .chrome
            .as_mut()
            .and_then(|instance| instance.process.as_mut())
            .expect("first session launched Chrome");
        assert!(
            first_chrome.try_wait().unwrap().is_none(),
            "the second session must not kill the running Chrome"
        );
        first.ensure().unwrap().navigate(&page("three")).unwrap();
    }
}
