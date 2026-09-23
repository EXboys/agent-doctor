//! Mall catalog: always TeamUps packs/skills (personal store), then install.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;

use super::resolve::{load_local_skills_layout_from_store, resolve_cached_skill_dir};
use super::sync_router::{execute_skills_sync_with_store, SkillsSyncOptions};
use super::teamups::{TeamupsClient, TeamupsMallCatalog};
use crate::evotown::SyncReport;
use crate::store::{default_teamups_base_url, open_settings_store, SkillsSourceKind};

/// List the TeamUps store. Independent of Evotown/SkillLite skills-source overrides.
pub fn list_teamups_mall_catalog() -> Result<TeamupsMallCatalog> {
    let store = open_settings_store()?;
    let base_url = teamups_mall_base_url(&store)?;
    let token = store.get_teamups_license()?;
    let client = TeamupsClient::new(&base_url, token.clone())?;
    let mut items = client
        .list_catalog()
        .with_context(|| format!("could not load TeamUps store from {base_url}"))?;

    let installed = local_installed_skill_ids(&store)?;
    for item in &mut items {
        item.installed = match item.kind.as_str() {
            "skill" => {
                installed.contains(&item.id)
                    || resolve_cached_skill_dir(
                        &load_local_skills_layout_from_store(&store)?.skills_dir,
                        &item.id,
                    )
                    .is_some()
            }
            _ => pack_looks_installed(&store, &item.id, &installed),
        };
    }

    Ok(TeamupsMallCatalog {
        base_url,
        has_license: token
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        items,
    })
}

pub fn install_teamups_mall_item(
    kind: &str,
    id: &str,
    pack_slug: Option<&str>,
) -> Result<SyncReport> {
    let kind = kind.trim().to_ascii_lowercase();
    let id = id.trim();
    if id.is_empty() {
        bail!("mall item id must not be empty");
    }

    let store = open_settings_store()?;
    // Mall installs always go through TeamUps, even if skills sync is set to Evotown.
    let (pack_id, only_skills) = if kind == "skill" {
        // Single-skill purchases (e.g. title-polish) use their slug as the doctor manifest id.
        // Do not fall back to Provider "default pack" — that is for bulk sync only.
        let pack = pack_slug
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| id.to_string());
        (pack, vec![id.to_string()])
    } else {
        (id.to_string(), Vec::new())
    };

    let report = execute_skills_sync_with_store(
        &store,
        &SkillsSyncOptions {
            dry_run: false,
            only_skills,
            runtime_target: None,
            pack_or_bundle_id: Some(pack_id),
            source_override: Some(SkillsSourceKind::Teamups),
        },
    )?;
    if report.installed == 0 {
        let detail = report
            .outcomes
            .iter()
            .find(|o| o.outcome == "failed")
            .and_then(|o| o.detail.clone())
            .unwrap_or_else(|| "nothing was downloaded".to_string());
        bail!("TeamUps install failed: {detail}");
    }
    Ok(report)
}

fn teamups_mall_base_url(store: &crate::store::SettingsStore) -> Result<String> {
    // Prefer an explicit TeamUps base_url only when the configured source is TeamUps/Custom.
    // Evotown/SkillLite overrides must not steal the personal store.
    let configured = store.get_skills_source_settings()?;
    let kind = configured.source.unwrap_or(SkillsSourceKind::Teamups);
    if matches!(kind, SkillsSourceKind::Teamups | SkillsSourceKind::Custom) {
        if let Some(url) = configured
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(url.trim_end_matches('/').to_string());
        }
    }
    Ok(default_teamups_base_url())
}

fn local_installed_skill_ids(store: &crate::store::SettingsStore) -> Result<HashSet<String>> {
    let layout = load_local_skills_layout_from_store(store)?;
    let mut ids = HashSet::new();
    if layout.skills_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&layout.skills_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        ids.insert(name.to_string());
                    }
                }
            }
        }
    }
    if layout.skills_lock_path.is_file() {
        if let Ok(raw) = fs::read_to_string(&layout.skills_lock_path) {
            if let Ok(value) = serde_json::from_str::<Value>(&raw) {
                if let Some(map) = value.get("skills").and_then(Value::as_object) {
                    for key in map.keys() {
                        ids.insert(key.clone());
                    }
                }
            }
        }
    }
    Ok(ids)
}

fn pack_looks_installed(
    store: &crate::store::SettingsStore,
    pack_id: &str,
    installed: &HashSet<String>,
) -> bool {
    let Ok(layout) = load_local_skills_layout_from_store(store) else {
        return false;
    };
    if !layout.skills_lock_path.is_file() {
        return false;
    }
    let Ok(raw) = fs::read_to_string(&layout.skills_lock_path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    let lock_pack = value
        .get("bundle_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    lock_pack == pack_id && !installed.is_empty()
}

#[cfg(test)]
mod tests {
    use super::super::teamups::parse_catalog_entries;
    use serde_json::json;

    #[test]
    fn parses_teamups_pack_catalog() {
        let raw = json!({
            "packs": [
                {
                    "slug": "starter",
                    "name": "Starter Pack",
                    "description": "Free combo",
                    "free": true,
                    "skills": [{"id": "a"}, {"id": "b"}]
                },
                {
                    "slug": "pro",
                    "name": "Pro Pack",
                    "price_cents": 1990,
                    "currency": "CNY",
                    "owned": false
                },
                {
                    "slug": "browser",
                    "name": "Browser Pack",
                    "free": true,
                    "skills": [{"id": "agent-browser"}]
                }
            ]
        });
        let items = parse_catalog_entries(&raw, "pack", "https://teamups.vip");
        assert_eq!(items.len(), 3);
        assert!(items[0].free);
        assert!(!items[1].free);
        assert_eq!(items[2].id, "browser");
    }

    #[test]
    #[ignore = "hits live teamups.vip + local license"]
    fn live_install_title_polish() {
        let report =
            super::install_teamups_mall_item("skill", "title-polish", Some("title-polish"))
                .expect("install title-polish");
        assert!(
            report.installed >= 1,
            "expected install, got report: {:?}",
            report
        );
    }

    #[test]
    #[ignore = "hits live teamups.vip"]
    fn live_teamups_vip_catalog() {
        let client =
            super::super::teamups::TeamupsClient::new("https://teamups.vip", None).expect("client");
        let items = client.list_catalog().expect("list catalog");
        assert!(
            items.len() >= 2,
            "expected published packs from teamups.vip, got {}",
            items.len()
        );
        assert!(items.iter().any(|i| i.id == "ecommerce-cs"));
        assert!(items.iter().any(|i| i.id == "weekly-report"));
        assert!(items
            .iter()
            .all(|i| !i.free || i.price_label.as_deref() == Some("Free")));
    }
}
