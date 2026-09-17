use anyhow::{Context, Result};

use super::backends::{
    bind_claude_code, bind_codex_for_project, bind_hermes, bind_openclaw,
    scaffold_claude_mcp_isolation,
};
use super::claude_mcp::migrate_claude_global_mcp_to_project;
use super::gateway::restart_workspace_gateways;
use super::snapshot::{apply_workspace_snapshot, save_workspace_snapshot};
use super::{
    load_workspaces, save_workspaces, workspace_data_root, workspace_doctor, write_active_env,
    WorkspaceCheckStatus, WorkspaceDoctorReport, WorkspaceEntry,
};

#[derive(Debug, Clone, Default)]
pub struct WorkspaceFixOptions {
    pub dry_run: bool,
    pub restart_gateways: bool,
    pub migrate_claude_mcp: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkspaceFixAction {
    pub id: String,
    pub title: String,
    pub applied: bool,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkspaceFixReport {
    pub active: Option<String>,
    pub actions: Vec<WorkspaceFixAction>,
}

pub fn workspace_fix(options: &WorkspaceFixOptions) -> Result<WorkspaceFixReport> {
    let doc = load_workspaces()?;
    let Some(active_name) = doc.active.clone() else {
        return Ok(WorkspaceFixReport {
            active: None,
            actions: vec![WorkspaceFixAction {
                id: "workspace.active.missing".into(),
                title: "No active workspace".into(),
                applied: false,
                detail: "Run `agent-doctor workspace init` then `workspace use <name>`.".into(),
            }],
        });
    };

    let Some(entry) = doc.workspaces.get(&active_name).cloned() else {
        return Ok(WorkspaceFixReport {
            active: Some(active_name),
            actions: vec![WorkspaceFixAction {
                id: "workspace.active.invalid".into(),
                title: "Active workspace entry missing".into(),
                applied: false,
                detail: "Re-register or pick a valid workspace with `workspace use`.".into(),
            }],
        });
    };

    let doctor = workspace_doctor()?;
    let mut actions = plan_fixes(&active_name, &entry, &doctor, options);

    if options.migrate_claude_mcp {
        let migration = migrate_claude_global_mcp_to_project(&entry.path, options.dry_run)?;
        actions.insert(
            0,
            WorkspaceFixAction {
                id: "workspace.claude.mcp_migration".into(),
                title: "Merge global Claude MCP into project .mcp.json".into(),
                applied: migration.applied,
                detail: format_migration_detail(&migration),
            },
        );
    }

    if !options.dry_run {
        for action in &mut actions {
            if action.applied {
                continue;
            }
            match action.id.as_str() {
                "workspace.claude.mcp_migration" => {}
                "workspace.hermes.profile" => {
                    bind_hermes(&entry.hermes_profile, &entry.path)?;
                    action.applied = true;
                    action.detail = format!("Activated Hermes profile '{}'", entry.hermes_profile);
                }
                "workspace.openclaw.workspace"
                | "workspace.openclaw.routing.default"
                | "workspace.openclaw.routing.defaults_workspace" => {
                    bind_openclaw(&entry.openclaw_agent_id, &entry.openclaw_workspace)?;
                    action.applied = true;
                    action.detail = format!(
                        "Set default agent '{}' and agents.defaults.workspace",
                        entry.openclaw_agent_id
                    );
                }
                "workspace.openclaw.agent_env" => {
                    write_active_env(&active_name, &entry)?;
                    action.applied = true;
                    action.detail = "Refreshed active-workspace.env (OPENCLAW_AGENT_ID)".into();
                }
                "workspace.codex.home"
                | "workspace.codex.global_memory"
                | "workspace.codex.isolation_marker"
                | "workspace.codex.shared_global_home" => {
                    write_active_env(&active_name, &entry)?;
                    bind_codex_for_project(&entry.codex_home, Some(&entry.path))?;
                    action.applied = true;
                    action.detail = format!(
                        "Refreshed isolated CODEX_HOME at {}",
                        entry.codex_home.display()
                    );
                }
                "workspace.claude.project_mcp" | "workspace.claude.global_mcp" => {
                    let data_root = workspace_data_root(&active_name)?;
                    let report = apply_workspace_snapshot(&entry, &data_root)?;
                    bind_claude_code(&entry.path)?;
                    save_workspace_snapshot(&entry, &data_root)?;
                    let hint = scaffold_claude_mcp_isolation(&entry.path)?;
                    action.applied = true;
                    action.detail = if report.mcp_applied {
                        format!(
                            "Restored .mcp.json; wrote migration hint {}",
                            hint.display()
                        )
                    } else {
                        format!("Scaffolded project MCP + hint {}", hint.display())
                    };
                }
                "workspace.hermes.gateway_mismatch" if options.restart_gateways => {
                    let reports = restart_workspace_gateways(&entry);
                    action.applied = reports.iter().any(|report| report.success);
                    action.detail = reports
                        .into_iter()
                        .map(|report| format!("{}: {}", report.runtime_id, report.detail))
                        .collect::<Vec<_>>()
                        .join("; ");
                }
                _ => {}
            }
        }
    }

    Ok(WorkspaceFixReport {
        active: Some(active_name),
        actions,
    })
}

fn plan_fixes(
    _active_name: &str,
    entry: &WorkspaceEntry,
    doctor: &WorkspaceDoctorReport,
    options: &WorkspaceFixOptions,
) -> Vec<WorkspaceFixAction> {
    let mut actions = Vec::new();

    for check in &doctor.checks {
        if check.status == WorkspaceCheckStatus::Pass {
            continue;
        }

        let fixable = matches!(
            check.id.as_str(),
            "workspace.hermes.profile"
                | "workspace.openclaw.workspace"
                | "workspace.openclaw.routing.default"
                | "workspace.openclaw.routing.defaults_workspace"
                | "workspace.openclaw.agent_env"
                | "workspace.codex.home"
                | "workspace.codex.global_memory"
                | "workspace.codex.isolation_marker"
                | "workspace.codex.shared_global_home"
                | "workspace.claude.project_mcp"
                | "workspace.claude.global_mcp"
                | "workspace.hermes.gateway_mismatch"
        );

        if !fixable {
            continue;
        }

        actions.push(WorkspaceFixAction {
            id: check.id.clone(),
            title: check.title.clone(),
            applied: false,
            detail: if check.id == "workspace.claude.project_mcp"
                || check.id == "workspace.claude.global_mcp"
            {
                "Will restore/scaffold .mcp.json and write .agent-doctor/claude-mcp-isolation.md"
                    .into()
            } else if check.id == "workspace.hermes.gateway_mismatch" {
                "Will attempt Hermes/OpenClaw gateway restart (pass --restart-gateways)".into()
            } else {
                check.detail.clone()
            },
        });
    }

    if actions.is_empty() && !options.migrate_claude_mcp {
        actions.push(WorkspaceFixAction {
            id: "workspace.fix.nothing".into(),
            title: "No auto-fixable issues".into(),
            applied: false,
            detail: "Run `workspace doctor` for manual hints (cwd mismatch, gateway restart, global MCP)."
                .into(),
        });
    }

    let _ = entry;
    actions
}

fn format_migration_detail(migration: &super::claude_mcp::ClaudeMcpMigrationReport) -> String {
    let mode = if migration.dry_run {
        "preview"
    } else if migration.applied {
        "applied"
    } else {
        "no-op"
    };
    format!(
        "{mode}: sources=[{}]; add={:?}; skip={:?}; conflicts={:?}; wrote={}; global servers NOT removed — review before deleting ~/.claude/settings.json mcpServers",
        migration.global_sources.join(", "),
        migration.added_servers,
        migration.skipped_servers,
        migration.conflict_servers,
        migration.project_mcp_path.display(),
    )
}

pub fn remove_workspace(name: &str, purge_data: bool) -> Result<()> {
    let mut doc = load_workspaces()?;
    if !doc.workspaces.contains_key(name) {
        anyhow::bail!("workspace '{name}' not found");
    }

    doc.workspaces.remove(name);
    if doc.active.as_deref() == Some(name) {
        doc.active = doc.workspaces.keys().next().cloned();
    }
    save_workspaces(&doc)?;

    if purge_data {
        let data_root = workspace_data_root(name)?;
        if data_root.exists() {
            std::fs::remove_dir_all(&data_root)
                .with_context(|| format!("purge {}", data_root.display()))?;
        }
    }

    Ok(())
}
