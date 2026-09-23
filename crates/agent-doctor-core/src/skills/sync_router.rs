//! Route skills sync through the configured (or default TeamUps) source.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::custom::CustomSkillsSource;
use super::resolve::{
    load_local_skills_layout_from_store, resolve_skills_source_from_store, ResolvedSkillsSource,
};
use super::teamups::TeamupsClient;
use super::SkillsSyncSource;
use crate::evotown::{
    execute_sync as execute_evotown_sync, EvotownConfig, SyncOptions, SyncReport,
};
use crate::setup::{DEFAULT_EVOTOWN_BUNDLE_ID, DEFAULT_EVOTOWN_RUNTIME};
use crate::store::{open_settings_store, SettingsStore, SkillsSourceKind};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsSyncOptions {
    pub dry_run: bool,
    pub only_skills: Vec<String>,
    pub runtime_target: Option<String>,
    /// Pack slug (TeamUps/custom) or bundle id (Evotown).
    pub pack_or_bundle_id: Option<String>,
    /// One-shot CLI override; does not persist.
    pub source_override: Option<SkillsSourceKind>,
}

pub fn execute_skills_sync(options: &SkillsSyncOptions) -> Result<SyncReport> {
    let store = open_settings_store()?;
    execute_skills_sync_with_store(&store, options)
}

pub fn execute_skills_sync_with_store(
    store: &SettingsStore,
    options: &SkillsSyncOptions,
) -> Result<SyncReport> {
    let resolved = resolve_skills_source_from_store(store, options.source_override)?;
    match resolved.kind {
        SkillsSourceKind::Evotown => sync_via_evotown(store, &resolved, options),
        SkillsSourceKind::Local => {
            bail!(
                "skills source is `local` — nothing to download; mount skills from the local cache instead"
            )
        }
        SkillsSourceKind::Teamups | SkillsSourceKind::Custom => {
            sync_via_generic(store, &resolved, options)
        }
    }
}

fn sync_via_evotown(
    store: &SettingsStore,
    resolved: &ResolvedSkillsSource,
    options: &SkillsSyncOptions,
) -> Result<SyncReport> {
    if resolved.base_url.trim().is_empty() {
        bail!(
            "Evotown skills source has no URL — connect your team in Provider, or clear the \
             skills source override to use the default TeamUps catalog"
        );
    }
    let Some(api_key) = resolved
        .token
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        bail!(
            "Evotown API key missing — open Agent Doctor → Provider to connect your team \
             (key is stored in the {})",
            crate::store::platform_secret_store_name()
        );
    };
    let layout = load_local_skills_layout_from_store(store)?;
    let team = store.get_team_settings()?;
    let config = EvotownConfig {
        base_url: resolved.base_url.clone(),
        api_key: api_key.to_string(),
        runtime_target: options
            .runtime_target
            .clone()
            .or(team.runtime)
            .unwrap_or_else(|| DEFAULT_EVOTOWN_RUNTIME.to_string()),
        bundle_id: options
            .pack_or_bundle_id
            .clone()
            .or_else(|| resolved.pack_slug.clone())
            .or(team.bundle_id)
            .unwrap_or_else(|| DEFAULT_EVOTOWN_BUNDLE_ID.to_string()),
        skills_dir: layout.skills_dir,
        skills_lock_path: layout.skills_lock_path,
        policy_cache_path: dirs::config_dir()
            .map(|b| b.join("evotown").join("policies-cache.json"))
            .unwrap_or_else(|| Path::new("policies-cache.json").to_path_buf()),
        config_source: format!("settings.db+{}", crate::store::platform_secret_store_name()),
    };
    execute_evotown_sync(
        &config,
        &SyncOptions {
            dry_run: options.dry_run,
            only_skills: options.only_skills.clone(),
            runtime_target: options.runtime_target.clone(),
            bundle_id: options.pack_or_bundle_id.clone(),
        },
    )
}

fn sync_via_generic(
    store: &SettingsStore,
    resolved: &ResolvedSkillsSource,
    options: &SkillsSyncOptions,
) -> Result<SyncReport> {
    let source: Box<dyn SkillsSyncSource> = match resolved.kind {
        SkillsSourceKind::Teamups => Box::new(TeamupsClient::new(
            &resolved.base_url,
            resolved.token.clone(),
        )?),
        SkillsSourceKind::Custom => Box::new(CustomSkillsSource::new(
            &resolved.base_url,
            resolved.token.clone(),
        )?),
        _ => unreachable!(),
    };

    let pack_id = options
        .pack_or_bundle_id
        .clone()
        .or_else(|| resolved.pack_slug.clone())
        .filter(|v| !v.trim().is_empty())
        .context(
            "pack slug required — set skills pack in Provider, or pass --bundle / pack id to sync",
        )?;

    let runtime_target = options
        .runtime_target
        .clone()
        .unwrap_or_else(|| DEFAULT_EVOTOWN_RUNTIME.to_string());

    let layout = load_local_skills_layout_from_store(store)?;
    fs::create_dir_all(&layout.skills_dir)?;

    let body = source.fetch_manifest(&pack_id, &runtime_target)?;
    let manifest = body.get("manifest").cloned().unwrap_or(body.clone());
    let skills = manifest
        .get("skills")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut state = load_lock_state(&layout.skills_lock_path)?;
    state["bundle_id"] = json!(pack_id);
    state["channel"] = json!(resolved.kind.as_str());
    state["runtime_target"] = json!(runtime_target);
    state["source"] = json!(resolved.kind.as_str());
    state["base_url"] = json!(resolved.base_url);
    state["updated_at"] = json!(utc_now());

    let lock = state
        .as_object_mut()
        .context("skills lock must be an object")?
        .entry("skills")
        .or_insert_with(|| json!({}));
    let lock_map = lock
        .as_object_mut()
        .context("skills lock.skills must be an object")?;

    let only: std::collections::HashSet<String> = options
        .only_skills
        .iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect();

    let mut installed = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut outcomes = Vec::new();

    for entry in skills {
        let skill_id = entry
            .get("id")
            .or_else(|| entry.get("skill_id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if skill_id.is_empty() {
            continue;
        }
        if !only.is_empty() && !only.contains(&skill_id) {
            continue;
        }
        let version = entry
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("0.0.0")
            .to_string();
        let expected_sha = entry
            .get("sha256")
            .or_else(|| entry.get("package_sha256"))
            .and_then(Value::as_str)
            .map(str::to_string);

        let target = layout.skills_dir.join(&skill_id);
        let prev = lock_map.get(&skill_id).cloned().unwrap_or(Value::Null);
        let prev_version = prev.get("version").and_then(Value::as_str).unwrap_or("");
        if prev_version == version
            && target.is_dir()
            && target
                .read_dir()
                .map(|mut d| d.next().is_some())
                .unwrap_or(false)
        {
            skipped += 1;
            outcomes.push(crate::evotown::SkillSyncOutcome {
                skill_id,
                version,
                outcome: "skipped".into(),
                detail: Some("up to date".into()),
            });
            continue;
        }

        if options.dry_run {
            installed += 1;
            outcomes.push(crate::evotown::SkillSyncOutcome {
                skill_id,
                version,
                outcome: "installed".into(),
                detail: Some("would download".into()),
            });
            continue;
        }

        match source.download_skill_zip(&pack_id, &skill_id) {
            Ok(blob) => {
                let digest = hex_sha256(&blob);
                if let Some(expected) = expected_sha.as_deref().filter(|v| !v.is_empty()) {
                    if digest != expected {
                        failed += 1;
                        outcomes.push(crate::evotown::SkillSyncOutcome {
                            skill_id,
                            version,
                            outcome: "failed".into(),
                            detail: Some("sha256 mismatch".into()),
                        });
                        continue;
                    }
                }
                if target.exists() {
                    let _ = fs::remove_dir_all(&target);
                }
                if let Err(err) = extract_zip_bytes(&blob, &target)
                    .and_then(|_| normalize_skill_extract_dir(&target))
                {
                    failed += 1;
                    outcomes.push(crate::evotown::SkillSyncOutcome {
                        skill_id,
                        version,
                        outcome: "failed".into(),
                        detail: Some(err.to_string()),
                    });
                    continue;
                }
                lock_map.insert(
                    skill_id.clone(),
                    json!({
                        "version": version,
                        "sha256": digest,
                        "installed_at": utc_now(),
                    }),
                );
                installed += 1;
                outcomes.push(crate::evotown::SkillSyncOutcome {
                    skill_id,
                    version,
                    outcome: "installed".into(),
                    detail: None,
                });
            }
            Err(err) => {
                failed += 1;
                outcomes.push(crate::evotown::SkillSyncOutcome {
                    skill_id,
                    version,
                    outcome: "failed".into(),
                    detail: Some(err.to_string()),
                });
            }
        }
    }

    if !options.dry_run {
        save_lock_state(&layout.skills_lock_path, &state)?;
    }

    Ok(SyncReport {
        base_url: resolved.base_url.clone(),
        bundle_id: pack_id,
        runtime_target,
        skills_dir: layout.skills_dir.display().to_string(),
        lock_path: layout.skills_lock_path.display().to_string(),
        installed,
        skipped,
        failed,
        outcomes,
    })
}

fn load_lock_state(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({ "skills": {} }));
    }
    let raw = fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({ "skills": {} }));
    Ok(value)
}

fn save_lock_state(path: &Path, state: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(state)? + "\n")?;
    Ok(())
}

/// If zip unpacked to `{id}/{id}/SKILL.md`, hoist files to `{id}/SKILL.md`.
fn normalize_skill_extract_dir(target_dir: &Path) -> Result<()> {
    if target_dir.join("SKILL.md").exists() {
        return Ok(());
    }
    let Some(name) = target_dir.file_name().and_then(|n| n.to_str()) else {
        return Ok(());
    };
    let nested = target_dir.join(name);
    if !nested.join("SKILL.md").exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&nested)? {
        let entry = entry?;
        let dest = target_dir.join(entry.file_name());
        if dest.exists() {
            if dest.is_dir() {
                fs::remove_dir_all(&dest)?;
            } else {
                fs::remove_file(&dest)?;
            }
        }
        fs::rename(entry.path(), &dest)?;
    }
    fs::remove_dir(&nested)?;
    Ok(())
}

fn extract_zip_bytes(content: &[u8], target_dir: &Path) -> Result<()> {
    fs::create_dir_all(target_dir)?;
    let reader = Cursor::new(content);
    let mut archive = zip::ZipArchive::new(reader).context("invalid skill zip archive")?;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .context("failed to read zip entry")?;
        let outpath = match file.enclosed_name() {
            Some(path) => target_dir.join(path),
            None => continue,
        };
        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath)?;
            continue;
        }
        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outfile = fs::File::create(&outpath)?;
        std::io::copy(&mut file, &mut outfile)?;
    }
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn utc_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::secrets::MemorySecretBackend;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn resolve_defaults_to_teamups_when_unset() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("settings.db");
        let store = SettingsStore::open_at(&db, Arc::new(MemorySecretBackend::new())).unwrap();
        let resolved = resolve_skills_source_from_store(&store, None).unwrap();
        assert_eq!(resolved.kind, SkillsSourceKind::Teamups);
        assert!(resolved.using_default);
    }

    #[test]
    fn resolve_respects_explicit_evotown() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("settings.db");
        let store = SettingsStore::open_at(&db, Arc::new(MemorySecretBackend::new())).unwrap();
        store
            .set_skills_source_settings(&crate::store::SkillsSourceSettings {
                source: Some(SkillsSourceKind::Evotown),
                base_url: Some("https://evotown.example".into()),
                pack_slug: Some("default-agent-skills".into()),
            })
            .unwrap();
        store.set_team_api_key("evk_test").unwrap();
        let resolved = resolve_skills_source_from_store(&store, None).unwrap();
        assert_eq!(resolved.kind, SkillsSourceKind::Evotown);
        assert!(!resolved.using_default);
        assert_eq!(resolved.base_url, "https://evotown.example");
    }

    #[test]
    fn local_source_refuses_remote_sync() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("settings.db");
        let store = SettingsStore::open_at(&db, Arc::new(MemorySecretBackend::new())).unwrap();
        store
            .set_skills_source_settings(&crate::store::SkillsSourceSettings {
                source: Some(SkillsSourceKind::Local),
                base_url: None,
                pack_slug: None,
            })
            .unwrap();
        let err =
            execute_skills_sync_with_store(&store, &SkillsSyncOptions::default()).unwrap_err();
        assert!(err.to_string().contains("local"));
        let _ = crate::skills::local::LocalSkillsSource;
    }
}
