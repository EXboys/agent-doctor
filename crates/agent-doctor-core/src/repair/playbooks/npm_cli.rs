//! Lightweight Claude Code / Codex repair playbooks.
//!
//! - Gateway drift → re-run active mode switch (`apply_claude_code` / `apply_codex_slot`)
//! - Browser MCP missing/unhealthy → `wire_browser_mcp` (`configure_for`)

use anyhow::{bail, Context, Result};

use agent_doctor_mcp::{wire_browser_mcp, WireBrowserMcpOptions};

use crate::probe::{ProbeStatus, RuntimeProbeReport};
use crate::repair::{SkippedRepairAction, SuggestedRepair};
use crate::setup::{
    load_mode_status, switch_to_personal_mode, switch_to_team_mode, MODE_PERSONAL, MODE_TEAM,
};
use crate::workspace::resolve_agent_doctor_binary;

use super::should_run;
use super::PlaybookApplyResult;

pub fn suggest_claude_code_repairs(probe: &RuntimeProbeReport) -> Vec<SuggestedRepair> {
    suggest_npm_cli_repairs("claude-code", "Claude Code", probe)
}

pub fn suggest_codex_repairs(probe: &RuntimeProbeReport) -> Vec<SuggestedRepair> {
    suggest_npm_cli_repairs("codex", "Codex CLI", probe)
}

fn suggest_npm_cli_repairs(
    runtime_id: &str,
    display: &str,
    probe: &RuntimeProbeReport,
) -> Vec<SuggestedRepair> {
    let mut items = Vec::new();
    let mode_ready = load_mode_status()
        .ok()
        .is_some_and(|s| s.mode == MODE_PERSONAL || s.mode == MODE_TEAM);

    for check in &probe.checks {
        if check.id == "mode.overlay_mismatch"
            && matches!(check.status, ProbeStatus::Warn | ProbeStatus::Fail)
        {
            items.push(SuggestedRepair {
                id: format!("fix-{runtime_id}-gateway-from-mode"),
                title: format!("Rewrite {display} gateway from active mode"),
                description: if mode_ready {
                    "Re-apply Personal/Team overlay into this runtime (merge, keep other settings)."
                        .to_string()
                } else {
                    "Configure a Personal Provider or connect Evotown, then switch mode."
                        .to_string()
                },
                auto_fixable: mode_ready,
            });
        }

        if check.id == "mcp.browser.configured"
            && matches!(check.status, ProbeStatus::Warn | ProbeStatus::Fail)
        {
            items.push(SuggestedRepair {
                id: format!("fix-{runtime_id}-browser-mcp"),
                title: format!("Write Browser MCP into {display} config"),
                description: "Upsert mcpServers/mcp_servers.browser → agent-doctor (keeps other MCP entries)."
                    .to_string(),
                auto_fixable: true,
            });
        }

        if check.id == "mcp.browser.healthy" && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: format!("fix-{runtime_id}-browser-mcp"),
                title: format!("Repair Browser MCP path for {display}"),
                description: "Rewrite the browser MCP command to the real agent-doctor CLI binary."
                    .to_string(),
                auto_fixable: true,
            });
        }
    }

    items
}

pub fn apply_claude_code_playbook(probe: &RuntimeProbeReport) -> Result<PlaybookApplyResult> {
    apply_claude_code_playbook_filtered(probe, None)
}

pub fn apply_claude_code_playbook_filtered(
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
) -> Result<PlaybookApplyResult> {
    apply_npm_cli_playbook("claude-code", probe, only_ids)
}

pub fn apply_codex_playbook(probe: &RuntimeProbeReport) -> Result<PlaybookApplyResult> {
    apply_codex_playbook_filtered(probe, None)
}

pub fn apply_codex_playbook_filtered(
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
) -> Result<PlaybookApplyResult> {
    apply_npm_cli_playbook("codex", probe, only_ids)
}

fn apply_npm_cli_playbook(
    runtime_id: &str,
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
) -> Result<PlaybookApplyResult> {
    let mut result = PlaybookApplyResult::default();
    let gateway_id = format!("fix-{runtime_id}-gateway-from-mode");
    let browser_id = format!("fix-{runtime_id}-browser-mcp");

    if should_run(&gateway_id, only_ids) && needs_gateway_rewire(probe) {
        match rewire_gateway_from_active_mode() {
            Ok(()) => result.executed.push(gateway_id),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: gateway_id,
                reason: error.to_string(),
            }),
        }
    }

    if should_run(&browser_id, only_ids) && needs_browser_mcp_rewire(probe) {
        match wire_browser_mcp_for_runtime(runtime_id) {
            Ok(()) => result.executed.push(browser_id),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: browser_id,
                reason: error.to_string(),
            }),
        }
    }

    Ok(result)
}

fn needs_gateway_rewire(probe: &RuntimeProbeReport) -> bool {
    probe.checks.iter().any(|check| {
        check.id == "mode.overlay_mismatch"
            && matches!(check.status, ProbeStatus::Warn | ProbeStatus::Fail)
    })
}

fn needs_browser_mcp_rewire(probe: &RuntimeProbeReport) -> bool {
    probe.checks.iter().any(|check| {
        (check.id == "mcp.browser.configured" || check.id == "mcp.browser.healthy")
            && matches!(check.status, ProbeStatus::Warn | ProbeStatus::Fail)
    })
}

fn rewire_gateway_from_active_mode() -> Result<()> {
    let status = load_mode_status().context("load mode status")?;
    match status.mode.as_str() {
        MODE_PERSONAL => {
            let report = switch_to_personal_mode(status.personal_active_id.as_deref())?;
            if report.runtimes.iter().all(|r| !r.applied) {
                bail!(
                    "mode switch produced no applied runtimes: {}",
                    report.message
                );
            }
            Ok(())
        }
        MODE_TEAM => {
            let report = switch_to_team_mode()?;
            if report.runtimes.iter().all(|r| !r.applied) {
                bail!(
                    "mode switch produced no applied runtimes: {}",
                    report.message
                );
            }
            Ok(())
        }
        _ => bail!(
            "no active Personal/Team mode — configure a provider first, then switch mode"
        ),
    }
}

fn wire_browser_mcp_for_runtime(runtime_id: &str) -> Result<()> {
    let discovery = agent_doctor_mcp::discover_chrome().context("discover Chrome")?;
    let binary = resolve_agent_doctor_binary().context("resolve agent-doctor binary")?;
    let mut options = WireBrowserMcpOptions::with_binary(binary);
    options.runtimes = vec![runtime_id.to_string()];
    if let Ok(doc) = crate::workspace::load_workspaces() {
        if let Some(active) = doc.active.as_ref() {
            if let Some(entry) = doc.workspaces.get(active) {
                options.project_path = Some(entry.path.clone());
                options.codex_home = Some(entry.codex_home.clone());
            }
        }
    }
    let report = wire_browser_mcp(&discovery, &options);
    let Some(item) = report.results.into_iter().find(|r| r.runtime == runtime_id) else {
        bail!("browser MCP wire returned no result for {runtime_id}");
    };
    if !item.ok {
        bail!("{}", item.message);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{ProbeCheck, ProbeSeverity, ProbeStatus};
    use crate::repair::SensitivityLevel;

    fn probe_with(checks: Vec<ProbeCheck>) -> RuntimeProbeReport {
        RuntimeProbeReport {
            runtime_id: "codex".into(),
            display_name: "Codex".into(),
            binary_name: "codex".into(),
            checks,
            facts: Vec::new(),
        }
    }

    #[test]
    fn suggests_gateway_and_browser_fixes() {
        let probe = probe_with(vec![
            ProbeCheck::new(
                "mode.overlay_mismatch",
                "drift",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                "mismatch",
                SensitivityLevel::ConfigShape,
            ),
            ProbeCheck::new(
                "mcp.browser.configured",
                "browser",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                "missing",
                SensitivityLevel::ConfigShape,
            ),
        ]);
        let items = suggest_codex_repairs(&probe);
        assert!(items
            .iter()
            .any(|i| i.id == "fix-codex-gateway-from-mode"));
        assert!(items.iter().any(|i| i.id == "fix-codex-browser-mcp"));
    }
}
