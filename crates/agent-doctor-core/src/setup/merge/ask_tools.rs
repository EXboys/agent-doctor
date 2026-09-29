use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result as AnyhowResult;
use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;

use crate::adapters::util::home_dir;

/// Drop Codex tools whose program file is already gone.
///
/// Codex starts every `[mcp_servers.*]` entry when a message is sent. A path
/// left behind by ChatGPT, an old Codex install, or a removed app then shows
/// up as a failure on that message. Names like `agent-doctor` stay, because
/// those are looked up when the tool actually starts.
pub fn drop_unreachable_codex_mcp_servers(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw = fs::read_to_string(path)?;
    let mut doc = raw.parse::<toml_edit::DocumentMut>()?;
    let Some(servers) = doc
        .get_mut("mcp_servers")
        .and_then(|item| item.as_table_mut())
    else {
        return Ok(false);
    };
    let stale: Vec<String> = servers
        .iter()
        .filter_map(|(name, entry)| {
            let command = entry
                .as_table()
                .and_then(|table| table.get("command"))
                .and_then(|value| value.as_str())
                .unwrap_or("");
            command_path_missing(command).then(|| name.to_string())
        })
        .collect();
    if stale.is_empty() {
        return Ok(false);
    }
    for name in &stale {
        servers.remove(name);
    }
    fs::write(path, doc.to_string())?;
    Ok(true)
}

/// True when `command` is a file path and that file is not there.
/// A bare program name is left alone.
pub(crate) fn command_path_missing(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }
    let looks_like_path =
        Path::new(command).is_absolute() || command.contains('/') || command.contains('\\');
    looks_like_path && !Path::new(command).is_file()
}

pub(crate) struct AskToolRoots {
    pub(crate) home: PathBuf,
    pub(crate) cwd: PathBuf,
    pub(crate) codex_homes: Vec<PathBuf>,
    pub(crate) hermes_configs: Vec<PathBuf>,
    pub(crate) dsh_patches: Vec<PathBuf>,
    pub(crate) extra_mcp_json: Vec<PathBuf>,
}

/// Remove tools whose program file is gone, from every config Ask is about to start.
///
/// Claude, Codex, Hermes, OpenClaw, and DeepSeek Harness all launch their tool
/// list when a message is sent. This runs first, so a missing program never
/// becomes an error on that message.
pub fn drop_unreachable_ask_tools(cwd: &Path) {
    let mut roots = AskToolRoots {
        home: home_dir(),
        cwd: cwd.to_path_buf(),
        codex_homes: Vec::new(),
        hermes_configs: Vec::new(),
        dsh_patches: Vec::new(),
        extra_mcp_json: Vec::new(),
    };
    if let Ok(doc) = crate::workspace::load_workspaces() {
        if let Some(entry) = doc
            .active
            .as_ref()
            .and_then(|name| doc.workspaces.get(name))
        {
            roots.codex_homes.push(entry.codex_home.clone());
            roots.extra_mcp_json.push(entry.path.join(".mcp.json"));
            roots
                .extra_mcp_json
                .push(entry.openclaw_workspace.join(".mcp.json"));
            roots.hermes_configs.push(
                home_dir()
                    .join(".hermes/profiles")
                    .join(&entry.hermes_profile)
                    .join("config.yaml"),
            );
        }
    }
    let overlay = crate::prompt_session::env::collect_overlay_env();
    if let Some(home) = overlay.get("CODEX_HOME") {
        roots.codex_homes.push(PathBuf::from(home));
    }
    if let Some(home) = overlay.get("HERMES_HOME") {
        roots
            .hermes_configs
            .push(PathBuf::from(home).join("config.yaml"));
    }
    if let Some(home) = overlay.get("DSH_HOME") {
        roots
            .dsh_patches
            .push(PathBuf::from(home).join("cordis.patch.yml"));
    }
    drop_unreachable_ask_tools_in(&roots);
}

pub(crate) fn drop_unreachable_ask_tools_in(roots: &AskToolRoots) {
    let _ = drop_unreachable_codex_mcp_servers(&roots.home.join(".codex/config.toml"));
    let _ = drop_unreachable_codex_mcp_servers(&roots.cwd.join(".codex").join("config.toml"));
    let _ = drop_unreachable_json_mcp_servers(&roots.cwd.join(".mcp.json"));
    let _ = drop_unreachable_json_mcp_servers(&roots.home.join(".claude.json"));
    let _ = drop_unreachable_json_mcp_servers(&roots.home.join(".claude/settings.json"));
    let _ = drop_unreachable_yaml_mcp_servers(&roots.home.join(".hermes/config.yaml"));
    let _ = drop_unreachable_json_mcp_servers(&roots.home.join(".openclaw/openclaw.json"));
    let _ = drop_unreachable_dsh_mcp_plugins(&roots.home.join(".dsh/cordis.patch.yml"));
    for home in &roots.codex_homes {
        let _ = drop_unreachable_codex_mcp_servers(&home.join("config.toml"));
    }
    for path in &roots.hermes_configs {
        let _ = drop_unreachable_yaml_mcp_servers(path);
    }
    for path in &roots.dsh_patches {
        let _ = drop_unreachable_dsh_mcp_plugins(path);
    }
    for path in &roots.extra_mcp_json {
        let _ = drop_unreachable_json_mcp_servers(path);
    }
}

/// Claude, OpenClaw, and project `.mcp.json` store tools under `mcpServers`
/// and/or `mcp.servers`.
pub fn drop_unreachable_json_mcp_servers(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw = fs::read_to_string(path)?;
    let mut doc: JsonValue = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let mut changed = false;
    if let Some(map) = doc.get_mut("mcpServers").and_then(JsonValue::as_object_mut) {
        changed |= prune_json_server_map(map);
    }
    if let Some(map) = doc
        .pointer_mut("/mcp/servers")
        .and_then(JsonValue::as_object_mut)
    {
        changed |= prune_json_server_map(map);
    }
    if !changed {
        return Ok(false);
    }
    let rendered = serde_json::to_string_pretty(&doc)?;
    fs::write(path, format!("{rendered}\n"))?;
    Ok(true)
}

pub(crate) fn prune_json_server_map(map: &mut serde_json::Map<String, JsonValue>) -> bool {
    let stale: Vec<String> = map
        .iter()
        .filter(|(_, entry)| json_command_missing(entry))
        .map(|(name, _)| name.clone())
        .collect();
    if stale.is_empty() {
        return false;
    }
    for name in stale {
        map.remove(&name);
    }
    true
}

pub(crate) fn json_command_missing(entry: &JsonValue) -> bool {
    entry
        .get("command")
        .and_then(JsonValue::as_str)
        .is_some_and(command_path_missing)
}

/// Hermes `config.yaml` stores tools under `mcp_servers`.
pub fn drop_unreachable_yaml_mcp_servers(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw = fs::read_to_string(path)?;
    let mut root: YamlValue = match serde_yaml::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let Some(servers) = root
        .get_mut("mcp_servers")
        .and_then(YamlValue::as_mapping_mut)
    else {
        return Ok(false);
    };
    let stale: Vec<YamlValue> = servers
        .iter()
        .filter_map(|(name, entry)| {
            let command = entry.get("command").and_then(YamlValue::as_str)?;
            command_path_missing(command).then(|| name.clone())
        })
        .collect();
    if stale.is_empty() {
        return Ok(false);
    }
    for name in stale {
        servers.remove(&name);
    }
    fs::write(path, serde_yaml::to_string(&root)?)?;
    Ok(true)
}

/// DeepSeek Harness lists MCP plugins in `cordis.patch.yml`.
pub fn drop_unreachable_dsh_mcp_plugins(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw = fs::read_to_string(path)?;
    let mut root: YamlValue = match serde_yaml::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let changed = match &mut root {
        YamlValue::Sequence(seq) => prune_dsh_plugin_seq(seq),
        YamlValue::Mapping(map) => {
            let key = YamlValue::String("plugins".into());
            match map.get_mut(&key) {
                Some(YamlValue::Sequence(seq)) => prune_dsh_plugin_seq(seq),
                _ => false,
            }
        }
        _ => false,
    };
    if !changed {
        return Ok(false);
    }
    fs::write(path, serde_yaml::to_string(&root)?)?;
    Ok(true)
}

pub(crate) fn prune_dsh_plugin_seq(seq: &mut Vec<YamlValue>) -> bool {
    let before = seq.len();
    seq.retain(|item| !dsh_plugin_command_missing(item));
    seq.len() != before
}

pub(crate) fn dsh_plugin_command_missing(item: &YamlValue) -> bool {
    let Some(map) = item.as_mapping() else {
        return false;
    };
    let name = map
        .get(YamlValue::String("name".into()))
        .and_then(YamlValue::as_str)
        .unwrap_or("");
    let id = map
        .get(YamlValue::String("id".into()))
        .and_then(YamlValue::as_str)
        .unwrap_or("");
    if !name.contains("dsh-mcp-client") && id != "mcp-browser" {
        return false;
    }
    let Some(config) = map.get(YamlValue::String("config".into())) else {
        return false;
    };
    config
        .get("command")
        .and_then(YamlValue::as_str)
        .is_some_and(command_path_missing)
}
