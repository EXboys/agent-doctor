use agent_doctor_core::{
    ensure_default_workspace, enter_workspace, init_workspace, install_bash_hook,
    install_fish_hook, install_powershell_hook, install_zsh_hook, load_workspaces,
    match_workspace_for_path, remove_workspace, render_direnv_envrc, render_shell_env_for_name,
    use_workspace_with_options, workspace_capability_matrix, workspace_doctor, workspace_fix,
    workspace_hook_status, workspace_show, workspace_status, write_direnv_envrc,
    UseWorkspaceOptions, WorkspaceCheckStatus, WorkspaceFixOptions, WorkspacesDocument,
};
use anyhow::{bail, Result};

pub fn init(path: Option<std::path::PathBuf>, name: Option<String>, git_root: bool) -> Result<()> {
    let report = init_workspace(path, name, git_root)?;
    println!("Created workspace: {}", report.name);
    println!("  project: {}", report.path.display());
    println!("  config:  {}", report.config_path.display());
    println!();
    for binding in &report.bindings {
        println!(
            "✓ {} — {} ({})",
            binding.runtime_id, binding.action, binding.isolation_tier
        );
        println!("    {}", binding.detail);
    }
    println!();
    println!("Next:");
    println!("  agent-doctor workspace enter");
    Ok(())
}

pub fn list(json: bool) -> Result<()> {
    let doc = ensure_default_workspace().or_else(|_| load_workspaces())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&doc)?);
        return Ok(());
    }
    print_document(&doc);
    Ok(())
}

pub fn show(name: &str, json: bool) -> Result<()> {
    let report = workspace_show(name)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!(
        "Workspace: {} {}",
        report.name,
        if report.active { "(active)" } else { "" }
    );
    println!("  project:  {}", report.entry.path.display());
    println!("  hermes:   {}", report.entry.hermes_profile);
    println!("  codex:    {}", report.entry.codex_home.display());
    println!(
        "  openclaw: {} → {}",
        report.entry.openclaw_agent_id,
        report.entry.openclaw_workspace.display()
    );
    println!("  data:     {}", report.data_root.display());
    println!(
        "  snapshot: mcp={} skills={}",
        report.snapshot.mcp_snapshot, report.snapshot.skills_snapshot
    );
    Ok(())
}

pub fn matrix(json: bool) -> Result<()> {
    let matrix = workspace_capability_matrix();
    if json {
        println!("{}", serde_json::to_string_pretty(&matrix)?);
        return Ok(());
    }

    println!(
        "Agent Doctor workspace capability matrix ({})\n",
        matrix.version
    );
    println!(
        "{:<12} {:<18} {:<28} Agent Doctor",
        "Runtime", "Dimension", "Native"
    );
    for row in &matrix.rows {
        println!(
            "{:<12} {:<18} {:<28} {} [{}]",
            row.runtime, row.dimension, row.native, row.agent_doctor, row.tier
        );
    }
    Ok(())
}

pub fn activate(name: &str, backup: bool, restart_gateways: bool) -> Result<()> {
    let report = use_workspace_with_options(
        name,
        &UseWorkspaceOptions {
            backup,
            restart_gateways,
        },
    )?;
    print_use_report(&report);
    Ok(())
}

pub fn enter(path: Option<std::path::PathBuf>, git_root: bool) -> Result<()> {
    let report = enter_workspace(path, git_root)?;
    if report.switched {
        println!("Switched to workspace: {}", report.name);
    } else {
        println!("Workspace already active: {}", report.name);
    }
    if let Some(backup_id) = &report.use_report.backup_id {
        println!("  backup: {backup_id}");
    }
    println!("  project: {}", report.path.display());
    println!();
    println!("Run in this shell:");
    println!("  {}", report.zsh_enter);
    println!();
    println!("PowerShell:");
    println!("  {}", report.powershell_enter);
    Ok(())
}

pub fn r#match(path: Option<std::path::PathBuf>, git_root: bool) -> Result<()> {
    match match_workspace_for_path(path, git_root)? {
        Some(name) => {
            println!("{name}");
            Ok(())
        }
        None => bail!("no workspace registered for this path"),
    }
}

pub fn env(shell: &str, name: Option<&str>) -> Result<()> {
    let doc = load_workspaces()?;
    let workspace_name = match name {
        Some(name) => name.to_string(),
        None => doc.active.clone().ok_or_else(|| {
            anyhow::anyhow!("no active workspace — pass --name or run workspace use")
        })?,
    };
    print!("{}", render_shell_env_for_name(&workspace_name, shell)?);
    Ok(())
}

pub fn hook_install(shell: &str) -> Result<()> {
    match shell {
        "zsh" => {
            let path = install_zsh_hook()?;
            print_hook_instructions("zsh", &path);
        }
        "bash" => {
            let path = install_bash_hook()?;
            print_hook_instructions("bash", &path);
        }
        "fish" => {
            let path = install_fish_hook()?;
            print_hook_instructions("fish", &path);
        }
        "powershell" | "pwsh" => {
            let path = install_powershell_hook()?;
            print_powershell_hook_instructions(&path);
        }
        "all" => {
            let zsh = install_zsh_hook()?;
            let bash = install_bash_hook()?;
            let fish = install_fish_hook()?;
            let powershell = install_powershell_hook()?;
            print_hook_instructions("zsh", &zsh);
            println!();
            print_hook_instructions("bash", &bash);
            println!();
            print_hook_instructions("fish", &fish);
            println!();
            print_powershell_hook_instructions(&powershell);
        }
        other => bail!("unsupported shell '{other}' — use zsh, bash, fish, powershell, or all"),
    }
    Ok(())
}

fn print_hook_instructions(shell: &str, path: &std::path::Path) {
    println!("Installed {shell} hook: {}", path.display());
    println!();
    let rc = match shell {
        "zsh" => "~/.zshrc",
        "fish" => "~/.config/fish/config.fish",
        _ => "~/.bashrc",
    };
    println!("Add to {rc}:");
    println!("  source \"{}\"", path.display());
}

fn print_powershell_hook_instructions(path: &std::path::Path) {
    println!("Installed PowerShell hook: {}", path.display());
    println!();
    println!("Add to $PROFILE (Documents/PowerShell/Microsoft.PowerShell_profile.ps1):");
    println!("  . \"{}\"", path.display());
}

pub fn hook_status(json: bool) -> Result<()> {
    let statuses = workspace_hook_status()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&statuses)?);
        return Ok(());
    }

    println!("Workspace shell hook status\n");
    for status in &statuses {
        let hook_marker = if status.hook_installed { "✓" } else { "✗" };
        let rc_marker = if status.rc_sources_hook {
            "✓"
        } else if status.rc_file.is_some() {
            "!"
        } else {
            "·"
        };
        println!(
            "{hook_marker} {} hook: {}",
            status.shell,
            status.hook_path.display()
        );
        if let Some(rc) = &status.rc_file {
            println!("    {rc_marker} rc sources hook: {}", rc.display());
        } else {
            println!("    · rc file not found");
        }
    }
    Ok(())
}

pub fn status(path: Option<std::path::PathBuf>, json: bool) -> Result<()> {
    let report = workspace_status(path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!(
        "Active workspace: {}",
        report.active.as_deref().unwrap_or("(none)")
    );
    println!("Current cwd:      {}", report.cwd.display());
    if let Some(name) = &report.matched_workspace {
        println!("Cwd matches:      {name}");
    } else {
        println!("Cwd matches:      (no registered workspace)");
    }
    println!();
    for runtime in &report.runtimes {
        let marker = if runtime.aligned { "✓" } else { "!" };
        println!(
            "{marker} {} [{}] expected={} actual={}",
            runtime.runtime_id, runtime.isolation_tier, runtime.expected, runtime.actual
        );
        if !runtime.aligned {
            println!("    hint: {}", runtime.hint);
        }
    }
    Ok(())
}

pub fn doctor(json: bool) -> Result<()> {
    let report = workspace_doctor()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!(
        "Workspace doctor (active: {})\n",
        report.active.as_deref().unwrap_or("(none)")
    );
    for check in &report.checks {
        let marker = match check.status {
            WorkspaceCheckStatus::Pass => "✓",
            WorkspaceCheckStatus::Warn => "!",
            WorkspaceCheckStatus::Fail => "✗",
        };
        println!("{marker} {} — {}", check.id, check.title);
        println!("    {}", check.detail);
    }
    Ok(())
}

pub fn fix(
    dry_run: bool,
    restart_gateways: bool,
    migrate_claude_mcp: bool,
    json: bool,
) -> Result<()> {
    let report = workspace_fix(&WorkspaceFixOptions {
        dry_run,
        restart_gateways,
        migrate_claude_mcp,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let mode = if dry_run { "dry-run" } else { "apply" };
    println!(
        "Workspace fix ({mode}, active: {})\n",
        report.active.as_deref().unwrap_or("(none)")
    );
    for action in &report.actions {
        let marker = if action.applied { "✓" } else { "·" };
        println!("{marker} {} — {}", action.id, action.title);
        println!("    {}", action.detail);
    }
    if dry_run {
        println!();
        println!("Run without --dry-run to apply fixes.");
    }
    Ok(())
}

pub fn remove(name: &str, purge: bool) -> Result<()> {
    remove_workspace(name, purge)?;
    println!("Removed workspace: {name}");
    if purge {
        println!("  purged data under ~/.config/agent-doctor/workspaces/{name}/");
    }
    Ok(())
}

pub fn direnv(name: Option<&str>, write: bool) -> Result<()> {
    let doc = load_workspaces()?;
    let workspace_name = match name {
        Some(name) => name.to_string(),
        None => doc.active.clone().ok_or_else(|| {
            anyhow::anyhow!("no active workspace — pass --name or run workspace use")
        })?,
    };

    if write {
        let path = write_direnv_envrc(&workspace_name)?;
        println!("Wrote {}", path.display());
        println!("Run: direnv allow {}", path.parent().unwrap().display());
        return Ok(());
    }

    print!("{}", render_direnv_envrc(&workspace_name)?);
    Ok(())
}

fn print_use_report(report: &agent_doctor_core::UseWorkspaceReport) {
    println!("Active workspace: {}\n", report.name);
    println!("  project: {}", report.path.display());
    println!("  env:     {}", report.env_file.display());
    if let Some(backup_id) = &report.backup_id {
        println!("  backup:  {backup_id}");
    }
    println!();
    for binding in &report.bindings {
        println!(
            "✓ {} — {} ({})",
            binding.runtime_id, binding.action, binding.isolation_tier
        );
        println!("    {}", binding.detail);
    }
    for restart in &report.gateway_restarts {
        if restart.attempted {
            let marker = if restart.success { "✓" } else { "!" };
            println!(
                "{marker} gateway {} — {}",
                restart.runtime_id, restart.detail
            );
        }
    }
    println!();
    println!("Apply in this shell:");
    println!(
        "  cd {} && eval \"$(agent-doctor workspace env --shell zsh --name {})\"",
        report.path.display(),
        report.name
    );
}

fn print_document(doc: &WorkspacesDocument) {
    let path = agent_doctor_core::workspaces_path().ok();
    if let Some(path) = path {
        println!("Workspaces ({})", path.display());
    }
    println!("Active: {}\n", doc.active.as_deref().unwrap_or("(none)"));
    if doc.workspaces.is_empty() {
        println!("No workspaces. Create one with: agent-doctor workspace init");
        return;
    }
    for (name, entry) in &doc.workspaces {
        let marker = if doc.active.as_deref() == Some(name.as_str()) {
            "*"
        } else {
            " "
        };
        println!(
            "{marker} {name} — {}\n    hermes={} codex={}",
            entry.path.display(),
            entry.hermes_profile,
            entry.codex_home.display()
        );
    }
}
