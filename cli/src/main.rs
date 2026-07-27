mod commands;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "agent-doctor",
    about = "Diagnose, back up, and repair local AI agent runtimes"
)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Discover installed runtimes, config paths, and gateway wiring
    Doctor {
        /// Emit JSON instead of human-readable output
        #[arg(long)]
        json: bool,
        /// AI explanation of probe results (per runtime with issues)
        #[arg(long)]
        explain: bool,
    },
    /// Rule-based install for registered runtimes (rule install when available, else AI)
    Install {
        /// Runtime id (e.g. openclaw, hermes)
        runtime: String,
        /// AI diagnosis after install (or on failure)
        #[arg(long)]
        explain: bool,
        /// After successful rule install, run AI repair loop for remaining issues
        #[arg(long)]
        plan: Option<String>,
        /// After successful rule install, run deterministic repair loop when issues remain
        #[arg(long)]
        repair: bool,
        /// Extra rule-based install retries on failure
        #[arg(long, default_value_t = 0)]
        retry: u8,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// List, create, and switch local model presets
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
    /// Show or update runtime-specific configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Back up, diagnose, and repair a runtime
    Repair {
        /// Runtime id (e.g. openclaw, hermes, claude-code, codex)
        runtime: String,
        /// Execute backup, typed actions, re-probe verification, and write audit metadata
        #[arg(long, conflicts_with = "rollback")]
        apply: bool,
        /// Restore config files from a backup snapshot (latest, or --backup id)
        #[arg(long, conflicts_with = "apply")]
        rollback: bool,
        /// Backup id to restore (with --rollback); default is latest for this runtime
        #[arg(long, requires = "rollback")]
        backup: Option<String>,
        /// Bounded probe → plan → apply → verify loop (pair with --apply to execute fixes)
        #[arg(long = "loop", conflicts_with = "rollback")]
        repair_loop: bool,
        /// Planner for --loop: deterministic (default) or ai (placeholder)
        #[arg(long, default_value = "deterministic")]
        plan: String,
        /// AI explanation of probe results and suggested fixes
        #[arg(long)]
        explain: bool,
        /// Emit JSON (with --apply or --rollback)
        #[arg(long)]
        json: bool,
    },
    /// Apply company gateway profile to local runtimes
    Setup {
        /// Company gateway base URL (e.g. https://gateway.company.internal/v1)
        #[arg(long)]
        url: String,
        /// Company API key (written to profile.env and runtime configs)
        #[arg(long)]
        key: String,
        /// Hermes provider id when creating config (default: openai)
        #[arg(long, default_value = "openai")]
        provider: String,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage MCP servers (browser control via CDP)
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
    /// Pull private SkillHub bundle from Evotown
    Sync {
        /// Show planned actions without downloading
        #[arg(long)]
        dry_run: bool,
        /// Only install these skill_ids from the bundle manifest (repeatable)
        #[arg(long = "only")]
        only: Vec<String>,
        /// Runtime target for manifest (default from evotown.agent.env or openclaw)
        #[arg(long)]
        runtime: Option<String>,
        /// Skill bundle id (default: default-agent-skills)
        #[arg(long)]
        bundle: Option<String>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// Stay connected to Evotown over WebSocket (presence + inventory)
    Connect {
        /// Seconds between inventory refreshes
        #[arg(long, default_value_t = 60)]
        inventory_interval: u64,
        /// Seconds between heartbeat messages
        #[arg(long, default_value_t = 25)]
        heartbeat_interval: u64,
        /// Max reconnect backoff seconds
        #[arg(long, default_value_t = 60)]
        max_backoff: u64,
    },
    /// Default local agent runtime for Evotown job dispatch (`EVOTOWN_RUNTIME`)
    #[command(name = "preferred-runtime")]
    PreferredRuntime {
        #[command(subcommand)]
        action: PreferredRuntimeAction,
    },
    /// Open an interactive CLI session (Claude Code deep link / system terminal)
    Open {
        /// Runtime id: claude-code, codex, hermes, openclaw
        runtime: String,
        /// Working directory (default: active workspace or cwd)
        #[arg(long)]
        cwd: Option<String>,
        /// Optional prompt to pre-fill (Claude Code deep link only)
        #[arg(long, short = 'q')]
        prompt: Option<String>,
        /// Skip deep link and always open a system terminal
        #[arg(long)]
        terminal: bool,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// Cache policies from control plane (not yet implemented)
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
    /// Per-project workspace isolation (Hermes, Claude Code, Codex, OpenClaw)
    Workspace {
        #[command(subcommand)]
        action: WorkspaceAction,
    },
}

#[derive(Subcommand)]
enum PreferredRuntimeAction {
    /// Show the configured preferred runtime
    Show {
        #[arg(long)]
        json: bool,
    },
    /// Set preferred runtime (claude-code, hermes, openclaw, codex)
    Use {
        /// Runtime id
        runtime: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ProfileAction {
    /// Create example ~/.config/agent-doctor/profiles.yaml
    Init,
    /// List configured presets
    List,
    /// Activate a preset and apply it to installed runtimes
    Use {
        /// Profile name (e.g. work, personal)
        name: String,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current model settings for a runtime
    Show {
        /// Runtime id (e.g. hermes, openclaw, claude-code)
        runtime: String,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// Write model settings to a runtime config file
    Set {
        /// Runtime id (e.g. hermes)
        runtime: String,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        base_url: Option<String>,
    },
}

#[derive(Subcommand)]
enum WorkspaceAction {
    /// Register a project directory as an isolated workspace
    Init {
        /// Project path (default: current directory)
        path: Option<std::path::PathBuf>,
        /// Workspace name (default: directory name)
        #[arg(long)]
        name: Option<String>,
        /// Resolve git repository root instead of the given directory
        #[arg(long)]
        git_root: bool,
    },
    /// List registered workspaces
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show details for one workspace
    Show {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Print workspace isolation capability matrix
    Matrix {
        #[arg(long)]
        json: bool,
    },
    /// Activate a workspace and write active-workspace.env
    Use {
        name: String,
        /// Skip config backup before switching
        #[arg(long)]
        no_backup: bool,
        /// Restart Hermes/OpenClaw gateways after switching
        #[arg(long)]
        restart_gateways: bool,
    },
    /// Match cwd to a registered workspace (prints name)
    Match {
        path: Option<std::path::PathBuf>,
        #[arg(long)]
        git_root: bool,
    },
    /// Print shell exports for a workspace (eval "$(agent-doctor workspace env --shell zsh)")
    Env {
        #[arg(long, default_value = "zsh")]
        shell: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Activate workspace for path, backup, and print eval snippet
    Enter {
        path: Option<std::path::PathBuf>,
        #[arg(long)]
        git_root: bool,
    },
    /// Print or write a direnv .envrc for a workspace
    Direnv {
        #[arg(long)]
        name: Option<String>,
        /// Write .envrc into the project directory
        #[arg(long)]
        write: bool,
    },
    /// Install shell cd hooks for auto workspace env alignment
    Hook {
        #[command(subcommand)]
        action: WorkspaceHookAction,
    },
    /// Show active workspace and runtime alignment
    Status {
        path: Option<std::path::PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Detect memory/config bleed risks for the active workspace
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// Auto-fix alignment issues detected by workspace doctor
    Fix {
        /// Preview fixes without applying
        #[arg(long)]
        dry_run: bool,
        /// Restart Hermes/OpenClaw gateways when fixing gateway mismatch
        #[arg(long)]
        restart_gateways: bool,
        /// Merge global Claude MCP servers into project .mcp.json (does not remove global)
        #[arg(long)]
        migrate_claude_mcp: bool,
        #[arg(long)]
        json: bool,
    },
    /// Remove a registered workspace
    Remove {
        name: String,
        /// Delete ~/.config/agent-doctor/workspaces/<name>/ data
        #[arg(long)]
        purge: bool,
    },
}

#[derive(Subcommand)]
enum WorkspaceHookAction {
    /// Install shell cd hooks (zsh, bash, fish, powershell, or all)
    Install {
        /// Shell hook to install: zsh, bash, fish, powershell, or all
        #[arg(long, default_value = "all")]
        shell: String,
    },
    /// Check whether workspace hooks are installed and sourced
    Status {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum PolicyAction {
    /// Cache enabled policies from Evotown locally
    Pull {
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
}


#[derive(Subcommand)]
enum McpAction {
    /// Start browser MCP server (connect Codex/Claude to Chrome)
    Browser {
        /// Chrome DevTools Protocol port
        #[arg(long, default_value_t = 9222)]
        port: u16,
        /// Run headless (no visible window)
        #[arg(long)]
        headless: bool,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// Check browser MCP server status
    Status {
        /// Chrome DevTools Protocol port to check
        #[arg(long, default_value_t = 9222)]
        port: u16,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// Configure browser MCP for a runtime (codex, claude-code)
    Configure {
        /// Runtime id (codex or claude-code)
        runtime: String,
        /// Chrome DevTools Protocol port
        #[arg(long, default_value_t = 9222)]
        port: u16,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Doctor { json, explain } => commands::doctor::run(json, explain)?,
        Commands::Profile { action } => match action {
            ProfileAction::Init => commands::profile::init()?,
            ProfileAction::List => commands::profile::list()?,
            ProfileAction::Use { name } => commands::profile::activate(&name)?,
        },
        Commands::Config { action } => match action {
            ConfigAction::Show { runtime, json } => commands::config::show(&runtime, json)?,
            ConfigAction::Set {
                runtime,
                provider,
                model,
                base_url,
            } => commands::config::set(&runtime, provider, model, base_url)?,
        },
        Commands::Install {
            runtime,
            explain,
            plan,
            repair,
            retry,
            json,
        } => {
            let plan_ai = plan.as_deref() == Some("ai");
            commands::install::run(&runtime, explain, plan_ai, repair, retry, json)?
        }
        Commands::Repair {
            runtime,
            apply,
            rollback,
            backup,
            repair_loop,
            plan,
            explain,
            json,
        } => commands::repair::run(
            &runtime,
            apply,
            rollback,
            backup.as_deref(),
            repair_loop,
            &plan,
            explain,
            json,
        )?,
        Commands::Setup {
            url,
            key,
            provider,
            json,
        } => commands::setup::run(&url, &key, Some(&provider), json)?,
        Commands::Sync {
            dry_run,
            only,
            runtime,
            bundle,
            json,
        } => commands::sync::run(dry_run, &only, runtime.as_deref(), bundle.as_deref(), json)?,
        Commands::Connect {
            inventory_interval,
            heartbeat_interval,
            max_backoff,
        } => commands::connect::run(inventory_interval, heartbeat_interval, max_backoff)?,
        Commands::PreferredRuntime { action } => match action {
            PreferredRuntimeAction::Show { json } => commands::preferred_runtime::show(json)?,
            PreferredRuntimeAction::Use { runtime, json } => {
                commands::preferred_runtime::use_runtime(&runtime, json)?
            }
        },
        Commands::Open {
            runtime,
            cwd,
            prompt,
            terminal,
            json,
        } => commands::open_session::run(
            &runtime,
            cwd.as_deref(),
            prompt.as_deref(),
            terminal,
            json,
        )?,
        Commands::Policy { action } => match action {
            PolicyAction::Pull { json } => commands::policy::pull(json)?,
        },
        Commands::Mcp { action } => match action {
            McpAction::Browser { port, headless, json } => {
                commands::mcp::run_browser(port, headless, json)?
            }
            McpAction::Status { port, json } => {
                commands::mcp::run_status(port, json)?
            }
            McpAction::Configure { runtime, port, json } => {
                commands::mcp::run_configure(&runtime, port, json)?
            }
        },
        Commands::Workspace { action } => match action {
            WorkspaceAction::Init {
                path,
                name,
                git_root,
            } => commands::workspace::init(path, name, git_root)?,
            WorkspaceAction::List { json } => commands::workspace::list(json)?,
            WorkspaceAction::Show { name, json } => commands::workspace::show(&name, json)?,
            WorkspaceAction::Matrix { json } => commands::workspace::matrix(json)?,
            WorkspaceAction::Use {
                name,
                no_backup,
                restart_gateways,
            } => commands::workspace::activate(&name, !no_backup, restart_gateways)?,
            WorkspaceAction::Match { path, git_root } => {
                commands::workspace::r#match(path, git_root)?
            }
            WorkspaceAction::Env { shell, name } => {
                commands::workspace::env(&shell, name.as_deref())?
            }
            WorkspaceAction::Enter { path, git_root } => {
                commands::workspace::enter(path, git_root)?
            }
            WorkspaceAction::Direnv { name, write } => {
                commands::workspace::direnv(name.as_deref(), write)?
            }
            WorkspaceAction::Hook { action } => match action {
                WorkspaceHookAction::Install { shell } => {
                    commands::workspace::hook_install(&shell)?
                }
                WorkspaceHookAction::Status { json } => commands::workspace::hook_status(json)?,
            },
            WorkspaceAction::Status { path, json } => commands::workspace::status(path, json)?,
            WorkspaceAction::Doctor { json } => commands::workspace::doctor(json)?,
            WorkspaceAction::Fix {
                dry_run,
                restart_gateways,
                migrate_claude_mcp,
                json,
            } => commands::workspace::fix(dry_run, restart_gateways, migrate_claude_mcp, json)?,
            WorkspaceAction::Remove { name, purge } => commands::workspace::remove(&name, purge)?,
        },
    }
    Ok(())
}
