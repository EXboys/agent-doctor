//! Lazy browser session — MCP handshake first, Chrome on first tool use.

use std::sync::Mutex;

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
                        if profile_locked_by_other_chrome(&discovery.user_data_dir, self.port) {
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
                    if profile_locked_by_other_chrome(&discovery.user_data_dir, self.port) {
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
            self.chrome = Some(instance);
            self.ctx = Some(ctx);
        }

        self.ctx
            .as_mut()
            .context("browser context missing after ensure")
    }
}

impl Drop for LazyBrowser {
    fn drop(&mut self) {
        if let Some(mut chrome) = self.chrome.take() {
            let _ = stop_chrome(&mut chrome);
        }
    }
}

pub type SharedBrowser = Mutex<LazyBrowser>;
