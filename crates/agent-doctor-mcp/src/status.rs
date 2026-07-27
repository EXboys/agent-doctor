//! Browser MCP readiness status for CLI / desktop UI.

use serde::{Deserialize, Serialize};

use crate::browser::{connect_chrome, discover_chrome};

pub const DEFAULT_BROWSER_MCP_PORT: u16 = 9222;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserMcpStatus {
    pub chrome_found: bool,
    pub binary: Option<String>,
    pub version: Option<String>,
    pub user_data_dir: Option<String>,
    pub cdp_connected: bool,
    pub ws_endpoint: Option<String>,
    pub port: u16,
}

pub fn browser_mcp_status(port: u16) -> BrowserMcpStatus {
    let discovery = discover_chrome().ok();
    let connected = connect_chrome(port).ok();

    BrowserMcpStatus {
        chrome_found: discovery.is_some(),
        binary: discovery
            .as_ref()
            .map(|d| d.binary_path.display().to_string()),
        version: discovery.as_ref().and_then(|d| d.version.clone()),
        user_data_dir: discovery
            .as_ref()
            .map(|d| d.user_data_dir.display().to_string()),
        cdp_connected: connected.is_some(),
        ws_endpoint: connected.and_then(|c| c.ws_endpoint),
        port,
    }
}
