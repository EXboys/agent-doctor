//! Resolve which skills source to use: explicit config > default TeamUps > local.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::{
    default_skills_cache_dir, default_skills_lock_path, default_teamups_base_url,
    open_settings_store, SettingsStore, SkillsSourceKind, SkillsSourceSettings,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedSkillsSource {
    pub kind: SkillsSourceKind,
    pub base_url: String,
    pub pack_slug: Option<String>,
    /// True when using product default because user/team did not configure a source.
    pub using_default: bool,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSkillsLayout {
    pub skills_dir: PathBuf,
    pub skills_lock_path: PathBuf,
}

pub fn load_local_skills_layout() -> Result<LocalSkillsLayout> {
    match open_settings_store() {
        Ok(store) => load_local_skills_layout_from_store(&store),
        Err(_) => {
            let primary = default_skills_lock_path().unwrap_or_else(|| {
                default_skills_cache_dir()
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("skills-lock.json")
            });
            Ok(LocalSkillsLayout {
                skills_dir: default_skills_cache_dir_with_legacy_fallback(),
                skills_lock_path: pick_skills_lock_path(primary),
            })
        }
    }
}

pub fn load_local_skills_layout_from_store(store: &SettingsStore) -> Result<LocalSkillsLayout> {
    let skills_dir = store.resolved_skills_dir()?;
    let primary_lock = default_skills_lock_path().unwrap_or_else(|| {
        skills_dir
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("skills-lock.json")
    });
    let skills_lock_path = pick_skills_lock_path(primary_lock);
    Ok(LocalSkillsLayout {
        skills_dir,
        skills_lock_path,
    })
}

/// Prefer the lock file that actually lists installed skills (legacy Evotown paths differ).
fn pick_skills_lock_path(primary: PathBuf) -> PathBuf {
    skills_lock_candidates(primary.clone())
        .into_iter()
        .filter(|p| p.exists())
        .max_by_key(|p| lock_skill_count(p))
        .unwrap_or(primary)
}

fn skills_lock_candidates(primary: PathBuf) -> Vec<PathBuf> {
    let mut out = vec![primary];
    if let Some(config) = dirs::config_dir() {
        out.push(config.join("evotown").join("skills-lock.json"));
    }
    if let Some(data) = dirs::data_local_dir() {
        out.push(data.join("evotown").join("skills-lock.json"));
    }
    out
}

fn lock_skill_count(path: &Path) -> usize {
    let Ok(raw) = fs::read_to_string(path) else {
        return 0;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return 0;
    };
    value
        .get("skills")
        .and_then(Value::as_object)
        .map(|m| m.len())
        .unwrap_or(0)
}

/// TeamUps zips often unpack to `{skill_id}/{skill_id}/SKILL.md` under the cache root.
pub fn resolve_cached_skill_dir(skills_dir: &Path, skill_id: &str) -> Option<PathBuf> {
    let direct = skills_dir.join(skill_id);
    if direct.join("SKILL.md").exists() {
        return Some(direct);
    }
    let nested = direct.join(skill_id);
    if nested.join("SKILL.md").exists() {
        return Some(nested);
    }
    let mut found = std::collections::BTreeMap::new();
    collect_skill_dirs(&direct, &mut found);
    if let Some(path) = found.get(skill_id) {
        return Some(path.clone());
    }
    if found.len() == 1 {
        return found.values().next().cloned();
    }
    let mut all = std::collections::BTreeMap::new();
    collect_skill_dirs(skills_dir, &mut all);
    all.get(skill_id).cloned()
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

fn default_skills_cache_dir_with_legacy_fallback() -> PathBuf {
    let modern = default_skills_cache_dir();
    if modern.is_dir() {
        return modern;
    }
    let legacy = crate::store::legacy_evotown_skills_dir();
    if legacy.is_dir() {
        return legacy;
    }
    modern
}

/// Explicit store config wins; otherwise product default is TeamUps.
pub fn resolve_skills_source() -> Result<ResolvedSkillsSource> {
    let store = open_settings_store()?;
    resolve_skills_source_from_store(&store, None)
}

pub fn resolve_skills_source_from_store(
    store: &SettingsStore,
    override_kind: Option<SkillsSourceKind>,
) -> Result<ResolvedSkillsSource> {
    let configured = store.get_skills_source_settings()?;
    let kind = override_kind
        .or(configured.source)
        .unwrap_or(SkillsSourceKind::Teamups);
    let using_default = override_kind.is_none() && configured.source.is_none();

    match kind {
        SkillsSourceKind::Teamups => {
            // Evotown/SkillLite profile URLs must not hijack TeamUps mall / license sync.
            let store_kind = configured.source.unwrap_or(SkillsSourceKind::Teamups);
            let base_url = if matches!(
                store_kind,
                SkillsSourceKind::Teamups | SkillsSourceKind::Custom
            ) {
                configured
                    .base_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(|v| v.trim_end_matches('/').to_string())
                    .unwrap_or_else(default_teamups_base_url)
            } else {
                default_teamups_base_url()
            };
            let token = store.get_teamups_license()?;
            Ok(ResolvedSkillsSource {
                kind,
                base_url,
                pack_slug: configured.pack_slug,
                using_default,
                token,
            })
        }
        SkillsSourceKind::Evotown => {
            let team = store.get_team_settings()?;
            let base_url = configured
                .base_url
                .as_deref()
                .or(team.base_url.as_deref())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.trim_end_matches('/').to_string())
                .unwrap_or_default();
            let token = store.get_team_api_key()?;
            Ok(ResolvedSkillsSource {
                kind,
                base_url,
                pack_slug: configured
                    .pack_slug
                    .or(team.bundle_id)
                    .or_else(|| Some("default-agent-skills".into())),
                using_default: false,
                token,
            })
        }
        SkillsSourceKind::Custom => {
            let base_url = configured
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.trim_end_matches('/').to_string())
                .unwrap_or_default();
            let token = store.get_custom_skills_token()?;
            Ok(ResolvedSkillsSource {
                kind,
                base_url,
                pack_slug: configured.pack_slug,
                using_default: false,
                token,
            })
        }
        SkillsSourceKind::Local => Ok(ResolvedSkillsSource {
            kind,
            base_url: String::new(),
            pack_slug: None,
            using_default: false,
            token: None,
        }),
    }
}

pub fn save_skills_source_override(
    store: &SettingsStore,
    settings: &SkillsSourceSettings,
) -> Result<()> {
    store.set_skills_source_settings(settings)
}
