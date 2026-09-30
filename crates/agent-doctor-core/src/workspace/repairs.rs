//! Per-runtime workspace repairs. The descriptor lists which checks call these.

use anyhow::Result;

use super::backends::{
    bind_claude_code, bind_codex_for_project, bind_hermes, bind_openclaw,
    scaffold_claude_mcp_isolation,
};
use super::gateway::restart_workspace_gateways;
use super::snapshot::{apply_workspace_snapshot, save_workspace_snapshot};
use super::{workspace_data_root, write_active_env, WorkspaceEntry};

pub struct WorkspaceRepairInput<'a> {
    pub active_name: &'a str,
    pub entry: &'a WorkspaceEntry,
    pub restart_gateways: bool,
}

pub struct WorkspaceRepairOutcome {
    pub applied: bool,
    pub detail: String,
}

/// `None` means the check was left for later (the planned detail stays).
pub type WorkspaceRepairFn =
    for<'a> fn(&WorkspaceRepairInput<'a>) -> Result<Option<WorkspaceRepairOutcome>>;

pub(crate) fn repair_openclaw_workspace(
    input: &WorkspaceRepairInput,
) -> Result<Option<WorkspaceRepairOutcome>> {
    bind_openclaw(
        input.entry.openclaw_agent_id.as_str(),
        &input.entry.openclaw_workspace,
    )?;
    Ok(Some(WorkspaceRepairOutcome {
        applied: true,
        detail: format!(
            "Set default agent '{}' and agents.defaults.workspace",
            input.entry.openclaw_agent_id
        ),
    }))
}

pub(crate) fn repair_openclaw_agent_env(
    input: &WorkspaceRepairInput,
) -> Result<Option<WorkspaceRepairOutcome>> {
    write_active_env(input.active_name, input.entry)?;
    Ok(Some(WorkspaceRepairOutcome {
        applied: true,
        detail: "Refreshed active-workspace.env (OPENCLAW_AGENT_ID)".into(),
    }))
}

pub(crate) fn repair_hermes_profile(
    input: &WorkspaceRepairInput,
) -> Result<Option<WorkspaceRepairOutcome>> {
    bind_hermes(&input.entry.hermes_profile, &input.entry.path)?;
    Ok(Some(WorkspaceRepairOutcome {
        applied: true,
        detail: format!("Activated Hermes profile '{}'", input.entry.hermes_profile),
    }))
}

pub(crate) fn repair_hermes_gateway(
    input: &WorkspaceRepairInput,
) -> Result<Option<WorkspaceRepairOutcome>> {
    if !input.restart_gateways {
        return Ok(None);
    }
    let reports = restart_workspace_gateways(input.entry);
    Ok(Some(WorkspaceRepairOutcome {
        applied: reports.iter().any(|report| report.success),
        detail: reports
            .into_iter()
            .map(|report| format!("{}: {}", report.runtime_id, report.detail))
            .collect::<Vec<_>>()
            .join("; "),
    }))
}

pub(crate) fn repair_codex_home(
    input: &WorkspaceRepairInput,
) -> Result<Option<WorkspaceRepairOutcome>> {
    write_active_env(input.active_name, input.entry)?;
    bind_codex_for_project(&input.entry.codex_home, Some(&input.entry.path))?;
    Ok(Some(WorkspaceRepairOutcome {
        applied: true,
        detail: format!(
            "Refreshed isolated CODEX_HOME at {}",
            input.entry.codex_home.display()
        ),
    }))
}

pub(crate) fn repair_claude_project_mcp(
    input: &WorkspaceRepairInput,
) -> Result<Option<WorkspaceRepairOutcome>> {
    let data_root = workspace_data_root(input.active_name)?;
    let report = apply_workspace_snapshot(input.entry, &data_root)?;
    bind_claude_code(&input.entry.path)?;
    save_workspace_snapshot(input.entry, &data_root)?;
    let hint = scaffold_claude_mcp_isolation(&input.entry.path)?;
    let detail = if report.mcp_applied {
        format!(
            "Restored .mcp.json; wrote migration hint {}",
            hint.display()
        )
    } else {
        format!("Scaffolded project MCP + hint {}", hint.display())
    };
    Ok(Some(WorkspaceRepairOutcome {
        applied: true,
        detail,
    }))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn entry() -> WorkspaceEntry {
        WorkspaceEntry {
            path: PathBuf::from("/tmp/agent-doctor-repair-test"),
            hermes_profile: "demo".into(),
            codex_home: PathBuf::from("/tmp/agent-doctor-repair-test/codex"),
            openclaw_agent_id: "demo".into(),
            openclaw_workspace: PathBuf::from("/tmp/agent-doctor-repair-test/openclaw"),
        }
    }

    #[test]
    fn hermes_gateway_repair_waits_until_restart_is_requested() {
        let workspace = entry();
        let skipped = repair_hermes_gateway(&WorkspaceRepairInput {
            active_name: "demo",
            entry: &workspace,
            restart_gateways: false,
        })
        .unwrap();
        assert!(skipped.is_none());
    }
}
