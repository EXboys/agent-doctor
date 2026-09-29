//! Wire Browser MCP into Claude Code / Codex after setup or mode switch.

use std::path::Path;

use anyhow::Result;

pub use agent_doctor_core::{
    BrowserMcpWireReport, BrowserMcpWireResult, WireBrowserMcpOptions, BROWSER_MCP_WIRE_RUNTIMES,
};

use crate::browser::{
    discover_chrome, resolve_profile_directory, resolve_user_data_dir, BrowserDiscovery,
};
use crate::config::{configure_for, mcp_servers_path_with_openclaw, McpConfigureOptions};

/// Upsert the `browser` MCP entry for each requested runtime.
///
/// Preserves other MCP servers / unrelated config keys. Failures for one
/// runtime do not abort the rest.
pub fn wire_browser_mcp(
    discovery: &BrowserDiscovery,
    options: &WireBrowserMcpOptions,
) -> BrowserMcpWireReport {
    let runtimes: Vec<&str> = if options.runtimes.is_empty() {
        BROWSER_MCP_WIRE_RUNTIMES.to_vec()
    } else {
        options.runtimes.iter().map(String::as_str).collect()
    };

    let user_data_dir = Some(resolve_user_data_dir(
        options.user_data_dir.as_ref(),
        Some(&discovery.binary_path),
    ));
    let profile_directory = Some(resolve_profile_directory(
        options.profile_directory.as_deref(),
    ));

    let mut results = Vec::with_capacity(runtimes.len());
    for runtime in runtimes {
        let configure = McpConfigureOptions {
            runtime: runtime.to_string(),
            port: options.port,
            headless: options.headless,
            user_data_dir: user_data_dir.clone(),
            profile_directory: profile_directory.clone(),
            binary: options.binary.clone(),
            project_path: options.project_path.clone(),
            codex_home: options.codex_home.clone(),
            hermes_home: options.hermes_home.clone(),
            openclaw_workspace: options.openclaw_workspace.clone(),
        };

        match configure_for(discovery, &configure) {
            Ok(()) => {
                let config_path = mcp_servers_path_with_openclaw(
                    runtime,
                    options.project_path.as_deref(),
                    options.codex_home.as_deref(),
                    options.hermes_home.as_deref(),
                    options.openclaw_workspace.as_deref(),
                )
                .ok()
                .map(|p| p.display().to_string());
                let message = if runtime == "openclaw" && options.openclaw_workspace.is_some() {
                    "wrote browser MCP for openclaw (global ~/.openclaw + workspace .mcp.json mirror; runtime is still global)".to_string()
                } else if runtime == "openclaw" {
                    "wrote browser MCP for openclaw (global ~/.openclaw — not per-workspace isolated)".to_string()
                } else {
                    format!("wrote browser MCP entry for {runtime}")
                };
                results.push(BrowserMcpWireResult {
                    runtime: runtime.to_string(),
                    ok: true,
                    config_path,
                    message,
                });
            }
            Err(err) => {
                results.push(BrowserMcpWireResult {
                    runtime: runtime.to_string(),
                    ok: false,
                    config_path: None,
                    message: err.to_string(),
                });
            }
        }
    }

    BrowserMcpWireReport { results }
}

/// Discover Chrome and wire Browser MCP using defaults.
pub fn wire_browser_mcp_defaults(binary: &Path) -> Result<BrowserMcpWireReport> {
    let discovery = discover_chrome()?;
    Ok(wire_browser_mcp(
        &discovery,
        &WireBrowserMcpOptions::with_binary(binary.to_path_buf()),
    ))
}

/// Human-readable next steps after installing Claude Code / Codex / Hermes / OpenClaw.
pub fn wiring_next_steps_for_runtime(runtime: &str) -> Option<Vec<String>> {
    match runtime {
        "claude-code" | "claude" | "codex" | "hermes" | "openclaw" | "deepseek-harness"
        | "dsh" => Some(vec![
            "Configure a Personal Provider or connect Evotown (Team), then switch mode to write LLM gateway settings into this runtime.".to_string(),
            "CLI: `agent-doctor mode personal` or `agent-doctor mode team` (add `--with-browser-mcp` to also write Browser MCP).".to_string(),
            "Optional Browser MCP only: `agent-doctor mcp configure <codex|claude-code|hermes|openclaw|deepseek-harness>`.".to_string(),
        ]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiring_hints_cover_four_runtimes() {
        assert!(wiring_next_steps_for_runtime("claude-code").is_some());
        assert!(wiring_next_steps_for_runtime("codex").is_some());
        assert!(wiring_next_steps_for_runtime("hermes").is_some());
        assert!(wiring_next_steps_for_runtime("openclaw").is_some());
        assert!(wiring_next_steps_for_runtime("deepseek-harness").is_some());
    }

    #[test]
    fn default_wire_runtimes_are_stable() {
        assert_eq!(
            BROWSER_MCP_WIRE_RUNTIMES,
            &[
                "codex",
                "claude-code",
                "hermes",
                "openclaw",
                "deepseek-harness"
            ]
        );
    }
}
