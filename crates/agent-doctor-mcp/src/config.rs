use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use toml::Value as TomlValue;

use crate::browser::BrowserDiscovery;

/// Options for configuring an MCP server entry in a runtime's config.
#[derive(Debug, Clone)]
pub struct McpConfigureOptions {
    /// The runtime to configure (codex, claude-code)
    pub runtime: String,
    /// Port for the Chrome debugging endpoint
    pub port: u16,
    /// When true, Chrome launches without a visible window (`--headless`).
    /// Default is false: show the browser UI.
    pub headless: bool,
    /// Chrome user-data-dir. Default: everyday system Chrome profile.
    pub user_data_dir: Option<PathBuf>,
    /// Chrome profile directory name (`Default`, `Profile 2`, …).
    pub profile_directory: Option<String>,
    /// The agent-doctor binary path (used for the MCP server command)
    pub binary: PathBuf,
    /// The project/workspace path (used for Claude project hints / legacy .mcp.json)
    pub project_path: Option<PathBuf>,
    /// Codex home directory (`CODEX_HOME` / workspace codex-home). Required for Codex.
    pub codex_home: Option<PathBuf>,
}

/// Build `agent-doctor mcp browser …` args. Headed (visible UI) is the default.
pub fn browser_mcp_args(
    port: u16,
    headless: bool,
    user_data_dir: Option<&Path>,
    profile_directory: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "mcp".to_string(),
        "browser".to_string(),
        "--port".to_string(),
        port.to_string(),
    ];
    if headless {
        args.push("--headless".to_string());
    }
    if let Some(dir) = user_data_dir {
        if !dir.as_os_str().is_empty() {
            args.push("--user-data-dir".to_string());
            args.push(dir.display().to_string());
        }
    }
    if let Some(profile) = profile_directory {
        let trimmed = profile.trim();
        if !trimmed.is_empty() {
            args.push("--profile-directory".to_string());
            args.push(trimmed.to_string());
        }
    }
    args
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

fn resolve_codex_home(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Ok(from_env) = std::env::var("CODEX_HOME") {
        let path = PathBuf::from(from_env);
        if !path.as_os_str().is_empty() {
            return Ok(path);
        }
    }
    home_dir()
        .map(|home| home.join(".codex"))
        .context("Cannot resolve Codex home (set CODEX_HOME or pass codex_home)")
}

/// Find the MCP servers config path for a given runtime.
///
/// - Claude Code with `project_path`: `<project>/.mcp.json` (workspace isolation)
/// - Claude Code without project: `~/.claude.json` (user-scope MCP)
/// - Codex: `$CODEX_HOME/config.toml` (`[mcp_servers.*]` TOML) — **not** project `.mcp.json`
pub fn mcp_servers_path(
    runtime: &str,
    project_path: Option<&Path>,
    codex_home: Option<&Path>,
) -> Result<PathBuf> {
    match runtime {
        "claude-code" | "claude" => {
            if let Some(project) = project_path {
                return Ok(project.join(".mcp.json"));
            }
            let path = home_dir()
                .context("Cannot find home directory")?
                .join(".claude.json");
            Ok(path)
        }
        "codex" => Ok(resolve_codex_home(codex_home)?.join("config.toml")),
        _ => anyhow::bail!("Unsupported runtime: {runtime}. Supported: codex, claude-code"),
    }
}

/// Read the existing MCP server entries from a config file.
/// Returns a JSON object map of server-name → entry (command/args).
pub fn read_mcp_servers(config_path: &Path) -> Result<Value> {
    if !config_path.exists() {
        return Ok(json!({}));
    }

    let content = fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;

    if config_path.extension().and_then(|e| e.to_str()) == Some("toml") {
        return read_mcp_servers_toml(&content);
    }

    let parsed: Value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", config_path.display()))?;
    Ok(parsed.get("mcpServers").cloned().unwrap_or(json!({})))
}

fn read_mcp_servers_toml(content: &str) -> Result<Value> {
    let doc: TomlValue = toml::from_str(content).context("Failed to parse Codex config.toml")?;
    let Some(servers) = doc.get("mcp_servers").and_then(TomlValue::as_table) else {
        return Ok(json!({}));
    };

    let mut out = serde_json::Map::new();
    for (name, entry) in servers {
        let Some(table) = entry.as_table() else {
            continue;
        };
        let command = table
            .get("command")
            .and_then(TomlValue::as_str)
            .unwrap_or_default();
        let args = table
            .get("args")
            .and_then(TomlValue::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        out.insert(
            name.clone(),
            json!({
                "command": command,
                "args": args,
            }),
        );
    }
    Ok(Value::Object(out))
}

/// Write MCP server entries to a JSON config file (`mcpServers`).
pub fn write_mcp_servers(config_path: &Path, mcp_servers: &Value) -> Result<()> {
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
        let doc = json!({ "mcpServers": mcp_servers });
        let content = serde_json::to_string_pretty(&doc)?;
        fs::write(config_path, format!("{content}\n"))
            .with_context(|| format!("Failed to write {}", config_path.display()))?;
    }

    Ok(())
}

fn write_mcp_servers_toml(
    config_path: &Path,
    name: &str,
    command: &str,
    args: &[String],
) -> Result<()> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let mut doc = if config_path.exists() {
        let content = fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;
        content
            .parse::<toml_edit::DocumentMut>()
            .with_context(|| format!("Failed to parse {}", config_path.display()))?
    } else {
        toml_edit::DocumentMut::new()
    };

    let servers = doc["mcp_servers"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let servers_table = servers
        .as_table_mut()
        .context("mcp_servers must be a table")?;
    servers_table.set_implicit(true);

    let entry = servers_table
        .entry(name)
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let entry_table = entry
        .as_table_mut()
        .with_context(|| format!("mcp_servers.{name} must be a table"))?;

    entry_table["command"] = toml_edit::value(command);
    let mut args_array = toml_edit::Array::new();
    for arg in args {
        args_array.push(arg.as_str());
    }
    entry_table["args"] = toml_edit::value(args_array);
    // Browser MCP may launch Chrome; give Codex more than the default 10s.
    entry_table["startup_timeout_sec"] = toml_edit::value(60.0);

    fs::write(config_path, doc.to_string())
        .with_context(|| format!("Failed to write {}", config_path.display()))?;
    Ok(())
}

/// Configure the browser MCP server for a given runtime.
pub fn configure_for(_discovery: &BrowserDiscovery, options: &McpConfigureOptions) -> Result<()> {
    let config_path = mcp_servers_path(
        &options.runtime,
        options.project_path.as_deref(),
        options.codex_home.as_deref(),
    )?;
    let command = options.binary.to_string_lossy().to_string();
    let args = browser_mcp_args(
        options.port,
        options.headless,
        options.user_data_dir.as_deref(),
        options.profile_directory.as_deref(),
    );

    match options.runtime.as_str() {
        "codex" => write_mcp_servers_toml(&config_path, "browser", &command, &args)?,
        "claude-code" | "claude" => {
            let mut servers = read_mcp_servers(&config_path)?;
            let entry = json!({ "command": command, "args": args });
            if let Some(obj) = servers.as_object_mut() {
                obj.insert("browser".to_string(), entry);
            } else {
                servers = json!({ "browser": entry });
            }
            write_mcp_servers(&config_path, &servers)?;
        }
        other => anyhow::bail!("Unsupported runtime: {other}"),
    }

    Ok(())
}

/// Generate the MCP configuration JSON snippet for display/export.
pub fn generate_config_snippet(
    binary: &Path,
    port: u16,
    headless: bool,
    user_data_dir: Option<&Path>,
    profile_directory: Option<&str>,
) -> Value {
    json!({
        "mcpServers": {
            "browser": {
                "command": binary.to_string_lossy().to_string(),
                "args": browser_mcp_args(port, headless, user_data_dir, profile_directory),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn claude_with_project_uses_mcp_json() {
        let project = PathBuf::from("/tmp/ws-demo");
        let path = mcp_servers_path("claude-code", Some(&project), None).unwrap();
        assert_eq!(path, project.join(".mcp.json"));
    }

    #[test]
    fn claude_without_project_uses_global_claude_json() {
        let path = mcp_servers_path("claude-code", None, None).unwrap();
        assert!(path.ends_with(".claude.json"));
    }

    #[test]
    fn codex_toml_write_preserves_comments() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"# keep this comment
model = "old"

[features]
# feature note
foo = true
"#,
        )
        .unwrap();

        write_mcp_servers_toml(
            &path,
            "browser",
            "/bin/agent-doctor",
            &["mcp".into(), "browser".into()],
        )
        .unwrap();

        let rendered = fs::read_to_string(&path).unwrap();
        assert!(rendered.contains("# keep this comment"));
        assert!(rendered.contains("# feature note"));
        assert!(rendered.contains("foo = true"));
        assert!(rendered.contains("[mcp_servers.browser]"));
        assert!(rendered.contains("command = \"/bin/agent-doctor\""));
    }
}
