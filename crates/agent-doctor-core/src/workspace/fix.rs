use anyhow::{Context, Result};

use super::claude_mcp::migrate_claude_global_mcp_to_project;
use super::repairs::WorkspaceRepairInput;
use super::{
    load_workspaces, save_workspaces, workspace_data_root, workspace_doctor, WorkspaceCheckStatus,
    WorkspaceDoctorReport,
};
use crate::runtime::find_workspace_repair;

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
    let mut actions = plan_fixes(&doctor, options);

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
        let repair_input = WorkspaceRepairInput {
            active_name: &active_name,
            entry: &entry,
            restart_gateways: options.restart_gateways,
        };
        for action in &mut actions {
            if action.applied {
                continue;
            }
            let Some(repair) = find_workspace_repair(&action.id) else {
                continue;
            };
            let Some(outcome) = repair.run(&repair_input)? else {
                continue;
            };
            action.applied = outcome.applied;
            action.detail = outcome.detail;
        }
    }

    Ok(WorkspaceFixReport {
        active: Some(active_name),
        actions,
    })
}

fn plan_fixes(
    doctor: &WorkspaceDoctorReport,
    options: &WorkspaceFixOptions,
) -> Vec<WorkspaceFixAction> {
    let mut actions = Vec::new();

    for check in &doctor.checks {
        if check.status == WorkspaceCheckStatus::Pass {
            continue;
        }

        let Some(repair) = find_workspace_repair(&check.id) else {
            continue;
        };

        actions.push(WorkspaceFixAction {
            id: check.id.clone(),
            title: check.title.clone(),
            applied: false,
            detail: repair
                .preview
                .map(str::to_string)
                .unwrap_or_else(|| check.detail.clone()),
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
