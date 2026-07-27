pub mod browser;
pub mod config;
pub mod mcp;
pub mod status;
pub mod tools;

pub use browser::{
    connect_chrome, discover_chrome, launch_chrome, stop_chrome, BrowserDiscovery, ChromeInstance,
};
pub use config::{
    configure_for, generate_config_snippet, mcp_servers_path, McpConfigureOptions, McpServerEntry,
};
pub use mcp::{run_mcp_server, HandleResult, McpRequest, McpResponse, ToolDefinition};
pub use status::{browser_mcp_status, BrowserMcpStatus, DEFAULT_BROWSER_MCP_PORT};
pub use tools::BrowserContext;
