//! Local MCP server inventory: project + Claude configs, path health, browser entry.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::adapters::util::{find_all_binaries, find_binary, home_join};
use crate::exec::{run_output, SHORT_PROBE_TIMEOUT};

use super::{load_workspaces, WorkspacesDocument};

const MCP_INVENTORY_CACHE_TTL: Duration = Duration::from_secs(45);
static MCP_INVENTORY_CACHE: Mutex<Option<(Instant, McpInventoryReport)>> = Mutex::new(None);

pub fn invalidate_mcp_inventory_cache() {
    if let Ok(mut guard) = MCP_INVENTORY_CACHE.lock() {
        *guard = None;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpInventoryItem {
    pub name: String,
    /// project | claude-global | claude-json
    pub scope: String,
    pub config_path: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub healthy: bool,
    pub issue: Option<String>,
    pub is_browser: bool,
    /// codex | claude-code | shared
    pub runtime_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpInventoryReport {
    pub workspace_name: Option<String>,
    pub workspace_path: Option<String>,
    pub servers: Vec<McpInventoryItem>,
    pub total: usize,
    pub healthy: usize,
    pub issues: usize,
    pub browser_configured: bool,
}

pub fn list_mcp_inventory() -> Result<McpInventoryReport> {
    if let Ok(guard) = MCP_INVENTORY_CACHE.lock() {
        if let Some((at, report)) = guard.as_ref() {
            if at.elapsed() < MCP_INVENTORY_CACHE_TTL {
                return Ok(report.clone());
            }
        }
    }
    let doc = load_workspaces().unwrap_or_default();
    let report = list_mcp_inventory_with_doc(&doc);
    if let Ok(mut guard) = MCP_INVENTORY_CACHE.lock() {
        *guard = Some((Instant::now(), report.clone()));
    }
    Ok(report)
}

pub fn list_mcp_inventory_with_doc(doc: &WorkspacesDocument) -> McpInventoryReport {
    let active = doc
        .active
        .as_ref()
        .and_then(|name| doc.workspaces.get(name).map(|entry| (name.clone(), entry)));

    let mut servers = Vec::new();

    if let Some((_, entry)) = active.as_ref() {
        // Project `.mcp.json` is Claude Code project-scope MCP (workspace isolation).
        // Cursor may share the same file; Agent Doctor treats it as claude-code.
        let project_mcp = entry.path.join(".mcp.json");
        servers.extend(read_servers_from_json(
            &project_mcp,
            "project",
            "claude-code",
        ));

        // Codex: workspace CODEX_HOME/config.toml → [mcp_servers.*]
        let codex_config = entry.codex_home.join("config.toml");
        servers.extend(read_servers_from_toml(&codex_config, "codex-home", "codex"));

        // OpenClaw workspace mirror (AD isolation narrative; runtime still loads global).
        let openclaw_ws_mcp = entry.openclaw_workspace.join(".mcp.json");
        servers.extend(read_servers_from_json(
            &openclaw_ws_mcp,
            "openclaw-workspace",
            "openclaw",
        ));
    } else {
        // No active workspace — still inventory global Codex MCP so probes/UI see it.
        let codex_config = home_join(".codex/config.toml");
        servers.extend(read_servers_from_toml(&codex_config, "codex-home", "codex"));
    }

    // Claude Code user-scope MCP lives in ~/.claude.json (settings.json is ignored for MCP).
    let claude_json = home_join(".claude.json");
    servers.extend(read_servers_from_json(
        &claude_json,
        "claude-user",
        "claude-code",
    ));

    // Hermes: active workspace profile home, else ~/.hermes/config.yaml
    let hermes_config = active
        .as_ref()
        .map(|(_, entry)| {
            home_join(".hermes/profiles")
                .join(&entry.hermes_profile)
                .join("config.yaml")
        })
        .unwrap_or_else(|| home_join(".hermes/config.yaml"));
    servers.extend(read_servers_from_yaml(
        &hermes_config,
        "hermes-home",
        "hermes",
    ));

    // OpenClaw runtime config: ~/.openclaw/openclaw.json (`mcp.servers` + legacy `mcpServers`)
    let openclaw_config = home_join(".openclaw/openclaw.json");
    servers.extend(read_servers_from_openclaw(
        &openclaw_config,
        "openclaw-global",
        "openclaw",
    ));

    let dsh_patch = crate::DeepSeekHarnessAdapter::home().join("cordis.patch.yml");
    servers.extend(read_servers_from_dsh_patch(
        &dsh_patch,
        "dsh-home",
        "deepseek-harness",
    ));

    // Legacy mistaken path — still surface if present so users can clean it up.
    let settings = home_join(".claude/settings.json");
    servers.extend(read_servers_from_json(
        &settings,
        "claude-settings-ignored",
        "shared",
    ));

    servers.sort_by(|a, b| {
        a.scope
            .cmp(&b.scope)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.config_path.cmp(&b.config_path))
    });

    let total = servers.len();
    let healthy = servers.iter().filter(|s| s.healthy).count();
    let issues = total.saturating_sub(healthy);
    let browser_configured = servers.iter().any(|s| s.is_browser);

    McpInventoryReport {
        workspace_name: active.as_ref().map(|(name, _)| name.clone()),
        workspace_path: active
            .as_ref()
            .map(|(_, entry)| entry.path.display().to_string()),
        servers,
        total,
        healthy,
        issues,
        browser_configured,
    }
}

/// Attach Browser MCP probe checks for Claude / Codex / Hermes / OpenClaw.
pub fn probe_browser_mcp_for_runtime(runtime_id: &str, checks: &mut Vec<crate::probe::ProbeCheck>) {
    use crate::probe::{ProbeCheck, ProbeSeverity, ProbeStatus};
    use crate::repair::SensitivityLevel;

    if !matches!(
        runtime_id,
        "claude-code" | "codex" | "hermes" | "openclaw" | "deepseek-harness"
    ) {
        return;
    }

    let inventory = list_mcp_inventory_with_doc(&load_workspaces().unwrap_or_default());
    // Prefer workspace-scoped entries (Claude project / OpenClaw mirror) over globals.
    let browser = inventory
        .servers
        .iter()
        .filter(|item| item.is_browser && item.runtime_hint == runtime_id)
        .max_by_key(|item| match item.scope.as_str() {
            "project" | "openclaw-workspace" | "codex-home" | "hermes-home" | "dsh-home" => 2,
            "openclaw-global" | "claude-user" => 1,
            _ => 0,
        });

    match browser {
        None => {
            checks.push(ProbeCheck::new(
                "mcp.browser.configured",
                "Browser MCP configured",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                format!(
                    "no browser MCP entry for {runtime_id}; open Diagnose → Repair, or Resources → Browser MCP → Diagnose & write",
                ),
                SensitivityLevel::ConfigShape,
            ));
        }
        Some(item) if !item.healthy => {
            checks.push(ProbeCheck::new(
                "mcp.browser.configured",
                "Browser MCP configured",
                ProbeStatus::Pass,
                ProbeSeverity::Info,
                openclaw_browser_detail(runtime_id, item),
                SensitivityLevel::ConfigShape,
            ));
            checks.push(ProbeCheck::new(
                "mcp.browser.healthy",
                "Browser MCP command healthy",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                item.issue
                    .clone()
                    .unwrap_or_else(|| "browser MCP command path looks broken".to_string()),
                SensitivityLevel::LocalPath,
            ));
        }
        Some(item) => {
            checks.push(ProbeCheck::new(
                "mcp.browser.configured",
                "Browser MCP configured",
                ProbeStatus::Pass,
                ProbeSeverity::Info,
                openclaw_browser_detail(runtime_id, item),
                SensitivityLevel::ConfigShape,
            ));
            checks.push(ProbeCheck::new(
                "mcp.browser.healthy",
                "Browser MCP command healthy",
                ProbeStatus::Pass,
                ProbeSeverity::Info,
                "browser MCP command resolves".to_string(),
                SensitivityLevel::LocalPath,
            ));
        }
    }
}

fn openclaw_browser_detail(runtime_id: &str, item: &McpInventoryItem) -> String {
    let base = format!(
        "browser MCP present at {} [{}]",
        item.config_path, item.scope
    );
    if runtime_id != "openclaw" {
        return base;
    }
    match item.scope.as_str() {
        "openclaw-workspace" => format!(
            "{base}; OpenClaw runtime still loads ~/.openclaw/openclaw.json globally — \
workspace .mcp.json is an Agent Doctor inventory mirror, not per-workspace MCP isolation"
        ),
        "openclaw-global" => format!(
            "{base}; OpenClaw MCP is global — switching workspaces does not swap MCP servers \
(workspace .mcp.json mirror missing; re-run repair/wire with an active workspace)"
        ),
        _ => format!("{base}; OpenClaw MCP is global (~/.openclaw) — not isolated per workspace"),
    }
}

fn read_servers_from_json(path: &Path, scope: &str, runtime_hint: &str) -> Vec<McpInventoryItem> {
    if !path.exists() {
        return Vec::new();
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<JsonValue>(&raw) else {
        return Vec::new();
    };
    let Some(map) = value.get("mcpServers").and_then(JsonValue::as_object) else {
        return Vec::new();
    };

    map.iter()
        .map(|(name, entry)| {
            item_from_command_args(
                name,
                entry.get("command").and_then(JsonValue::as_str),
                entry
                    .get("args")
                    .and_then(JsonValue::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
                path,
                scope,
                runtime_hint,
            )
        })
        .collect()
}

fn read_servers_from_yaml(path: &Path, scope: &str, runtime_hint: &str) -> Vec<McpInventoryItem> {
    if !path.exists() {
        return Vec::new();
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(&raw) else {
        return Vec::new();
    };
    let Some(map) = value
        .get("mcp_servers")
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return Vec::new();
    };

    map.iter()
        .filter_map(|(name, entry)| {
            let name = name.as_str()?;
            let command = entry.get("command").and_then(serde_yaml::Value::as_str);
            let args = entry
                .get("args")
                .and_then(serde_yaml::Value::as_sequence)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_yaml::Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Some(item_from_command_args(
                name,
                command,
                args,
                path,
                scope,
                runtime_hint,
            ))
        })
        .collect()
}

fn read_servers_from_dsh_patch(
    path: &Path,
    scope: &str,
    runtime_hint: &str,
) -> Vec<McpInventoryItem> {
    if !path.exists() {
        return Vec::new();
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(&raw) else {
        return Vec::new();
    };
    let items = match &value {
        serde_yaml::Value::Sequence(seq) => seq.as_slice(),
        serde_yaml::Value::Mapping(map) => map
            .get(serde_yaml::Value::String("plugins".into()))
            .and_then(serde_yaml::Value::as_sequence)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        _ => &[],
    };
    items
        .iter()
        .filter_map(|item| {
            let map = item.as_mapping()?;
            let name = map
                .get(serde_yaml::Value::String("name".into()))
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or("");
            if !name.contains("dsh-mcp-client") {
                return None;
            }
            let config = map.get(serde_yaml::Value::String("config".into()))?;
            let server_name = config
                .get("serverName")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or("mcp");
            let command = config.get("command").and_then(serde_yaml::Value::as_str);
            let args = config
                .get("args")
                .and_then(serde_yaml::Value::as_sequence)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(serde_yaml::Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Some(item_from_command_args(
                server_name,
                command,
                args,
                path,
                scope,
                runtime_hint,
            ))
        })
        .collect()
}

fn read_servers_from_openclaw(
    path: &Path,
    scope: &str,
    runtime_hint: &str,
) -> Vec<McpInventoryItem> {
    if !path.exists() {
        return Vec::new();
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<JsonValue>(&raw) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    if let Some(map) = value.pointer("/mcp/servers").and_then(JsonValue::as_object) {
        out.extend(map.iter().map(|(name, entry)| {
            item_from_command_args(
                name,
                entry.get("command").and_then(JsonValue::as_str),
                entry
                    .get("args")
                    .and_then(JsonValue::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
                path,
                scope,
                runtime_hint,
            )
        }));
    }
    if let Some(map) = value.get("mcpServers").and_then(JsonValue::as_object) {
        for (name, entry) in map {
            if out.iter().any(|item| item.name == *name) {
                continue;
            }
            out.push(item_from_command_args(
                name,
                entry.get("command").and_then(JsonValue::as_str),
                entry
                    .get("args")
                    .and_then(JsonValue::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
                path,
                scope,
                runtime_hint,
            ));
        }
    }
    out
}

fn read_servers_from_toml(path: &Path, scope: &str, runtime_hint: &str) -> Vec<McpInventoryItem> {
    if !path.exists() {
        return Vec::new();
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = toml::from_str::<toml::Value>(&raw) else {
        return Vec::new();
    };
    let Some(map) = value.get("mcp_servers").and_then(|v| v.as_table()) else {
        return Vec::new();
    };

    map.iter()
        .map(|(name, entry)| {
            let command = entry.get("command").and_then(|v| v.as_str());
            let args = entry
                .get("args")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            item_from_command_args(name, command, args, path, scope, runtime_hint)
        })
        .collect()
}

fn item_from_command_args(
    name: &str,
    command: Option<&str>,
    args: Vec<String>,
    path: &Path,
    scope: &str,
    runtime_hint: &str,
) -> McpInventoryItem {
    let is_browser = name == "browser"
        || (args.iter().any(|a| a == "mcp") && args.iter().any(|a| a == "browser"));
    let (healthy, issue) = assess_command(command);

    McpInventoryItem {
        name: name.to_string(),
        scope: scope.to_string(),
        config_path: path.display().to_string(),
        command: command.map(str::to_string),
        args,
        healthy,
        issue,
        is_browser,
        runtime_hint: runtime_hint.to_string(),
    }
}

fn assess_command(command: Option<&str>) -> (bool, Option<String>) {
    let Some(command) = command.filter(|c| !c.trim().is_empty()) else {
        return (false, Some("missing command".into()));
    };

    let path = PathBuf::from(command);
    if command.starts_with(r"\\?\") {
        return (
            false,
            Some("command uses a Windows \\\\?\\ path Codex cannot start".into()),
        );
    }
    if path.is_absolute() {
        if path.exists() {
            return (true, None);
        }
        return (false, Some(format!("path missing: {command}")));
    }

    if find_binary(command).is_some() {
        return (true, None);
    }

    // Relative / shell-style commands may still work when the runtime launches them.
    if command.contains('/') || command.contains('\\') {
        return (false, Some(format!("path missing: {command}")));
    }

    (true, None)
}

/// Drop the Windows verbatim prefix so other programs can start this path.
fn launchable_path(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = raw.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

/// Resolve the real `agent-doctor` CLI binary used as the MCP server command.
///
/// Skips the common Hermes workspace shim at `~/.local/bin/agent-doctor`
/// (`exec hermes -p agent-doctor …`), which is not the Agent Doctor CLI.
///
/// Prefers a stable, non-ephemeral path (and the current `agent-doctor` binary
/// name) over leftover `agent-doctor-cli` copies under Cursor sandbox caches —
/// those can be months old and break tools like `browser_screenshot`.
///
/// On an installed app, the CLI next to the running executable wins, then the
/// directory recorded by the Windows installer. A newer copy under the default
/// `%LOCALAPPDATA%\Agent Doctor` must not replace a custom directory such as
/// `D:\Agent Doctor`.
pub fn resolve_agent_doctor_binary() -> Result<PathBuf> {
    let mut candidates = Vec::new();
    let current_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let registered_dirs = registered_install_dirs();

    // Dev / Cursor sandbox: honor CARGO_TARGET_DIR before PATH or stale siblings.
    if let Ok(target) = std::env::var("CARGO_TARGET_DIR") {
        let root = PathBuf::from(target);
        for rel in [
            "release/agent-doctor",
            "debug/agent-doctor",
            "release/agent-doctor-cli",
            "debug/agent-doctor-cli",
        ] {
            candidates.push(root.join(rel));
        }
    }

    // Bundled CLI next to the running exe, then the installer-recorded directory.
    // `agent-doctor-cli` is listed after `agent-doctor` only as a probe order;
    // installed builds prefer whichever of these actually sits in the install dir.
    if let Some(dir) = current_dir.as_deref() {
        candidates.extend(bundled_cli_candidates(dir, true));
    }
    for dir in &registered_dirs {
        candidates.extend(bundled_cli_candidates(dir, false));
    }

    // Prefer an in-tree release/debug build when developing from source.
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let root = PathBuf::from(manifest_dir);
        for rel in [
            "../target/release/agent-doctor",
            "../target/debug/agent-doctor",
            "../../target/release/agent-doctor",
            "../../target/debug/agent-doctor",
            "target/release/agent-doctor",
            "target/debug/agent-doctor",
            "../target/release/agent-doctor-cli",
            "../target/debug/agent-doctor-cli",
            "../../target/release/agent-doctor-cli",
            "../../target/debug/agent-doctor-cli",
        ] {
            candidates.push(root.join(rel));
        }
    }

    candidates.extend(find_all_binaries("agent-doctor"));
    candidates.extend(find_all_binaries("agent-doctor-cli"));

    let mut real = Vec::new();
    let mut saw_unrunnable: Option<PathBuf> = None;
    for path in candidates {
        match classify_agent_doctor_cli(&path) {
            CliProbe::Real => {
                // `canonicalize` on Windows adds `\\?\`, which Codex cannot spawn
                // ("program not found") even though the file exists.
                let canonical = launchable_path(&path.canonicalize().unwrap_or(path));
                if !real.iter().any(|p: &PathBuf| p == &canonical) {
                    real.push(canonical);
                }
            }
            CliProbe::Unrunnable => {
                if saw_unrunnable.is_none() {
                    saw_unrunnable = Some(path);
                }
            }
            CliProbe::Skip => {}
        }
    }

    // Installed apps must follow the directory the user chose. A newer CLI
    // left in the default location or on PATH is the wrong program.
    let dev = std::env::var_os("CARGO_MANIFEST_DIR").is_some()
        || std::env::var_os("CARGO_TARGET_DIR").is_some();
    if !dev {
        if let Some(best) = pick_cli_for_install(&real, current_dir.as_deref(), &registered_dirs) {
            return Ok(best);
        }
    }

    if let Some(best) = pick_best_agent_doctor_cli(&real) {
        return Ok(best);
    }

    if let Some(path) = saw_unrunnable {
        #[cfg(windows)]
        {
            anyhow::bail!(
                "Agent Doctor CLI was found at {} but could not start. \
                 This usually means Microsoft Visual C++ Redistributable is missing \
                 (VCRUNTIME140.dll). Reinstall Agent Doctor (newer builds include the runtime), \
                 or install https://aka.ms/vc14/vc_redist.x64.exe and restart the app.",
                path.display()
            );
        }
        #[cfg(not(windows))]
        {
            anyhow::bail!(
                "Agent Doctor CLI was found at {} but could not start (`--version` failed).",
                path.display()
            );
        }
    }

    anyhow::bail!(
        "Could not find the Agent Doctor CLI. Install with `cargo install --path cli`, \
         or place `agent-doctor-cli` on PATH (note: ~/.local/bin/agent-doctor may be a Hermes shim)."
    )
}

fn bundled_cli_candidates(dir: &Path, allow_parent_resources: bool) -> Vec<PathBuf> {
    let mut rels = if cfg!(windows) {
        vec![
            "agent-doctor-cli.exe",
            "agent-doctor.exe",
            "resources/agent-doctor-cli.exe",
            "resources/agent-doctor.exe",
        ]
    } else {
        vec![
            "agent-doctor-cli",
            "agent-doctor",
            "resources/agent-doctor-cli",
            "resources/agent-doctor",
        ]
    };
    if allow_parent_resources {
        if cfg!(windows) {
            rels.push("../Resources/agent-doctor-cli.exe");
            rels.push("../Resources/agent-doctor.exe");
        } else {
            rels.push("../Resources/agent-doctor-cli");
            rels.push("../Resources/agent-doctor");
        }
    }
    rels.into_iter().map(|rel| dir.join(rel)).collect()
}

fn registered_install_dirs() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        return windows_registered_install_dirs();
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Installer `InstallLocation` is stored with quotes (`"D:\Agent Doctor"`).
#[cfg(any(windows, test))]
pub(crate) fn normalize_registered_install_dir(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim().trim_matches('"').trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn dir_key(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('"').trim();
    let stripped = trimmed
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .unwrap_or_else(|| trimmed.strip_prefix(r"\\?\").unwrap_or(trimmed).to_string());
    stripped
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn parent_dir_key(key: &str) -> Option<&str> {
    key.rfind('\\')
        .filter(|index| *index > 0)
        .map(|index| &key[..index])
}

/// True when `cli` is the bundled binary for `install_dir` (beside the app, or
/// under its `resources` folder).
pub(crate) fn cli_lives_in_install(cli: &Path, install_dir: &Path) -> bool {
    let cli_key = dir_key(&cli.to_string_lossy());
    let install_key = dir_key(&install_dir.to_string_lossy());
    let Some(parent) = parent_dir_key(&cli_key) else {
        return false;
    };
    if parent == install_key {
        return true;
    }
    parent
        .strip_suffix("\\resources")
        .is_some_and(|grand| grand == install_key)
}

/// Prefer the CLI in the running app's directory, then the directory the
/// installer recorded. Ignores newer copies elsewhere.
pub(crate) fn pick_cli_for_install(
    real: &[PathBuf],
    current_exe_dir: Option<&Path>,
    registered_dirs: &[PathBuf],
) -> Option<PathBuf> {
    let best_in = |dir: &Path| {
        let hits: Vec<PathBuf> = real
            .iter()
            .filter(|path| cli_lives_in_install(path, dir))
            .cloned()
            .collect();
        pick_best_agent_doctor_cli(&hits)
    };

    if let Some(dir) = current_exe_dir {
        if let Some(best) = best_in(dir) {
            return Some(best);
        }
        let key = dir_key(&dir.to_string_lossy());
        if let Some(parent) = key.strip_suffix("\\resources") {
            if let Some(best) = best_in(Path::new(parent)) {
                return Some(best);
            }
        }
    }
    for dir in registered_dirs {
        if let Some(best) = best_in(dir) {
            return Some(best);
        }
    }
    None
}

#[cfg(windows)]
fn windows_registered_install_dirs() -> Vec<PathBuf> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let mut dirs = Vec::new();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        push_registered_dir(
            &mut dirs,
            root,
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Agent Doctor",
            Some("InstallLocation"),
        );
        push_registered_dir(&mut dirs, root, r"Software\agentdoctor\Agent Doctor", None);
    }
    dirs
}

#[cfg(windows)]
fn push_registered_dir(
    dirs: &mut Vec<PathBuf>,
    root: windows::Win32::System::Registry::HKEY,
    subkey: &str,
    value: Option<&str>,
) {
    let Some(raw) = reg_sz(root, subkey, value) else {
        return;
    };
    let Some(dir) = normalize_registered_install_dir(&raw) else {
        return;
    };
    if dirs
        .iter()
        .any(|existing| dir_key(&existing.to_string_lossy()) == dir_key(&dir.to_string_lossy()))
    {
        return;
    }
    dirs.push(dir);
}

#[cfg(windows)]
fn reg_sz(
    root: windows::Win32::System::Registry::HKEY,
    subkey: &str,
    value: Option<&str>,
) -> Option<String> {
    use std::ffi::c_void;

    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ};

    let sub: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let owned: Vec<u16> = value
        .unwrap_or("")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let value_ptr = if value.is_some() {
        PCWSTR(owned.as_ptr())
    } else {
        PCWSTR::null()
    };

    let mut size = 0u32;
    // SAFETY: size query with a null buffer. advapi32 writes the byte count.
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(sub.as_ptr()),
            value_ptr,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    // ERROR_SUCCESS or ERROR_MORE_DATA.
    let sized = status.0 == 0 || status.0 == 234;
    if !sized || size < 2 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize / 2) + 1];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: buf is writable and sized from the previous query.
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(sub.as_ptr()),
            value_ptr,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut size),
        )
    };
    if status.0 != 0 {
        return None;
    }
    let chars = (size as usize / 2).saturating_sub(1).min(buf.len());
    let text = String::from_utf16_lossy(&buf[..chars]);
    let trimmed = text.trim_matches('\0').trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_ephemeral_cli_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("cursor-sandbox-cache")
        || s.contains("/T/cargo-target")
        || s.contains("/var/folders/")
        || s.contains("\\AppData\\Local\\Temp\\")
}

fn cli_version_label(path: &Path) -> Option<String> {
    let output = run_output(path, &["--version"], SHORT_PROBE_TIMEOUT).ok()?;
    if !output.status.success() {
        return None;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    crate::version_check::extract_version(&text)
}

fn cmp_version_labels(a: &str, b: &str) -> Ordering {
    let parts = |raw: &str| -> Vec<u64> {
        raw.trim()
            .trim_start_matches(['v', 'V'])
            .split(['.', '-', '+'])
            .map_while(|part| part.parse::<u64>().ok())
            .collect()
    };
    let (left, right) = (parts(a), parts(b));
    for index in 0..left.len().max(right.len()) {
        let l = left.get(index).copied().unwrap_or(0);
        let r = right.get(index).copied().unwrap_or(0);
        match l.cmp(&r) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

fn mtime_or_epoch(path: &Path) -> std::time::SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

fn pick_best_agent_doctor_cli(candidates: &[PathBuf]) -> Option<PathBuf> {
    if candidates.is_empty() {
        return None;
    }
    // Prefer the highest `--version`, then newest mtime. A months-old
    // `~/.local/bin/agent-doctor-cli` must not beat a bundled 0.1.60+ that
    // Codex needs for resources/list (empty list → tools stay in session).
    candidates
        .iter()
        .max_by(|a, b| {
            match (cli_version_label(a), cli_version_label(b)) {
                (Some(va), Some(vb)) => {
                    let ord = cmp_version_labels(&va, &vb);
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                (Some(_), None) => return Ordering::Greater,
                (None, Some(_)) => return Ordering::Less,
                (None, None) => {}
            }
            mtime_or_epoch(a).cmp(&mtime_or_epoch(b))
        })
        .cloned()
}

fn managed_cli_dest() -> Option<PathBuf> {
    let config = crate::adapters::util::config_dir()?;
    let bin_dir = config.join("agent-doctor").join("bin");
    #[cfg(windows)]
    let dest = bin_dir.join("agent-doctor.exe");
    #[cfg(not(windows))]
    let dest = bin_dir.join("agent-doctor");
    Some(dest)
}

fn same_launchable_file(left: &Path, right: &Path) -> bool {
    let l = launchable_path(&left.canonicalize().unwrap_or_else(|_| left.to_path_buf()));
    let r = launchable_path(&right.canonicalize().unwrap_or_else(|_| right.to_path_buf()));
    dir_key(&l.to_string_lossy()) == dir_key(&r.to_string_lossy())
}

fn should_refresh_managed_cli(source: &Path, dest: &Path) -> bool {
    if !dest.is_file() || is_ephemeral_cli_path(source) {
        return true;
    }
    match (cli_version_label(source), cli_version_label(dest)) {
        (Some(src), Some(dst)) => cmp_version_labels(&src, &dst) == Ordering::Greater,
        (Some(_), None) => true,
        _ => mtime_or_epoch(source) > mtime_or_epoch(dest),
    }
}

/// Keep a stable MCP command path under the user config bin.
///
/// Ephemeral Cursor/cargo builds are always copied. A newer resolved CLI also
/// refreshes the managed copy so `~/.codex` does not stay stuck on an old
/// `~/.local/bin/agent-doctor-cli` that rejects Codex resource probes.
pub fn ensure_stable_agent_doctor_cli(resolved: &Path) -> Result<PathBuf> {
    let Some(dest) = managed_cli_dest() else {
        return Ok(resolved.to_path_buf());
    };
    if same_launchable_file(resolved, &dest) {
        return Ok(dest);
    }
    if !should_refresh_managed_cli(resolved, &dest) {
        // Prefer the managed path when it already tracks the same-or-newer CLI,
        // so runtime configs converge on one refreshable command.
        if dest.is_file() {
            return Ok(dest);
        }
        return Ok(resolved.to_path_buf());
    }

    let bin_dir = dest.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(bin_dir).with_context(|| format!("create {}", bin_dir.display()))?;
    fs::copy(resolved, &dest).with_context(|| {
        format!(
            "copy {} → {} for stable MCP command",
            resolved.display(),
            dest.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))?;
    }
    Ok(dest)
}

fn looks_like_desktop_gui(path: &Path) -> bool {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    stem == "agent doctor" || stem == "agent-doctor-desktop"
}

enum CliProbe {
    Real,
    Unrunnable,
    Skip,
}

fn classify_agent_doctor_cli(path: &Path) -> CliProbe {
    if !path.is_file() || looks_like_desktop_gui(path) {
        return CliProbe::Skip;
    }
    // Skip shell wrappers such as `exec hermes -p agent-doctor "$@"`.
    if let Ok(bytes) = fs::read(path) {
        if bytes.starts_with(b"#!") {
            let text = String::from_utf8_lossy(&bytes);
            if text.contains("hermes") {
                return CliProbe::Skip;
            }
        }
    }

    // Identity is `--version` only. Never fall back to `mcp status` — that can
    // open Chrome / wait on CDP and freeze "Apply repair" on Windows.
    let Ok(output) = run_output(path, &["--version"], SHORT_PROBE_TIMEOUT) else {
        return CliProbe::Unrunnable;
    };
    if !output.status.success() {
        return CliProbe::Unrunnable;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    if text.contains("agent-doctor") {
        CliProbe::Real
    } else {
        CliProbe::Skip
    }
}

/// Group configured browser MCP entries by runtime hint.
pub fn browser_configured_runtimes(report: &McpInventoryReport) -> Vec<String> {
    let mut map = BTreeMap::new();
    for item in report.servers.iter().filter(|s| s.is_browser) {
        // Ignore leftover mistaken paths / unknown shared scopes.
        if item.runtime_hint == "shared" {
            continue;
        }
        map.insert(item.runtime_hint.clone(), ());
    }
    map.into_keys().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    use super::super::WorkspaceEntry;

    fn with_temp_home<T>(f: impl FnOnce(&Path) -> T) -> T {
        let temp = tempdir().unwrap();
        crate::adapters::util::with_test_home(temp.path(), || f(temp.path()))
    }

    #[test]
    fn version_labels_prefer_newer_cli() {
        assert_eq!(cmp_version_labels("0.1.61", "0.1.44"), Ordering::Greater);
        assert_eq!(cmp_version_labels("0.1.44", "0.1.61"), Ordering::Less);
        assert_eq!(cmp_version_labels("0.1.60", "0.1.60"), Ordering::Equal);
    }

    #[test]
    fn windows_verbatim_prefix_is_not_a_launchable_command() {
        let path = launchable_path(Path::new(r"\\?\D:\Agent Doctor\agent-doctor.exe"));
        assert_eq!(path, PathBuf::from(r"D:\Agent Doctor\agent-doctor.exe"));
        let (healthy, issue) = assess_command(Some(r"\\?\D:\Agent Doctor\agent-doctor.exe"));
        assert!(!healthy);
        assert!(issue.is_some());
    }

    #[test]
    fn installer_location_strips_quotes() {
        let dir = normalize_registered_install_dir(r#""D:\Agent Doctor""#).unwrap();
        assert_eq!(dir, PathBuf::from(r"D:\Agent Doctor"));
        assert!(normalize_registered_install_dir("  ").is_none());
    }

    #[test]
    fn custom_install_dir_beats_default_location() {
        let custom = PathBuf::from(r"D:\Agent Doctor\agent-doctor-cli.exe");
        let default_loc =
            PathBuf::from(r"C:\Users\Admin\AppData\Local\Agent Doctor\agent-doctor-cli.exe");
        let picked = pick_cli_for_install(
            &[default_loc, custom.clone()],
            Some(Path::new(r"D:\Agent Doctor")),
            &[PathBuf::from(r"C:\Users\Admin\AppData\Local\Agent Doctor")],
        );
        assert_eq!(picked, Some(custom));
    }

    #[test]
    fn registered_custom_dir_used_when_process_is_elsewhere() {
        let custom = PathBuf::from(r"D:\Agent Doctor\resources\agent-doctor-cli.exe");
        let picked = pick_cli_for_install(
            &[
                PathBuf::from(r"C:\Users\Admin\AppData\Roaming\npm\agent-doctor.exe"),
                custom.clone(),
            ],
            Some(Path::new(r"C:\Windows\System32")),
            &[PathBuf::from(r"D:\Agent Doctor")],
        );
        assert_eq!(picked, Some(custom));
    }

    #[test]
    fn inventories_hermes_and_openclaw_browser() {
        with_temp_home(|home| {
            let hermes = home.join(".hermes/profiles/demo");
            fs::create_dir_all(&hermes).unwrap();
            fs::write(
                hermes.join("config.yaml"),
                "mcp_servers:\n  browser:\n    command: agent-doctor\n    args: [mcp, browser]\n",
            )
            .unwrap();

            let openclaw = home.join(".openclaw");
            fs::create_dir_all(&openclaw).unwrap();
            fs::write(
                openclaw.join("openclaw.json"),
                r#"{"mcp":{"servers":{"browser":{"command":"agent-doctor","args":["mcp","browser"]}}}}"#,
            )
            .unwrap();

            let project = home.join("proj");
            fs::create_dir_all(&project).unwrap();
            let mut workspaces = BTreeMap::new();
            workspaces.insert(
                "demo".into(),
                WorkspaceEntry {
                    path: project,
                    hermes_profile: "demo".into(),
                    codex_home: home.join("codex"),
                    openclaw_agent_id: "demo".into(),
                    openclaw_workspace: home.join("oc"),
                },
            );
            let doc = WorkspacesDocument {
                active: Some("demo".into()),
                workspaces,
            };
            let report = list_mcp_inventory_with_doc(&doc);
            assert!(report
                .servers
                .iter()
                .any(|s| s.runtime_hint == "hermes" && s.is_browser));
            assert!(report
                .servers
                .iter()
                .any(|s| s.runtime_hint == "openclaw" && s.is_browser));
        });
    }

    #[test]
    fn lists_project_mcp_servers_and_flags_missing_binary() {
        with_temp_home(|home| {
            let project = home.join("proj");
            fs::create_dir_all(&project).unwrap();
            fs::write(
                project.join(".mcp.json"),
                r#"{"mcpServers":{"browser":{"command":"agent-doctor","args":["mcp","browser"]},"broken":{"command":"/no/such/mcp-bin"}}}"#,
            )
            .unwrap();

            let mut workspaces = BTreeMap::new();
            workspaces.insert(
                "demo".into(),
                WorkspaceEntry {
                    path: project,
                    hermes_profile: "default".into(),
                    codex_home: home.join("codex"),
                    openclaw_agent_id: "demo".into(),
                    openclaw_workspace: home.join("oc"),
                },
            );
            let doc = WorkspacesDocument {
                active: Some("demo".into()),
                workspaces,
            };

            let report = list_mcp_inventory_with_doc(&doc);
            let broken = report
                .servers
                .iter()
                .find(|s| s.name == "broken" && s.scope == "project")
                .unwrap();
            assert!(!broken.healthy);
            assert_eq!(broken.runtime_hint, "claude-code");
            let browser = report
                .servers
                .iter()
                .find(|s| s.name == "browser" && s.runtime_hint == "claude-code")
                .unwrap();
            assert!(browser.is_browser);
            assert!(report.browser_configured);
            assert!(browser_configured_runtimes(&report).contains(&"claude-code".to_string()));
        });
    }

    #[test]
    fn inventories_global_codex_without_active_workspace() {
        with_temp_home(|home| {
            let codex = home.join(".codex");
            fs::create_dir_all(&codex).unwrap();
            fs::write(
                codex.join("config.toml"),
                r#"
# keep me
[mcp_servers.browser]
command = "agent-doctor"
args = ["mcp", "browser"]
"#,
            )
            .unwrap();

            let doc = WorkspacesDocument {
                active: None,
                workspaces: BTreeMap::new(),
            };
            let report = list_mcp_inventory_with_doc(&doc);
            let browser = report
                .servers
                .iter()
                .find(|s| s.runtime_hint == "codex" && s.name == "browser")
                .expect("global codex browser");
            assert!(browser.is_browser);
            assert!(browser_configured_runtimes(&report).contains(&"codex".to_string()));
        });
    }

    #[test]
    fn lists_codex_toml_mcp_servers() {
        with_temp_home(|home| {
            let project = home.join("proj");
            let codex_home = home.join("codex-home");
            fs::create_dir_all(&project).unwrap();
            fs::create_dir_all(&codex_home).unwrap();
            fs::write(
                codex_home.join("config.toml"),
                r#"
model = "gpt-5"
[mcp_servers.browser]
command = "agent-doctor"
args = ["mcp", "browser", "--port", "9222"]
"#,
            )
            .unwrap();

            let mut workspaces = BTreeMap::new();
            workspaces.insert(
                "demo".into(),
                WorkspaceEntry {
                    path: project,
                    hermes_profile: "default".into(),
                    codex_home,
                    openclaw_agent_id: "demo".into(),
                    openclaw_workspace: home.join("oc"),
                },
            );
            let doc = WorkspacesDocument {
                active: Some("demo".into()),
                workspaces,
            };

            let report = list_mcp_inventory_with_doc(&doc);
            let codex = report
                .servers
                .iter()
                .find(|s| s.runtime_hint == "codex" && s.name == "browser")
                .expect("codex browser MCP");
            assert!(codex.is_browser);
            assert!(browser_configured_runtimes(&report).contains(&"codex".to_string()));
        });
    }

    #[test]
    fn list_mcp_inventory_reuses_cache_without_rereading_project() {
        with_temp_home(|home| {
            invalidate_mcp_inventory_cache();
            // Must go through `config_dir()` (test-home redirected). Never write
            // the real ~/Library/Application Support/agent-doctor/workspaces.yaml.
            let config = crate::adapters::util::config_dir().expect("config dir");
            let ws_dir = config.join("agent-doctor");
            fs::create_dir_all(&ws_dir).unwrap();
            let project = home.join("proj");
            fs::create_dir_all(&project).unwrap();
            fs::write(
                project.join(".mcp.json"),
                r#"{"mcpServers":{"browser":{"command":"agent-doctor","args":["mcp","browser"]}}}"#,
            )
            .unwrap();
            fs::write(
                ws_dir.join("workspaces.yaml"),
                format!(
                    "active: demo\nworkspaces:\n  demo:\n    path: {}\n    hermes_profile: demo\n    codex_home: {}\n    openclaw_agent_id: demo\n    openclaw_workspace: {}\n",
                    project.display(),
                    home.join("codex").display(),
                    home.join("oc").display()
                ),
            )
            .unwrap();

            let first = list_mcp_inventory().expect("first inventory");
            assert!(
                first
                    .servers
                    .iter()
                    .any(|s| s.scope == "project" && s.name == "browser"),
                "expected project browser from disk"
            );

            fs::write(project.join(".mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
            let second = list_mcp_inventory().expect("cached inventory");
            assert!(
                second
                    .servers
                    .iter()
                    .any(|s| s.scope == "project" && s.name == "browser"),
                "second call must reuse cache and not re-stat the project file"
            );

            invalidate_mcp_inventory_cache();
            let third = list_mcp_inventory().expect("fresh inventory");
            assert!(
                !third
                    .servers
                    .iter()
                    .any(|s| s.scope == "project" && s.name == "browser"),
                "after invalidate, empty mcpServers should drop the project browser"
            );
        });
    }
}
