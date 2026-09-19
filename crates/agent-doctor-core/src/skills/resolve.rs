//! Resolve which skills source to use: explicit config > default TeamUps > local.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

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
        Err(_) => Ok(LocalSkillsLayout {
            skills_dir: default_skills_cache_dir_with_legacy_fallback(),
            skills_lock_path: default_skills_lock_path().unwrap_or_else(|| {
                default_skills_cache_dir()
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join("skills-lock.json")
            }),
        }),
    }
}

pub fn load_local_skills_layout_from_store(store: &SettingsStore) -> Result<LocalSkillsLayout> {
    let skills_dir = store.resolved_skills_dir()?;
    let skills_lock_path = default_skills_lock_path().unwrap_or_else(|| {
        skills_dir
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("skills-lock.json")
    });
    // Prefer modern lock under agent-doctor; fall back to legacy evotown lock if present.
    let legacy_lock = dirs::config_dir().map(|base| base.join("evotown").join("skills-lock.json"));
    let skills_lock_path = if !skills_lock_path.exists() {
        if let Some(legacy) = legacy_lock.filter(|p| p.exists()) {
            legacy
        } else {
            skills_lock_path
        }
    } else {
        skills_lock_path
    };
    Ok(LocalSkillsLayout {
        skills_dir,
        skills_lock_path,
    })
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
            let base_url = configured
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.trim_end_matches('/').to_string())
                .unwrap_or_else(default_teamups_base_url);
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
