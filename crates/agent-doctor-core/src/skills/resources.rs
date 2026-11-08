//! Free storefront skills (agentdoctor.cn /resources) inside the mall.
//!
//! Server contract: `GET {base}/api/v1/resources/skills` lists the free
//! skills shown on the /resources page. Items with `hosted.status == "ok"`
//! carry a site-hosted zip (`zipDownload`), its sha256, and the upstream
//! version; the other rows ("missing") have no installable package yet and
//! stay off the mall list. Zip bytes come from `GET {base}{zipDownload}`.
//!
//! Installs land in the same global skills directory as TeamUps syncs and are
//! recorded in the same lock file — but only the per-skill entry, never
//! `bundle_id`, so a resource install cannot hijack an installed pack.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::time::Duration;

use super::resolve::{load_local_skills_layout_from_store, LocalSkillsLayout};
use super::sync_router::{
    extract_zip_bytes, load_lock_state, normalize_skill_extract_dir, save_lock_state, utc_now,
};
use super::teamups::TeamupsCatalogItem;
use crate::evotown::{SkillSyncOutcome, SyncReport};

const RESOURCES_PATH: &str = "/api/v1/resources/skills";
/// Mirror of the local-install budget; the storefront enforces smaller caps.
const MAX_ZIP_BYTES: u64 = 80 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct ResourcesPayload {
    #[serde(default)]
    categories: Vec<ResourceCategory>,
}

#[derive(Debug, Deserialize)]
struct ResourceCategory {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    items: Vec<ResourceItem>,
}

#[derive(Debug, Deserialize)]
struct ResourceItem {
    key: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    desc: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    downloads: Option<u64>,
    #[serde(rename = "detailUrl", default)]
    detail_url: Option<String>,
    #[serde(default)]
    hosted: Option<HostedZip>,
}

#[derive(Debug, Deserialize)]
struct HostedZip {
    #[serde(default)]
    status: Option<String>,
    #[serde(rename = "zipDownload", default)]
    zip_download: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

/// Fetch the free storefront catalog as mall rows (`kind: "skill"`, free).
pub fn fetch_free_resources(base_url: &str) -> Result<Vec<TeamupsCatalogItem>> {
    let base = base_url.trim().trim_end_matches('/');
    let url = format!("{base}{RESOURCES_PATH}");
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("failed to build free-resources HTTP client")?;
    let resp = http
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    if !status.is_success() {
        bail!("free resources list failed ({status}): {body}");
    }
    let payload: ResourcesPayload =
        serde_json::from_str(&body).context("invalid free resources JSON")?;
    Ok(resources_to_items(&payload, base))
}

fn resources_to_items(payload: &ResourcesPayload, base_url: &str) -> Vec<TeamupsCatalogItem> {
    let mut out = Vec::new();
    for category in &payload.categories {
        for item in &category.items {
            let Some(hosted) = item
                .hosted
                .as_ref()
                .filter(|h| h.status.as_deref() == Some("ok"))
            else {
                continue;
            };
            let Some(zip_download) = hosted
                .zip_download
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            else {
                continue;
            };
            let key = item.key.trim();
            if !safe_skill_id(key) {
                continue;
            }
            out.push(TeamupsCatalogItem {
                id: key.to_string(),
                kind: "skill".to_string(),
                name: item.name.clone().unwrap_or_else(|| key.to_string()),
                description: item.desc.clone().unwrap_or_default(),
                free: true,
                price_label: None,
                owned: false,
                installed: false,
                skill_count: None,
                pack_slug: None,
                purchase_url: item
                    .detail_url
                    .as_deref()
                    .map(|path| absolute_url(base_url, path)),
                version: hosted.version.clone(),
                origin: Some("resource".to_string()),
                category_label: category.label.clone(),
                download_url: Some(absolute_url(base_url, zip_download)),
                sha256: hosted.sha256.clone(),
                downloads: item.downloads,
                author: item.author.clone(),
            });
        }
    }
    out
}

/// Install one free storefront skill by its catalog key.
pub fn install_free_resource(id: &str) -> Result<SyncReport> {
    let id = id.trim();
    if !safe_skill_id(id) {
        bail!("free resource id must be a safe folder name");
    }
    let store = crate::store::open_settings_store()?;
    let base_url = super::mall::teamups_mall_base_url(&store)?;
    let items = fetch_free_resources(&base_url)
        .with_context(|| format!("could not load free resources from {base_url}"))?;
    let item = items
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| anyhow::anyhow!("free resource `{id}` is not in the storefront catalog"))?;
    let blob = download_resource_zip(item.download_url.as_deref().unwrap_or_default())?;
    let layout = load_local_skills_layout_from_store(&store)?;
    install_resource_blob(
        &layout,
        id,
        item.version.as_deref(),
        item.sha256.as_deref(),
        &blob,
    )
}

fn download_resource_zip(url: &str) -> Result<Vec<u8>> {
    if url.is_empty() {
        bail!("free resource has no hosted zip");
    }
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("failed to build free-resources HTTP client")?;
    let resp = http.get(url).send().with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        bail!("free resource download failed ({status}): {body}");
    }
    let blob = resp.bytes()?.to_vec();
    if blob.len() as u64 > MAX_ZIP_BYTES {
        bail!("free resource zip exceeds the {MAX_ZIP_BYTES} byte budget");
    }
    Ok(blob)
}

/// Extract, verify, and record one resource zip. Shared by the live path and tests.
fn install_resource_blob(
    layout: &LocalSkillsLayout,
    id: &str,
    version: Option<&str>,
    expected_sha: Option<&str>,
    blob: &[u8],
) -> Result<SyncReport> {
    if !safe_skill_id(id) {
        bail!("free resource id must be a safe folder name");
    }
    fs::create_dir_all(&layout.skills_dir)?;

    let digest = hex_sha256(blob);
    if let Some(expected) = expected_sha.map(str::trim).filter(|v| !v.is_empty()) {
        if !digest.eq_ignore_ascii_case(expected) {
            bail!("sha256 mismatch: expected {expected}, got {digest}");
        }
    }

    let target = layout.skills_dir.join(id);
    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    extract_zip_bytes(blob, &target)
        .and_then(|_| normalize_skill_extract_dir(&target))
        .context("invalid skill zip archive")?;
    if !target.join("SKILL.md").is_file() {
        let _ = fs::remove_dir_all(&target);
        bail!("invalid skill zip: {id} archive has no SKILL.md");
    }

    let mut state = load_lock_state(&layout.skills_lock_path)?;
    if let Some(lock) = state.get_mut("skills").and_then(Value::as_object_mut) {
        lock.insert(
            id.to_string(),
            json!({
                "version": version.unwrap_or("0.0.0"),
                "sha256": digest,
                "installed_at": utc_now(),
                "source": "resources",
            }),
        );
    }
    save_lock_state(&layout.skills_lock_path, &state)?;

    Ok(SyncReport {
        base_url: String::new(),
        bundle_id: id.to_string(),
        runtime_target: String::new(),
        skills_dir: layout.skills_dir.display().to_string(),
        lock_path: layout.skills_lock_path.display().to_string(),
        installed: 1,
        skipped: 0,
        failed: 0,
        outcomes: vec![SkillSyncOutcome {
            skill_id: id.to_string(),
            version: version.unwrap_or("0.0.0").to_string(),
            outcome: "installed".into(),
            detail: None,
        }],
    })
}

fn absolute_url(base_url: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    if path.starts_with('/') {
        return format!("{}{}", base_url.trim_end_matches('/'), path);
    }
    format!("{}/{}", base_url.trim_end_matches('/'), path)
}

/// Only plain folder names; the id becomes a directory under skills_dir.
fn safe_skill_id(id: &str) -> bool {
    !id.is_empty()
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains("..")
        && !id.starts_with('.')
        && id.chars().all(|ch| !ch.is_whitespace())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tempfile::tempdir;

    fn sample_payload() -> Value {
        json!({
            "ok": true,
            "updatedAt": "2026-11-07",
            "categories": [
                {
                    "id": "agents",
                    "label": "用助手",
                    "items": [
                        {
                            "author": "vercel",
                            "name": "Find Skills",
                            "desc": "按你当前的需求自动找合适的技能",
                            "downloads": 1303456,
                            "key": "find-skills",
                            "detailUrl": "/resources/s/find-skills",
                            "hosted": {
                                "status": "ok",
                                "zipDownload": "/api/v1/resources/skills/zip?name=Find%20Skills&author=vercel",
                                "sha256": "05e607d95c0af091bc4fc8d6a40c67b4e1a0a7b7b39868a1b83443589700b732",
                                "sizeBytes": 5013,
                                "version": "1.7.0",
                                "formatBytes": "4.9 KB"
                            }
                        },
                        {
                            "name": "Openai Whisper",
                            "key": "openai-whisper",
                            "hosted": {"status": "missing"}
                        },
                        {
                            "name": "No Package",
                            "key": "no-package"
                        }
                    ]
                }
            ]
        })
    }

    #[test]
    fn parses_hosted_resources_and_skips_missing_packages() {
        let payload: ResourcesPayload = serde_json::from_value(sample_payload()).unwrap();
        let items = resources_to_items(&payload, "https://agentdoctor.cn");
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, "find-skills");
        assert_eq!(item.kind, "skill");
        assert!(item.free);
        assert!(!item.owned);
        assert_eq!(item.origin.as_deref(), Some("resource"));
        assert_eq!(item.category_label.as_deref(), Some("用助手"));
        assert_eq!(item.version.as_deref(), Some("1.7.0"));
        assert_eq!(item.downloads, Some(1_303_456));
        assert_eq!(
            item.download_url.as_deref(),
            Some("https://agentdoctor.cn/api/v1/resources/skills/zip?name=Find%20Skills&author=vercel")
        );
        assert_eq!(
            item.purchase_url.as_deref(),
            Some("https://agentdoctor.cn/resources/s/find-skills")
        );
    }

    #[test]
    fn keeps_absolute_download_urls_and_rejects_unsafe_keys() {
        let mut payload: ResourcesPayload = serde_json::from_value(sample_payload()).unwrap();
        payload.categories[0].items[0]
            .hosted
            .as_mut()
            .unwrap()
            .zip_download = Some("https://mirror.example/x.zip".into());
        payload.categories[0].items[0].key = "../escape".into();
        let items = resources_to_items(&payload, "https://agentdoctor.cn");
        assert!(items.is_empty(), "unsafe keys must not become mall rows");
    }

    fn zip_with_skill(name: &str, body: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let options: zip::write::SimpleFileOptions = Default::default();
            writer
                .start_file(format!("{name}/SKILL.md"), options)
                .unwrap();
            writer.write_all(body.as_bytes()).unwrap();
            writer.finish().unwrap();
        }
        buf.into_inner()
    }

    fn temp_layout(tag: &str) -> (tempfile::TempDir, LocalSkillsLayout) {
        let dir = tempdir().unwrap();
        let layout = LocalSkillsLayout {
            skills_dir: dir.path().join(format!("{tag}-skills")),
            skills_lock_path: dir.path().join(format!("{tag}-skills-lock.json")),
        };
        (dir, layout)
    }

    #[test]
    fn installs_resource_zip_and_records_lock_without_bundle_id() {
        let (_dir, layout) = temp_layout("one");
        fs::create_dir_all(&layout.skills_dir).unwrap();
        // A pre-existing pack lock must survive the resource install untouched.
        fs::write(
            &layout.skills_lock_path,
            serde_json::to_string_pretty(&json!({
                "bundle_id": "pro-pack",
                "skills": {"title-polish": {"version": "1.0.0"}}
            }))
            .unwrap(),
        )
        .unwrap();

        let blob = zip_with_skill("find-skills", "---\nname: Find Skills\n---\n");
        let report =
            install_resource_blob(&layout, "find-skills", Some("1.7.0"), None, &blob).unwrap();
        assert_eq!(report.installed, 1);
        assert!(layout
            .skills_dir
            .join("find-skills")
            .join("SKILL.md")
            .is_file());

        let lock: Value =
            serde_json::from_str(&fs::read_to_string(&layout.skills_lock_path).unwrap()).unwrap();
        assert_eq!(
            lock.get("bundle_id").and_then(Value::as_str),
            Some("pro-pack")
        );
        assert!(lock["skills"].get("title-polish").is_some());
        let entry = lock.get("skills").and_then(|s| s.get("find-skills"));
        assert_eq!(
            entry.and_then(|v| v.get("version")).and_then(Value::as_str),
            Some("1.7.0")
        );
        assert_eq!(
            entry.and_then(|v| v.get("source")).and_then(Value::as_str),
            Some("resources")
        );
    }

    #[test]
    fn hoists_nested_skill_dirs_inside_the_zip() {
        let (_dir, layout) = temp_layout("nested");
        // TeamUps-style zips unpack to {id}/{id}/SKILL.md — normalize must hoist it.
        let blob = zip_with_skill("demo", "---\nname: demo\n---\n");
        install_resource_blob(&layout, "demo", None, None, &blob).unwrap();
        assert!(layout.skills_dir.join("demo").join("SKILL.md").is_file());
    }

    #[test]
    fn rejects_sha_mismatch_and_zips_without_skill_md() {
        let (_dir, layout) = temp_layout("bad");
        let blob = zip_with_skill("demo", "---\nname: demo\n---\n");

        let err =
            install_resource_blob(&layout, "demo", None, Some("deadbeef"), &blob).unwrap_err();
        assert!(err.to_string().contains("sha256 mismatch"), "{err}");

        let plain = b"not a zip".to_vec();
        let err = install_resource_blob(&layout, "demo", None, None, &plain).unwrap_err();
        assert!(err.to_string().contains("invalid skill zip"), "{err}");

        let no_skill_md = {
            let mut buf = std::io::Cursor::new(Vec::new());
            {
                let mut writer = zip::ZipWriter::new(&mut buf);
                let options: zip::write::SimpleFileOptions = Default::default();
                writer.start_file("readme.txt", options).unwrap();
                writer.write_all(b"hello").unwrap();
                writer.finish().unwrap();
            }
            buf.into_inner()
        };
        let err = install_resource_blob(&layout, "demo", None, None, &no_skill_md).unwrap_err();
        assert!(err.to_string().contains("no SKILL.md"), "{err}");
        assert!(
            !layout.skills_dir.join("demo").exists(),
            "failed installs must not leave a half skill behind"
        );
    }

    #[test]
    fn refuses_unsafe_ids() {
        let (_dir, layout) = temp_layout("unsafe");
        let blob = zip_with_skill("demo", "---\nname: demo\n---\n");
        for id in ["../escape", "", "a/b", ".hidden", "space id"] {
            let err = install_resource_blob(&layout, id, None, None, &blob).unwrap_err();
            assert!(err.to_string().contains("safe folder name"), "{id}: {err}");
        }
    }

    #[test]
    fn absolute_url_handles_relative_and_absolute() {
        assert_eq!(
            absolute_url("https://agentdoctor.cn", "/api/v1/x.zip"),
            "https://agentdoctor.cn/api/v1/x.zip"
        );
        assert_eq!(
            absolute_url("https://agentdoctor.cn", "https://else.example/x.zip"),
            "https://else.example/x.zip"
        );
    }

    #[test]
    #[ignore = "hits live agentdoctor.cn"]
    fn live_resources_catalog_lists_installable_rows() {
        let items = fetch_free_resources("https://agentdoctor.cn").expect("live resources catalog");
        assert!(
            items.len() >= 20,
            "expected the storefront free skills, got {}",
            items.len()
        );
        assert!(items.iter().all(|item| item.free));
        assert!(items.iter().all(|item| item.download_url.is_some()));
        assert!(items.iter().any(|item| item.id == "find-skills"));
        assert!(items.iter().all(|item| item
            .category_label
            .as_deref()
            .is_some_and(|v| !v.is_empty())));
    }
}
