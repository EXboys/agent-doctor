use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::browser::BrowserDiscovery;

/// Options for configuring an MCP server entry in a runtime's config.
#[derive(Debug, Clone)]
pub struct McpConfigureOptions {
    /// The runtime to configure (codex, claude-code)
    pub runtime: String,
    /// Port for the Chrome debugging endpoint
    pub port: u16,
    /// The agent-doctor binary path (used for the MCP server command)
    pub binary: PathBuf,
    /// The project/workspace path to write .mcp.json into
    pub project_path: Option<PathBuf>,
}

/// A single MCP server entry for .mcp.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerEntry {
    pub command: String,
    pub args: Vec<String>,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

/// Find the MCP servers config path for a given runtime.
pub fn mcp_servers_path(runtime: &str, project_path: Option<&PathBuf>) -> Result<PathBuf> {
    match runtime {
        "claude-code" | "claude" => {
            let path = home_dir()
                .context("Cannot find home directory")?
                .join(".claude/settings.json");
            Ok(path)
        }
        "codex" => {
            if let Some(project) = project_path {
                Ok(project.join(".mcp.json"))
            } else {
                Ok(std::env::current_dir()
                    .context("Cannot determine current directory")?
                    .join(".mcp.json"))
            }
        }
        _ => anyhow::bail!("Unsupported runtime: {runtime}. Supported: codex, claude-code"),
    }
}

/// Read the existing MCP server entries from a config file.
pub fn read_mcp_servers(config_path: &PathBuf) -> Result<Value> {
    if !config_path.exists() {
        return Ok(json!({}));
    }

    let content = fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;

    let is_json = config_path
        .extension()
        .map(|e| e == "json")
        .unwrap_or(false);

    if is_json {
        let parsed: Value = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", config_path.display()))?;
        Ok(parsed.get("mcpServers").cloned().unwrap_or(json!({})))
    } else {
        anyhow::bail!("Unsupported config format: {}", config_path.display());
    }
}

/// Write MCP server entries to a config file.
pub fn write_mcp_servers(config_path: &PathBuf, mcp_servers: &Value) -> Result<()> {
    let is_json = config_path
        .extension()
        .map(|e| e == "json")
        .unwrap_or(false);

    if !is_json {
        anyhow::bail!("Unsupported config format: {}", config_path.display());
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    if config_path.exists() {
        let content = fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;
        let mut doc: Value = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", config_path.display()))?;

        if let Some(obj) = doc.as_object_mut() {
            obj.insert("mcpServers".to_string(), mcp_servers.clone());
        }

        let updated = serde_json::to_string_pretty(&doc)?;
        fs::write(config_path, format!("{updated}\n"))
            .with_context(|| format!("Failed to write {}", config_path.display()))?;
    } else {
        let doc = json!({
            "mcpServers": mcp_servers
        });
        let content = serde_json::to_string_pretty(&doc)?;
        fs::write(config_path, format!("{content}\n"))
            .with_context(|| format!("Failed to write {}", config_path.display()))?;
    }

    Ok(())
}

/// Configure the browser MCP server for a given runtime.
pub fn configure_for(_discovery: &BrowserDiscovery, options: &McpConfigureOptions) -> Result<()> {
    let config_path = mcp_servers_path(&options.runtime, options.project_path.as_ref())?;
    let mut servers = read_mcp_servers(&config_path)?;

    let entry = json!({
        "command": options.binary.to_string_lossy().to_string(),
        "args": [
            "mcp",
            "browser",
            "--port", options.port.to_string(),
        ]
    });

    if let Some(obj) = servers.as_object_mut() {
        obj.insert("browser".to_string(), entry);
    } else {
        servers = json!({ "browser": entry });
    }

    write_mcp_servers(&config_path, &servers)?;

    Ok(())
}

/// Generate the MCP configuration JSON snippet for display/export.
pub fn generate_config_snippet(binary: &PathBuf, port: u16) -> Value {
    json!({
        "mcpServers": {
            "browser": {
                "command": binary.to_string_lossy().to_string(),
                "args": [
                    "mcp",
                    "browser",
                    "--port", port.to_string(),
                ]
            }
        }
    })
}
