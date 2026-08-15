//! Browser MCP readiness status for CLI / desktop UI.

use serde::{Deserialize, Serialize};

use crate::browser::{
    connect_chrome, discover_chrome, isolated_chrome_user_data_dir, resolve_profile_directory,
    system_chrome_user_data_dir,
};

pub const DEFAULT_BROWSER_MCP_PORT: u16 = 9222;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserMcpStatus {
    pub chrome_found: bool,
    pub binary: Option<String>,
    pub version: Option<String>,
    /// Resolved profile that will be used to launch (env / default).
    pub user_data_dir: Option<String>,
    /// Chrome profile directory name (`Default`, `Profile 2`, …).
    pub profile_directory: String,
    /// Everyday browser profile path for this Chrome binary.
    pub system_user_data_dir: String,
    /// Isolated Agent Doctor profile (no shared login).
    pub isolated_user_data_dir: String,
    pub cdp_connected: bool,
    pub ws_endpoint: Option<String>,
    pub port: u16,
}

pub fn browser_mcp_status(port: u16) -> BrowserMcpStatus {
    browser_mcp_status_with_probe(port, true)
}

/// When `probe_live` is false, skip CDP connect so UI status never wakes Chrome.
pub fn browser_mcp_status_with_probe(port: u16, probe_live: bool) -> BrowserMcpStatus {
    let discovery = discover_chrome().ok();
    let connected = if probe_live {
        connect_chrome(port).ok()
    } else {
        None
    };
    let system = system_chrome_user_data_dir(discovery.as_ref().map(|d| &d.binary_path));
    let isolated = isolated_chrome_user_data_dir();

    BrowserMcpStatus {
        chrome_found: discovery.is_some(),
        binary: discovery
            .as_ref()
            .map(|d| d.binary_path.display().to_string()),
        version: discovery.as_ref().and_then(|d| d.version.clone()),
        user_data_dir: discovery
            .as_ref()
            .map(|d| d.user_data_dir.display().to_string()),
        profile_directory: discovery
            .as_ref()
            .map(|d| d.profile_directory.clone())
            .unwrap_or_else(|| resolve_profile_directory(None)),
        system_user_data_dir: system.display().to_string(),
        isolated_user_data_dir: isolated.display().to_string(),
        cdp_connected: connected.is_some(),
        ws_endpoint: connected.and_then(|c| c.ws_endpoint),
        port,
    }
}
