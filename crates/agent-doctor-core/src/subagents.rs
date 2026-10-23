//! One subagent definition per project, written out in each installed assistant's own files.
//!
//! The record under `.agent-doctor/subagents/` is the only text anyone edits.
//! The other files are generated. A later save rewrites them. A delete removes
//! them. They are never read back as the definition.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::adapters::util::{find_binary, hermes_root, home_join};
use crate::adapters::{locate_zcode, DeepSeekHarnessAdapter};
use crate::version_check::{extract_version, version_is_older};
use crate::workspace::backends::workspace_paths_match;
use crate::workspace::load_workspaces;

const MARKER_PREFIX: &str = "agent-doctor-subagent:";
const MAX_NAME_CHARS: usize = 80;
const MAX_DUTY_CHARS: usize = 8_000;
const NAMED_HERMES: &str = "0.22.5";
const DELEGATING_HERMES: &str = "0.21.3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentError {
    NoProject,
    EmptyName,
    EmptyDuty,
    TooLong,
    NotFound,
    SaveFailed,
}

impl std::fmt::Display for SubagentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoProject => "先选定一个项目。",
            Self::EmptyName => "先写一个名称。",
            Self::EmptyDuty => "先写它负责什么。",
            Self::TooLong => "这段话太长了，缩短一点再保存。",
            Self::NotFound => "没有找到这个子助手。",
            Self::SaveFailed => "没有保存成功，再试一次。",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentRecord {
    pub id: String,
    pub name: String,
    pub responsibility: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubagentSaveReport {
    pub subagent: SubagentRecord,
    /// Short codes the page turns into one sentence each.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSubagent {
    id: String,
    name: String,
    responsibility: String,
    slug: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HermesMode {
    Off,
    TooOld,
    Skill,
    Named,
}

struct Homes {
    hermes_root: PathBuf,
    openclaw_config: PathBuf,
    dsh_home: PathBuf,
}

struct Flags {
    claude: bool,
    codex: bool,
    hermes: HermesMode,
    openclaw: bool,
    openclaw_parent: String,
    dsh: bool,
    zcode: bool,
}

pub fn list_project_subagents(project_name: &str) -> Result<Vec<SubagentRecord>, SubagentError> {
    let project = project_dir(project_name)?;
    list_in(&project)
}

pub fn save_project_subagent(
    project_name: &str,
    id: Option<&str>,
    name: &str,
    responsibility: &str,
) -> Result<SubagentSaveReport, SubagentError> {
    let project = project_dir(project_name)?;
    let parent = openclaw_parent_for(&project);
    save_in(
        &project,
        &live_homes(),
        &live_flags(&parent),
        id,
        name,
        responsibility,
    )
}

pub fn delete_project_subagent(project_name: &str, id: &str) -> Result<(), SubagentError> {
    let project = project_dir(project_name)?;
    let stored = read_stored(&store_file(&project, id)).ok_or(SubagentError::NotFound)?;
    forget(&project, &live_homes(), &stored);
    let path = store_file(&project, &stored.id);
    if path.is_file() {
        fs::remove_file(&path).map_err(|_| SubagentError::SaveFailed)?;
    }
    Ok(())
}

fn project_dir(project_name: &str) -> Result<PathBuf, SubagentError> {
    let name = project_name.trim();
    if name.is_empty() {
        return Err(SubagentError::NoProject);
    }
    let doc = load_workspaces().map_err(|_| SubagentError::NoProject)?;
    doc.workspaces
        .get(name)
        .map(|entry| entry.path.clone())
        .ok_or(SubagentError::NoProject)
}

fn openclaw_parent_for(project: &Path) -> String {
    let Ok(doc) = load_workspaces() else {
        return String::new();
    };
    doc.workspaces
        .values()
        .find(|entry| workspace_paths_match(&entry.path, project))
        .map(|entry| entry.openclaw_agent_id.trim().to_string())
        .unwrap_or_default()
}

fn live_homes() -> Homes {
    Homes {
        hermes_root: hermes_root(),
        openclaw_config: home_join(".openclaw/openclaw.json"),
        dsh_home: DeepSeekHarnessAdapter::home(),
    }
}

fn live_flags(openclaw_parent: &str) -> Flags {
    Flags {
        claude: find_binary("claude").is_some(),
        codex: find_binary("codex").is_some(),
        hermes: live_hermes_mode(),
        openclaw: find_binary("openclaw").is_some()
            || home_join(".openclaw/openclaw.json").is_file(),
        openclaw_parent: openclaw_parent.to_string(),
        dsh: find_binary("dsh").is_some(),
        zcode: locate_zcode().is_some() || find_binary("zcode").is_some(),
    }
}

fn live_hermes_mode() -> HermesMode {
    let installed = find_binary("hermes").is_some() || hermes_root().join("config.yaml").is_file();
    if !installed {
        return HermesMode::Off;
    }
    match hermes_version_at(&hermes_root()).as_deref() {
        Some(version) if version_is_older(version, DELEGATING_HERMES) => HermesMode::TooOld,
        Some(version) if !version_is_older(version, NAMED_HERMES) => HermesMode::Named,
        _ => HermesMode::Skill,
    }
}

fn hermes_version_at(root: &Path) -> Option<String> {
    let raw = fs::read_to_string(root.join("hermes-agent/pyproject.toml")).ok()?;
    for line in raw.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("version") else {
            continue;
        };
        let rest = rest.trim().trim_start_matches('=').trim();
        if let Some(version) = extract_version(rest) {
            return Some(version);
        }
    }
    None
}

fn list_in(project: &Path) -> Result<Vec<SubagentRecord>, SubagentError> {
    let dir = store_dir(project);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Some(stored) = read_stored(&path) {
            found.push(stored.record());
        }
    }
    found.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    Ok(found)
}

fn save_in(
    project: &Path,
    homes: &Homes,
    flags: &Flags,
    id: Option<&str>,
    name: &str,
    responsibility: &str,
) -> Result<SubagentSaveReport, SubagentError> {
    let name = name.trim();
    let responsibility = responsibility.trim();
    if name.is_empty() {
        return Err(SubagentError::EmptyName);
    }
    if responsibility.is_empty() {
        return Err(SubagentError::EmptyDuty);
    }
    if name.chars().count() > MAX_NAME_CHARS || responsibility.chars().count() > MAX_DUTY_CHARS {
        return Err(SubagentError::TooLong);
    }
    let id = match id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(existing) => {
            let prior =
                read_stored(&store_file(project, existing)).ok_or(SubagentError::NotFound)?;
            prior.id
        }
        None => new_id(),
    };
    let stored = StoredSubagent {
        slug: format!("role-{id}"),
        id,
        name: name.to_string(),
        responsibility: responsibility.to_string(),
    };
    write_stored(project, &stored)?;
    let notes = project_out(project, homes, flags, &stored);
    Ok(SubagentSaveReport {
        subagent: stored.record(),
        notes,
    })
}

fn write_stored(project: &Path, stored: &StoredSubagent) -> Result<(), SubagentError> {
    let path = store_file(project, &stored.id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| SubagentError::SaveFailed)?;
    }
    let body = serde_json::to_string_pretty(stored).map_err(|_| SubagentError::SaveFailed)?;
    fs::write(&path, format!("{body}\n")).map_err(|_| SubagentError::SaveFailed)
}

/// Write every installed assistant, then drop an older shape we no longer use.
fn project_out(
    project: &Path,
    homes: &Homes,
    flags: &Flags,
    stored: &StoredSubagent,
) -> Vec<String> {
    let mut notes = Vec::new();
    if flags.claude {
        write_text(
            &claude_file(project, stored),
            &claude_markdown(stored),
            &mut notes,
        );
    }
    if flags.codex {
        write_text(
            &codex_file(project, stored),
            &codex_toml(stored),
            &mut notes,
        );
    }
    if flags.zcode {
        write_text(
            &zcode_file(project, stored),
            &zcode_markdown(stored),
            &mut notes,
        );
    }
    match flags.hermes {
        HermesMode::Skill => {
            write_text(
                &hermes_skill_file(homes, stored),
                &hermes_skill(stored),
                &mut notes,
            );
            remove_hermes_profile(homes, stored);
            notes.push("hermes-unnamed".into());
        }
        HermesMode::Named => {
            remove_path(&hermes_skill_file(homes, stored));
            notes.extend(write_hermes_profile(homes, stored));
        }
        HermesMode::TooOld | HermesMode::Off => {}
    }
    if flags.openclaw {
        notes.extend(write_openclaw(homes, flags, stored));
    }
    if flags.dsh {
        notes.extend(write_dsh(homes, stored));
    }
    notes.sort();
    notes.dedup();
    notes
}

fn forget(project: &Path, homes: &Homes, stored: &StoredSubagent) {
    for path in [
        claude_file(project, stored),
        codex_file(project, stored),
        zcode_file(project, stored),
        hermes_skill_file(homes, stored),
    ] {
        if file_is_ours(&path, &stored.id) {
            remove_path(&path);
        }
    }
    remove_hermes_profile(homes, stored);
    remove_openclaw(homes, stored);
    remove_dsh(homes, stored);
}

fn write_text(path: &Path, body: &str, notes: &mut Vec<String>) {
    if file_blocks(path, marker_id_from(body)) {
        notes.push("occupied".into());
        return;
    }
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            notes.push("write-failed".into());
            return;
        }
    }
    if fs::write(path, body).is_err() {
        notes.push("write-failed".into());
    }
}

fn file_blocks(path: &Path, id: &str) -> bool {
    path.is_file() && !id.is_empty() && !file_is_ours(path, id)
}

fn file_is_ours(path: &Path, id: &str) -> bool {
    fs::read_to_string(path)
        .ok()
        .is_some_and(|text| text.contains(&marker(id)))
}

fn marker(id: &str) -> String {
    format!("{MARKER_PREFIX} {id}")
}

fn marker_id_from(body: &str) -> &str {
    body.split(MARKER_PREFIX)
        .nth(1)
        .map(str::trim)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("")
}

fn claude_markdown(stored: &StoredSubagent) -> String {
    format!(
        "---\nname: {slug}\ndescription: {description}\n---\n<!-- {marker} -->\n\n{body}\n",
        slug = stored.slug,
        description = yaml_scalar(&duty_line(stored)),
        marker = marker(&stored.id),
        body = persona(stored),
    )
}

fn codex_toml(stored: &StoredSubagent) -> String {
    format!(
        "name = \"{slug}\"\ndescription = {description}\ndeveloper_instructions = {instructions}\n# {marker}\n",
        slug = stored.slug,
        description = toml_string(&duty_line(stored)),
        instructions = toml_string(&persona(stored)),
        marker = marker(&stored.id),
    )
}

fn zcode_markdown(stored: &StoredSubagent) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\n---\n<!-- {marker} -->\n\n{body}\n",
        name = yaml_scalar(&stored.name),
        description = yaml_scalar(&duty_line(stored)),
        marker = marker(&stored.id),
        body = persona(stored),
    )
}

fn hermes_skill(stored: &StoredSubagent) -> String {
    format!(
        "---\nname: {slug}\ndescription: {description}\n---\n<!-- {marker} -->\n\n遇到这类事，就派一个孩子去做，并把下面这段原样写进任务。\n\n{body}\n",
        slug = stored.slug,
        description = yaml_scalar(&duty_line(stored)),
        marker = marker(&stored.id),
        body = persona(stored),
    )
}

fn duty_line(stored: &StoredSubagent) -> String {
    format!("{}。{}", stored.name.trim(), stored.responsibility.trim())
}

fn persona(stored: &StoredSubagent) -> String {
    format!("{}\n\n{}", stored.name.trim(), stored.responsibility.trim())
}

fn yaml_scalar(text: &str) -> String {
    serde_yaml::to_string(text)
        .unwrap_or_else(|_| format!("\"{}\"", text.replace('"', "\\\"")))
        .trim()
        .trim_start_matches("---")
        .trim()
        .to_string()
}

fn toml_string(text: &str) -> String {
    if !text.contains("'''") {
        return format!("'''\n{text}\n'''");
    }
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

fn write_hermes_profile(homes: &Homes, stored: &StoredSubagent) -> Vec<String> {
    let mut notes = Vec::new();
    let soul = homes
        .hermes_root
        .join("profiles")
        .join(&stored.slug)
        .join("SOUL.md");
    write_text(
        &soul,
        &format!("<!-- {} -->\n\n{}\n", marker(&stored.id), persona(stored)),
        &mut notes,
    );
    if notes.iter().any(|note| note == "occupied") {
        return notes;
    }
    let config = homes.hermes_root.join("config.yaml");
    if !config.is_file() {
        notes.push("hermes-config".into());
        return notes;
    }
    let parent_config = homes.hermes_root.join("config.yaml");
    let profile_config = soul.parent().unwrap_or(Path::new("")).join("config.yaml");
    if !profile_config.exists() {
        let _ = fs::copy(&parent_config, &profile_config);
    }
    match register_hermes_route(&config, stored) {
        Ok(extra) => notes.extend(extra),
        Err(()) => notes.push("hermes-config".into()),
    }
    notes
}

fn register_hermes_route(config: &Path, stored: &StoredSubagent) -> Result<Vec<String>, ()> {
    let raw = fs::read_to_string(config).map_err(|_| ())?;
    let mut root: YamlValue = serde_yaml::from_str(&raw).map_err(|_| ())?;
    let map = root.as_mapping_mut().ok_or(())?;
    let delegation_key = YamlValue::String("delegation".into());
    let delegation = map
        .entry(delegation_key)
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
    let delegation = delegation.as_mapping_mut().ok_or(())?;
    let backend = delegation
        .get(YamlValue::String("backend".into()))
        .and_then(YamlValue::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let mut notes = Vec::new();
    if backend == "model_map" {
        notes.push("hermes-model-map".into());
    } else if backend.is_empty() || backend == "default" {
        delegation.insert(
            YamlValue::String("backend".into()),
            YamlValue::String("profile_routes".into()),
        );
    }
    let profiles = hermes_profile_list(delegation)?;
    profiles
        .retain(|item| item.get("name").and_then(YamlValue::as_str) != Some(stored.slug.as_str()));
    let mut entry = Mapping::new();
    entry.insert(
        YamlValue::String("name".into()),
        YamlValue::String(stored.slug.clone()),
    );
    entry.insert(
        YamlValue::String("description".into()),
        YamlValue::String(duty_line(stored)),
    );
    profiles.push(YamlValue::Mapping(entry));
    backup_once(config);
    let updated = serde_yaml::to_string(&root).map_err(|_| ())?;
    fs::write(config, updated).map_err(|_| ())?;
    Ok(notes)
}

fn hermes_profile_list(delegation: &mut Mapping) -> Result<&mut Vec<YamlValue>, ()> {
    let backends_key = YamlValue::String("backends".into());
    let backends = delegation
        .entry(backends_key)
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
    let backends = backends.as_mapping_mut().ok_or(())?;
    let routes_key = YamlValue::String("profile_routes".into());
    let routes = backends
        .entry(routes_key)
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
    let routes = routes.as_mapping_mut().ok_or(())?;
    let profiles_key = YamlValue::String("profiles".into());
    let profiles = routes
        .entry(profiles_key)
        .or_insert_with(|| YamlValue::Sequence(Vec::new()));
    profiles.as_sequence_mut().ok_or(())
}

fn remove_hermes_profile(homes: &Homes, stored: &StoredSubagent) {
    let soul = homes
        .hermes_root
        .join("profiles")
        .join(&stored.slug)
        .join("SOUL.md");
    if file_is_ours(&soul, &stored.id) {
        remove_path(&soul);
        if let Some(dir) = soul.parent() {
            let config = dir.join("config.yaml");
            if config.is_file() {
                let _ = fs::remove_file(&config);
            }
            let _ = fs::remove_dir(dir);
        }
    }
    let config = homes.hermes_root.join("config.yaml");
    let _ = unregister_hermes_route(&config, &stored.slug);
}

fn unregister_hermes_route(config: &Path, slug: &str) -> Result<(), ()> {
    if !config.is_file() {
        return Ok(());
    }
    let raw = fs::read_to_string(config).map_err(|_| ())?;
    let mut root: YamlValue = serde_yaml::from_str(&raw).map_err(|_| ())?;
    let (removed, empty) = {
        let Some(delegation) = root
            .as_mapping_mut()
            .and_then(|map| map.get_mut(YamlValue::String("delegation".into())))
            .and_then(YamlValue::as_mapping_mut)
        else {
            return Ok(());
        };
        let Ok(profiles) = hermes_profile_list(delegation) else {
            return Ok(());
        };
        let before = profiles.len();
        profiles.retain(|item| item.get("name").and_then(YamlValue::as_str) != Some(slug));
        (profiles.len() != before, profiles.is_empty())
    };
    if !removed {
        return Ok(());
    }
    if empty {
        if let Some(delegation) = root
            .as_mapping_mut()
            .and_then(|map| map.get_mut(YamlValue::String("delegation".into())))
            .and_then(YamlValue::as_mapping_mut)
        {
            if delegation
                .get(YamlValue::String("backend".into()))
                .and_then(YamlValue::as_str)
                == Some("profile_routes")
            {
                delegation.insert(
                    YamlValue::String("backend".into()),
                    YamlValue::String("default".into()),
                );
            }
        }
    }
    let updated = serde_yaml::to_string(&root).map_err(|_| ())?;
    fs::write(config, updated).map_err(|_| ())?;
    Ok(())
}

fn write_openclaw(homes: &Homes, flags: &Flags, stored: &StoredSubagent) -> Vec<String> {
    if !homes.openclaw_config.is_file() {
        return vec!["openclaw-unready".into()];
    }
    let workspace = homes
        .openclaw_config
        .parent()
        .map(|dir| dir.join("agent-doctor-roles").join(&stored.slug))
        .unwrap_or_else(|| PathBuf::from("agent-doctor-roles").join(&stored.slug));
    let raw = match fs::read_to_string(&homes.openclaw_config) {
        Ok(raw) => raw,
        Err(_) => return vec!["openclaw-unready".into()],
    };
    let mut value: JsonValue = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return vec!["openclaw-unready".into()],
    };
    let Some(root) = value.as_object_mut() else {
        return vec!["openclaw-unready".into()];
    };
    let agents = root.entry("agents").or_insert_with(|| json!({}));
    let Some(agents) = agents.as_object_mut() else {
        return vec!["openclaw-unready".into()];
    };
    let entries = agents.entry("entries").or_insert_with(|| json!({}));
    let Some(entries) = entries.as_object_mut() else {
        return vec!["openclaw-unready".into()];
    };
    let agent_id = format!("ad-{}", stored.slug);
    if let Some(existing) = entries.get(&agent_id).and_then(JsonValue::as_object) {
        let owned = existing
            .get("agentDoctorSubagent")
            .and_then(JsonValue::as_str)
            == Some(stored.id.as_str());
        if !owned && !existing.is_empty() {
            return vec!["occupied".into()];
        }
    }
    let mut notes = Vec::new();
    let soul = workspace.join("SOUL.md");
    write_text(
        &soul,
        &format!("<!-- {} -->\n\n{}\n", marker(&stored.id), persona(stored)),
        &mut notes,
    );
    if notes.iter().any(|note| note == "occupied") {
        return notes;
    }
    entries.insert(
        agent_id.clone(),
        json!({
            "name": stored.name,
            "workspace": workspace.display().to_string(),
            "agentDoctorSubagent": stored.id,
        }),
    );
    let parent = flags.openclaw_parent.trim();
    let listed = if parent.is_empty() {
        false
    } else if let Some(parent_entry) = entries.get_mut(parent).and_then(JsonValue::as_object_mut) {
        allow_agent(parent_entry, &agent_id);
        true
    } else {
        false
    };
    if !listed {
        if let Some(defaults) = agents
            .get_mut("defaults")
            .and_then(JsonValue::as_object_mut)
        {
            allow_agent(defaults, &agent_id);
        }
    }
    backup_once(&homes.openclaw_config);
    if serde_json::to_string_pretty(&value)
        .ok()
        .and_then(|body| fs::write(&homes.openclaw_config, format!("{body}\n")).ok())
        .is_none()
    {
        notes.push("write-failed".into());
    }
    notes
}

fn allow_agent(entry: &mut serde_json::Map<String, JsonValue>, agent_id: &str) {
    let subagents = entry.entry("subagents").or_insert_with(|| json!({}));
    let Some(subagents) = subagents.as_object_mut() else {
        return;
    };
    let allow = subagents.entry("allowAgents").or_insert_with(|| json!([]));
    let Some(allow) = allow.as_array_mut() else {
        return;
    };
    if !allow.iter().any(|item| item.as_str() == Some(agent_id)) {
        allow.push(json!(agent_id));
    }
}

fn remove_openclaw(homes: &Homes, stored: &StoredSubagent) {
    let agent_id = format!("ad-{}", stored.slug);
    if homes.openclaw_config.is_file() {
        if let Ok(raw) = fs::read_to_string(&homes.openclaw_config) {
            if let Ok(mut value) = serde_json::from_str::<JsonValue>(&raw) {
                let mut changed = false;
                if let Some(entries) = value
                    .pointer_mut("/agents/entries")
                    .and_then(JsonValue::as_object_mut)
                {
                    if let Some(existing) = entries.get(&agent_id) {
                        if existing
                            .get("agentDoctorSubagent")
                            .and_then(JsonValue::as_str)
                            == Some(stored.id.as_str())
                        {
                            entries.remove(&agent_id);
                            changed = true;
                        }
                    }
                    for entry in entries.values_mut() {
                        changed |= deny_agent(entry, &agent_id);
                    }
                }
                if let Some(defaults) = value.pointer_mut("/agents/defaults") {
                    changed |= deny_agent(defaults, &agent_id);
                }
                if changed {
                    if let Ok(body) = serde_json::to_string_pretty(&value) {
                        let _ = fs::write(&homes.openclaw_config, format!("{body}\n"));
                    }
                }
            }
        }
    }
    if let Some(dir) = homes.openclaw_config.parent() {
        let soul = dir
            .join("agent-doctor-roles")
            .join(&stored.slug)
            .join("SOUL.md");
        if file_is_ours(&soul, &stored.id) {
            remove_path(&soul);
            if let Some(folder) = soul.parent() {
                let _ = fs::remove_dir(folder);
            }
        }
    }
}

fn deny_agent(entry: &mut JsonValue, agent_id: &str) -> bool {
    let Some(allow) = entry
        .pointer_mut("/subagents/allowAgents")
        .and_then(JsonValue::as_array_mut)
    else {
        return false;
    };
    let before = allow.len();
    allow.retain(|item| item.as_str() != Some(agent_id));
    allow.len() != before
}

fn write_dsh(homes: &Homes, stored: &StoredSubagent) -> Vec<String> {
    let dir = homes.dsh_home.join(".agent-presets").join(&stored.slug);
    let mut notes = Vec::new();
    let preset = preset_yaml(stored);
    let cordis = cordis_yaml(stored);
    write_text(&dir.join("preset.yml"), &preset, &mut notes);
    write_text(&dir.join("agent.cordis.yml"), &cordis, &mut notes);
    notes
}

fn preset_yaml(stored: &StoredSubagent) -> String {
    let mut map = Mapping::new();
    map.insert(
        YamlValue::String("name".into()),
        YamlValue::String(stored.name.clone()),
    );
    map.insert(
        YamlValue::String("description".into()),
        YamlValue::String(stored.responsibility.clone()),
    );
    map.insert(
        YamlValue::String("order".into()),
        YamlValue::Number(50.into()),
    );
    let body = serde_yaml::to_string(&YamlValue::Mapping(map)).unwrap_or_default();
    format!("# {}\n{body}", marker(&stored.id))
}

fn cordis_yaml(stored: &StoredSubagent) -> String {
    let mut config = Mapping::new();
    config.insert(
        YamlValue::String("prefix".into()),
        YamlValue::String(persona(stored)),
    );
    config.insert(YamlValue::String("complete".into()), YamlValue::Bool(true));
    config.insert(
        YamlValue::String("includeRuntimeContext".into()),
        YamlValue::Bool(false),
    );
    let mut item = Mapping::new();
    item.insert(
        YamlValue::String("id".into()),
        YamlValue::String("persona".into()),
    );
    item.insert(
        YamlValue::String("name".into()),
        YamlValue::String("@deepseek-ai/dsh-persona".into()),
    );
    item.insert(
        YamlValue::String("config".into()),
        YamlValue::Mapping(config),
    );
    let body = serde_yaml::to_string(&YamlValue::Sequence(vec![YamlValue::Mapping(item)]))
        .unwrap_or_default();
    format!("# {}\n{body}", marker(&stored.id))
}

fn remove_dsh(homes: &Homes, stored: &StoredSubagent) {
    let dir = homes.dsh_home.join(".agent-presets").join(&stored.slug);
    let preset = dir.join("preset.yml");
    if file_is_ours(&preset, &stored.id) {
        remove_path(&preset);
        remove_path(&dir.join("agent.cordis.yml"));
        let _ = fs::remove_dir(&dir);
    }
}

fn backup_once(path: &Path) {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let backup = path.with_file_name(format!("{name}.agent-doctor-bak"));
    if path.is_file() && !backup.exists() {
        let _ = fs::copy(path, backup);
    }
}

fn remove_path(path: &Path) {
    if path.is_file() {
        let _ = fs::remove_file(path);
    }
}

fn store_dir(project: &Path) -> PathBuf {
    project.join(".agent-doctor").join("subagents")
}

fn store_file(project: &Path, id: &str) -> PathBuf {
    store_dir(project).join(format!("{id}.json"))
}

fn claude_file(project: &Path, stored: &StoredSubagent) -> PathBuf {
    project
        .join(".claude")
        .join("agents")
        .join(format!("{}.md", stored.slug))
}

fn codex_file(project: &Path, stored: &StoredSubagent) -> PathBuf {
    project
        .join(".codex")
        .join("agents")
        .join(format!("{}.toml", stored.slug))
}

fn zcode_file(project: &Path, stored: &StoredSubagent) -> PathBuf {
    project
        .join(".zcode")
        .join("agents")
        .join(format!("{}.md", stored.slug))
}

fn hermes_skill_file(homes: &Homes, stored: &StoredSubagent) -> PathBuf {
    homes
        .hermes_root
        .join("skills")
        .join(&stored.slug)
        .join("SKILL.md")
}

fn read_stored(path: &Path) -> Option<StoredSubagent> {
    let raw = fs::read_to_string(path).ok()?;
    let stored: StoredSubagent = serde_json::from_str(&raw).ok()?;
    if stored.id.trim().is_empty() || stored.name.trim().is_empty() {
        return None;
    }
    Some(stored)
}

impl StoredSubagent {
    fn record(&self) -> SubagentRecord {
        SubagentRecord {
            id: self.id.clone(),
            name: self.name.clone(),
            responsibility: self.responsibility.clone(),
        }
    }
}

fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::util::with_test_home;

    fn homes(root: &Path) -> Homes {
        Homes {
            hermes_root: root.join(".hermes"),
            openclaw_config: root.join(".openclaw/openclaw.json"),
            dsh_home: root.join(".dsh"),
        }
    }

    fn flags(hermes: HermesMode) -> Flags {
        Flags {
            claude: true,
            codex: true,
            hermes,
            openclaw: true,
            openclaw_parent: "main".into(),
            dsh: true,
            zcode: true,
        }
    }

    fn sample(id: &str) -> StoredSubagent {
        StoredSubagent {
            id: id.into(),
            name: "审查员".into(),
            responsibility: "看这次改动有没有明显问题。".into(),
            slug: format!("role-{id}"),
        }
    }

    #[test]
    fn one_record_is_the_only_text_that_is_read_back() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("proj");
        fs::create_dir_all(&project).unwrap();
        let root = temp.path().join("home");
        fs::create_dir_all(root.join(".hermes")).unwrap();
        fs::create_dir_all(root.join(".openclaw")).unwrap();
        fs::write(
            root.join(".openclaw/openclaw.json"),
            r#"{"agents":{"entries":{"main":{"name":"main"}}}}"#,
        )
        .unwrap();
        let homes = homes(&root);
        let report = save_in(
            &project,
            &homes,
            &flags(HermesMode::Skill),
            None,
            "审查员",
            "看这次改动有没有明显问题。",
        )
        .unwrap();
        assert!(report.notes.iter().any(|note| note == "hermes-unnamed"));
        let claude = fs::read_to_string(
            project
                .join(".claude/agents")
                .join(format!("role-{}.md", report.subagent.id)),
        )
        .unwrap();
        assert!(claude.contains("看这次改动有没有明显问题。"));
        assert!(claude.contains(&marker(&report.subagent.id)));
        let skill = fs::read_to_string(
            root.join(".hermes/skills")
                .join(format!("role-{}", report.subagent.id))
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(skill.contains("派一个孩子"));
        assert!(skill.contains("看这次改动有没有明显问题。"));
        let openclaw = fs::read_to_string(root.join(".openclaw/openclaw.json")).unwrap();
        assert!(openclaw.contains(&format!("ad-role-{}", report.subagent.id)));
        assert!(openclaw.contains("allowAgents"));
        fs::write(
            project
                .join(".claude/agents")
                .join(format!("role-{}.md", report.subagent.id)),
            "改过的另一套说法\n",
        )
        .unwrap();
        let listed = list_in(&project).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].responsibility, "看这次改动有没有明显问题。");
        assert!(!listed[0].responsibility.contains("另一套"));
    }

    #[test]
    fn named_hermes_registers_a_profile_and_drops_the_skill() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("proj");
        fs::create_dir_all(&project).unwrap();
        let root = temp.path().join("home");
        fs::create_dir_all(root.join(".hermes/hermes-agent")).unwrap();
        fs::write(
            root.join(".hermes/hermes-agent/pyproject.toml"),
            "[project]\nversion = \"0.22.5\"\n",
        )
        .unwrap();
        fs::write(
            root.join(".hermes/config.yaml"),
            "delegation:\n  max_iterations: 50\n",
        )
        .unwrap();
        let homes = homes(&root);
        let stored = sample("abc123");
        let notes = project_out(&project, &homes, &flags(HermesMode::Named), &stored);
        assert!(!notes.iter().any(|note| note == "hermes-unnamed"));
        let soul = fs::read_to_string(root.join(".hermes/profiles/role-abc123/SOUL.md")).unwrap();
        assert!(soul.contains("看这次改动有没有明显问题。"));
        let config = fs::read_to_string(root.join(".hermes/config.yaml")).unwrap();
        assert!(config.contains("profile_routes"));
        assert!(config.contains("role-abc123"));
        assert!(!root.join(".hermes/skills/role-abc123/SKILL.md").exists());
    }

    #[test]
    fn a_file_we_did_not_write_is_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("proj");
        let path = project.join(".claude/agents/role-abc123.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "---\nname: mine\n---\n别人的角色\n").unwrap();
        let homes = homes(&temp.path().join("home"));
        fs::create_dir_all(&homes.hermes_root).unwrap();
        let notes = project_out(&project, &homes, &flags(HermesMode::Off), &sample("abc123"));
        assert!(notes.iter().any(|note| note == "occupied"));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "---\nname: mine\n---\n别人的角色\n"
        );
    }

    #[test]
    fn delete_removes_owned_files_only() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("proj");
        fs::create_dir_all(&project).unwrap();
        let root = temp.path().join("home");
        fs::create_dir_all(&root).unwrap();
        let homes = homes(&root);
        let stored = sample("abc123");
        write_stored(&project, &stored).unwrap();
        let _ = project_out(&project, &homes, &flags(HermesMode::Skill), &stored);
        forget(&project, &homes, &stored);
        assert!(!claude_file(&project, &stored).exists());
        assert!(!hermes_skill_file(&homes, &stored).exists());
        assert!(store_file(&project, &stored.id).is_file());
    }

    #[test]
    fn hermes_mode_follows_the_installed_version() {
        let temp = tempfile::tempdir().unwrap();
        with_test_home(temp.path(), || {
            fs::create_dir_all(temp.path().join(".hermes/hermes-agent")).unwrap();
            fs::write(temp.path().join(".hermes/config.yaml"), "delegation: {}\n").unwrap();
            fs::write(
                temp.path().join(".hermes/hermes-agent/pyproject.toml"),
                "version = \"0.21.3\"\n",
            )
            .unwrap();
            assert_eq!(live_hermes_mode(), HermesMode::Skill);
            fs::write(
                temp.path().join(".hermes/hermes-agent/pyproject.toml"),
                "version = \"0.22.5\"\n",
            )
            .unwrap();
            assert_eq!(live_hermes_mode(), HermesMode::Named);
            fs::write(
                temp.path().join(".hermes/hermes-agent/pyproject.toml"),
                "version = \"0.21.2\"\n",
            )
            .unwrap();
            assert_eq!(live_hermes_mode(), HermesMode::TooOld);
        });
    }
}
