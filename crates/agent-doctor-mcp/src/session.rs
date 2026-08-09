//! Lazy browser session — MCP handshake first, Chrome on first tool use.

use std::sync::Mutex;

use anyhow::{Context, Result};

use crate::browser::{
    cdp_port_is_headless, connect_chrome, discover_chrome, kill_chrome_on_port, launch_chrome,
    stop_chrome, ChromeInstance,
};
use crate::tools::BrowserContext;

pub struct LazyBrowser {
    port: u16,
    headless: bool,
    user_data_dir: Option<std::path::PathBuf>,
    chrome: Option<ChromeInstance>,
    ctx: Option<BrowserContext>,
}

impl LazyBrowser {
    pub fn new(port: u16, headless: bool) -> Self {
        Self::with_user_data_dir(port, headless, None)
    }

    pub fn with_user_data_dir(
        port: u16,
        headless: bool,
        user_data_dir: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            port,
            headless,
            user_data_dir,
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
                discovery.user_data_dir = crate::browser::resolve_user_data_dir(
                    None,
                    Some(&discovery.binary_path),
                );
            }
            let instance = match connect_chrome(self.port) {
                Ok(existing) => {
                    // Prefer reusing CDP, but never keep a headless Chrome when the
                    // user asked for a visible window (and vice versa).
                    let existing_headless = cdp_port_is_headless(self.port);
                    let mode_mismatch = matches!(
                        (self.headless, existing_headless),
                        (false, Some(true)) | (true, Some(false))
                    );
                    if mode_mismatch {
                        eprintln!(
                            "Existing Chrome on port {} is {} but requested {}; restarting…",
                            self.port,
                            if existing_headless == Some(true) {
                                "headless"
                            } else {
                                "headed"
                            },
                            if self.headless { "headless" } else { "headed" }
                        );
                        kill_chrome_on_port(self.port)?;
                        eprintln!(
                            "Starting Chrome on port {} (profile: {}, ui={})",
                            self.port,
                            discovery.user_data_dir.display(),
                            if self.headless { "hidden" } else { "visible" }
                        );
                        launch_chrome(&discovery, self.port, self.headless)?
                    } else {
                        eprintln!(
                            "Connected to existing Chrome CDP on port {}",
                            self.port
                        );
                        existing
                    }
                }
                Err(_) => {
                    eprintln!(
                        "Starting Chrome on port {} (profile: {}, ui={})",
                        self.port,
                        discovery.user_data_dir.display(),
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

            let ctx = BrowserContext::connect(&ws_endpoint)?;
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
