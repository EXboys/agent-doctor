//! Registers the browser config writer with agent-doctor-core.

use agent_doctor_core::{
    BrowserMcpWireBackend, BrowserMcpWireReport, BrowserMcpWireResult, WireBrowserMcpOptions,
};

use crate::browser::discover_chrome;
use crate::wire::wire_browser_mcp;

struct McpWireBackend;

impl BrowserMcpWireBackend for McpWireBackend {
    fn ensure_chrome(&self) -> Result<(), String> {
        discover_chrome()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn wire(&self, options: &WireBrowserMcpOptions) -> BrowserMcpWireReport {
        match discover_chrome() {
            Ok(discovery) => wire_browser_mcp(&discovery, options),
            Err(error) => BrowserMcpWireReport {
                results: options
                    .runtimes
                    .iter()
                    .map(|runtime| BrowserMcpWireResult {
                        runtime: runtime.clone(),
                        ok: false,
                        config_path: None,
                        message: error.to_string(),
                    })
                    .collect(),
            },
        }
    }
}

static MCP_WIRE_BACKEND: McpWireBackend = McpWireBackend;

/// Connect core diagnosis and repair to this crate's browser writer.
pub fn register_browser_mcp_wire_backend() {
    agent_doctor_core::install_browser_mcp_wire_backend(&MCP_WIRE_BACKEND);
}
