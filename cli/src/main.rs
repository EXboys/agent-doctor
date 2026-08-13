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
        /// Also upsert Browser MCP (`browser` → agent-doctor) into Codex/Claude configs
        #[arg(long)]
        with_browser_mcp: bool,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// Show or switch exclusive LLM mode (personal provider vs Evotown team)
    Mode {
        #[command(subcommand)]
        action: ModeAction,
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
    /// Register this machine as an Evotown engine (writes evi_ for connect)
    Register {
        /// IT bootstrap ingest token (or set EVOTOWN_INGEST_TOKEN)
        #[arg(long = "bootstrap-token")]
        bootstrap_token: Option<String>,
        /// Engine id (default: <runtime>-<user> or existing EVOTOWN_ENGINE_ID)
        #[arg(long = "engine-id")]
        engine_id: Option<String>,
        /// Evotown engine_type (default: derived from --runtime)
        #[arg(long = "engine-type")]
        engine_type: Option<String>,
        /// Preferred local runtime / engine type hint (openclaw, hermes, claude-code, …)
        #[arg(long)]
        runtime: Option<String>,
        /// Display name in Evotown fleet
        #[arg(long = "name")]
        display_name: Option<String>,
        /// Owner team id
        #[arg(long = "team")]
        owner_team: Option<String>,
        /// Mint a new evi_ even if this engine_id already exists
        #[arg(long)]
        rotate: bool,
        /// Print the issued token but do not write evotown.agent.env
        #[arg(long = "no-save-token")]
        no_save_token: bool,
        /// Emit JSON
        #[arg(long)]
        json: bool,
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
    /// Agentless remote VPS project health over SSH (read-only doctor)
    Remote {
        #[command(subcommand)]
        action: RemoteAction,
    },
}

#[derive(Subcommand)]
enum RemoteAction {
    /// Manage SSH hosts registered for remote doctor
    Host {
        #[command(subcommand)]
        action: RemoteHostAction,
    },
    /// Manage remote project paths on a host
    Project {
        #[command(subcommand)]
        action: RemoteProjectAction,
    },
    /// Run read-only remote doctor for host/project
    Doctor {
        /// Target as host/project (e.g. prod-vps/api)
        target: String,
        #[arg(long)]
        json: bool,
        /// Limit checks to one runtime id
        #[arg(long)]
        runtime: Option<String>,
    },
}

#[derive(Subcommand)]
enum RemoteHostAction {
    /// Register a host (OpenSSH config Host alias)
    Add {
        /// Local registry id (e.g. prod-vps)
        id: String,
        /// Value of `Host` in ~/.ssh/config
        #[arg(long)]
        ssh_config_host: String,
    },
    /// List registered hosts
    List {
        #[arg(long)]
        json: bool,
    },
    /// Remove a registered host
    Remove { id: String },
}

#[derive(Subcommand)]
enum RemoteProjectAction {
    /// Register a project path on a host
    Add {
        /// Host id
        host: String,
        /// Project name
        name: String,
        /// Absolute path on the remote host
        #[arg(long)]
        path: String,
        /// Runtime ids to check (repeatable; default: all)
        #[arg(long = "runtime")]
        runtimes: Vec<String>,
    },
    /// List projects (optionally filter by host)
    List {
        host: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Remove a project from a host
    Remove { host: String, name: String },
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
enum ModeAction {
    /// Show current personal/team mode and readiness
    #[command(alias = "status")]
    Show {
        #[arg(long)]
        json: bool,
    },
    /// Wire runtimes to the active personal provider
    Personal {
        /// Saved personal provider id (default: last active)
        #[arg(long)]
        provider_id: Option<String>,
        /// Also upsert Browser MCP into Codex/Claude configs
        #[arg(long)]
        with_browser_mcp: bool,
        #[arg(long)]
        json: bool,
    },
    /// Wire runtimes to Evotown / company gateway
    Team {
        /// Also upsert Browser MCP into Codex/Claude configs
        #[arg(long)]
        with_browser_mcp: bool,
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
        /// Chrome user-data-dir (default: everyday system Chrome profile)
        #[arg(long)]
        user_data_dir: Option<std::path::PathBuf>,
        /// Chrome profile directory name (default: Default)
        #[arg(long)]
        profile_directory: Option<String>,
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
        /// Launch Chrome without a visible window (default: show UI)
        #[arg(long)]
        headless: bool,
        /// Chrome user-data-dir (default: isolated automation profile)
        #[arg(long)]
        user_data_dir: Option<std::path::PathBuf>,
        /// Chrome profile directory name (default: Default)
        #[arg(long)]
        profile_directory: Option<String>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// Prepare a Chrome instance for Browser MCP (isolated by default)
    Chrome {
        #[command(subcommand)]
        action: McpChromeAction,
    },
}

#[derive(Subcommand)]
enum McpChromeAction {
    /// Ensure CDP is ready without touching everyday Chrome (isolated profile)
    Ensure {
        /// Chrome DevTools Protocol port
        #[arg(long, default_value_t = 9222)]
        port: u16,
        /// Hide the browser window
        #[arg(long)]
        headless: bool,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// Restart everyday Chrome with remote debugging so MCP can attach (keeps logins)
    AttachDaily {
        /// Chrome DevTools Protocol port
        #[arg(long, default_value_t = 9222)]
        port: u16,
        /// Profile directory inside the everyday user-data-dir
        #[arg(long, default_value = "Default")]
        profile_directory: String,
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
            with_browser_mcp,
            json,
        } => commands::setup::run(&url, &key, Some(&provider), with_browser_mcp, json)?,
        Commands::Mode { action } => match action {
            ModeAction::Show { json } => commands::mode::show(json)?,
            ModeAction::Personal {
                provider_id,
                with_browser_mcp,
                json,
            } => commands::mode::switch_personal(provider_id.as_deref(), with_browser_mcp, json)?,
            ModeAction::Team {
                with_browser_mcp,
                json,
            } => commands::mode::switch_team(with_browser_mcp, json)?,
        },
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
        Commands::Register {
            bootstrap_token,
            engine_id,
            engine_type,
            runtime,
            display_name,
            owner_team,
            rotate,
            no_save_token,
            json,
        } => commands::register::run(
            agent_doctor_core::RegisterOptions {
                bootstrap_token,
                engine_id,
                engine_type,
                runtime,
                display_name,
                owner_team,
                deployment_kind: None,
                engine_version: None,
                rotate,
                save_token: !no_save_token,
            },
            json,
        )?,
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
            McpAction::Browser {
                port,
                headless,
                user_data_dir,
                profile_directory,
                json,
            } => {
                commands::mcp::run_browser(port, headless, user_data_dir, profile_directory, json)?
            }
            McpAction::Status { port, json } => commands::mcp::run_status(port, json)?,
            McpAction::Configure {
                runtime,
                port,
                headless,
                user_data_dir,
                profile_directory,
                json,
            } => commands::mcp::run_configure(
                &runtime,
                port,
                headless,
                user_data_dir,
                profile_directory,
                json,
            )?,
            McpAction::Chrome { action } => match action {
                McpChromeAction::Ensure {
                    port,
                    headless,
                    json,
                } => commands::mcp::run_chrome_ensure(port, headless, json)?,
                McpChromeAction::AttachDaily {
                    port,
                    profile_directory,
                    json,
                } => commands::mcp::run_chrome_attach_daily(port, &profile_directory, json)?,
            },
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
        Commands::Remote { action } => match action {
            RemoteAction::Host { action } => match action {
                RemoteHostAction::Add {
                    id,
                    ssh_config_host,
                } => commands::remote::host_add(&id, &ssh_config_host)?,
                RemoteHostAction::List { json } => commands::remote::host_list(json)?,
                RemoteHostAction::Remove { id } => commands::remote::host_remove(&id)?,
            },
            RemoteAction::Project { action } => match action {
                RemoteProjectAction::Add {
                    host,
                    name,
                    path,
                    runtimes,
                } => commands::remote::project_add(&host, &name, &path, runtimes)?,
                RemoteProjectAction::List { host, json } => {
                    commands::remote::project_list(host.as_deref(), json)?
                }
                RemoteProjectAction::Remove { host, name } => {
                    commands::remote::project_remove(&host, &name)?
                }
            },
            RemoteAction::Doctor {
                target,
                json,
                runtime,
            } => commands::remote::doctor(&target, json, runtime.as_deref())?,
        },
    }
    Ok(())
}
