//! One-shot migration from legacy `.env` / `providers.json` into settings.db + keychain.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::db::{PersonalProviderRecord, SettingsStore, SkillsSourceKind, SkillsSourceSettings};
use crate::setup::{
    EVOTOWN_API_KEY_ENV, EVOTOWN_BUNDLE_ID_ENV, EVOTOWN_RUNTIME_ENV, EVOTOWN_SKILLS_DIR_ENV,
    EVOTOWN_URL_ENV,
};

const ENGINE_ID_ENV: &str = "EVOTOWN_ENGINE_ID";
const ENGINE_INGEST_ENV: &str = "EVOTOWN_ENGINE_INGEST_TOKEN";

pub fn migrate_legacy_into(store: &SettingsStore) -> Result<()> {
    migrate_team_from_env_files(store)?;
    migrate_personal_providers(store)?;
    migrate_skills_dir_hint(store)?;
    Ok(())
}

fn migrate_team_from_env_files(store: &SettingsStore) -> Result<()> {
    let existing = store.get_team_settings()?;
    let has_url = existing
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some();
    let has_key = store
        .get_team_api_key()?
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some();
    if has_url && has_key {
        return Ok(());
    }

    for path in candidate_evotown_env_paths() {
        if !path.exists() {
            continue;
        }
        let map = read_env_map(&path)?;
        apply_team_env_map(store, &map)?;
        if store.get_team_api_key()?.is_some()
            && store
                .get_team_settings()?
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .is_some()
        {
            // Prefer Evotown as explicit skills source when migrating team env.
            let skills = store.get_skills_source_settings()?;
            if skills.source.is_none() {
                store.set_skills_source_settings(&SkillsSourceSettings {
                    source: Some(SkillsSourceKind::Evotown),
                    base_url: map.get(EVOTOWN_URL_ENV).cloned(),
                    pack_slug: None,
                })?;
            }
            return Ok(());
        }
    }

    for path in candidate_company_profile_paths() {
        if !path.exists() {
            continue;
        }
        let map = read_env_map(&path)?;
        let gateway = map
            .get("AGENT_DOCTOR_GATEWAY_URL")
            .cloned()
            .or_else(|| map.get("OPENAI_BASE_URL").cloned());
        let key = map
            .get("AGENT_DOCTOR_COMPANY_API_KEY")
            .cloned()
            .or_else(|| map.get("OPENAI_API_KEY").cloned())
            .or_else(|| map.get(EVOTOWN_API_KEY_ENV).cloned());
        let mut team = store.get_team_settings()?;
        if team.base_url.is_none() {
            if let Some(url) = map.get(EVOTOWN_URL_ENV).cloned().or_else(|| {
                gateway
                    .as_deref()
                    .map(crate::setup::evotown_base_from_gateway)
            }) {
                team.base_url = Some(url.trim().trim_end_matches('/').to_string());
            }
        }
        store.set_team_settings(&team)?;
        if store.get_team_api_key()?.is_none() {
            if let Some(key) = key.filter(|v| !v.trim().is_empty()) {
                store.set_team_api_key(key.trim())?;
            }
        }
        if let Some(overlay) = map
            .get("AGENT_DOCTOR_COMPANY_API_KEY")
            .cloned()
            .or_else(|| map.get("OPENAI_API_KEY").cloned())
            .filter(|v| !v.trim().is_empty())
        {
            if store.get_overlay_api_key()?.is_none() {
                store.set_overlay_api_key(overlay.trim())?;
            }
        }
    }

    Ok(())
}

fn apply_team_env_map(store: &SettingsStore, map: &HashMap<String, String>) -> Result<()> {
    let mut team = store.get_team_settings()?;
    if team.base_url.is_none() {
        if let Some(url) = map.get(EVOTOWN_URL_ENV) {
            team.base_url = Some(url.trim().trim_end_matches('/').to_string());
        }
    }
    if team.runtime.is_none() {
        team.runtime = map.get(EVOTOWN_RUNTIME_ENV).cloned();
    }
    if team.bundle_id.is_none() {
        team.bundle_id = map.get(EVOTOWN_BUNDLE_ID_ENV).cloned();
    }
    if team.skills_dir.is_none() {
        team.skills_dir = map.get(EVOTOWN_SKILLS_DIR_ENV).cloned();
    }
    if team.engine_id.is_none() {
        team.engine_id = map.get(ENGINE_ID_ENV).cloned();
    }
    store.set_team_settings(&team)?;

    if store.get_team_api_key()?.is_none() {
        if let Some(key) = map.get(EVOTOWN_API_KEY_ENV) {
            if !key.trim().is_empty() {
                store.set_team_api_key(key.trim())?;
            }
        }
    }
    if store.get_team_engine_ingest_token()?.is_none() {
        if let Some(token) = map.get(ENGINE_INGEST_ENV) {
            if !token.trim().is_empty() {
                store.set_team_engine_ingest_token(token.trim())?;
            }
        }
    }
    Ok(())
}

fn migrate_personal_providers(store: &SettingsStore) -> Result<()> {
    if !store.list_personal_providers()?.is_empty() {
        return Ok(());
    }
    for path in candidate_providers_json_paths() {
        if !path.exists() {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let value: serde_json::Value = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let active_id = value
            .get("active_id")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let providers = value
            .get("providers")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for entry in providers {
            let id = entry
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if id.is_empty() {
                continue;
            }
            let name = entry
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(id.as_str())
                .to_string();
            let url = entry
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let model = entry
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let protocol = entry
                .get("protocol")
                .and_then(|v| v.as_str())
                .unwrap_or("openai")
                .to_string();
            let api_key = entry
                .get("api_key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let active = active_id.as_deref() == Some(id.as_str());
            store.upsert_personal_provider(&PersonalProviderRecord {
                id: id.clone(),
                name,
                url,
                model,
                protocol,
                active,
            })?;
            if !api_key.is_empty() {
                store.set_personal_api_key(&id, &api_key)?;
            }
        }
        break;
    }
    Ok(())
}

fn migrate_skills_dir_hint(store: &SettingsStore) -> Result<()> {
    let mut team = store.get_team_settings()?;
    if team
        .skills_dir
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some()
    {
        return Ok(());
    }
    let modern = super::db::default_skills_cache_dir();
    let legacy = super::db::legacy_evotown_skills_dir();
    if !modern.exists() && legacy.is_dir() {
        // Prefer keeping legacy path readable rather than copying large trees.
        team.skills_dir = Some(legacy.display().to_string());
        store.set_team_settings(&team)?;
    }
    Ok(())
}

fn candidate_evotown_env_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(config) = dirs::config_dir() {
        paths.push(config.join("evotown").join("evotown.agent.env"));
    }
    if let Some(home) = dirs::home_dir() {
        paths.push(
            home.join(".config")
                .join("evotown")
                .join("evotown.agent.env"),
        );
    }
    paths
}

fn candidate_company_profile_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(config) = dirs::config_dir() {
        let base = config.join("agent-doctor");
        paths.push(base.join("company-profile.env"));
        paths.push(base.join("profile.env"));
    }
    if let Some(home) = dirs::home_dir() {
        let base = home.join(".config").join("agent-doctor");
        paths.push(base.join("company-profile.env"));
        paths.push(base.join("profile.env"));
    }
    paths
}

fn candidate_providers_json_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(config) = dirs::config_dir() {
        paths.push(config.join("agent-doctor").join("providers.json"));
    }
    if let Some(home) = dirs::home_dir() {
        paths.push(
            home.join(".config")
                .join("agent-doctor")
                .join("providers.json"),
        );
    }
    paths
}

pub fn read_env_map(path: &Path) -> Result<HashMap<String, String>> {
    let raw = fs::read_to_string(path)?;
    let mut values = HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let assignment = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = assignment.split_once('=') else {
            continue;
        };
        values.insert(
            key.trim().to_string(),
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string(),
        );
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::secrets::MemorySecretBackend;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn migrates_evotown_agent_env() {
        let dir = tempdir().unwrap();
        let env_path = dir.path().join("evotown.agent.env");
        fs::write(
            &env_path,
            "EVOTOWN_URL=https://evotown.example\nEVOTOWN_API_KEY=evk_testkey123\nEVOTOWN_RUNTIME=claude-code\n",
        )
        .unwrap();

        let db_path = dir.path().join("settings.db");
        let store = SettingsStore::open_at(&db_path, Arc::new(MemorySecretBackend::new())).unwrap();

        // Inject by reading the file we wrote (simulate candidate path by calling apply directly).
        let map = read_env_map(&env_path).unwrap();
        apply_team_env_map(&store, &map).unwrap();

        let team = store.get_team_settings().unwrap();
        assert_eq!(team.base_url.as_deref(), Some("https://evotown.example"));
        assert_eq!(team.runtime.as_deref(), Some("claude-code"));
        assert_eq!(
            store.get_team_api_key().unwrap().as_deref(),
            Some("evk_testkey123")
        );
    }
}
