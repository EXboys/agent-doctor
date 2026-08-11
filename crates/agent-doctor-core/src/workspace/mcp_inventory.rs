//! Local MCP server inventory: project + Claude configs, path health, browser entry.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::adapters::util::{find_all_binaries, find_binary, home_join};

use super::{load_workspaces, WorkspacesDocument};

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
    let doc = load_workspaces().unwrap_or_default();
    Ok(list_mcp_inventory_with_doc(&doc))
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

/// Attach Browser MCP probe checks for Claude Code / Codex.
pub fn probe_browser_mcp_for_runtime(runtime_id: &str, checks: &mut Vec<crate::probe::ProbeCheck>) {
    use crate::probe::{ProbeCheck, ProbeSeverity, ProbeStatus};
    use crate::repair::SensitivityLevel;

    if runtime_id != "claude-code" && runtime_id != "codex" {
        return;
    }

    let inventory = list_mcp_inventory_with_doc(&load_workspaces().unwrap_or_default());
    let browser = inventory
        .servers
        .iter()
        .find(|item| item.is_browser && item.runtime_hint == runtime_id);

    match browser {
        None => {
            checks.push(ProbeCheck::new(
                "mcp.browser.configured",
                "Browser MCP configured",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                format!(
                    "no browser MCP entry for {runtime_id}; write via repair or `agent-doctor mcp configure {runtime_id}`"
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
                format!("browser MCP present at {}", item.config_path),
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
                format!("browser MCP present at {}", item.config_path),
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

/// Resolve the real `agent-doctor` CLI binary used as the MCP server command.
///
/// Skips the common Hermes workspace shim at `~/.local/bin/agent-doctor`
/// (`exec hermes -p agent-doctor …`), which is not the Agent Doctor CLI.
pub fn resolve_agent_doctor_binary() -> Result<PathBuf> {
    let mut candidates = Vec::new();
    candidates.extend(find_all_binaries("agent-doctor-cli"));
    candidates.extend(find_all_binaries("agent-doctor"));

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
        ] {
            candidates.push(root.join(rel));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Same folder as the desktop binary (Contents/MacOS on macOS).
            candidates.push(dir.join("agent-doctor-cli"));
            candidates.push(dir.join("agent-doctor"));
            #[cfg(windows)]
            {
                candidates.push(dir.join("agent-doctor-cli.exe"));
                candidates.push(dir.join("agent-doctor.exe"));
            }
            // Tauri bundle.resources → Contents/Resources/ (macOS) or resources/ beside exe.
            candidates.push(dir.join("../Resources/agent-doctor-cli"));
            candidates.push(dir.join("../Resources/agent-doctor"));
            candidates.push(dir.join("resources/agent-doctor-cli"));
            candidates.push(dir.join("resources/agent-doctor"));
            #[cfg(windows)]
            {
                candidates.push(dir.join("../Resources/agent-doctor-cli.exe"));
                candidates.push(dir.join("resources/agent-doctor-cli.exe"));
            }
        }
    }

    for path in candidates {
        if is_real_agent_doctor_cli(&path) {
            return Ok(path.canonicalize().unwrap_or(path));
        }
    }

    anyhow::bail!(
        "Could not find the Agent Doctor CLI. Install with `cargo install --path cli`, \
         or place `agent-doctor-cli` on PATH (note: ~/.local/bin/agent-doctor may be a Hermes shim)."
    )
}

fn is_real_agent_doctor_cli(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    // Skip shell wrappers such as `exec hermes -p agent-doctor "$@"`.
    if let Ok(bytes) = fs::read(path) {
        if bytes.starts_with(b"#!") {
            let text = String::from_utf8_lossy(&bytes);
            if text.contains("hermes") {
                return false;
            }
        }
    }

    // Prefer a cheap --version probe (works even when Chrome discovery fails in
    // GUI / sandboxed environments). Fall back to mcp status for older builds.
    if let Ok(output) = std::process::Command::new(path)
        .args(["--version"])
        .output()
    {
        if output.status.success() {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if text.to_ascii_lowercase().contains("agent-doctor") {
                return true;
            }
        }
    }

    let Ok(output) = std::process::Command::new(path)
        .args(["mcp", "status", "--json"])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.contains("chrome_found") || stdout.contains("cdp_connected")
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
        let previous = std::env::var_os("HOME");
        // SAFETY: test-only HOME override, restored after the closure.
        unsafe { std::env::set_var("HOME", temp.path()) };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(temp.path())));
        match previous {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
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
}
