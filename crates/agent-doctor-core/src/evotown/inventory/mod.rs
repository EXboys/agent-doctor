//! Local Evotown skills inventory: cached packages, which agents mount them, and efficacy metrics.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::client::EvotownClient;
use super::config::{
    default_policy_cache_path, default_skills_dir, default_skills_lock_path, load_evotown_config,
    EvotownConfig, DEFAULT_BUNDLE_ID, DEFAULT_RUNTIME_TARGET,
};
use crate::adapters::util::home_join;
use crate::workspace::{load_workspaces, WorkspacesDocument};
use crate::DeepSeekHarnessAdapter;

mod agents;
mod mount;

pub(crate) use agents::*;
pub use mount::{
    mount_synced_skills, mount_synced_skills_with_config, unmount_synced_skills,
    unmount_synced_skills_with_config, SkillMountAction, SkillMountOptions, SkillMountReport,
};

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
    /// Installed runtimes that support skill mounts (Hermes, Claude, DeepSeek Harness, …).
    pub available_mount_runtimes: Vec<String>,
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
    match load_evotown_config() {
        Ok(config) => list_skills_inventory_with_config(&config, options),
        Err(_) => {
            // No team credentials: still list/mount from local skills cache.
            let layout = crate::skills::load_local_skills_layout().unwrap_or_else(|_| {
                crate::skills::LocalSkillsLayout {
                    skills_dir: default_skills_dir(),
                    skills_lock_path: default_skills_lock_path()
                        .unwrap_or_else(|| default_skills_dir().join("../skills-lock.json")),
                }
            });
            let stub = EvotownConfig {
                base_url: String::new(),
                api_key: String::new(),
                runtime_target: DEFAULT_RUNTIME_TARGET.to_string(),
                bundle_id: DEFAULT_BUNDLE_ID.to_string(),
                skills_dir: layout.skills_dir,
                skills_lock_path: layout.skills_lock_path,
                policy_cache_path: default_policy_cache_path()
                    .unwrap_or_else(|| PathBuf::from("policies-cache.json")),
                config_source: "local-skills-layout".into(),
            };
            let mut report = list_skills_inventory_with_config(
                &stub,
                &SkillsInventoryOptions {
                    remote_stats: false,
                },
            )?;
            report.remote_stats_ok = false;
            report.remote_stats_error = Some("remote_source_not_configured".into());
            Ok(report)
        }
    }
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
        for id in lock_skills.keys() {
            if crate::skills::resolve_cached_skill_dir(&config.skills_dir, id).is_some() {
                skill_ids.insert(id.clone());
            }
        }
    }

    // Also include skills already present on agent runtimes ("built-in" / previously
    // mounted). Resources used to only list ~/.evotown/skills, so installed agents
    // with local skills showed Skills: 0 after a fresh sync miss.
    let agent_skills = discover_agent_skills(&workspaces);
    for id in agent_skills.keys() {
        skill_ids.insert(id.clone());
    }

    let skills = build_skill_items(
        skill_ids,
        &lock_skills,
        &agent_skills,
        &config.skills_dir,
        &workspaces,
        &remote,
    );

    Ok(SkillsInventoryReport {
        skills_dir: config.skills_dir.display().to_string(),
        lock_path: config.skills_lock_path.display().to_string(),
        bundle_id,
        skills,
        available_mount_runtimes: skill_mount_runtime_ids(),
        remote_stats_ok: remote.is_ok(),
        remote_stats_error: remote.err().map(|e| e.to_string()),
    })
}

/// Runtime ids eligible for `mount_synced_skills` on this machine.
/// Order follows the runtime descriptor; only installed runtimes are included.
pub fn skill_mount_runtime_ids() -> Vec<String> {
    crate::runtime::descriptor_skill_mount_runtime_ids()
        .into_iter()
        .filter(|id| runtime_present(id))
        .map(str::to_string)
        .collect()
}

fn build_skill_items(
    skill_ids: impl IntoIterator<Item = String>,
    lock_skills: &serde_json::Map<String, Value>,
    agent_skills: &std::collections::BTreeMap<String, PathBuf>,
    skills_dir: &Path,
    workspaces: &WorkspacesDocument,
    remote: &Result<std::collections::HashMap<String, RemoteSkillStats>, anyhow::Error>,
) -> Vec<SkillInventoryItem> {
    let presence = detect_runtime_presence();
    let mut skills = Vec::new();
    for skill_id in skill_ids {
        let installed_path = if let Some(cache_path) =
            crate::skills::resolve_cached_skill_dir(skills_dir, &skill_id)
        {
            cache_path
        } else if let Some(agent_path) = agent_skills.get(&skill_id) {
            agent_path.clone()
        } else {
            continue;
        };

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

        let agents = detect_agents_using(&skill_id, &installed_path, workspaces, &presence);

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
    skills
}

/// Walk known agent skill roots and return `skill_id → first path found`.
fn discover_agent_skills(
    workspaces: &WorkspacesDocument,
) -> std::collections::BTreeMap<String, PathBuf> {
    let mut found = std::collections::BTreeMap::new();
    collect_skill_dirs(&home_join(".claude/skills"), &mut found);
    collect_skill_dirs(&home_join(".codex/skills"), &mut found);
    collect_skill_dirs(&home_join(".hermes/skills"), &mut found);
    collect_skill_dirs(&home_join(".openclaw/skills"), &mut found);
    collect_skill_dirs(&home_join(".openclaw/workspace/skills"), &mut found);
    collect_skill_dirs(&DeepSeekHarnessAdapter::home().join("skills"), &mut found);

    for entry in workspaces.workspaces.values() {
        collect_skill_dirs(&entry.path.join(".claude/skills"), &mut found);
        collect_skill_dirs(&entry.path.join(".dsh/skills"), &mut found);
        collect_skill_dirs(&entry.codex_home.join("skills"), &mut found);
        collect_skill_dirs(&entry.openclaw_workspace.join("skills"), &mut found);
        if !entry.hermes_profile.is_empty() {
            collect_skill_dirs(
                &home_join(".hermes")
                    .join("profiles")
                    .join(&entry.hermes_profile)
                    .join("skills"),
                &mut found,
            );
        }
    }
    found
}

fn collect_skill_dirs(root: &Path, out: &mut std::collections::BTreeMap<String, PathBuf>) {
    if !root.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("SKILL.md").exists() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                out.entry(name.to_string()).or_insert(path);
            }
            continue;
        }
        // Hermes stores skills under category folders: skills/<category>/<id>
        if let Ok(nested) = fs::read_dir(&path) {
            for child in nested.flatten() {
                let nested_path = child.path();
                if nested_path.is_dir() && nested_path.join("SKILL.md").exists() {
                    if let Some(name) = nested_path.file_name().and_then(|n| n.to_str()) {
                        out.entry(name.to_string()).or_insert(nested_path);
                    }
                }
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::mount::{link_skill_dir, unlink_skill_dir};
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn discovers_agent_local_skills() {
        let temp = TempDir::new().unwrap();
        let claude = temp.path().join(".claude/skills/calculator");
        fs::create_dir_all(&claude).unwrap();
        fs::write(claude.join("SKILL.md"), "---\nname: calculator\n---\n").unwrap();

        let mut found = std::collections::BTreeMap::new();
        collect_skill_dirs(temp.path().join(".claude/skills").as_path(), &mut found);
        assert_eq!(found.len(), 1);
        assert!(found.contains_key("calculator"));
    }

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

    #[test]
    fn deepseek_in_skill_agent_list_when_harness_present() {
        if !DeepSeekHarnessAdapter::home().is_dir() && !which_exists("dsh") {
            return;
        }
        assert!(
            runtime_present("deepseek-harness"),
            "expected deepseek-harness to be present on this machine"
        );
        let report = list_skills_inventory_with_options(&SkillsInventoryOptions {
            remote_stats: false,
        })
        .expect("inventory");
        let Some(skill) = report.skills.first() else {
            return;
        };
        let runtimes: Vec<&str> = skill.agents.iter().map(|a| a.runtime.as_str()).collect();
        assert!(
            runtimes.contains(&"deepseek-harness"),
            "first skill agents missing deepseek-harness: {runtimes:?}"
        );
        assert!(
            report
                .available_mount_runtimes
                .iter()
                .any(|id| id == "deepseek-harness"),
            "available_mount_runtimes missing deepseek-harness: {:?}",
            report.available_mount_runtimes
        );
    }

    #[test]
    fn cursor_in_skill_agent_list_when_present() {
        if !cursor_present() {
            return;
        }
        let report = list_skills_inventory_with_options(&SkillsInventoryOptions {
            remote_stats: false,
        })
        .expect("inventory");
        assert!(
            report
                .available_mount_runtimes
                .iter()
                .any(|id| id == "cursor"),
            "available_mount_runtimes missing cursor: {:?}",
            report.available_mount_runtimes
        );
        let Some(skill) = report.skills.first() else {
            return;
        };
        let runtimes: Vec<&str> = skill.agents.iter().map(|a| a.runtime.as_str()).collect();
        assert!(
            runtimes.contains(&"cursor"),
            "first skill agents missing cursor: {runtimes:?}"
        );
    }

    #[test]
    fn mount_uses_local_cache_without_remote_credentials() {
        let temp = TempDir::new().unwrap();
        let cache = temp.path().join("skills");
        let skill = cache.join("demo-skill");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: demo-skill\n---\n").unwrap();

        let config = EvotownConfig {
            base_url: String::new(),
            api_key: String::new(),
            runtime_target: "openclaw".into(),
            bundle_id: "test".into(),
            skills_dir: cache,
            skills_lock_path: temp.path().join("lock.json"),
            policy_cache_path: temp.path().join("policies.json"),
            config_source: "test".into(),
        };
        let report = mount_synced_skills_with_config(
            &config,
            &SkillMountOptions {
                skill_ids: vec!["demo-skill".into()],
                // Hermetic: do not touch real agent homes on dev machines.
                runtimes: vec!["__test-no-runtime__".into()],
                include_active_workspace: false,
            },
        )
        .unwrap();
        assert_eq!(report.failed, 0);
        assert_eq!(report.mounted, 0);
    }
}
