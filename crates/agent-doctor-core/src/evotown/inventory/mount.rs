//! Local Evotown skills inventory: cached packages, which agents mount them, and efficacy metrics.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::adapters::util::home_join;
use crate::evotown::config::{
    default_policy_cache_path, EvotownConfig, DEFAULT_BUNDLE_ID, DEFAULT_RUNTIME_TARGET,
};
use crate::workspace::{load_workspaces, WorkspacesDocument};
use crate::DeepSeekHarnessAdapter;

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMountOptions {
    /// Empty = all cached skills.
    pub skill_ids: Vec<String>,
    /// Empty = all present runtimes (hermes/openclaw/claude-code/codex/cursor).
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

/// Symlink cached skills into each agent’s native skills directory.
/// Does **not** require a remote skills source or team credentials.
pub fn mount_synced_skills(options: &SkillMountOptions) -> Result<SkillMountReport> {
    let layout = crate::skills::load_local_skills_layout()?;
    let config = EvotownConfig {
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
    mount_synced_skills_with_config(&config, options)
}

pub fn mount_synced_skills_with_config(
    config: &EvotownConfig,
    options: &SkillMountOptions,
) -> Result<SkillMountReport> {
    let workspaces = load_workspaces().unwrap_or_default();
    let agent_skills = discover_agent_skills(&workspaces);
    let skill_ids = resolve_mount_skill_ids(config, &options.skill_ids, &agent_skills)?;
    let runtimes = resolve_mount_runtimes(&options.runtimes);

    let mut mounted = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut actions = Vec::new();

    for skill_id in &skill_ids {
        let source = if let Some(cache_source) =
            crate::skills::resolve_cached_skill_dir(&config.skills_dir, skill_id)
        {
            cache_source
        } else if let Some(agent_path) = agent_skills.get(skill_id) {
            agent_path.clone()
        } else {
            failed += 1;
            actions.push(SkillMountAction {
                skill_id: skill_id.clone(),
                runtime: "*".into(),
                path: config.skills_dir.join(skill_id).display().to_string(),
                outcome: "failed".into(),
                detail: Some("skill cache missing SKILL.md".into()),
            });
            continue;
        };

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
    let layout = crate::skills::load_local_skills_layout()?;
    let config = EvotownConfig {
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
    unmount_synced_skills_with_config(&config, options)
}

pub fn unmount_synced_skills_with_config(
    config: &EvotownConfig,
    options: &SkillMountOptions,
) -> Result<SkillMountReport> {
    let workspaces = load_workspaces().unwrap_or_default();
    let agent_skills = discover_agent_skills(&workspaces);
    let skill_ids = resolve_mount_skill_ids(config, &options.skill_ids, &agent_skills)?;
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

pub(crate) fn resolve_mount_skill_ids(
    config: &EvotownConfig,
    only: &[String],
    agent_skills: &std::collections::BTreeMap<String, PathBuf>,
) -> Result<Vec<String>> {
    if !only.is_empty() {
        return Ok(only
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect());
    }
    let mut ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if config.skills_dir.is_dir() {
        for entry in fs::read_dir(&config.skills_dir)? {
            let path = entry?.path();
            if path.is_dir() && path.join("SKILL.md").exists() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    ids.insert(name.to_string());
                }
            }
        }
    }
    for id in agent_skills.keys() {
        ids.insert(id.clone());
    }
    Ok(ids.into_iter().collect())
}

pub(crate) fn resolve_mount_runtimes(only: &[String]) -> Vec<String> {
    let all = [
        "hermes",
        "openclaw",
        "claude-code",
        "codex",
        "deepseek-harness",
        "cursor",
    ];
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

pub(crate) fn mount_targets_for(
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
        "deepseek-harness" => {
            targets.push(DeepSeekHarnessAdapter::home().join("skills").join(skill_id));
            if include_active_workspace {
                if let Some(active) = workspaces.active.as_deref() {
                    if let Some(entry) = workspaces.workspaces.get(active) {
                        targets.push(entry.path.join(".dsh/skills").join(skill_id));
                    }
                }
            }
        }
        "cursor" => {
            targets.push(home_join(".cursor/skills").join(skill_id));
            if include_active_workspace {
                if let Some(active) = workspaces.active.as_deref() {
                    if let Some(entry) = workspaces.workspaces.get(active) {
                        targets.push(entry.path.join(".cursor/skills").join(skill_id));
                    }
                }
            }
        }
        _ => {}
    }
    targets
}

pub(crate) fn link_skill_dir(source: &Path, target: &Path) -> Result<String> {
    if let (Ok(src), Ok(dst)) = (fs::canonicalize(source), fs::canonicalize(target)) {
        if src == dst {
            return Ok("skipped".into());
        }
    }
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

pub(crate) fn unlink_skill_dir(source: &Path, target: &Path) -> Result<String> {
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
pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
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
