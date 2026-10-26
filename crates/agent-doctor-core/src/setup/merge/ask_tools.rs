use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result as AnyhowResult};
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
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
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
    fs::write(path, doc.to_string())
        .with_context(|| format!("failed to write {}", path.display()))?;
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
    drop_unreachable_ask_tools_in(&ask_tool_roots(cwd));
}

fn ask_tool_roots(cwd: &Path) -> AskToolRoots {
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
    let overlay = crate::session_env::collect_overlay_env();
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
    roots
}

struct AskToolFiles {
    codex: Vec<PathBuf>,
    json: Vec<PathBuf>,
    yaml: Vec<PathBuf>,
    dsh: Vec<PathBuf>,
}

fn ask_tool_files(roots: &AskToolRoots) -> AskToolFiles {
    let mut codex = vec![
        roots.home.join(".codex/config.toml"),
        roots.cwd.join(".codex").join("config.toml"),
    ];
    codex.extend(
        roots
            .codex_homes
            .iter()
            .map(|home| home.join("config.toml")),
    );
    let mut json = vec![
        roots.cwd.join(".mcp.json"),
        roots.home.join(".claude.json"),
        roots.home.join(".claude/settings.json"),
        roots.home.join(".openclaw/openclaw.json"),
    ];
    json.extend(roots.extra_mcp_json.iter().cloned());
    let mut yaml = vec![roots.home.join(".hermes/config.yaml")];
    yaml.extend(roots.hermes_configs.iter().cloned());
    let mut dsh = vec![roots.home.join(".dsh/cordis.patch.yml")];
    dsh.extend(roots.dsh_patches.iter().cloned());
    AskToolFiles {
        codex,
        json,
        yaml,
        dsh,
    }
}

pub(crate) fn drop_unreachable_ask_tools_in(roots: &AskToolRoots) {
    let files = ask_tool_files(roots);
    for path in &files.codex {
        let _ = drop_unreachable_codex_mcp_servers(path);
    }
    for path in &files.json {
        let _ = drop_unreachable_json_mcp_servers(path);
    }
    for path in &files.yaml {
        let _ = drop_unreachable_yaml_mcp_servers(path);
    }
    for path in &files.dsh {
        let _ = drop_unreachable_dsh_mcp_plugins(path);
    }
}

/// Point Agent Doctor MCP entries at the GUI launcher before the agent starts.
///
/// Each message starts every configured MCP server. The console CLI opens a
/// terminal when the agent that starts it has no console. Rewriting the command
/// here, on the files this turn is about to launch, keeps the server and its
/// arguments and only changes which executable Windows creates.
pub fn retarget_windows_mcp_hosts(cwd: &Path) {
    if !cfg!(windows) {
        let _ = cwd;
        return;
    }
    let mut roots = ask_tool_roots(cwd);
    if let Some(path) = crate::claude_computer_mcp_config_path() {
        roots.extra_mcp_json.push(path);
    }
    let files = ask_tool_files(&roots);
    for path in &files.codex {
        let _ = retarget_codex_mcp_servers(path);
    }
    for path in &files.json {
        let _ = retarget_json_mcp_servers(path);
    }
    for path in &files.yaml {
        let _ = retarget_yaml_mcp_servers(path);
    }
    for path in &files.dsh {
        let _ = retarget_dsh_mcp_plugins(path);
    }
}

fn retarget_codex_mcp_servers(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut doc = match raw.parse::<toml_edit::DocumentMut>() {
        Ok(doc) => doc,
        Err(_) => return Ok(false),
    };
    let Some(servers) = doc
        .get_mut("mcp_servers")
        .and_then(|item| item.as_table_mut())
    else {
        return Ok(false);
    };
    if !retarget_toml_servers(servers) {
        return Ok(false);
    }
    fs::write(path, doc.to_string())
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

fn retarget_toml_servers(servers: &mut toml_edit::Table) -> bool {
    let mut changed = false;
    for (_, entry) in servers.iter_mut() {
        let Some(table) = entry.as_table_mut() else {
            continue;
        };
        if retarget_toml_command(table) {
            changed = true;
        }
    }
    changed
}

fn retarget_toml_command(table: &mut toml_edit::Table) -> bool {
    let is_mcp = table
        .get("args")
        .and_then(|value| value.as_array())
        .is_some_and(|args| args.iter().any(|item| item.as_str() == Some("mcp")));
    if !is_mcp {
        return false;
    }
    let command = table
        .get("command")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    let Some(host) = prepared_mcp_command(&command) else {
        return false;
    };
    table["command"] = toml_edit::value(host);
    true
}

/// Copy the launcher into place on Windows, then return the command to write.
fn prepared_mcp_command(command: &str) -> Option<String> {
    if cfg!(windows) {
        let _ =
            crate::workspace::mcp_inventory::prepare_windows_mcp_host(Path::new(command.trim()));
    }
    crate::workspace::mcp_inventory::retargeted_mcp_command(command)
}

/// Claude, OpenClaw, and project `.mcp.json` store tools under `mcpServers`
/// and/or `mcp.servers`.
pub fn drop_unreachable_json_mcp_servers(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
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
    fs::write(path, format!("{rendered}\n"))
        .with_context(|| format!("failed to write {}", path.display()))?;
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
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
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
    fs::write(path, serde_yaml::to_string(&root)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

/// DeepSeek Harness lists MCP plugins in `cordis.patch.yml`.
pub fn drop_unreachable_dsh_mcp_plugins(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
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
    fs::write(path, serde_yaml::to_string(&root)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

pub(crate) fn prune_dsh_plugin_seq(seq: &mut Vec<YamlValue>) -> bool {
    let mut changed = prune_dsh_plugin_seq_items(seq);
    // Patch entries wrap plugins in an `insert` list; prune inside them too,
    // and drop the wrapper when pruning emptied it.
    let mut emptied = Vec::new();
    for (index, patch) in seq.iter_mut().enumerate() {
        let Some(list) = patch
            .as_mapping_mut()
            .and_then(|map| map.get_mut(YamlValue::String("insert".into())))
            .and_then(YamlValue::as_sequence_mut)
        else {
            continue;
        };
        if prune_dsh_plugin_seq_items(list) {
            changed = true;
            if list.is_empty() {
                emptied.push(index);
            }
        }
    }
    for index in emptied.into_iter().rev() {
        seq.remove(index);
    }
    changed
}

fn prune_dsh_plugin_seq_items(seq: &mut Vec<YamlValue>) -> bool {
    let before = seq.len();
    seq.retain(|item| !dsh_plugin_command_missing(item));
    seq.len() != before
}

fn retarget_json_mcp_servers(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut doc: JsonValue = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let mut changed = false;
    if let Some(map) = doc.get_mut("mcpServers").and_then(JsonValue::as_object_mut) {
        changed |= retarget_json_server_map(map);
    }
    if let Some(map) = doc
        .pointer_mut("/mcp/servers")
        .and_then(JsonValue::as_object_mut)
    {
        changed |= retarget_json_server_map(map);
    }
    if !changed {
        return Ok(false);
    }
    let rendered = serde_json::to_string_pretty(&doc)?;
    fs::write(path, format!("{rendered}\n"))
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

fn retarget_json_server_map(map: &mut serde_json::Map<String, JsonValue>) -> bool {
    let mut changed = false;
    for entry in map.values_mut() {
        if retarget_json_entry(entry) {
            changed = true;
        }
    }
    changed
}

fn retarget_json_entry(entry: &mut JsonValue) -> bool {
    if !json_args_include_mcp(entry) {
        return false;
    }
    let Some(command) = entry
        .get("command")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
    else {
        return false;
    };
    let Some(host) = prepared_mcp_command(&command) else {
        return false;
    };
    let Some(obj) = entry.as_object_mut() else {
        return false;
    };
    obj.insert("command".into(), JsonValue::String(host));
    true
}

fn json_args_include_mcp(entry: &JsonValue) -> bool {
    entry
        .get("args")
        .and_then(JsonValue::as_array)
        .is_some_and(|args| args.iter().any(|item| item.as_str() == Some("mcp")))
}

fn retarget_yaml_mcp_servers(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
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
    if !retarget_yaml_server_map(servers) {
        return Ok(false);
    }
    fs::write(path, serde_yaml::to_string(&root)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

fn retarget_yaml_server_map(servers: &mut serde_yaml::Mapping) -> bool {
    let keys: Vec<YamlValue> = servers.keys().cloned().collect();
    let mut changed = false;
    for key in keys {
        let Some(entry) = servers.get_mut(&key) else {
            continue;
        };
        if retarget_yaml_mcp_entry(entry) {
            changed = true;
        }
    }
    changed
}

fn retarget_yaml_mcp_entry(entry: &mut YamlValue) -> bool {
    if !yaml_args_include_mcp(entry) {
        return false;
    }
    let Some(command) = entry
        .get("command")
        .and_then(YamlValue::as_str)
        .map(str::to_string)
    else {
        return false;
    };
    let Some(host) = prepared_mcp_command(&command) else {
        return false;
    };
    let Some(map) = entry.as_mapping_mut() else {
        return false;
    };
    map.insert(YamlValue::String("command".into()), YamlValue::String(host));
    true
}

fn yaml_args_include_mcp(entry: &YamlValue) -> bool {
    entry
        .get("args")
        .and_then(YamlValue::as_sequence)
        .is_some_and(|args| args.iter().any(|item| item.as_str() == Some("mcp")))
}

fn retarget_dsh_mcp_plugins(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut root: YamlValue = match serde_yaml::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let changed = match &mut root {
        YamlValue::Sequence(seq) => retarget_dsh_plugin_seq(seq),
        YamlValue::Mapping(map) => {
            let key = YamlValue::String("plugins".into());
            match map.get_mut(&key) {
                Some(YamlValue::Sequence(seq)) => retarget_dsh_plugin_seq(seq),
                _ => false,
            }
        }
        _ => false,
    };
    if !changed {
        return Ok(false);
    }
    fs::write(path, serde_yaml::to_string(&root)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

fn retarget_dsh_plugin_seq(seq: &mut [YamlValue]) -> bool {
    let mut changed = false;
    for item in seq.iter_mut() {
        if retarget_dsh_item(item) {
            changed = true;
        }
        let Some(list) = item
            .as_mapping_mut()
            .and_then(|map| map.get_mut(YamlValue::String("insert".into())))
            .and_then(YamlValue::as_sequence_mut)
        else {
            continue;
        };
        for plugin in list.iter_mut() {
            if retarget_dsh_item(plugin) {
                changed = true;
            }
        }
    }
    changed
}

fn retarget_dsh_item(item: &mut YamlValue) -> bool {
    let (name, id) = {
        let Some(map) = item.as_mapping() else {
            return false;
        };
        let name = map
            .get(YamlValue::String("name".into()))
            .and_then(YamlValue::as_str)
            .unwrap_or("")
            .to_string();
        let id = map
            .get(YamlValue::String("id".into()))
            .and_then(YamlValue::as_str)
            .unwrap_or("")
            .to_string();
        (name, id)
    };
    if !name.contains("dsh-mcp-client") && id != "mcp-browser" && id != "mcp-computer" {
        return false;
    }
    let Some(config) = item
        .as_mapping_mut()
        .and_then(|map| map.get_mut(YamlValue::String("config".into())))
    else {
        return false;
    };
    retarget_yaml_mcp_entry(config)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Vec<YamlValue> {
        serde_yaml::from_str::<YamlValue>(raw)
            .unwrap()
            .as_sequence()
            .unwrap()
            .clone()
    }

    #[test]
    fn prune_keeps_a_patch_whose_insert_list_still_has_live_plugins() {
        // "Live" means the command exists on THIS machine, so point at a file
        // every platform has: this very test binary.
        let live_command = std::env::current_exe().unwrap();
        let live_command = live_command.display().to_string().replace('\\', "/");
        let mut seq = parse(&format!(
            "- insert:\n  - id: dead\n    name: '@deepseek-ai/dsh-mcp-client'\n    config:\n      command: /missing/agent-doctor\n  - id: alive\n    name: '@deepseek-ai/dsh-mcp-client'\n    config:\n      command: {live_command}\n"
        ));
        assert!(prune_dsh_plugin_seq(&mut seq));
        let rendered = serde_yaml::to_string(&YamlValue::Sequence(seq)).unwrap();
        assert!(rendered.contains("id: alive"), "{rendered}");
        assert!(!rendered.contains("id: dead"), "{rendered}");
        assert!(rendered.contains("insert:"), "{rendered}");
    }

    #[test]
    fn prune_drops_a_patch_whose_insert_list_became_empty() {
        let mut seq = parse(
            "- id: keep-me\n  name: other\n- insert:\n  - id: dead\n    name: '@deepseek-ai/dsh-mcp-client'\n    config:\n      command: /missing/agent-doctor\n",
        );
        assert!(prune_dsh_plugin_seq(&mut seq));
        let rendered = serde_yaml::to_string(&YamlValue::Sequence(seq)).unwrap();
        assert!(rendered.contains("keep-me"), "{rendered}");
        assert!(!rendered.contains("insert:"), "{rendered}");
    }

    #[test]
    fn retarget_json_entry_swaps_only_our_mcp_command() {
        let dir = tempfile::tempdir().unwrap();
        let cli = dir.path().join("agent-doctor.exe");
        let host = dir.path().join("agent-doctor-mcp.exe");
        fs::write(&cli, b"cli").unwrap();
        fs::write(&host, b"host").unwrap();
        let mut entry = serde_json::json!({
            "command": cli.display().to_string(),
            "args": ["mcp", "browser", "--port", "9222"]
        });
        assert!(retarget_json_entry(&mut entry));
        assert_eq!(entry["command"], host.display().to_string());
        assert_eq!(entry["args"][1], "browser");

        let mut other = serde_json::json!({
            "command": "npx",
            "args": ["-y", "some-server"]
        });
        assert!(!retarget_json_entry(&mut other));
        assert_eq!(other["command"], "npx");

        let mut foreign = serde_json::json!({
            "command": cli.with_file_name("other-tool.exe").display().to_string(),
            "args": ["mcp", "serve"]
        });
        assert!(!retarget_json_entry(&mut foreign));
    }

    #[test]
    fn prune_leaves_unrelated_patch_entries_alone() {
        let mut seq = parse("- id: disable-telemetry\n  disabled: true\n");
        assert!(!prune_dsh_plugin_seq(&mut seq));
        assert_eq!(seq.len(), 1);
    }
}
