//! Local Evotown skills inventory: cached packages, which agents mount them, and efficacy metrics.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::client::EvotownClient;
use super::config::{load_evotown_config, EvotownConfig};
use crate::adapters::util::home_join;
use crate::workspace::{load_workspaces, WorkspacesDocument};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillAgentUsage {
    pub runtime: String,
    pub scope: String,
    pub path: String,
    pub mounted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInventoryItem {
    pub skill_id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub installed_path: String,
    pub agents: Vec<SkillAgentUsage>,
    pub call_count: Option<u64>,
    pub success_count: Option<u64>,
    pub success_rate: Option<f64>,
    pub first_success_rate: Option<f64>,
    pub download_count: Option<u64>,
    pub metrics_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsInventoryReport {
    pub skills_dir: String,
    pub lock_path: String,
    pub bundle_id: Option<String>,
    pub skills: Vec<SkillInventoryItem>,
    pub remote_stats_ok: bool,
    pub remote_stats_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SkillsInventoryOptions {
    /// When false, skip Evotown skill-stats HTTP (fast path after mount/unmount).
    pub remote_stats: bool,
}

pub fn list_skills_inventory() -> Result<SkillsInventoryReport> {
    list_skills_inventory_with_options(&SkillsInventoryOptions { remote_stats: true })
}

pub fn list_skills_inventory_with_options(
    options: &SkillsInventoryOptions,
) -> Result<SkillsInventoryReport> {
    let config = load_evotown_config()?;
    list_skills_inventory_with_config(&config, options)
}

pub fn list_skills_inventory_with_config(
    config: &EvotownConfig,
    options: &SkillsInventoryOptions,
) -> Result<SkillsInventoryReport> {
    let lock = load_lock(&config.skills_lock_path).unwrap_or(Value::Null);
    let lock_skills = lock
        .get("skills")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let bundle_id = lock
        .get("bundle_id")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .or_else(|| Some(config.bundle_id.clone()));

    let workspaces = load_workspaces().unwrap_or_default();
    let remote = if options.remote_stats {
        fetch_remote_stats(config)
    } else {
        Ok(std::collections::HashMap::new())
    };

    let mut skill_ids: std::collections::BTreeSet<String> = lock_skills.keys().cloned().collect();
    if config.skills_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&config.skills_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join("SKILL.md").exists() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        skill_ids.insert(name.to_string());
                    }
                }
            }
        }
    }

    let mut skills = Vec::new();
    for skill_id in skill_ids {
        let installed_path = config.skills_dir.join(&skill_id);
        if !installed_path.is_dir() {
            continue;
        }
        let lock_entry = lock_skills.get(&skill_id).cloned().unwrap_or(Value::Null);
        let version = lock_entry
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let (name, description) = read_skill_frontmatter(&installed_path);
        let local_metrics = read_local_metrics(&installed_path);
        let remote_item = remote
            .as_ref()
            .ok()
            .and_then(|map| map.get(&skill_id).cloned());

        let mut call_count = local_metrics.call_count;
        let mut success_count = local_metrics.success_count;
        let mut success_rate = local_metrics.success_rate;
        let mut first_success_rate = local_metrics.first_success_rate;
        let mut download_count = None;
        let mut metrics_source = if call_count.is_some() {
            "local_meta".to_string()
        } else {
            "none".to_string()
        };

        if let Some(remote_item) = remote_item {
            download_count = remote_item.download_count;
            if call_count.is_none() {
                call_count = remote_item.call_count;
                success_count = remote_item.success_count;
                success_rate = remote_item.success_rate;
                first_success_rate = remote_item.first_success_rate;
                if call_count.is_some() {
                    metrics_source = "evotown".to_string();
                }
            } else if remote_item.call_count.is_some() {
                metrics_source = "local_meta+evotown".to_string();
            }
        }

        let agents = detect_agents_using(&skill_id, &installed_path, &workspaces);

        skills.push(SkillInventoryItem {
            skill_id: skill_id.clone(),
            name: name.unwrap_or(skill_id),
            version: if version.is_empty() {
                "—".to_string()
            } else {
                version
            },
            description,
            installed_path: installed_path.display().to_string(),
            agents,
            call_count,
            success_count,
            success_rate,
            first_success_rate,
            download_count,
            metrics_source,
        });
    }

    skills.sort_by(|a, b| a.skill_id.cmp(&b.skill_id));

    Ok(SkillsInventoryReport {
        skills_dir: config.skills_dir.display().to_string(),
        lock_path: config.skills_lock_path.display().to_string(),
        bundle_id,
        skills,
        remote_stats_ok: remote.is_ok(),
        remote_stats_error: remote.err().map(|e| e.to_string()),
    })
}

#[derive(Debug, Default)]
struct LocalMetrics {
    call_count: Option<u64>,
    success_count: Option<u64>,
    success_rate: Option<f64>,
    first_success_rate: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteSkillStats {
    #[serde(default)]
    call_count: Option<u64>,
    #[serde(default)]
    success_count: Option<u64>,
    #[serde(default)]
    success_rate: Option<f64>,
    #[serde(default)]
    first_success_rate: Option<f64>,
    #[serde(default)]
    download_count: Option<u64>,
}

fn fetch_remote_stats(
    config: &EvotownConfig,
) -> Result<std::collections::HashMap<String, RemoteSkillStats>> {
    // Short timeout: stats are optional enrichment; never block the Skills panel.
    let client = EvotownClient::with_timeout(
        &config.base_url,
        &config.api_key,
        std::time::Duration::from_secs(3),
    )?;
    let body = client.get_json("/api/v1/market/skill-stats")?;
    let mut map = std::collections::HashMap::new();
    let Some(items) = body.get("skills").and_then(Value::as_array) else {
        return Ok(map);
    };
    for item in items {
        let Some(skill_id) = item.get("skill_id").and_then(Value::as_str) else {
            continue;
        };
        if let Ok(stats) = serde_json::from_value::<RemoteSkillStats>(item.clone()) {
            map.insert(skill_id.to_string(), stats);
        }
    }
    Ok(map)
}

fn load_lock(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(Value::Null);
    }
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw).unwrap_or(Value::Null))
}

fn read_skill_frontmatter(dir: &Path) -> (Option<String>, Option<String>) {
    let path = dir.join("SKILL.md");
    let Ok(raw) = fs::read_to_string(path) else {
        return (None, None);
    };
    let mut name = None;
    let mut description = None;
    if let Some(fm) = raw.strip_prefix("---") {
        if let Some((block, _)) = fm.split_once("\n---") {
            for line in block.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("name:") {
                    name = Some(v.trim().trim_matches('"').to_string());
                } else if let Some(v) = line.strip_prefix("description:") {
                    description = Some(v.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    if description.is_none() {
        for line in raw.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') || t.starts_with("---") {
                continue;
            }
            description = Some(t.chars().take(120).collect());
            break;
        }
    }
    (name, description)
}

fn read_local_metrics(dir: &Path) -> LocalMetrics {
    let meta_path = dir.join(".meta.json");
    let Ok(raw) = fs::read_to_string(meta_path) else {
        return LocalMetrics::default();
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return LocalMetrics::default();
    };
    let call_count = value.get("call_count").and_then(Value::as_u64);
    let success_count = value.get("success_count").and_then(Value::as_u64);
    let success_rate = value
        .get("success_rate")
        .and_then(Value::as_f64)
        .or_else(|| match (call_count, success_count) {
            (Some(c), Some(s)) if c > 0 => Some(s as f64 / c as f64),
            _ => None,
        });
    let first_success_rate = value.get("first_success_rate").and_then(Value::as_f64);
    LocalMetrics {
        call_count,
        success_count,
        success_rate,
        first_success_rate,
    }
}

/// One row per installed runtime (Hermes / OpenClaw / Claude Code / Codex).
fn detect_agents_using(
    skill_id: &str,
    cache_path: &Path,
    workspaces: &WorkspacesDocument,
) -> Vec<SkillAgentUsage> {
    let mut agents = Vec::new();

    if runtime_present("hermes") {
        agents.push(probe_hermes(skill_id, cache_path, workspaces));
    }
    if runtime_present("openclaw") {
        agents.push(probe_openclaw(skill_id, cache_path, workspaces));
    }
    if runtime_present("claude-code") {
        agents.push(probe_claude_code(skill_id, cache_path, workspaces));
    }
    if runtime_present("codex") {
        agents.push(probe_codex(skill_id, cache_path, workspaces));
    }

    // Stable order matching Agents tab.
    let order = ["hermes", "openclaw", "claude-code", "codex"];
    agents.sort_by_key(|a| order.iter().position(|id| *id == a.runtime).unwrap_or(99));
    agents
}

fn runtime_present(runtime_id: &str) -> bool {
    match runtime_id {
        "hermes" => home_join(".hermes").is_dir() || which_exists("hermes"),
        "openclaw" => home_join(".openclaw").is_dir() || which_exists("openclaw"),
        "claude-code" => {
            home_join(".claude").is_dir() || which_exists("claude") || which_exists("claude-code")
        }
        "codex" => home_join(".codex").is_dir() || which_exists("codex"),
        _ => false,
    }
}

fn which_exists(binary: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(binary);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
}

fn probe_claude_code(
    skill_id: &str,
    cache_path: &Path,
    workspaces: &WorkspacesDocument,
) -> SkillAgentUsage {
    let mut hits: Vec<(String, PathBuf)> = Vec::new();
    let user = home_join(".claude/skills").join(skill_id);
    if path_has_skill(&user, cache_path) {
        hits.push(("user".into(), user.clone()));
    }
    for (name, entry) in &workspaces.workspaces {
        let project = entry.path.join(".claude/skills").join(skill_id);
        if path_has_skill(&project, cache_path) {
            let active = workspaces.active.as_deref() == Some(name.as_str());
            hits.push((
                if active {
                    format!("ws:{name}*")
                } else {
                    format!("ws:{name}")
                },
                project,
            ));
        }
    }
    let primary = hits.first().map(|(_, p)| p.clone()).unwrap_or_else(|| user);
    SkillAgentUsage {
        runtime: "claude-code".into(),
        scope: if hits.is_empty() {
            "not mounted".into()
        } else {
            hits.iter()
                .map(|(s, _)| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        },
        path: primary.display().to_string(),
        mounted: !hits.is_empty(),
    }
}

fn probe_openclaw(
    skill_id: &str,
    cache_path: &Path,
    workspaces: &WorkspacesDocument,
) -> SkillAgentUsage {
    let mut hits: Vec<(String, PathBuf)> = Vec::new();

    let global = home_join(".openclaw/skills").join(skill_id);
    if path_has_skill(&global, cache_path) {
        hits.push(("global".into(), global));
    }

    let default_ws = home_join(".openclaw/workspace/skills").join(skill_id);
    if path_has_skill(&default_ws, cache_path) {
        hits.push(("workspace".into(), default_ws));
    }

    for (name, entry) in &workspaces.workspaces {
        let candidate = entry.openclaw_workspace.join("skills").join(skill_id);
        if path_has_skill(&candidate, cache_path) {
            let active = workspaces.active.as_deref() == Some(name.as_str());
            hits.push((
                if active {
                    format!("ws:{name}*")
                } else {
                    format!("ws:{name}")
                },
                candidate,
            ));
        }
    }

    if openclaw_entry_enabled(skill_id) {
        hits.push(("config".into(), home_join(".openclaw/openclaw.json")));
    }

    let primary = hits
        .first()
        .map(|(_, p)| p.clone())
        .unwrap_or_else(|| home_join(".openclaw/skills").join(skill_id));
    SkillAgentUsage {
        runtime: "openclaw".into(),
        scope: if hits.is_empty() {
            "not mounted".into()
        } else {
            hits.iter()
                .map(|(s, _)| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        },
        path: primary.display().to_string(),
        mounted: !hits.is_empty(),
    }
}

fn probe_hermes(
    skill_id: &str,
    cache_path: &Path,
    workspaces: &WorkspacesDocument,
) -> SkillAgentUsage {
    let mut hits: Vec<(String, PathBuf)> = Vec::new();
    let default_root = home_join(".hermes/skills");
    if let Some(found) = find_skill_under(&default_root, skill_id, cache_path) {
        hits.push(("bundled".into(), found));
    }

    for (name, entry) in &workspaces.workspaces {
        if entry.hermes_profile.is_empty() {
            continue;
        }
        let profile_root = home_join(".hermes")
            .join("profiles")
            .join(&entry.hermes_profile)
            .join("skills");
        if let Some(found) = find_skill_under(&profile_root, skill_id, cache_path) {
            let active = workspaces.active.as_deref() == Some(name.as_str());
            hits.push((
                if active {
                    format!("profile:{}*", entry.hermes_profile)
                } else {
                    format!("profile:{}", entry.hermes_profile)
                },
                found,
            ));
        }
    }

    let primary = hits
        .first()
        .map(|(_, p)| p.clone())
        .unwrap_or_else(|| default_root.join(skill_id));
    SkillAgentUsage {
        runtime: "hermes".into(),
        scope: if hits.is_empty() {
            "not mounted".into()
        } else {
            hits.iter()
                .map(|(s, _)| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        },
        path: primary.display().to_string(),
        mounted: !hits.is_empty(),
    }
}

fn probe_codex(
    skill_id: &str,
    cache_path: &Path,
    workspaces: &WorkspacesDocument,
) -> SkillAgentUsage {
    let mut hits: Vec<(String, PathBuf)> = Vec::new();
    let global = home_join(".codex/skills").join(skill_id);
    if path_has_skill(&global, cache_path) {
        hits.push(("global".into(), global));
    }
    for (name, entry) in &workspaces.workspaces {
        let candidate = entry.codex_home.join("skills").join(skill_id);
        if path_has_skill(&candidate, cache_path) {
            let active = workspaces.active.as_deref() == Some(name.as_str());
            hits.push((
                if active {
                    format!("ws:{name}*")
                } else {
                    format!("ws:{name}")
                },
                candidate,
            ));
        }
    }
    let primary = hits
        .first()
        .map(|(_, p)| p.clone())
        .unwrap_or_else(|| home_join(".codex/skills").join(skill_id));
    SkillAgentUsage {
        runtime: "codex".into(),
        scope: if hits.is_empty() {
            "not mounted".into()
        } else {
            hits.iter()
                .map(|(s, _)| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        },
        path: primary.display().to_string(),
        mounted: !hits.is_empty(),
    }
}

fn find_skill_under(root: &Path, skill_id: &str, cache_path: &Path) -> Option<PathBuf> {
    if !root.is_dir() {
        return None;
    }
    let direct = root.join(skill_id);
    if path_has_skill(&direct, cache_path) {
        return Some(direct);
    }
    // Hermes stores skills in category folders: skills/<category>/<skill_id>
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let nested = path.join(skill_id);
            if path_has_skill(&nested, cache_path) {
                return Some(nested);
            }
            // Also accept category dir itself named as the skill.
            if path.file_name().and_then(|n| n.to_str()) == Some(skill_id)
                && path_has_skill(&path, cache_path)
            {
                return Some(path);
            }
        }
    }
    None
}

fn openclaw_entry_enabled(skill_id: &str) -> bool {
    let path = home_join(".openclaw/openclaw.json");
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    value
        .pointer(&format!("/skills/entries/{skill_id}/enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn path_has_skill(path: &Path, cache_path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    if path.join("SKILL.md").exists() {
        return true;
    }
    if let (Ok(a), Ok(b)) = (fs::canonicalize(path), fs::canonicalize(cache_path)) {
        return a == b;
    }
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMountOptions {
    /// Empty = all cached skills.
    pub skill_ids: Vec<String>,
    /// Empty = all present runtimes (hermes/openclaw/claude-code/codex).
    pub runtimes: Vec<String>,
    /// Also symlink into the active workspace project `.claude/skills/`.
    pub include_active_workspace: bool,
}

impl Default for SkillMountOptions {
    fn default() -> Self {
        Self {
            skill_ids: Vec::new(),
            runtimes: Vec::new(),
            include_active_workspace: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMountAction {
    pub skill_id: String,
    pub runtime: String,
    pub path: String,
    pub outcome: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMountReport {
    pub mounted: usize,
    pub unmounted: usize,
    pub skipped: usize,
    pub failed: usize,
    pub actions: Vec<SkillMountAction>,
}

/// Symlink Evotown-cached skills into each agent’s native skills directory.
pub fn mount_synced_skills(options: &SkillMountOptions) -> Result<SkillMountReport> {
    let config = load_evotown_config()?;
    mount_synced_skills_with_config(&config, options)
}

pub fn mount_synced_skills_with_config(
    config: &EvotownConfig,
    options: &SkillMountOptions,
) -> Result<SkillMountReport> {
    let workspaces = load_workspaces().unwrap_or_default();
    let skill_ids = resolve_mount_skill_ids(config, &options.skill_ids)?;
    let runtimes = resolve_mount_runtimes(&options.runtimes);

    let mut mounted = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut actions = Vec::new();

    for skill_id in &skill_ids {
        let source = config.skills_dir.join(skill_id);
        if !source.is_dir() || !source.join("SKILL.md").exists() {
            failed += 1;
            actions.push(SkillMountAction {
                skill_id: skill_id.clone(),
                runtime: "*".into(),
                path: source.display().to_string(),
                outcome: "failed".into(),
                detail: Some("skill cache missing SKILL.md".into()),
            });
            continue;
        }

        for runtime in &runtimes {
            let targets = mount_targets_for(
                runtime,
                skill_id,
                &workspaces,
                options.include_active_workspace,
            );
            if targets.is_empty() {
                skipped += 1;
                actions.push(SkillMountAction {
                    skill_id: skill_id.clone(),
                    runtime: runtime.clone(),
                    path: String::new(),
                    outcome: "skipped".into(),
                    detail: Some("runtime not present on this machine".into()),
                });
                continue;
            }
            for target in targets {
                match link_skill_dir(&source, &target) {
                    Ok(outcome) => {
                        match outcome.as_str() {
                            "mounted" => mounted += 1,
                            "skipped" => skipped += 1,
                            _ => failed += 1,
                        }
                        actions.push(SkillMountAction {
                            skill_id: skill_id.clone(),
                            runtime: runtime.clone(),
                            path: target.display().to_string(),
                            outcome,
                            detail: None,
                        });
                    }
                    Err(err) => {
                        failed += 1;
                        actions.push(SkillMountAction {
                            skill_id: skill_id.clone(),
                            runtime: runtime.clone(),
                            path: target.display().to_string(),
                            outcome: "failed".into(),
                            detail: Some(err.to_string()),
                        });
                    }
                }
            }
        }
    }

    Ok(SkillMountReport {
        mounted,
        unmounted: 0,
        skipped,
        failed,
        actions,
    })
}

/// Remove Doctor-managed skill symlinks from agent skills directories.
/// Only deletes symlinks that resolve to the Evotown cache; never removes real copies.
pub fn unmount_synced_skills(options: &SkillMountOptions) -> Result<SkillMountReport> {
    let config = load_evotown_config()?;
    unmount_synced_skills_with_config(&config, options)
}

pub fn unmount_synced_skills_with_config(
    config: &EvotownConfig,
    options: &SkillMountOptions,
) -> Result<SkillMountReport> {
    let workspaces = load_workspaces().unwrap_or_default();
    let skill_ids = resolve_mount_skill_ids(config, &options.skill_ids)?;
    let runtimes = resolve_mount_runtimes(&options.runtimes);

    let mut unmounted = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut actions = Vec::new();

    for skill_id in &skill_ids {
        let source = config.skills_dir.join(skill_id);
        for runtime in &runtimes {
            let targets = mount_targets_for(
                runtime,
                skill_id,
                &workspaces,
                options.include_active_workspace,
            );
            if targets.is_empty() {
                skipped += 1;
                actions.push(SkillMountAction {
                    skill_id: skill_id.clone(),
                    runtime: runtime.clone(),
                    path: String::new(),
                    outcome: "skipped".into(),
                    detail: Some("runtime not present on this machine".into()),
                });
                continue;
            }
            for target in targets {
                match unlink_skill_dir(&source, &target) {
                    Ok(outcome) => {
                        match outcome.as_str() {
                            "unmounted" => unmounted += 1,
                            "skipped" => skipped += 1,
                            _ => failed += 1,
                        }
                        actions.push(SkillMountAction {
                            skill_id: skill_id.clone(),
                            runtime: runtime.clone(),
                            path: target.display().to_string(),
                            outcome,
                            detail: None,
                        });
                    }
                    Err(err) => {
                        failed += 1;
                        actions.push(SkillMountAction {
                            skill_id: skill_id.clone(),
                            runtime: runtime.clone(),
                            path: target.display().to_string(),
                            outcome: "failed".into(),
                            detail: Some(err.to_string()),
                        });
                    }
                }
            }
        }
    }

    Ok(SkillMountReport {
        mounted: 0,
        unmounted,
        skipped,
        failed,
        actions,
    })
}

fn resolve_mount_skill_ids(config: &EvotownConfig, only: &[String]) -> Result<Vec<String>> {
    if !only.is_empty() {
        return Ok(only
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect());
    }
    let mut ids = Vec::new();
    if config.skills_dir.is_dir() {
        for entry in fs::read_dir(&config.skills_dir)? {
            let path = entry?.path();
            if path.is_dir() && path.join("SKILL.md").exists() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    ids.push(name.to_string());
                }
            }
        }
    }
    ids.sort();
    Ok(ids)
}

fn resolve_mount_runtimes(only: &[String]) -> Vec<String> {
    let all = ["hermes", "openclaw", "claude-code", "codex"];
    if only.is_empty() {
        return all
            .iter()
            .filter(|id| runtime_present(id))
            .map(|s| (*s).to_string())
            .collect();
    }
    only.iter()
        .map(|s| s.trim().to_string())
        .filter(|s| all.contains(&s.as_str()))
        .collect()
}

fn mount_targets_for(
    runtime: &str,
    skill_id: &str,
    workspaces: &WorkspacesDocument,
    include_active_workspace: bool,
) -> Vec<PathBuf> {
    if !runtime_present(runtime) {
        return Vec::new();
    }
    let mut targets = Vec::new();
    match runtime {
        "hermes" => targets.push(home_join(".hermes/skills").join(skill_id)),
        "openclaw" => targets.push(home_join(".openclaw/skills").join(skill_id)),
        "claude-code" => {
            targets.push(home_join(".claude/skills").join(skill_id));
            if include_active_workspace {
                if let Some(active) = workspaces.active.as_deref() {
                    if let Some(entry) = workspaces.workspaces.get(active) {
                        targets.push(entry.path.join(".claude/skills").join(skill_id));
                    }
                }
            }
        }
        "codex" => targets.push(home_join(".codex/skills").join(skill_id)),
        _ => {}
    }
    targets
}

fn link_skill_dir(source: &Path, target: &Path) -> Result<String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    if target.exists() || target.symlink_metadata().is_ok() {
        if let (Ok(src), Ok(dst)) = (fs::canonicalize(source), fs::canonicalize(target)) {
            if src == dst {
                return Ok("skipped".into());
            }
        }
        let meta = fs::symlink_metadata(target)?;
        if meta.file_type().is_symlink() {
            fs::remove_file(target)?;
        } else if target.is_dir() {
            bail!("target already exists as a real directory");
        } else {
            fs::remove_file(target)?;
        }
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target)?;
    }
    #[cfg(not(unix))]
    {
        // Windows: directory junction / symlink often needs elevation; copy as fallback.
        copy_dir_recursive(source, target)?;
    }
    Ok("mounted".into())
}

fn unlink_skill_dir(source: &Path, target: &Path) -> Result<String> {
    let meta = match fs::symlink_metadata(target) {
        Ok(meta) => meta,
        Err(_) => return Ok("skipped".into()),
    };

    if meta.file_type().is_symlink() {
        if let (Ok(src), Ok(dst)) = (fs::canonicalize(source), fs::canonicalize(target)) {
            if src == dst {
                fs::remove_file(target)?;
                return Ok("unmounted".into());
            }
        }
        // Symlink exists but points elsewhere — leave it alone.
        return Ok("skipped".into());
    }

    // Real directory/file: do not delete user-owned skill copies.
    Ok("skipped".into())
}

#[cfg(not(unix))]
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn reads_frontmatter_and_meta_metrics() {
        let temp = TempDir::new().unwrap();
        let skill = temp.path().join("calculator");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: calculator\ndescription: Math helper\n---\n\nBody\n",
        )
        .unwrap();
        let mut meta = fs::File::create(skill.join(".meta.json")).unwrap();
        write!(
            meta,
            r#"{{"call_count":10,"success_count":8,"first_success_rate":0.9}}"#
        )
        .unwrap();

        let (name, desc) = read_skill_frontmatter(&skill);
        assert_eq!(name.as_deref(), Some("calculator"));
        assert_eq!(desc.as_deref(), Some("Math helper"));
        let m = read_local_metrics(&skill);
        assert_eq!(m.call_count, Some(10));
        assert_eq!(m.success_count, Some(8));
        assert!((m.success_rate.unwrap() - 0.8).abs() < 1e-9);
        assert_eq!(m.first_success_rate, Some(0.9));
    }

    #[test]
    fn link_skill_dir_is_idempotent() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("src").join("calc");
        let target = temp.path().join("dst").join("calc");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("SKILL.md"), "# calc\n").unwrap();

        assert_eq!(link_skill_dir(&source, &target).unwrap(), "mounted");
        assert_eq!(link_skill_dir(&source, &target).unwrap(), "skipped");
        assert!(target.join("SKILL.md").exists());
    }

    #[test]
    fn unlink_skill_dir_removes_cache_symlink() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("src").join("calc");
        let target = temp.path().join("dst").join("calc");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("SKILL.md"), "# calc\n").unwrap();
        assert_eq!(link_skill_dir(&source, &target).unwrap(), "mounted");
        assert_eq!(unlink_skill_dir(&source, &target).unwrap(), "unmounted");
        assert!(!target.exists());
        assert_eq!(unlink_skill_dir(&source, &target).unwrap(), "skipped");
    }
}
