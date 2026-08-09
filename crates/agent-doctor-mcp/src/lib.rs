pub mod browser;
pub mod config;
pub mod mcp;
pub mod session;
pub mod status;
pub mod tools;

pub use browser::{
    cdp_port_is_headless, cdp_user_data_dir, connect_chrome, discover_chrome,
    isolated_chrome_user_data_dir, kill_chrome_on_port, launch_chrome,
    profile_locked_by_other_chrome, resolve_profile_directory, resolve_user_data_dir, stop_chrome,
    system_chrome_user_data_dir, BrowserDiscovery, ChromeInstance,
};
pub use config::{
    browser_mcp_args, configure_for, generate_config_snippet, mcp_servers_path,
    McpConfigureOptions, McpServerEntry,
};
pub use mcp::{run_mcp_server, HandleResult, McpRequest, McpResponse, ToolDefinition};
pub use session::{LazyBrowser, SharedBrowser};
pub use status::{browser_mcp_status, BrowserMcpStatus, DEFAULT_BROWSER_MCP_PORT};
pub use tools::BrowserContext;
