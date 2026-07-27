//! Local MCP server inventory: project + Claude configs, path health, browser entry.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::adapters::util::{find_binary, home_join};

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
        let project_mcp = entry.path.join(".mcp.json");
        servers.extend(read_servers_from_file(&project_mcp, "project", "codex"));
    }

    let settings = home_join(".claude/settings.json");
    servers.extend(read_servers_from_file(
        &settings,
        "claude-global",
        "claude-code",
    ));

    let claude_json = home_join(".claude.json");
    servers.extend(read_servers_from_file(
        &claude_json,
        "claude-json",
        "claude-code",
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

fn read_servers_from_file(path: &Path, scope: &str, runtime_hint: &str) -> Vec<McpInventoryItem> {
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
            let command = entry
                .get("command")
                .and_then(JsonValue::as_str)
                .map(str::to_string);
            let args = entry
                .get("args")
                .and_then(JsonValue::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let is_browser = name == "browser"
                || (args.iter().any(|a| a == "mcp") && args.iter().any(|a| a == "browser"));
            let (healthy, issue) = assess_command(command.as_deref());

            McpInventoryItem {
                name: name.clone(),
                scope: scope.to_string(),
                config_path: path.display().to_string(),
                command,
                args,
                healthy,
                issue,
                is_browser,
                runtime_hint: runtime_hint.to_string(),
            }
        })
        .collect()
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

/// Resolve the `agent-doctor` CLI binary used as the MCP server command.
pub fn resolve_agent_doctor_binary() -> Result<PathBuf> {
    if let Some(path) = find_binary("agent-doctor") {
        return Ok(path);
    }
    let exe = std::env::current_exe()?;
    Ok(exe)
}

/// Group configured browser MCP entries by runtime hint.
pub fn browser_configured_runtimes(report: &McpInventoryReport) -> Vec<String> {
    let mut map = BTreeMap::new();
    for item in report.servers.iter().filter(|s| s.is_browser) {
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

    #[test]
    fn lists_project_mcp_servers_and_flags_missing_binary() {
        let temp = tempdir().unwrap();
        let project = temp.path().join("proj");
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
                codex_home: temp.path().join("codex"),
                openclaw_agent_id: "demo".into(),
                openclaw_workspace: temp.path().join("oc"),
            },
        );
        let doc = WorkspacesDocument {
            active: Some("demo".into()),
            workspaces,
        };

        let report = list_mcp_inventory_with_doc(&doc);
        assert_eq!(report.total, 2);
        assert!(report.browser_configured);
        let broken = report.servers.iter().find(|s| s.name == "broken").unwrap();
        assert!(!broken.healthy);
        let browser = report.servers.iter().find(|s| s.name == "browser").unwrap();
        assert!(browser.is_browser);
    }
}
