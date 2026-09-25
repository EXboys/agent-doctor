//! Browser MCP target discovery + auto wire for installed runtimes only.

use std::path::PathBuf;

use agent_doctor_mcp::{
    discover_chrome, wire_browser_mcp, BrowserMcpWireReport, WireBrowserMcpOptions,
    BROWSER_MCP_WIRE_RUNTIMES,
};
use serde::{Deserialize, Serialize};

use crate::doctor::run_doctor;
use crate::workspace::{
    browser_configured_runtimes, ensure_stable_agent_doctor_cli, list_mcp_inventory,
    load_workspaces, resolve_agent_doctor_binary,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserMcpTargetStatus {
    pub runtime_id: String,
    pub display_name: String,
    pub installed: bool,
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserMcpDiagnoseIssue {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserMcpTargetAction {
    pub runtime_id: String,
    pub display_name: String,
    pub installed: bool,
    pub configured_before: bool,
    /// wrote | skipped_not_installed | failed
    pub action: String,
    pub ok: bool,
    pub config_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserMcpDiagnoseWireReport {
    pub chrome_ok: bool,
    pub cli_ok: bool,
    pub issues: Vec<BrowserMcpDiagnoseIssue>,
    pub targets: Vec<BrowserMcpTargetAction>,
    pub wrote: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// List wireable Browser MCP targets with install + configured flags.
pub fn list_browser_mcp_targets() -> Vec<BrowserMcpTargetStatus> {
    let doctor = run_doctor();
    let configured = list_mcp_inventory()
        .map(|inventory| browser_configured_runtimes(&inventory))
        .unwrap_or_default();

    BROWSER_MCP_WIRE_RUNTIMES
        .iter()
        .map(|id| {
            let runtime = doctor.runtimes.iter().find(|runtime| runtime.id == *id);
            BrowserMcpTargetStatus {
                runtime_id: (*id).to_string(),
                display_name: runtime
                    .map(|runtime| runtime.display_name.clone())
                    .unwrap_or_else(|| (*id).to_string()),
                installed: runtime.map(|runtime| runtime.installed).unwrap_or(false),
                configured: configured.iter().any(|item| item == *id),
            }
        })
        .collect()
}

/// Runtime ids among Browser MCP wire targets that are installed on this machine.
pub fn installed_browser_mcp_runtime_ids() -> Vec<String> {
    list_browser_mcp_targets()
        .into_iter()
        .filter(|target| target.installed)
        .map(|target| target.runtime_id)
        .collect()
}

fn display_name_for(runtime_id: &str, targets: &[BrowserMcpTargetStatus]) -> String {
    targets
        .iter()
        .find(|target| target.runtime_id == runtime_id)
        .map(|target| target.display_name.clone())
        .unwrap_or_else(|| runtime_id.to_string())
}

/// Build desktop/CLI wire options for the active workspace (no runtime filter yet).
pub fn browser_mcp_wire_options_for_active_workspace(binary: PathBuf) -> WireBrowserMcpOptions {
    let workspaces = load_workspaces().unwrap_or_default();
    let active_entry = workspaces
        .active
        .as_ref()
        .and_then(|name| workspaces.workspaces.get(name));
    let mut options = WireBrowserMcpOptions::with_binary(binary);
    options.project_path = active_entry.map(|entry| entry.path.clone());
    options.codex_home = active_entry.map(|entry| entry.codex_home.clone());
    options.hermes_home = active_entry.map(|entry| {
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".hermes/profiles")
            .join(&entry.hermes_profile)
    });
    options.openclaw_workspace = active_entry.map(|entry| entry.openclaw_workspace.clone());
    options
}

/// Wire Browser MCP into installed runtimes only (auto-detect).
pub fn wire_browser_mcp_installed(
    options: &WireBrowserMcpOptions,
) -> Result<BrowserMcpWireReport, String> {
    let installed = installed_browser_mcp_runtime_ids();
    if installed.is_empty() {
        return Ok(BrowserMcpWireReport {
            results: Vec::new(),
        });
    }
    let discovery = discover_chrome().map_err(|error| error.to_string())?;
    let mut scoped = options.clone();
    scoped.runtimes = installed;
    Ok(wire_browser_mcp(&discovery, &scoped))
}

/// Diagnose Chrome / CLI / installed agents, then write Browser MCP into each installed target.
pub fn diagnose_and_wire_browser_mcp(
    mut options: WireBrowserMcpOptions,
) -> BrowserMcpDiagnoseWireReport {
    let targets_before = list_browser_mcp_targets();
    let mut issues = Vec::new();

    let chrome = discover_chrome();
    let chrome_ok = chrome.is_ok();
    if let Err(error) = &chrome {
        issues.push(BrowserMcpDiagnoseIssue {
            code: "chrome_missing".into(),
            message: error.to_string(),
        });
    }

    let binary = if options.binary.as_os_str().is_empty() {
        resolve_agent_doctor_binary()
            .ok()
            .and_then(|p| ensure_stable_agent_doctor_cli(&p).ok())
    } else {
        ensure_stable_agent_doctor_cli(&options.binary)
            .ok()
            .or_else(|| Some(options.binary.clone()))
    };
    let cli_ok = binary.is_some();
    if !cli_ok {
        issues.push(BrowserMcpDiagnoseIssue {
            code: "cli_missing".into(),
            message: "Agent Doctor CLI binary not resolved".into(),
        });
    }

    let installed_ids: Vec<String> = targets_before
        .iter()
        .filter(|target| target.installed)
        .map(|target| target.runtime_id.clone())
        .collect();

    if installed_ids.is_empty() {
        issues.push(BrowserMcpDiagnoseIssue {
            code: "no_installed_runtimes".into(),
            message:
                "No installed Codex / Claude Code / Hermes / OpenClaw / DeepSeek Harness to write"
                    .into(),
        });
    }

    let mut actions: Vec<BrowserMcpTargetAction> = targets_before
        .iter()
        .filter(|target| !target.installed)
        .map(|target| BrowserMcpTargetAction {
            runtime_id: target.runtime_id.clone(),
            display_name: target.display_name.clone(),
            installed: false,
            configured_before: target.configured,
            action: "skipped_not_installed".into(),
            ok: true,
            config_path: None,
            message: format!("{} not installed — skipped", target.display_name),
        })
        .collect();

    let mut wrote = 0usize;
    let mut failed = 0usize;
    let skipped = actions.len();

    if chrome_ok && cli_ok && !installed_ids.is_empty() {
        if let Some(path) = binary {
            options.binary = path;
        }
        options.runtimes = installed_ids;
        let discovery = chrome.expect("chrome_ok");
        let wire = wire_browser_mcp(&discovery, &options);
        for result in wire.results {
            let display_name = display_name_for(&result.runtime, &targets_before);
            let configured_before = targets_before
                .iter()
                .find(|target| target.runtime_id == result.runtime)
                .map(|target| target.configured)
                .unwrap_or(false);
            if result.ok {
                wrote += 1;
                actions.push(BrowserMcpTargetAction {
                    runtime_id: result.runtime,
                    display_name,
                    installed: true,
                    configured_before,
                    action: "wrote".into(),
                    ok: true,
                    config_path: result.config_path,
                    message: result.message,
                });
            } else {
                failed += 1;
                actions.push(BrowserMcpTargetAction {
                    runtime_id: result.runtime,
                    display_name,
                    installed: true,
                    configured_before,
                    action: "failed".into(),
                    ok: false,
                    config_path: result.config_path,
                    message: result.message,
                });
            }
        }
    } else {
        for target in targets_before.iter().filter(|target| target.installed) {
            failed += 1;
            actions.push(BrowserMcpTargetAction {
                runtime_id: target.runtime_id.clone(),
                display_name: target.display_name.clone(),
                installed: true,
                configured_before: target.configured,
                action: "failed".into(),
                ok: false,
                config_path: None,
                message: "Blocked by diagnose issues (Chrome or CLI)".into(),
            });
        }
    }

    // Stable order: follow BROWSER_MCP_WIRE_RUNTIMES
    actions.sort_by_key(|action| {
        BROWSER_MCP_WIRE_RUNTIMES
            .iter()
            .position(|id| *id == action.runtime_id.as_str())
            .unwrap_or(usize::MAX)
    });

    BrowserMcpDiagnoseWireReport {
        chrome_ok,
        cli_ok,
        issues,
        targets: actions,
        wrote,
        failed,
        skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_cover_five_wire_runtimes() {
        let targets = list_browser_mcp_targets();
        assert_eq!(targets.len(), 5);
        assert_eq!(
            targets
                .iter()
                .map(|t| t.runtime_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "codex",
                "claude-code",
                "hermes",
                "openclaw",
                "deepseek-harness"
            ]
        );
    }
}
