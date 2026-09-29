//! Types for writing Browser MCP into an agent config.
//!
//! The Chromium discovery and config writer stay in `agent-doctor-mcp`.
//! That crate registers a backend when the CLI or desktop starts. Diagnosis,
//! repair, and Ask call the backend. This crate does not link the browser server.

use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Default Chrome DevTools port written into agent MCP config.
pub const DEFAULT_BROWSER_MCP_PORT: u16 = 9222;

/// Stable order for browser target lists.
///
/// Membership matches `RuntimeDescriptor::browser_mcp`.
pub const BROWSER_MCP_WIRE_RUNTIMES: &[&str] = &[
    "codex",
    "claude-code",
    "hermes",
    "openclaw",
    "deepseek-harness",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserMcpWireResult {
    pub runtime: String,
    pub ok: bool,
    pub config_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserMcpWireReport {
    pub results: Vec<BrowserMcpWireResult>,
}

#[derive(Debug, Clone)]
pub struct WireBrowserMcpOptions {
    pub port: u16,
    pub headless: bool,
    pub user_data_dir: Option<PathBuf>,
    pub profile_directory: Option<String>,
    pub binary: PathBuf,
    pub project_path: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
    pub hermes_home: Option<PathBuf>,
    /// OpenClaw agent workspace — mirrors Browser MCP into `<ws>/.mcp.json`.
    pub openclaw_workspace: Option<PathBuf>,
    /// When empty, wires [`BROWSER_MCP_WIRE_RUNTIMES`].
    pub runtimes: Vec<String>,
}

impl WireBrowserMcpOptions {
    pub fn with_binary(binary: PathBuf) -> Self {
        Self {
            port: DEFAULT_BROWSER_MCP_PORT,
            headless: false,
            user_data_dir: None,
            profile_directory: None,
            binary,
            project_path: None,
            codex_home: None,
            hermes_home: None,
            openclaw_workspace: None,
            runtimes: Vec::new(),
        }
    }
}

/// Writes Browser MCP config. Implemented by `agent-doctor-mcp`.
pub trait BrowserMcpWireBackend: Send + Sync {
    fn ensure_chrome(&self) -> Result<(), String>;
    fn wire(&self, options: &WireBrowserMcpOptions) -> BrowserMcpWireReport;
}

static BACKEND: OnceLock<&'static dyn BrowserMcpWireBackend> = OnceLock::new();

pub fn install_browser_mcp_wire_backend(backend: &'static dyn BrowserMcpWireBackend) {
    let _ = BACKEND.set(backend);
}

pub(crate) fn ensure_chrome_for_wire() -> Result<(), String> {
    backend()?.ensure_chrome()
}

pub(crate) fn wire_browser_mcp(
    options: &WireBrowserMcpOptions,
) -> Result<BrowserMcpWireReport, String> {
    Ok(backend()?.wire(options))
}

fn backend() -> Result<&'static dyn BrowserMcpWireBackend, String> {
    BACKEND
        .get()
        .copied()
        .ok_or_else(|| "Browser wiring is not available in this process".to_string())
}
