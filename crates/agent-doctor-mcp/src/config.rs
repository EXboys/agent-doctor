use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use toml::Value as TomlValue;

use crate::browser::BrowserDiscovery;

/// Options for configuring an MCP server entry in a runtime's config.
#[derive(Debug, Clone)]
pub struct McpConfigureOptions {
    /// The runtime to configure (codex, claude-code, hermes, openclaw, deepseek-harness)
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
    /// Hermes home (`HERMES_HOME` / workspace profile). Defaults to `~/.hermes`.
    pub hermes_home: Option<PathBuf>,
    /// OpenClaw agent workspace dir. When set, also mirrors Browser MCP into
    /// `<openclaw_workspace>/.mcp.json` (workspace-aware inventory / isolation narrative).
    /// OpenClaw runtime still loads `~/.openclaw/openclaw.json`.
    pub openclaw_workspace: Option<PathBuf>,
}

/// Build `agent-doctor mcp browser …` args. Headed (visible UI) is the default.
/// Path string another program can pass to CreateProcess.
///
/// Windows `canonicalize` / `current_exe` often return `\\?\D:\...`. That form
/// exists for our own checks, but Codex reports `program not found` when it
/// tries to start it. Strip the prefix at the write boundary so no caller can
/// persist it. Paths that truly need the prefix (longer than MAX_PATH) cannot
/// be started by Codex either way.
pub fn command_path_for_runtime(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = raw.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    raw.into_owned()
}

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
            args.push(command_path_for_runtime(dir));
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

fn resolve_hermes_home(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Ok(from_env) = std::env::var("HERMES_HOME") {
        let path = PathBuf::from(from_env.trim());
        if !path.as_os_str().is_empty() {
            return Ok(path);
        }
    }
    Ok(agent_doctor_core::adapters::hermes_root())
}

fn resolve_openclaw_config_path() -> Result<PathBuf> {
    home_dir()
        .map(|home| home.join(".openclaw/openclaw.json"))
        .context("Cannot find home directory for OpenClaw config")
}

fn resolve_dsh_home() -> Result<PathBuf> {
    if let Ok(from_env) = std::env::var("DSH_HOME") {
        let path = PathBuf::from(from_env.trim());
        if !path.as_os_str().is_empty() {
            return Ok(path);
        }
    }
    home_dir()
        .map(|home| home.join(".dsh"))
        .context("Cannot resolve DeepSeek Harness home (set DSH_HOME)")
}

/// Find the MCP servers config path for a given runtime.
///
/// - Claude Code with `project_path`: `<project>/.mcp.json` (workspace isolation)
/// - Claude Code without project: `~/.claude.json` (user-scope MCP)
/// - Codex: `$CODEX_HOME/config.toml` (`[mcp_servers.*]` TOML) — **not** project `.mcp.json`
/// - Hermes: `$HERMES_HOME/config.yaml` (`mcp_servers.*`)
/// - OpenClaw: workspace `.mcp.json` when `openclaw_workspace` is set (inventory + mirror);
///   otherwise `~/.openclaw/openclaw.json` (`mcp.servers.*`)
pub fn mcp_servers_path(
    runtime: &str,
    project_path: Option<&Path>,
    codex_home: Option<&Path>,
    hermes_home: Option<&Path>,
) -> Result<PathBuf> {
    mcp_servers_path_with_openclaw(runtime, project_path, codex_home, hermes_home, None)
}

/// Like [`mcp_servers_path`], with optional OpenClaw workspace mirror path.
pub fn mcp_servers_path_with_openclaw(
    runtime: &str,
    project_path: Option<&Path>,
    codex_home: Option<&Path>,
    hermes_home: Option<&Path>,
    openclaw_workspace: Option<&Path>,
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
        "hermes" => Ok(resolve_hermes_home(hermes_home)?.join("config.yaml")),
        "openclaw" => {
            if let Some(ws) = openclaw_workspace {
                return Ok(ws.join(".mcp.json"));
            }
            resolve_openclaw_config_path()
        }
        "deepseek-harness" | "dsh" => Ok(resolve_dsh_home()?.join("cordis.patch.yml")),
        _ => anyhow::bail!(
            "Unsupported runtime: {runtime}. Supported: codex, claude-code, hermes, openclaw, deepseek-harness"
        ),
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

/// Codex configs that must list the browser command.
///
/// `primary` is the workspace `CODEX_HOME/config.toml` Ask launches. The
/// installed Codex app still reads `~/.codex/config.toml` unless that variable
/// is set, so a custom install path has to be written there too.
pub(crate) fn codex_browser_config_paths(primary: &Path) -> Vec<PathBuf> {
    let mut paths = vec![primary.to_path_buf()];
    if let Some(global) = home_dir().map(|home| home.join(".codex").join("config.toml")) {
        if !same_config_path(&global, primary) {
            paths.push(global);
        }
    }
    paths
}

fn same_config_path(left: &Path, right: &Path) -> bool {
    dir_key(&left.to_string_lossy()) == dir_key(&right.to_string_lossy())
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

/// Configure the browser MCP server for a given runtime.
pub fn configure_for(_discovery: &BrowserDiscovery, options: &McpConfigureOptions) -> Result<()> {
    let config_path = mcp_servers_path_with_openclaw(
        &options.runtime,
        options.project_path.as_deref(),
        options.codex_home.as_deref(),
        options.hermes_home.as_deref(),
        options.openclaw_workspace.as_deref(),
    )?;
    let command = command_path_for_runtime(&options.binary);
    let args = browser_mcp_args(
        options.port,
        options.headless,
        options.user_data_dir.as_deref(),
        options.profile_directory.as_deref(),
    );

    match options.runtime.as_str() {
        "codex" => {
            // Workspace CODEX_HOME is what Ask launches. The Codex app the
            // person opens themselves reads ~/.codex. Write both, with the
            // CLI from the install directory they actually chose.
            for path in codex_browser_config_paths(&config_path) {
                write_mcp_servers_toml(&path, "browser", &command, &args)?;
            }
        }
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
        "hermes" => write_hermes_browser_mcp(&config_path, &command, &args)?,
        "deepseek-harness" | "dsh" => write_dsh_browser_mcp(&config_path, &command, &args)?,
        "openclaw" => {
            // Runtime loads global openclaw.json; also mirror into workspace .mcp.json
            // so inventory / Doctor narrative matches Claude project isolation.
            let global = resolve_openclaw_config_path()?;
            write_openclaw_browser_mcp(&global, &command, &args)?;
            if options.openclaw_workspace.is_some() {
                write_claude_style_browser_mcp(&config_path, &command, &args)?;
            }
        }
        other => anyhow::bail!("Unsupported runtime: {other}"),
    }

    Ok(())
}

/// Write one stdio MCP server (`computer`, and the same shape as browser).
#[allow(clippy::too_many_arguments)]
pub fn configure_stdio_mcp(
    runtime: &str,
    server_name: &str,
    command: &str,
    args: &[String],
    project_path: Option<&Path>,
    codex_home: Option<&Path>,
    hermes_home: Option<&Path>,
    openclaw_workspace: Option<&Path>,
) -> Result<()> {
    let config_path = mcp_servers_path_with_openclaw(
        runtime,
        project_path,
        codex_home,
        hermes_home,
        openclaw_workspace,
    )?;
    match runtime {
        // Only the workspace CODEX_HOME. `computer` reaches the screen through
        // the running desktop app, so it would fail in the user's own Codex.
        "codex" => write_mcp_servers_toml(&config_path, server_name, command, args)?,
        "claude-code" | "claude" if server_name == "computer" => {
            let path = agent_doctor_core::claude_computer_mcp_config_path()
                .context("no config directory for the private computer MCP file")?;
            write_claude_style_mcp(&path, server_name, command, args)?;
            if let Some(project) = project_path {
                let _ = agent_doctor_core::forget_project_computer_mcp(project);
            }
        }
        "claude-code" | "claude" => {
            write_claude_style_mcp(&config_path, server_name, command, args)?;
        }
        "hermes" => write_hermes_stdio_mcp(&config_path, server_name, command, args)?,
        "deepseek-harness" | "dsh" => {
            write_dsh_stdio_mcp(&config_path, server_name, command, args)?
        }
        "openclaw" => {
            let global = resolve_openclaw_config_path()?;
            write_openclaw_stdio_mcp(&global, server_name, command, args)?;
            if let Some(workspace) = openclaw_workspace {
                // `computer` only works while this app is open. Leave it out of
                // the workspace file the person's own OpenClaw would read.
                if server_name == "computer" {
                    let _ = agent_doctor_core::forget_project_computer_mcp(workspace);
                } else {
                    write_claude_style_mcp(
                        &workspace.join(".mcp.json"),
                        server_name,
                        command,
                        args,
                    )?;
                }
            }
        }
        other => anyhow::bail!("Unsupported runtime: {other}"),
    }
    Ok(())
}

fn write_claude_style_browser_mcp(
    config_path: &Path,
    command: &str,
    args: &[String],
) -> Result<()> {
    write_claude_style_mcp(config_path, "browser", command, args)
}

fn write_claude_style_mcp(
    config_path: &Path,
    name: &str,
    command: &str,
    args: &[String],
) -> Result<()> {
    let mut servers = read_mcp_servers(config_path)?;
    let entry = json!({ "command": command, "args": args });
    if let Some(obj) = servers.as_object_mut() {
        obj.insert(name.to_string(), entry);
    } else {
        servers = json!({ name: entry });
    }
    write_mcp_servers(config_path, &servers)
}

const DSH_MCP_CLIENT: &str = "@deepseek-ai/dsh-mcp-client";
const DSH_BROWSER_PLUGIN_ID: &str = "mcp-browser";

fn dsh_browser_plugin_entry(
    server_name: &str,
    plugin_id: &str,
    command: &str,
    args: &[String],
) -> serde_yaml::Value {
    use serde_yaml::{Mapping, Value as YamlValue};

    let mut config = Mapping::new();
    config.insert(
        YamlValue::String("serverName".into()),
        YamlValue::String(server_name.into()),
    );
    config.insert(
        YamlValue::String("transport".into()),
        YamlValue::String("stdio".into()),
    );
    config.insert(
        YamlValue::String("command".into()),
        YamlValue::String(command.into()),
    );
    config.insert(
        YamlValue::String("args".into()),
        YamlValue::Sequence(
            args.iter()
                .map(|arg| YamlValue::String(arg.clone()))
                .collect(),
        ),
    );
    let mut item = Mapping::new();
    item.insert(
        YamlValue::String("id".into()),
        YamlValue::String(plugin_id.into()),
    );
    item.insert(
        YamlValue::String("name".into()),
        YamlValue::String(DSH_MCP_CLIENT.into()),
    );
    item.insert(
        YamlValue::String("config".into()),
        YamlValue::Mapping(config),
    );
    YamlValue::Mapping(item)
}

fn is_dsh_browser_plugin(value: &serde_yaml::Value, plugin_id: &str, server_name: &str) -> bool {
    let Some(map) = value.as_mapping() else {
        return false;
    };
    if map
        .get(serde_yaml::Value::String("id".into()))
        .and_then(serde_yaml::Value::as_str)
        == Some(plugin_id)
    {
        return true;
    }
    let name = map
        .get(serde_yaml::Value::String("name".into()))
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or("");
    if !name.contains("dsh-mcp-client") {
        return false;
    }
    map.get(serde_yaml::Value::String("config".into()))
        .and_then(|config| config.get("serverName"))
        .and_then(serde_yaml::Value::as_str)
        == Some(server_name)
}

/// The profile Ask boots. `--dump-config` composes exactly this tree without
/// starting it, so it is the composition the wire has to satisfy.
const DSH_ASK_PROFILE: &str = "acp";

/// Read the patch list dsh applies: a top-level YAML array.
///
/// Older Agent Doctor builds wrote bare plugin entries (dsh warns
/// `patch: entry … not found` and skips them) or a `plugins:` mapping (which
/// dsh refuses to boot). Both are migrated here instead of being overwritten.
/// Anything unrecognizable fails the wire: a hand-edited file must never be
/// clobbered by a blind rewrite.
fn read_dsh_patch_list(config_path: &Path) -> Result<Vec<serde_yaml::Value>> {
    if !config_path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;
    if yaml_has_tag(&raw) {
        anyhow::bail!(
            "{} uses YAML tags (like !!js) that Agent Doctor cannot rewrite without dropping them; edit that file by hand",
            config_path.display()
        );
    }
    let root: serde_yaml::Value = serde_yaml::from_str(&raw).with_context(|| {
        format!(
            "{} is not valid YAML; fix or remove that file by hand so Agent Doctor does not overwrite your edits",
            config_path.display()
        )
    })?;
    match root {
        serde_yaml::Value::Sequence(seq) => Ok(seq),
        serde_yaml::Value::Mapping(map) => {
            let plugins = map
                .get(serde_yaml::Value::String("plugins".into()))
                .and_then(serde_yaml::Value::as_sequence)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "{} must be a top-level YAML array of patch entries; Agent Doctor will not rewrite an unknown mapping",
                        config_path.display()
                    )
                })?;
            Ok(plugins.clone())
        }
        serde_yaml::Value::Null => Ok(Vec::new()),
        _ => anyhow::bail!(
            "{} must be a top-level YAML array of patch entries",
            config_path.display()
        ),
    }
}

fn dsh_insert_patch(entry: serde_yaml::Value) -> serde_yaml::Value {
    let mut map = serde_yaml::Mapping::new();
    map.insert(
        serde_yaml::Value::String("insert".into()),
        serde_yaml::Value::Sequence(vec![entry]),
    );
    serde_yaml::Value::Mapping(map)
}

/// serde_yaml parses `!!js`-style YAML.org tags into plain strings and drops
/// the tag on rewrite — `!!js process.env.X` would silently become the literal
/// text `process.env.X`, changing what dsh executes. A file using any tag in
/// tag position must be edited by hand instead.
fn yaml_has_tag(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.iter().enumerate().any(|(index, &byte)| {
        if byte != b'!' {
            return false;
        }
        match index {
            0 => true,
            _ => matches!(
                bytes[index - 1],
                b' ' | b'\t' | b'\n' | b'-' | b':' | b'[' | b','
            ),
        }
    })
}

type DshVerifyKey = (String, Option<SystemTime>, Option<u64>, String, String);

fn dsh_verify_cache() -> &'static Mutex<HashMap<DshVerifyKey, Result<(), String>>> {
    static CACHE: OnceLock<Mutex<HashMap<DshVerifyKey, Result<(), String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// After writing, ask dsh itself whether the entry reached the composed tree.
///
/// A patch dsh silently skips (wrong shape, unknown file) used to leave the
/// wire reporting a green light while the model saw no tools at all. Best
/// effort: without a dsh binary there is nothing to ask, and the result is
/// cached per file content so repeated wiring costs nothing.
fn verify_dsh_patch_loads(config_path: &Path, plugin_id: &str) -> Result<(), String> {
    let Some(dsh) = agent_doctor_core::adapters::util::find_binary(
        agent_doctor_core::adapters::DEEPSEEK_HARNESS_CLI,
    ) else {
        return Ok(());
    };
    let content = fs::read_to_string(config_path).unwrap_or_default();
    let meta = fs::metadata(&dsh).ok();
    let key: DshVerifyKey = (
        dsh.to_string_lossy().into_owned(),
        meta.as_ref().and_then(|meta| meta.modified().ok()),
        meta.as_ref().map(|meta| meta.len()),
        content,
        plugin_id.to_string(),
    );
    let cache = dsh_verify_cache();
    if let Some(hit) = cache
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .get(&key)
    {
        return hit.clone();
    }
    let result = run_dsh_dump_config(&dsh, plugin_id);
    cache
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .insert(key, result.clone());
    result
}

fn run_dsh_dump_config(dsh: &Path, plugin_id: &str) -> Result<(), String> {
    let mut child = Command::new(dsh)
        .args(["--profile", DSH_ASK_PROFILE, "--dump-config"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to start dsh: {err}"))?;
    // The pipes must drain while dsh runs or a large tree can deadlock both
    // processes, so the readers own their pipe and the watchdog only waits.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_handle = thread::spawn(move || read_pipe_text(stdout_pipe));
    let stderr_handle = thread::spawn(move || read_pipe_text(stderr_pipe));
    const DSH_VERIFY_TIMEOUT: Duration = Duration::from_secs(30);
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < DSH_VERIFY_TIMEOUT => {
                thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();
                return Err("dsh --dump-config timed out".into());
            }
            Err(err) => return Err(format!("failed to run dsh: {err}")),
        }
    };
    let Some(status) = status else {
        return Err("dsh --dump-config did not run".into());
    };
    let tree = stdout_handle.join().unwrap_or_default();
    let stderr_text = stderr_handle.join().unwrap_or_default();
    if !status.success() {
        return Err(format!("exit {}: {}", status, excerpt(&stderr_text)));
    }
    if !tree.contains(&format!("id: {plugin_id}")) {
        return Err(format!(
            "the composed dsh config has no '{plugin_id}' entry; dsh said: {}",
            excerpt(&stderr_text)
        ));
    }
    Ok(())
}

fn read_pipe_text<R: Read + Send + 'static>(pipe: Option<R>) -> String {
    pipe.and_then(|mut pipe| {
        let mut text = String::new();
        pipe.read_to_string(&mut text).ok()?;
        Some(text)
    })
    .unwrap_or_default()
}

fn excerpt(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= 300 {
        return trimmed.to_string();
    }
    trimmed.chars().take(300).collect()
}

fn write_dsh_browser_mcp(config_path: &Path, command: &str, args: &[String]) -> Result<()> {
    write_dsh_plugin_mcp(config_path, "browser", DSH_BROWSER_PLUGIN_ID, command, args)
}

fn write_dsh_stdio_mcp(
    config_path: &Path,
    server_name: &str,
    command: &str,
    args: &[String],
) -> Result<()> {
    let plugin_id = format!("mcp-{server_name}");
    write_dsh_plugin_mcp(config_path, server_name, &plugin_id, command, args)
}

fn write_dsh_plugin_mcp(
    config_path: &Path,
    server_name: &str,
    plugin_id: &str,
    command: &str,
    args: &[String],
) -> Result<()> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let mut patches = read_dsh_patch_list(config_path)?;
    let managed = |item: &serde_yaml::Value| is_dsh_browser_plugin(item, plugin_id, server_name);

    // Drop this server wherever a previous wire left it: inside an insert
    // list (the shape dsh applies) or bare at the top level (the pre-insert
    // shape dsh warns about and skips). Rewritten in place so repeated wires
    // never duplicate an entry — dsh accepts duplicate insert ids silently.
    let mut slot: Option<usize> = None;
    for (index, patch) in patches.iter_mut().enumerate() {
        let Some(map) = patch.as_mapping_mut() else {
            continue;
        };
        if map.contains_key(serde_yaml::Value::String("id".into())) {
            continue; // group-targeted insert: our entries belong at the root
        }
        let Some(list) = map
            .get_mut(serde_yaml::Value::String("insert".into()))
            .and_then(serde_yaml::Value::as_sequence_mut)
        else {
            continue;
        };
        let before = list.len();
        list.retain(|item| !managed(item));
        if list.len() != before && slot.is_none() {
            slot = Some(index);
        }
    }
    patches.retain(|item| !managed(item));

    let entry = dsh_browser_plugin_entry(server_name, plugin_id, command, args);
    match slot {
        Some(index) => {
            let list = patches[index]
                .as_mapping_mut()
                .and_then(|map| map.get_mut(serde_yaml::Value::String("insert".into())))
                .and_then(serde_yaml::Value::as_sequence_mut);
            let Some(list) = list else {
                anyhow::bail!(
                    "the insert slot in {} vanished while rewriting",
                    config_path.display()
                );
            };
            list.push(entry);
        }
        None => patches.push(dsh_insert_patch(entry)),
    }

    let rendered = serde_yaml::to_string(&serde_yaml::Value::Sequence(patches))
        .with_context(|| format!("Failed to serialize {}", config_path.display()))?;
    fs::write(config_path, rendered)
        .with_context(|| format!("Failed to write {}", config_path.display()))?;

    verify_dsh_patch_loads(config_path, plugin_id).map_err(|message| {
        anyhow::anyhow!(
            "wrote {} but dsh rejected it: {message}",
            config_path.display()
        )
    })
}

fn write_hermes_browser_mcp(config_path: &Path, command: &str, args: &[String]) -> Result<()> {
    write_hermes_stdio_mcp(config_path, "browser", command, args)
}

fn write_hermes_stdio_mcp(
    config_path: &Path,
    name: &str,
    command: &str,
    args: &[String],
) -> Result<()> {
    use serde_yaml::{Mapping, Value as YamlValue};

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let mut root: YamlValue = if config_path.exists() {
        let raw = fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;
        serde_yaml::from_str(&raw).unwrap_or_else(|_| YamlValue::Mapping(Mapping::new()))
    } else {
        YamlValue::Mapping(Mapping::new())
    };

    let root_map = root
        .as_mapping_mut()
        .context("Hermes config.yaml root must be a mapping")?;
    let servers_key = YamlValue::String("mcp_servers".into());
    if !root_map.contains_key(&servers_key) {
        root_map.insert(servers_key.clone(), YamlValue::Mapping(Mapping::new()));
    }
    let servers = root_map
        .get_mut(&servers_key)
        .and_then(YamlValue::as_mapping_mut)
        .context("mcp_servers must be a mapping")?;

    let mut entry = Mapping::new();
    entry.insert(
        YamlValue::String("command".into()),
        YamlValue::String(command.to_string()),
    );
    let args_seq: Vec<YamlValue> = args.iter().map(|a| YamlValue::String(a.clone())).collect();
    entry.insert(
        YamlValue::String("args".into()),
        YamlValue::Sequence(args_seq),
    );
    servers.insert(YamlValue::String(name.into()), YamlValue::Mapping(entry));

    fs::write(config_path, serde_yaml::to_string(&root)?)
        .with_context(|| format!("Failed to write {}", config_path.display()))?;
    Ok(())
}

fn write_openclaw_browser_mcp(config_path: &Path, command: &str, args: &[String]) -> Result<()> {
    write_openclaw_stdio_mcp(config_path, "browser", command, args)
}

fn write_openclaw_stdio_mcp(
    config_path: &Path,
    name: &str,
    command: &str,
    args: &[String],
) -> Result<()> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let mut doc: Value = if config_path.exists() {
        let raw = fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;
        serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse {}", config_path.display()))?
    } else {
        json!({})
    };

    let entry = json!({ "command": command, "args": args });
    let root = doc
        .as_object_mut()
        .context("openclaw.json root must be an object")?;

    // Official shape: mcp.servers.browser
    let mcp = root.entry("mcp".to_string()).or_insert_with(|| json!({}));
    let mcp_obj = mcp
        .as_object_mut()
        .context("openclaw.json mcp must be an object")?;
    let servers = mcp_obj
        .entry("servers".to_string())
        .or_insert_with(|| json!({}));
    let servers_obj = servers
        .as_object_mut()
        .context("openclaw.json mcp.servers must be an object")?;
    servers_obj.insert(name.to_string(), entry.clone());

    // Compat: some community docs use top-level mcpServers
    if let Some(legacy) = root.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        legacy.insert(name.to_string(), entry);
    }

    let updated = serde_json::to_string_pretty(&doc)?;
    fs::write(config_path, format!("{updated}\n"))
        .with_context(|| format!("Failed to write {}", config_path.display()))?;
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
    fn runtime_command_drops_windows_verbatim_prefix() {
        assert_eq!(
            command_path_for_runtime(Path::new(r"\\?\D:\Agent Doctor\agent-doctor.exe")),
            r"D:\Agent Doctor\agent-doctor.exe"
        );
        assert_eq!(
            command_path_for_runtime(Path::new(r"\\?\UNC\server\share\agent-doctor.exe")),
            r"\\server\share\agent-doctor.exe"
        );
        assert_eq!(
            command_path_for_runtime(Path::new(r"D:\Agent Doctor\agent-doctor.exe")),
            r"D:\Agent Doctor\agent-doctor.exe"
        );
    }

    #[test]
    fn claude_with_project_uses_mcp_json() {
        let project = PathBuf::from("/tmp/ws-demo");
        let path = mcp_servers_path("claude-code", Some(&project), None, None).unwrap();
        assert_eq!(path, project.join(".mcp.json"));
    }

    #[test]
    fn claude_without_project_uses_global_claude_json() {
        let path = mcp_servers_path("claude-code", None, None, None).unwrap();
        assert!(path.ends_with(".claude.json"));
    }

    #[test]
    fn hermes_uses_config_yaml_under_hermes_home() {
        let home = PathBuf::from("/tmp/hermes-profile");
        let path = mcp_servers_path("hermes", None, None, Some(&home)).unwrap();
        assert_eq!(path, home.join("config.yaml"));
    }

    #[test]
    fn openclaw_uses_openclaw_json() {
        let path = mcp_servers_path("openclaw", None, None, None).unwrap();
        assert!(path.ends_with(".openclaw/openclaw.json"));
    }

    #[test]
    fn openclaw_with_workspace_uses_mcp_json_mirror() {
        let ws = PathBuf::from("/tmp/oc-ws");
        let path = mcp_servers_path_with_openclaw("openclaw", None, None, None, Some(&ws)).unwrap();
        assert_eq!(path, ws.join(".mcp.json"));
    }

    #[test]
    fn hermes_yaml_write_preserves_other_keys() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        fs::write(&path, "model:\n  provider: openai\n").unwrap();
        write_hermes_browser_mcp(
            &path,
            "/bin/agent-doctor",
            &["mcp".into(), "browser".into()],
        )
        .unwrap();
        let rendered = fs::read_to_string(&path).unwrap();
        assert!(rendered.contains("provider: openai"));
        assert!(rendered.contains("mcp_servers:"));
        assert!(rendered.contains("command: /bin/agent-doctor"));
    }

    #[test]
    fn openclaw_json_write_uses_mcp_servers() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("openclaw.json");
        fs::write(&path, r#"{"env":{"vars":{}}}"#).unwrap();
        write_openclaw_browser_mcp(
            &path,
            "/bin/agent-doctor",
            &["mcp".into(), "browser".into()],
        )
        .unwrap();
        let doc: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            doc.pointer("/mcp/servers/browser/command")
                .and_then(|v| v.as_str()),
            Some("/bin/agent-doctor")
        );
        assert!(doc.pointer("/env/vars").is_some());
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

    #[test]
    fn codex_write_includes_user_config_outside_workspace_home() {
        let workspace = PathBuf::from(
            r"C:\Users\Admin\AppData\Roaming\agent-doctor\workspaces\default\codex-home\config.toml",
        );
        let paths = codex_browser_config_paths(&workspace);
        assert!(paths.iter().any(|path| same_config_path(path, &workspace)));
        if home_dir().is_some() {
            assert!(
                paths.iter().any(|path| {
                    let key = dir_key(&path.to_string_lossy());
                    key.ends_with(r"\.codex\config.toml") && !key.contains("workspaces")
                }),
                "standalone Codex reads ~/.codex, not only the workspace home: {paths:?}"
            );
        }
    }

    #[test]
    fn dsh_patch_path_is_under_dsh_home() {
        let path = mcp_servers_path("deepseek-harness", None, None, None).unwrap();
        assert!(path.ends_with("cordis.patch.yml"));
    }

    /// The dsh patch tests point `DSH_HOME` at a temp home so the writer's
    /// `--dump-config` verification composes the temp file, not the
    /// developer's real config. One lock: env vars are process-wide.
    fn dsh_env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// `verify_dsh_patch_loads` shells out to a real dsh when one is
    /// installed. Under `DSH_HOME=<temp>` that end-to-end check runs on dev
    /// machines and is silently skipped where dsh is absent.
    #[test]
    fn dsh_patch_write_upserts_browser_plugin_in_insert_form() {
        let _env = dsh_env_lock();
        let dir = tempdir().unwrap();
        std::env::set_var("DSH_HOME", dir.path());
        let path = dir.path().join("cordis.patch.yml");
        fs::write(&path, "- id: keep-me\n  name: other\n").unwrap();
        write_dsh_browser_mcp(
            &path,
            "/bin/agent-doctor",
            &["mcp".into(), "browser".into()],
        )
        .unwrap();
        write_dsh_browser_mcp(
            &path,
            "/bin/agent-doctor",
            &[
                "mcp".into(),
                "browser".into(),
                "--port".into(),
                "9222".into(),
            ],
        )
        .unwrap();
        std::env::remove_var("DSH_HOME");
        let rendered = fs::read_to_string(&path).unwrap();
        assert!(rendered.contains("keep-me"), "{rendered}");
        assert_eq!(rendered.matches("id: mcp-browser").count(), 1, "{rendered}");
        assert_eq!(rendered.matches("insert:").count(), 1, "{rendered}");
        assert!(
            rendered.contains("@deepseek-ai/dsh-mcp-client"),
            "{rendered}"
        );
        assert!(rendered.contains("serverName: browser"), "{rendered}");
        assert!(rendered.contains("--port"), "{rendered}");
    }

    #[test]
    fn dsh_patch_write_migrates_legacy_bare_entries_for_the_server_it_wires() {
        let _env = dsh_env_lock();
        let dir = tempdir().unwrap();
        std::env::set_var("DSH_HOME", dir.path());
        let path = dir.path().join("cordis.patch.yml");
        // The shape earlier Agent Doctor builds left on user machines: bare
        // plugin entries, which dsh warns about and skips.
        fs::write(
            &path,
            "- id: mcp-browser\n  name: '@deepseek-ai/dsh-mcp-client'\n  config:\n    serverName: browser\n    transport: stdio\n    command: /bin/agent-doctor\n    args: [mcp, browser]\n- id: mcp-computer\n  name: '@deepseek-ai/dsh-mcp-client'\n  config:\n    serverName: computer\n    transport: stdio\n    command: /bin/agent-doctor\n    args: [mcp, computer]\n",
        )
        .unwrap();
        write_dsh_stdio_mcp(
            &path,
            "computer",
            "/bin/agent-doctor",
            &["mcp".into(), "computer".into()],
        )
        .unwrap();
        std::env::remove_var("DSH_HOME");
        let rendered = fs::read_to_string(&path).unwrap();
        assert_eq!(
            rendered.matches("id: mcp-computer").count(),
            1,
            "{rendered}"
        );
        assert!(rendered.contains("insert:"), "{rendered}");
        // The browser entry migrates when its own wire runs, not before.
        assert!(rendered.contains("serverName: browser"), "{rendered}");
    }

    #[test]
    fn dsh_patch_write_migrates_the_legacy_plugins_mapping_root() {
        let _env = dsh_env_lock();
        let dir = tempdir().unwrap();
        std::env::set_var("DSH_HOME", dir.path());
        let path = dir.path().join("cordis.patch.yml");
        fs::write(
            &path,
            "plugins:\n- id: keep-me\n  name: other\n- id: mcp-computer\n  name: '@deepseek-ai/dsh-mcp-client'\n  config:\n    serverName: computer\n    command: /old\n",
        )
        .unwrap();
        write_dsh_stdio_mcp(
            &path,
            "computer",
            "/bin/agent-doctor",
            &["mcp".into(), "computer".into()],
        )
        .unwrap();
        std::env::remove_var("DSH_HOME");
        let rendered = fs::read_to_string(&path).unwrap();
        // dsh refuses a mapping root, so the file must come back a sequence.
        assert!(rendered.starts_with('-'), "{rendered}");
        assert!(rendered.contains("keep-me"), "{rendered}");
        assert!(rendered.contains("insert:"), "{rendered}");
        assert!(!rendered.contains("plugins:"), "{rendered}");
    }

    #[test]
    fn dsh_patch_write_preserves_unrelated_patches() {
        let _env = dsh_env_lock();
        let dir = tempdir().unwrap();
        std::env::set_var("DSH_HOME", dir.path());
        let path = dir.path().join("cordis.patch.yml");
        fs::write(
            &path,
            "- id: keep-me\n  name: other\n- insert:\n  - id: mcp-github\n    name: '@deepseek-ai/dsh-mcp-client'\n    config:\n      serverName: github\n      transport: stdio\n      command: npx\n",
        )
        .unwrap();
        write_dsh_stdio_mcp(
            &path,
            "computer",
            "/bin/agent-doctor",
            &["mcp".into(), "computer".into()],
        )
        .unwrap();
        std::env::remove_var("DSH_HOME");
        let rendered = fs::read_to_string(&path).unwrap();
        assert!(rendered.contains("keep-me"), "{rendered}");
        assert!(rendered.contains("serverName: github"), "{rendered}");
        assert!(rendered.contains("command: npx"), "{rendered}");
        assert_eq!(
            rendered.matches("id: mcp-computer").count(),
            1,
            "{rendered}"
        );
    }

    #[test]
    fn dsh_patch_write_fails_without_overwriting_an_unreadable_file() {
        let _env = dsh_env_lock();
        let dir = tempdir().unwrap();
        let path = dir.path().join("cordis.patch.yml");
        fs::write(&path, ":::: not yaml").unwrap();
        let result = write_dsh_stdio_mcp(
            &path,
            "computer",
            "/bin/agent-doctor",
            &["mcp".into(), "computer".into()],
        );
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), ":::: not yaml");
    }

    #[test]
    fn dsh_patch_write_refuses_an_unknown_mapping_root() {
        let _env = dsh_env_lock();
        let dir = tempdir().unwrap();
        let path = dir.path().join("cordis.patch.yml");
        fs::write(&path, "some: other meaning\n").unwrap();
        let result = write_dsh_stdio_mcp(
            &path,
            "computer",
            "/bin/agent-doctor",
            &["mcp".into(), "computer".into()],
        );
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "some: other meaning\n");
    }

    #[test]
    fn dsh_patch_write_fails_cleanly_when_the_file_uses_tags() {
        let _env = dsh_env_lock();
        let dir = tempdir().unwrap();
        let path = dir.path().join("cordis.patch.yml");
        let seed = "- insert:\n  - id: user-plugin\n    name: '@deepseek-ai/dsh-mcp-client'\n    config:\n      env:\n        TOKEN: !!js process.env.TOKEN\n";
        fs::write(&path, seed).unwrap();
        let result = write_dsh_stdio_mcp(
            &path,
            "computer",
            "/bin/agent-doctor",
            &["mcp".into(), "computer".into()],
        );
        assert!(result.is_err());
        // The tag would be lost on rewrite, so the file must stay untouched.
        assert_eq!(fs::read_to_string(&path).unwrap(), seed);
    }

    /// The wiring is only truthful when a dropped patch is reported. With a
    /// `DSH_HOME` that does not contain the file just written, dsh composes a
    /// tree without the entry and the wire must fail instead of going green.
    #[test]
    fn dsh_wire_reports_when_dsh_drops_the_entry() {
        let Some(_) = agent_doctor_core::adapters::util::find_binary(
            agent_doctor_core::adapters::DEEPSEEK_HARNESS_CLI,
        ) else {
            return;
        };
        let _env = dsh_env_lock();
        let home = tempdir().unwrap();
        let elsewhere = tempdir().unwrap();
        std::env::set_var("DSH_HOME", home.path());
        let path = elsewhere.path().join("cordis.patch.yml");
        let result = write_dsh_stdio_mcp(
            &path,
            "computer",
            "/bin/agent-doctor",
            &["mcp".into(), "computer".into()],
        );
        std::env::remove_var("DSH_HOME");
        let message = result
            .expect_err("dsh should not see this file")
            .to_string();
        assert!(message.contains("dsh rejected"), "{message}");
    }
}
