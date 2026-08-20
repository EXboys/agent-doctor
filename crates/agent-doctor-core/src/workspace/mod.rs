use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::adapters::util::home_join;

use self::backends::{
    bind_claude_code, bind_codex, bind_hermes, bind_openclaw, claude_mcp_summary_for_project,
    codex_home_from_env, hermes_active_profile, hermes_gateway_profiles, openclaw_agent_workspace,
    workspace_paths_match, RuntimeBindReport,
};
use self::path::{
    cwd, default_workspace_name, paths_equal, resolve_project_path, sanitize_workspace_name,
};

pub mod backends;
pub mod backup;
pub mod baseline;
pub mod claude_mcp;
pub mod fix;
pub mod gateway;
pub mod hook_status;
pub mod matrix;
pub mod mcp_inventory;
pub mod path;
pub mod paths_check;
pub mod shell;
pub mod snapshot;

const WORKSPACES_FILE: &str = "workspaces.yaml";
const ACTIVE_ENV_FILE: &str = "active-workspace.env";
pub const DEFAULT_WORKSPACE_NAME: &str = "default";

static ENSURE_DEFAULT: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspacesDocument {
    #[serde(default)]
    pub active: Option<String>,
    #[serde(default)]
    pub workspaces: BTreeMap<String, WorkspaceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub path: PathBuf,
    #[serde(default)]
    pub hermes_profile: String,
    pub codex_home: PathBuf,
    #[serde(default)]
    pub openclaw_agent_id: String,
    pub openclaw_workspace: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitWorkspaceReport {
    pub name: String,
    pub path: PathBuf,
    pub config_path: PathBuf,
    pub bindings: Vec<RuntimeBindReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UseWorkspaceReport {
    pub name: String,
    pub path: PathBuf,
    pub env_file: PathBuf,
    pub bindings: Vec<RuntimeBindReport>,
    pub backup_id: Option<String>,
    #[serde(default)]
    pub gateway_restarts: Vec<gateway::GatewayRestartReport>,
}

#[derive(Debug, Clone, Default)]
pub struct UseWorkspaceOptions {
    pub backup: bool,
    pub restart_gateways: bool,
}

pub use claude_mcp::{migrate_claude_global_mcp_to_project, ClaudeMcpMigrationReport};
pub use fix::{
    remove_workspace, workspace_fix, WorkspaceFixAction, WorkspaceFixOptions, WorkspaceFixReport,
};
pub use gateway::{gateway_restart_hint, restart_workspace_gateways, GatewayRestartReport};
pub use hook_status::{workspace_hook_status, ShellHookStatus};
pub use matrix::{workspace_capability_matrix, CapabilityCell, CapabilityMatrix};
pub use mcp_inventory::{
    browser_configured_runtimes, list_mcp_inventory, list_mcp_inventory_with_doc,
    probe_browser_mcp_for_runtime, resolve_agent_doctor_binary, McpInventoryItem,
    McpInventoryReport,
};
pub use paths_check::{scan_workspace_path_references, PathReferenceIssue};
pub use shell::{
    bash_hook_file_path, enter_workspace, fish_hook_file_path, hook_file_path, install_bash_hook,
    install_fish_hook, install_powershell_hook, install_zsh_hook, match_workspace_for_path,
    powershell_hook_file_path, render_direnv_envrc, render_shell_env, render_shell_env_for_name,
    write_direnv_envrc, EnterWorkspaceReport,
};
pub use snapshot::{
    apply_workspace_snapshot, save_workspace_snapshot, snapshot_dir, SnapshotReport,
};

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceSnapshotStatus {
    pub mcp_snapshot: bool,
    pub skills_snapshot: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceShowReport {
    pub name: String,
    pub active: bool,
    pub entry: WorkspaceEntry,
    pub data_root: PathBuf,
    pub env_file: PathBuf,
    pub snapshot: WorkspaceSnapshotStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceStatusReport {
    pub active: Option<String>,
    pub cwd: PathBuf,
    pub matched_workspace: Option<String>,
    pub runtimes: Vec<RuntimeStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStatus {
    pub runtime_id: &'static str,
    pub isolation_tier: &'static str,
    pub expected: String,
    pub actual: String,
    pub aligned: bool,
    pub hint: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceDoctorReport {
    pub active: Option<String>,
    pub checks: Vec<WorkspaceCheck>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceCheck {
    pub id: String,
    pub title: String,
    pub status: WorkspaceCheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceCheckStatus {
    Pass,
    Warn,
    Fail,
}

pub fn workspaces_path() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|dir| dir.join("agent-doctor").join(WORKSPACES_FILE))
        .context("could not resolve config directory")
}

pub fn active_env_path() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|dir| dir.join("agent-doctor").join(ACTIVE_ENV_FILE))
        .context("could not resolve config directory")
}

pub fn workspace_data_root(name: &str) -> Result<PathBuf> {
    dirs::config_dir()
        .map(|dir| dir.join("agent-doctor").join("workspaces").join(name))
        .context("could not resolve config directory")
}

pub fn load_workspaces() -> Result<WorkspacesDocument> {
    let path = workspaces_path()?;
    if !path.exists() {
        return Ok(WorkspacesDocument::default());
    }
    let raw = fs::read_to_string(&path)?;
    serde_yaml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
}

fn default_workspace_project_path() -> PathBuf {
    dirs::home_dir()
        .or_else(dirs::document_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Create and activate a `default` workspace (user home) when the registry is empty,
/// or activate an existing entry when `active` is missing. Idempotent.
pub fn ensure_default_workspace() -> Result<WorkspacesDocument> {
    let _guard = ENSURE_DEFAULT
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());

    let doc = load_workspaces()?;
    if let Some(active) = doc.active.as_deref() {
        if doc.workspaces.contains_key(active) {
            return Ok(doc);
        }
    }

    if let Some(name) = doc.workspaces.keys().next().cloned() {
        use_workspace_with_options(
            &name,
            &UseWorkspaceOptions {
                backup: false,
                restart_gateways: false,
            },
        )?;
        return load_workspaces();
    }

    let project_path = default_workspace_project_path();
    let report = init_workspace(
        Some(project_path),
        Some(DEFAULT_WORKSPACE_NAME.to_string()),
        false,
    )?;
    use_workspace_with_options(
        &report.name,
        &UseWorkspaceOptions {
            backup: false,
            restart_gateways: false,
        },
    )?;
    load_workspaces()
}

pub fn save_workspaces(doc: &WorkspacesDocument) -> Result<PathBuf> {
    let path = workspaces_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_yaml::to_string(doc)?)?;
    Ok(path)
}

pub fn init_workspace(
    path: Option<PathBuf>,
    name: Option<String>,
    prefer_git_root: bool,
) -> Result<InitWorkspaceReport> {
    let project_path = resolve_project_path(path, prefer_git_root)?;
    let mut doc = load_workspaces()?;

    if let Some((existing_name, _)) = doc
        .workspaces
        .iter()
        .find(|(_, entry)| paths_equal(&entry.path, &project_path))
    {
        bail!(
            "project already registered as workspace '{existing_name}' ({})",
            project_path.display()
        );
    }

    let base_name = name
        .as_deref()
        .map(sanitize_workspace_name)
        .unwrap_or_else(|| default_workspace_name(&project_path));
    let workspace_name = unique_workspace_name(&base_name, &doc.workspaces);

    let data_root = workspace_data_root(&workspace_name)?;
    fs::create_dir_all(&data_root)?;

    let hermes_profile = workspace_name.clone();
    let codex_home = data_root.join("codex-home");
    let openclaw_agent_id = workspace_name.clone();
    let openclaw_workspace = data_root.join("openclaw-workspace");

    let bindings = vec![
        bind_hermes(&hermes_profile, &project_path)?,
        bind_claude_code(&project_path)?,
        bind_codex(&codex_home)?,
        bind_openclaw(&openclaw_agent_id, &openclaw_workspace)?,
    ];

    let snapshot = save_workspace_snapshot(
        &WorkspaceEntry {
            path: project_path.clone(),
            hermes_profile: hermes_profile.clone(),
            codex_home: codex_home.clone(),
            openclaw_agent_id: openclaw_agent_id.clone(),
            openclaw_workspace: openclaw_workspace.clone(),
        },
        &data_root,
    )?;

    doc.workspaces.insert(
        workspace_name.clone(),
        WorkspaceEntry {
            path: project_path.clone(),
            hermes_profile,
            codex_home,
            openclaw_agent_id,
            openclaw_workspace,
        },
    );
    if doc.active.is_none() {
        doc.active = Some(workspace_name.clone());
    }
    let config_path = save_workspaces(&doc)?;

    let mut init_bindings = bindings;
    if snapshot.mcp_saved || snapshot.skills_saved {
        init_bindings.push(RuntimeBindReport {
            runtime_id: "snapshot",
            action: "save MCP/skills snapshot".to_string(),
            detail: format!(
                "mcp={} skills={} dir={}",
                snapshot.mcp_saved,
                snapshot.skills_saved,
                snapshot::snapshot_dir(&data_root).display()
            ),
            isolation_tier: "L3 (project-scoped MCP/skills)",
        });
    }

    Ok(InitWorkspaceReport {
        name: workspace_name,
        path: project_path,
        config_path,
        bindings: init_bindings,
    })
}

pub fn use_workspace(name: &str) -> Result<UseWorkspaceReport> {
    use_workspace_with_options(
        name,
        &UseWorkspaceOptions {
            backup: true,
            restart_gateways: false,
        },
    )
}

pub fn use_workspace_with_options(
    name: &str,
    options: &UseWorkspaceOptions,
) -> Result<UseWorkspaceReport> {
    let mut doc = load_workspaces()?;
    let entry = doc
        .workspaces
        .get(name)
        .with_context(|| format!("workspace '{name}' not found"))?
        .clone();

    let backup_id = if options.backup {
        Some(
            backup::create_workspace_switch_backup(name, &entry)?
                .id
                .clone(),
        )
    } else {
        None
    };

    doc.active = Some(name.to_string());
    save_workspaces(&doc)?;

    let data_root = workspace_data_root(name)?;
    let bindings = vec![
        bind_hermes(&entry.hermes_profile, &entry.path)?,
        bind_claude_code(&entry.path)?,
        bind_codex(&entry.codex_home)?,
        bind_openclaw(&entry.openclaw_agent_id, &entry.openclaw_workspace)?,
    ];

    let snapshot = apply_workspace_snapshot(&entry, &data_root)?;
    save_workspace_snapshot(&entry, &data_root)?;

    let env_file = write_active_env(name, &entry)?;

    let gateway_restarts = if options.restart_gateways {
        restart_workspace_gateways(&entry)
    } else {
        Vec::new()
    };

    let mut use_bindings = bindings;
    if snapshot.mcp_applied || snapshot.skills_applied {
        use_bindings.push(RuntimeBindReport {
            runtime_id: "snapshot",
            action: "apply MCP/skills snapshot".to_string(),
            detail: format!(
                "mcp={} skills={}",
                snapshot.mcp_applied, snapshot.skills_applied
            ),
            isolation_tier: "L3 (project-scoped MCP/skills)",
        });
    }

    Ok(UseWorkspaceReport {
        name: name.to_string(),
        path: entry.path,
        env_file,
        bindings: use_bindings,
        backup_id,
        gateway_restarts,
    })
}

pub fn workspace_status(at: Option<PathBuf>) -> Result<WorkspaceStatusReport> {
    let doc = load_workspaces()?;
    let current = at.map_or_else(cwd, |path| resolve_project_path(Some(path), false))?;
    let matched = doc
        .workspaces
        .iter()
        .find(|(_, entry)| paths_equal(&entry.path, &current))
        .map(|(name, _)| name.clone());

    let active_name = doc.active.clone();
    let active_entry = active_name
        .as_ref()
        .and_then(|name| doc.workspaces.get(name));

    let mut runtimes = Vec::new();
    if let Some(entry) = active_entry {
        runtimes.push(hermes_runtime_status(entry));
        runtimes.push(claude_runtime_status(entry, &current));
        runtimes.push(codex_runtime_status(entry));
        runtimes.push(openclaw_runtime_status(entry));
    }

    Ok(WorkspaceStatusReport {
        active: active_name,
        cwd: current,
        matched_workspace: matched,
        runtimes,
    })
}

pub fn workspace_show(name: &str) -> Result<WorkspaceShowReport> {
    let doc = load_workspaces()?;
    let entry = doc
        .workspaces
        .get(name)
        .with_context(|| format!("workspace '{name}' not found"))?
        .clone();
    let data_root = workspace_data_root(name)?;
    let snap_dir = snapshot_dir(&data_root);
    Ok(WorkspaceShowReport {
        name: name.to_string(),
        active: doc.active.as_deref() == Some(name),
        entry,
        data_root,
        env_file: active_env_path()?,
        snapshot: WorkspaceSnapshotStatus {
            mcp_snapshot: snap_dir.join("mcp.json").exists(),
            skills_snapshot: snap_dir.join("skills").is_dir(),
        },
    })
}

pub fn workspace_doctor() -> Result<WorkspaceDoctorReport> {
    let doc = load_workspaces()?;
    let current = cwd()?;
    let mut checks = Vec::new();

    let Some(active_name) = doc.active.clone() else {
        checks.push(WorkspaceCheck {
            id: "workspace.active.missing".to_string(),
            title: "No active workspace".to_string(),
            status: WorkspaceCheckStatus::Warn,
            detail: "Run `agent-doctor workspace init` then `workspace use <name>`.".to_string(),
        });
        return Ok(WorkspaceDoctorReport {
            active: None,
            checks,
        });
    };

    let Some(entry) = doc.workspaces.get(&active_name).cloned() else {
        checks.push(WorkspaceCheck {
            id: "workspace.active.invalid".to_string(),
            title: "Active workspace entry missing".to_string(),
            status: WorkspaceCheckStatus::Fail,
            detail: format!("Active workspace '{active_name}' is not in workspaces.yaml"),
        });
        return Ok(WorkspaceDoctorReport {
            active: Some(active_name),
            checks,
        });
    };

    if !paths_equal(&current, &entry.path) && !current.starts_with(&entry.path) {
        checks.push(WorkspaceCheck {
            id: "workspace.cwd.mismatch".to_string(),
            title: "Shell cwd differs from active workspace".to_string(),
            status: WorkspaceCheckStatus::Warn,
            detail: format!("cwd={} active={}", current.display(), entry.path.display()),
        });
    } else {
        checks.push(WorkspaceCheck {
            id: "workspace.cwd.mismatch".to_string(),
            title: "Shell cwd matches active workspace".to_string(),
            status: WorkspaceCheckStatus::Pass,
            detail: entry.path.display().to_string(),
        });
    }

    let hermes_status = hermes_runtime_status(&entry);
    checks.push(check_from_runtime(
        "workspace.hermes.profile",
        "Hermes profile alignment",
        &hermes_status,
    ));

    let codex_status = codex_runtime_status(&entry);
    checks.push(check_from_runtime(
        "workspace.codex.home",
        "Codex CODEX_HOME alignment",
        &codex_status,
    ));

    let openclaw_status = openclaw_runtime_status(&entry);
    checks.push(check_from_runtime(
        "workspace.openclaw.workspace",
        "OpenClaw agent workspace alignment",
        &openclaw_status,
    ));

    checks.extend(claude_doctor_checks(&entry));
    checks.extend(baseline::openclaw_routing_checks(&entry));
    checks.extend(baseline::baseline_drift_checks(&entry));
    checks.extend(baseline::codex_isolation_checks(&entry));

    let gateways = hermes_gateway_profiles();
    if !gateways.is_empty() {
        if !gateways
            .iter()
            .any(|profile| profile == &entry.hermes_profile)
        {
            checks.push(WorkspaceCheck {
                id: "workspace.hermes.gateway_mismatch".to_string(),
                title: "Hermes gateway running under a different profile".to_string(),
                status: WorkspaceCheckStatus::Warn,
                detail: format!(
                    "expected profile '{}' but gateway lock found for: {}",
                    entry.hermes_profile,
                    gateways.join(", ")
                ),
            });
        } else if !gateways.is_empty() {
            checks.push(WorkspaceCheck {
                id: "workspace.hermes.gateway_mismatch".to_string(),
                title: "Hermes gateway profile matches workspace".to_string(),
                status: WorkspaceCheckStatus::Pass,
                detail: entry.hermes_profile.clone(),
            });
        }
    }

    if !workspace_paths_match(&entry.codex_home, &codex_home_from_env()) {
        let actual = codex_home_from_env();
        let detail = if std::env::var_os("CODEX_HOME").is_none() {
            format!(
                "This process has no CODEX_HOME (falls back to {}). \
                 Launch Codex via Agents → Open (loads workspace env), \
                 or: source active-workspace.env then run codex. \
                 expected={}",
                actual.display(),
                entry.codex_home.display()
            )
        } else {
            format!(
                "expected={} actual={} — source active-workspace.env before launching Codex",
                entry.codex_home.display(),
                actual.display()
            )
        };
        checks.push(WorkspaceCheck {
            id: "workspace.codex.global_memory".to_string(),
            title: "Codex not using workspace CODEX_HOME".to_string(),
            status: WorkspaceCheckStatus::Warn,
            detail,
        });
    }

    let openclaw_env = std::env::var("OPENCLAW_AGENT_ID").unwrap_or_default();
    if !openclaw_env.is_empty() && openclaw_env != entry.openclaw_agent_id {
        checks.push(WorkspaceCheck {
            id: "workspace.openclaw.agent_env".to_string(),
            title: "OPENCLAW_AGENT_ID differs from workspace".to_string(),
            status: WorkspaceCheckStatus::Warn,
            detail: format!(
                "expected='{}' actual='{openclaw_env}' — source active-workspace.env",
                entry.openclaw_agent_id
            ),
        });
    }

    let path_issues = scan_workspace_path_references(&entry);
    if path_issues.is_empty() {
        checks.push(WorkspaceCheck {
            id: "workspace.paths.references".to_string(),
            title: "MCP/skills path references look healthy".to_string(),
            status: WorkspaceCheckStatus::Pass,
            detail: "No missing obvious MCP/skills/workspace path references".to_string(),
        });
    } else {
        let sample = path_issues
            .iter()
            .take(3)
            .map(|issue| format!("{} → {}", issue.source, issue.path))
            .collect::<Vec<_>>()
            .join("; ");
        checks.push(WorkspaceCheck {
            id: "workspace.paths.references".to_string(),
            title: "Missing MCP/skills path references".to_string(),
            status: WorkspaceCheckStatus::Warn,
            detail: format!(
                "{} missing path(s): {}{}",
                path_issues.len(),
                sample,
                if path_issues.len() > 3 { " …" } else { "" }
            ),
        });
    }

    Ok(WorkspaceDoctorReport {
        active: Some(active_name),
        checks,
    })
}

fn claude_doctor_checks(entry: &WorkspaceEntry) -> Vec<WorkspaceCheck> {
    let summary = claude_mcp_summary_for_project(&entry.path);
    let project_mcp = entry.path.join(".mcp.json").exists();
    let mut checks = Vec::new();

    let global_mcp_total = summary.user_scope_servers + summary.claude_json_servers;
    if global_mcp_total > 0 {
        let mut sources = Vec::new();
        if summary.user_scope_servers > 0 {
            sources.push(format!(
                "{} in ~/.claude/settings.json",
                summary.user_scope_servers
            ));
        }
        if summary.claude_json_servers > 0 {
            sources.push(format!(
                "{} in ~/.claude.json (global)",
                summary.claude_json_servers
            ));
        }
        checks.push(WorkspaceCheck {
            id: "workspace.claude.global_mcp".to_string(),
            title: "Claude Code user-scoped MCP servers detected".to_string(),
            status: WorkspaceCheckStatus::Warn,
            detail: format!(
                "{} user-scoped MCP server(s) ({}) may apply across projects — prefer project .mcp.json",
                global_mcp_total,
                sources.join(", ")
            ),
        });
    } else {
        checks.push(WorkspaceCheck {
            id: "workspace.claude.global_mcp".to_string(),
            title: "No Claude Code user-scoped MCP bleed detected".to_string(),
            status: WorkspaceCheckStatus::Pass,
            detail: "No global mcpServers in ~/.claude/settings.json or ~/.claude.json".to_string(),
        });
    }

    if summary.claude_json_project_servers > 0 {
        checks.push(WorkspaceCheck {
            id: "workspace.claude.project_json_mcp".to_string(),
            title: "Claude Code project entry in ~/.claude.json".to_string(),
            status: WorkspaceCheckStatus::Pass,
            detail: format!(
                "{} project-scoped MCP server(s) under ~/.claude.json projects[{}]",
                summary.claude_json_project_servers,
                entry.path.display()
            ),
        });
    }

    if summary.project_mcp_file_servers > 0 {
        checks.push(WorkspaceCheck {
            id: "workspace.claude.project_mcp".to_string(),
            title: "Project .mcp.json configured".to_string(),
            status: WorkspaceCheckStatus::Pass,
            detail: format!(
                "{} MCP server(s) in {}",
                summary.project_mcp_file_servers,
                entry.path.join(".mcp.json").display()
            ),
        });
    } else if !project_mcp {
        checks.push(WorkspaceCheck {
            id: "workspace.claude.project_mcp".to_string(),
            title: "Project .mcp.json not present".to_string(),
            status: WorkspaceCheckStatus::Warn,
            detail: format!(
                "Consider adding {} for project-scoped MCP",
                entry.path.join(".mcp.json").display()
            ),
        });
    }

    checks
}

fn check_from_runtime(id: &str, title: &str, status: &RuntimeStatus) -> WorkspaceCheck {
    WorkspaceCheck {
        id: id.to_string(),
        title: title.to_string(),
        status: if status.aligned {
            WorkspaceCheckStatus::Pass
        } else {
            WorkspaceCheckStatus::Warn
        },
        detail: if status.aligned {
            status.expected.clone()
        } else {
            format!(
                "expected={} actual={} — {}",
                status.expected, status.actual, status.hint
            )
        },
    }
}

fn hermes_runtime_status(entry: &WorkspaceEntry) -> RuntimeStatus {
    let expected = entry.hermes_profile.clone();
    let actual = hermes_active_profile().unwrap_or_else(|| "(unknown)".to_string());
    let aligned = actual == expected;
    RuntimeStatus {
        runtime_id: "hermes",
        isolation_tier: "L3",
        expected: expected.clone(),
        actual,
        aligned,
        hint: "Run `agent-doctor workspace use <name>` or `hermes profile use <profile>`".into(),
    }
}

fn claude_runtime_status(entry: &WorkspaceEntry, current: &Path) -> RuntimeStatus {
    let aligned = paths_equal(current, &entry.path) || current.starts_with(&entry.path);
    RuntimeStatus {
        runtime_id: "claude-code",
        isolation_tier: "L3",
        expected: entry.path.display().to_string(),
        actual: current.display().to_string(),
        aligned,
        hint: "Start Claude Code from the workspace project directory".into(),
    }
}

fn codex_runtime_status(entry: &WorkspaceEntry) -> RuntimeStatus {
    let expected = entry.codex_home.display().to_string();
    let actual = codex_home_from_env().display().to_string();
    let aligned = workspace_paths_match(&entry.codex_home, &codex_home_from_env());
    let hint = if std::env::var_os("CODEX_HOME").is_none() {
        "Desktop Check has no CODEX_HOME; use Agents → Open for Codex (or source active-workspace.env in the terminal)"
            .into()
    } else {
        "Source active-workspace.env before running codex".into()
    };
    RuntimeStatus {
        runtime_id: "codex",
        isolation_tier: "L2",
        expected,
        actual,
        aligned,
        hint,
    }
}

fn openclaw_runtime_status(entry: &WorkspaceEntry) -> RuntimeStatus {
    let expected = entry.openclaw_workspace.display().to_string();
    let actual = openclaw_agent_workspace(&entry.openclaw_agent_id)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "(not configured)".to_string());
    let aligned = openclaw_agent_workspace(&entry.openclaw_agent_id)
        .map(|path| workspace_paths_match(&entry.openclaw_workspace, &path))
        .unwrap_or(false);
    RuntimeStatus {
        runtime_id: "openclaw",
        isolation_tier: "L2",
        expected,
        actual,
        aligned,
        hint: "Run `agent-doctor workspace use <name>` to set default agent + agents.defaults.workspace"
            .into(),
    }
}

pub(crate) fn write_active_env(name: &str, entry: &WorkspaceEntry) -> Result<PathBuf> {
    let path = active_env_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let profile_home = home_join(".hermes/profiles").join(&entry.hermes_profile);
    let mut file = fs::File::create(&path).context("create active-workspace.env")?;
    writeln!(file, "# Agent Doctor active workspace: {name}")?;
    writeln!(
        file,
        "# Usage: set -a && source \"{}\" && set +a",
        path.display()
    )?;
    // Quote values: macOS config paths contain spaces (`Application Support`).
    writeln!(file, "AGENT_DOCTOR_WORKSPACE={}", shell_quote_env(name))?;
    writeln!(
        file,
        "AGENT_DOCTOR_PROJECT_ROOT={}",
        shell_quote_env(&entry.path.display().to_string())
    )?;
    writeln!(
        file,
        "HERMES_HOME={}",
        shell_quote_env(&profile_home.display().to_string())
    )?;
    writeln!(
        file,
        "CODEX_HOME={}",
        shell_quote_env(&entry.codex_home.display().to_string())
    )?;
    writeln!(
        file,
        "OPENCLAW_AGENT_ID={}",
        shell_quote_env(&entry.openclaw_agent_id)
    )?;
    writeln!(
        file,
        "OPENCLAW_WORKSPACE={}",
        shell_quote_env(&entry.openclaw_workspace.display().to_string())
    )?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(path)
}

/// POSIX-safe single-quoted value for `KEY=value` lines in sourced env files.
fn shell_quote_env(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.' | ':' | '@' | '+'))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn unique_workspace_name(base: &str, workspaces: &BTreeMap<String, WorkspaceEntry>) -> String {
    if !workspaces.contains_key(base) {
        return base.to_string();
    }
    for index in 2..1000 {
        let candidate = format!("{base}-{index}");
        if !workspaces.contains_key(&candidate) {
            return candidate;
        }
    }
    format!("{base}-{}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unique_workspace_name_appends_suffix() {
        let mut workspaces = BTreeMap::new();
        workspaces.insert(
            "foo".to_string(),
            WorkspaceEntry {
                path: PathBuf::from("/tmp/foo"),
                hermes_profile: "foo".into(),
                codex_home: PathBuf::from("/tmp/foo/codex"),
                openclaw_agent_id: "foo".into(),
                openclaw_workspace: PathBuf::from("/tmp/foo/openclaw"),
            },
        );
        assert_eq!(unique_workspace_name("foo", &workspaces), "foo-2");
    }

    #[test]
    fn shell_quote_env_handles_application_support_spaces() {
        assert_eq!(shell_quote_env("agent-doctor"), "agent-doctor");
        assert_eq!(
            shell_quote_env(
                "/Users/airlu/Library/Application Support/agent-doctor/workspaces/x/codex-home"
            ),
            "'/Users/airlu/Library/Application Support/agent-doctor/workspaces/x/codex-home'"
        );
        assert_eq!(shell_quote_env("a'b"), "'a'\\''b'");
    }
}
