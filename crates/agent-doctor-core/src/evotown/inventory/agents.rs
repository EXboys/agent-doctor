//! Local Evotown skills inventory: cached packages, which agents mount them, and efficacy metrics.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::adapters::util::{discover_binary, home_join};
use crate::workspace::WorkspacesDocument;
use crate::DeepSeekHarnessAdapter;

use super::*;

/// One row per installed runtime (Hermes / OpenClaw / Claude Code / Codex).
pub(crate) fn detect_agents_using(
    skill_id: &str,
    cache_path: &Path,
    workspaces: &WorkspacesDocument,
    presence: &RuntimePresence,
) -> Vec<SkillAgentUsage> {
    let mut agents = Vec::new();

    if presence.hermes {
        agents.push(probe_hermes(skill_id, cache_path, workspaces));
    }
    if presence.openclaw {
        agents.push(probe_openclaw(skill_id, cache_path, workspaces));
    }
    if presence.claude_code {
        agents.push(probe_claude_code(skill_id, cache_path, workspaces));
    }
    if presence.codex {
        agents.push(probe_codex(skill_id, cache_path, workspaces));
    }
    if presence.deepseek_harness {
        agents.push(probe_deepseek_harness(skill_id, cache_path, workspaces));
    }
    if presence.cursor {
        agents.push(probe_cursor(skill_id, cache_path, workspaces));
    }

    // Stable order matching Agents tab.
    let order = [
        "hermes",
        "openclaw",
        "claude-code",
        "codex",
        "deepseek-harness",
        "cursor",
    ];
    agents.sort_by_key(|a| order.iter().position(|id| *id == a.runtime).unwrap_or(99));
    agents
}

#[derive(Clone, Copy)]
pub(crate) struct RuntimePresence {
    hermes: bool,
    openclaw: bool,
    claude_code: bool,
    codex: bool,
    deepseek_harness: bool,
    cursor: bool,
}

pub(crate) fn detect_runtime_presence() -> RuntimePresence {
    // Finder-launched apps often have a minimal PATH; seed common bin dirs first.
    crate::adapters::util::ensure_managed_runtime_path();
    RuntimePresence {
        hermes: runtime_present("hermes"),
        openclaw: runtime_present("openclaw"),
        claude_code: runtime_present("claude-code"),
        codex: runtime_present("codex"),
        deepseek_harness: runtime_present("deepseek-harness"),
        cursor: runtime_present("cursor"),
    }
}

pub(crate) fn runtime_present(runtime_id: &str) -> bool {
    match runtime_id {
        "hermes" => home_join(".hermes").is_dir() || which_exists("hermes"),
        "openclaw" => home_join(".openclaw").is_dir() || which_exists("openclaw"),
        "claude-code" => {
            home_join(".claude").is_dir() || which_exists("claude") || which_exists("claude-code")
        }
        "codex" => home_join(".codex").is_dir() || which_exists("codex"),
        "deepseek-harness" => deepseek_harness_present(),
        "cursor" => cursor_present(),
        _ => false,
    }
}

pub(crate) fn cursor_present() -> bool {
    home_join(".cursor").is_dir()
        || PathBuf::from("/Applications/Cursor.app").is_dir()
        || home_join("Applications/Cursor.app").is_dir()
        || which_exists("cursor")
        || which_exists("cursor-agent")
}

pub(crate) fn deepseek_harness_present() -> bool {
    if DeepSeekHarnessAdapter::home().is_dir() {
        return true;
    }
    if discover_binary("dsh").installed {
        return true;
    }
    for path in deepseek_harness_binary_candidates() {
        if path.is_file() {
            return true;
        }
    }
    false
}

pub(crate) fn deepseek_harness_binary_candidates() -> [PathBuf; 4] {
    [
        home_join(".local/bin/dsh"),
        PathBuf::from("/opt/homebrew/bin/dsh"),
        PathBuf::from("/usr/local/bin/dsh"),
        home_join("bin/dsh"),
    ]
}

pub(crate) fn which_exists(binary: &str) -> bool {
    crate::adapters::util::find_binary(binary).is_some()
}

pub(crate) fn probe_claude_code(
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

pub(crate) fn probe_cursor(
    skill_id: &str,
    cache_path: &Path,
    workspaces: &WorkspacesDocument,
) -> SkillAgentUsage {
    let mut hits: Vec<(String, PathBuf)> = Vec::new();
    let user = home_join(".cursor/skills").join(skill_id);
    if path_has_skill(&user, cache_path) {
        hits.push(("user".into(), user.clone()));
    }
    for (name, entry) in &workspaces.workspaces {
        let project = entry.path.join(".cursor/skills").join(skill_id);
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
        runtime: "cursor".into(),
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

pub(crate) fn probe_openclaw(
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

pub(crate) fn probe_hermes(
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

pub(crate) fn probe_deepseek_harness(
    skill_id: &str,
    cache_path: &Path,
    workspaces: &WorkspacesDocument,
) -> SkillAgentUsage {
    let mut hits: Vec<(String, PathBuf)> = Vec::new();
    let user = DeepSeekHarnessAdapter::home().join("skills").join(skill_id);
    if path_has_skill(&user, cache_path) {
        hits.push(("user".into(), user.clone()));
    }
    for (name, entry) in &workspaces.workspaces {
        let project = entry.path.join(".dsh/skills").join(skill_id);
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
    let primary = hits
        .first()
        .map(|(_, p)| p.clone())
        .unwrap_or_else(|| DeepSeekHarnessAdapter::home().join("skills").join(skill_id));
    SkillAgentUsage {
        runtime: "deepseek-harness".into(),
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

pub(crate) fn probe_codex(
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

pub(crate) fn find_skill_under(root: &Path, skill_id: &str, cache_path: &Path) -> Option<PathBuf> {
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

pub(crate) fn openclaw_entry_enabled(skill_id: &str) -> bool {
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

pub(crate) fn path_has_skill(path: &Path, cache_path: &Path) -> bool {
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
