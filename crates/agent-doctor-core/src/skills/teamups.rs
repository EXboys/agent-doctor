//! TeamUps pack/skills HTTP client (manifest + zip + License).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::SkillsSyncSource;
use crate::store::SkillsSourceKind;

#[derive(Debug, Clone)]
pub struct TeamupsClient {
    base_url: String,
    license: Option<String>,
    http: reqwest::blocking::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamupsSkillEntry {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamupsPackManifest {
    pub slug: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub skills: Vec<TeamupsSkillEntry>,
}

/// One mall row: a TeamUps pack (组合) or a skill inside a pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamupsCatalogItem {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub free: bool,
    #[serde(default)]
    pub price_label: Option<String>,
    #[serde(default)]
    pub owned: bool,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub skill_count: Option<u32>,
    #[serde(default)]
    pub pack_slug: Option<String>,
    #[serde(default)]
    pub purchase_url: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamupsMallCatalog {
    pub base_url: String,
    pub has_license: bool,
    pub items: Vec<TeamupsCatalogItem>,
}

impl TeamupsClient {
    pub fn new(base_url: &str, license: Option<String>) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("failed to build TeamUps HTTP client")?;
        Ok(Self {
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            license,
            http,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn has_license(&self) -> bool {
        self.license
            .as_deref()
            .map(str::trim)
            .is_some_and(|v| !v.is_empty())
    }

    fn auth_header(
        &self,
        req: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        match &self.license {
            Some(token) if !token.trim().is_empty() => {
                req.header("Authorization", format!("Bearer {}", token.trim()))
            }
            _ => req,
        }
    }

    pub fn list_packs(&self) -> Result<Value> {
        // Public storefront catalog — no Bearer (teamups.vip).
        let url = format!("{}/api/v1/packs", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        if !status.is_success() {
            bail!("TeamUps packs list failed ({status}): {body}");
        }
        serde_json::from_str(&body).context("invalid TeamUps packs JSON")
    }

    /// Owned packs for the current license (`/api/v1/doctor/packs`).
    pub fn list_owned_packs(&self) -> Result<Value> {
        if !self.has_license() {
            bail!("TeamUps license required");
        }
        let url = format!("{}/api/v1/doctor/packs", self.base_url);
        let req = self.auth_header(self.http.get(&url).header("Accept", "application/json"));
        let resp = req.send().with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            bail!("TeamUps license rejected ({status}): {body}");
        }
        if !status.is_success() {
            bail!("TeamUps owned packs failed ({status}): {body}");
        }
        serde_json::from_str(&body).context("invalid TeamUps owned packs JSON")
    }

    /// Account summary (`/api/v1/doctor/me`) — `packs` is a string slug list.
    pub fn doctor_me(&self) -> Result<Value> {
        if !self.has_license() {
            bail!("TeamUps license required");
        }
        let url = format!("{}/api/v1/doctor/me", self.base_url);
        let req = self.auth_header(self.http.get(&url).header("Accept", "application/json"));
        let resp = req.send().with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            bail!("TeamUps license rejected ({status}): {body}");
        }
        if !status.is_success() {
            bail!("TeamUps account check failed ({status}): {body}");
        }
        serde_json::from_str(&body).context("invalid TeamUps doctor/me JSON")
    }

    /// Optional skills index; missing endpoint is treated as empty.
    pub fn list_skills_index(&self) -> Result<Value> {
        let url = format!("{}/api/v1/skills", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        if status.as_u16() == 404 {
            return Ok(json!({ "skills": [] }));
        }
        // Next.js HTML 404 sometimes returns 200 with a document — ignore non-JSON.
        if !status.is_success() || body.trim_start().starts_with('<') {
            return Ok(json!({ "skills": [] }));
        }
        serde_json::from_str(&body).context("invalid TeamUps skills JSON")
    }

    pub fn fetch_pack_manifest(&self, slug: &str) -> Result<TeamupsPackManifest> {
        // Doctor install contract on teamups.vip.
        let url = format!("{}/api/v1/doctor/packs/{slug}/manifest", self.base_url);
        let req = self.auth_header(self.http.get(&url).header("Accept", "application/json"));
        let resp = req.send().with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            bail!(
                "TeamUps pack `{slug}` requires a license — open Agent Doctor → Account / Provider \
                 and paste your TeamUps license"
            );
        }
        if !status.is_success() {
            bail!("TeamUps manifest failed ({status}): {body}");
        }
        let value: Value = serde_json::from_str(&body).context("invalid TeamUps manifest JSON")?;
        Ok(manifest_from_doctor_json(value, slug))
    }

    pub fn download_skill(&self, slug: &str, _skill_id: &str) -> Result<Vec<u8>> {
        // teamups.vip: whole-pack zip via doctor manifest artifact URL.
        let url = format!("{}/api/v1/doctor/packs/{slug}/manifest", self.base_url);
        let req = self.auth_header(self.http.get(&url).header("Accept", "application/json"));
        let resp = req.send().with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            bail!(
                "TeamUps pack `{slug}` requires a license — configure TeamUps license in Provider"
            );
        }
        if !status.is_success() {
            bail!("TeamUps manifest failed ({status}): {body}");
        }
        let value: Value = serde_json::from_str(&body).context("invalid TeamUps manifest JSON")?;
        let artifact_url = value
            .pointer("/artifact/url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .context("TeamUps manifest missing artifact.url")?;
        let download_url =
            if artifact_url.starts_with("http://") || artifact_url.starts_with("https://") {
                artifact_url.to_string()
            } else {
                format!(
                    "{}{}",
                    self.base_url,
                    if artifact_url.starts_with('/') {
                        artifact_url.to_string()
                    } else {
                        format!("/{artifact_url}")
                    }
                )
            };
        let req = self.auth_header(self.http.get(&download_url));
        let resp = req.send().with_context(|| format!("GET {download_url}"))?;
        let status = resp.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            bail!("TeamUps pack download requires a license");
        }
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            bail!("TeamUps pack download failed ({status}): {body}");
        }
        let bytes = resp.bytes().context("failed to read pack zip bytes")?;
        Ok(bytes.to_vec())
    }

    pub fn list_catalog(&self) -> Result<Vec<TeamupsCatalogItem>> {
        let packs_json = self.list_packs().context("TeamUps packs list failed")?;
        let mut items = parse_catalog_entries(&packs_json, "pack", &self.base_url);

        if let Ok(skills_json) = self.list_skills_index() {
            let skills = parse_catalog_entries(&skills_json, "skill", &self.base_url);
            for skill in skills {
                if !items
                    .iter()
                    .any(|item| item.id == skill.id && item.kind == skill.kind)
                {
                    items.push(skill);
                }
            }
        }

        if self.has_license() {
            let owned_json = self.list_owned_packs().ok();
            let me_json = self.doctor_me().ok();
            merge_owned_into_catalog(
                &mut items,
                owned_json.as_ref(),
                me_json.as_ref(),
                &self.base_url,
            );
        }

        Ok(items)
    }
}

impl SkillsSyncSource for TeamupsClient {
    fn kind(&self) -> SkillsSourceKind {
        SkillsSourceKind::Teamups
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn fetch_manifest(&self, pack_or_bundle_id: &str, _runtime_target: &str) -> Result<Value> {
        let manifest = self.fetch_pack_manifest(pack_or_bundle_id)?;
        let skills: Vec<Value> = manifest
            .skills
            .iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "version": s.version,
                    "sha256": s.sha256,
                })
            })
            .collect();
        Ok(json!({
            "manifest": {
                "bundle_id": manifest.slug,
                "channel": "teamups",
                "skills": skills,
            }
        }))
    }

    fn download_skill_zip(&self, pack_or_bundle_id: &str, skill_id: &str) -> Result<Vec<u8>> {
        self.download_skill(pack_or_bundle_id, skill_id)
    }
}

/// Doctor `/packs/{slug}/manifest` — skill-only rows often have `version` + `artifact` but no `skills[]`.
fn manifest_from_doctor_json(value: Value, slug: &str) -> TeamupsPackManifest {
    if let Ok(manifest) = serde_json::from_value::<TeamupsPackManifest>(value.clone()) {
        if !manifest.skills.is_empty() {
            return manifest;
        }
    }
    let version = value
        .get("version")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("artifact")
                .and_then(|a| a.get("version"))
                .and_then(Value::as_str)
        })
        .map(str::to_string);
    let skill_version = version.clone().unwrap_or_else(|| "0.0.0".into());
    TeamupsPackManifest {
        slug: value
            .get("slug")
            .and_then(Value::as_str)
            .unwrap_or(slug)
            .to_string(),
        version,
        skills: vec![TeamupsSkillEntry {
            id: slug.to_string(),
            version: skill_version,
            sha256: value
                .get("artifact")
                .and_then(|a| a.get("sha256"))
                .and_then(Value::as_str)
                .map(str::to_string),
        }],
    }
}

fn collect_pack_ids(value: &Value) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    let Some(entries) = json_array(value) else {
        return ids;
    };
    for entry in entries {
        if let Some(text) = entry.as_str() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                ids.insert(trimmed.to_string());
            }
            continue;
        }
        if let Some(id) = string_field(entry, &["slug", "id", "pack_slug", "pack_id", "bundle_id"])
        {
            ids.insert(id);
        }
    }
    ids
}

/// Mark public catalog rows the account owns, and append owned packs that
/// the public storefront did not list.
pub(crate) fn merge_owned_into_catalog(
    items: &mut Vec<TeamupsCatalogItem>,
    owned_json: Option<&Value>,
    me_json: Option<&Value>,
    base_url: &str,
) {
    let mut owned_ids = std::collections::HashSet::new();
    let mut owned_rows = Vec::new();
    if let Some(owned_json) = owned_json {
        owned_ids.extend(collect_pack_ids(owned_json));
        owned_rows = parse_catalog_entries(owned_json, "pack", base_url);
        for row in &mut owned_rows {
            row.owned = true;
            // doctor/packs has no priceCents; do not treat those rows as free.
            row.free = false;
        }
    }
    if let Some(me_json) = me_json {
        owned_ids.extend(collect_pack_ids(me_json));
    }
    if owned_ids.is_empty() && owned_rows.is_empty() {
        return;
    }

    for item in items.iter_mut() {
        if owned_ids.contains(&item.id)
            || item
                .pack_slug
                .as_deref()
                .is_some_and(|slug| owned_ids.contains(slug))
        {
            item.owned = true;
        }
    }

    for mut row in owned_rows {
        if items.iter().any(|item| item.id == row.id) {
            continue;
        }
        row.owned = true;
        items.push(row);
    }
    for id in owned_ids {
        if items.iter().any(|item| item.id == id) {
            continue;
        }
        items.push(TeamupsCatalogItem {
            id: id.clone(),
            kind: "pack".to_string(),
            name: id.clone(),
            description: String::new(),
            free: false,
            price_label: None,
            owned: true,
            installed: false,
            skill_count: None,
            pack_slug: Some(id),
            purchase_url: None,
            version: None,
        });
    }
}

fn json_array(value: &Value) -> Option<&Vec<Value>> {
    value
        .as_array()
        .or_else(|| value.get("packs").and_then(Value::as_array))
        .or_else(|| value.get("skills").and_then(Value::as_array))
        .or_else(|| value.get("items").and_then(Value::as_array))
        .or_else(|| value.get("data").and_then(Value::as_array))
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| data.get("packs").or_else(|| data.get("skills")))
                .and_then(Value::as_array)
        })
}

fn string_field(entry: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = entry.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn bool_field(entry: &Value, keys: &[&str]) -> Option<bool> {
    for key in keys {
        match entry.get(*key) {
            Some(Value::Bool(flag)) => return Some(*flag),
            Some(Value::String(text)) => {
                let lower = text.trim().to_ascii_lowercase();
                if matches!(lower.as_str(), "1" | "true" | "yes" | "free") {
                    return Some(true);
                }
                if matches!(lower.as_str(), "0" | "false" | "no" | "paid") {
                    return Some(false);
                }
            }
            Some(Value::Number(num)) => {
                if let Some(n) = num.as_i64() {
                    return Some(n == 0);
                }
            }
            _ => {}
        }
    }
    None
}

fn number_field(entry: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(num) = entry.get(*key).and_then(Value::as_f64) {
            return Some(num);
        }
        if let Some(text) = entry.get(*key).and_then(Value::as_str) {
            if let Ok(num) = text.trim().parse::<f64>() {
                return Some(num);
            }
        }
    }
    None
}

fn detect_free(entry: &Value) -> bool {
    if bool_field(entry, &["free", "is_free", "isFree"]).unwrap_or(false) {
        return true;
    }
    if bool_field(entry, &["paid", "is_paid", "isPaid"]).unwrap_or(false) {
        return false;
    }
    if let Some(price) = number_field(
        entry,
        &[
            "price",
            "price_cents",
            "priceCents",
            "amount",
            "amount_cents",
            "amountCents",
        ],
    ) {
        // priceCents / price_cents are in 分; bare `price` may be yuan — both 0 ⇒ free.
        return price <= 0.0;
    }
    if let Some(pricing) = entry.get("pricing") {
        if bool_field(pricing, &["free", "is_free"]).unwrap_or(false) {
            return true;
        }
        if let Some(price) = number_field(
            pricing,
            &["price", "amount", "cents", "priceCents", "price_cents"],
        ) {
            return price <= 0.0;
        }
    }
    // Default catalog rows without price metadata are installable directly.
    true
}

fn price_label(entry: &Value, free: bool) -> Option<String> {
    if free {
        return Some("Free".to_string());
    }
    if let Some(label) = string_field(
        entry,
        &[
            "price_label",
            "priceLabel",
            "price_display",
            "display_price",
        ],
    ) {
        return Some(label);
    }
    if let Some(cents) = number_field(
        entry,
        &["price_cents", "priceCents", "amount_cents", "amountCents"],
    ) {
        let yuan = cents / 100.0;
        if yuan.fract() == 0.0 {
            return Some(format!("¥{yuan:.0}"));
        }
        return Some(format!("¥{yuan:.2}"));
    }
    if let Some(price) = number_field(entry, &["price", "amount"]) {
        return Some(format!("¥{price}"));
    }
    Some("Paid".to_string())
}

fn default_purchase_url(base_url: &str, kind: &str, id: &str) -> String {
    if kind == "skill" {
        format!("{base_url}/skills/{id}")
    } else {
        format!("{base_url}/packs/{id}")
    }
}

pub(crate) fn parse_catalog_entries(
    value: &Value,
    default_kind: &str,
    base_url: &str,
) -> Vec<TeamupsCatalogItem> {
    let Some(entries) = json_array(value) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries {
        let kind = string_field(entry, &["kind", "type", "item_type"])
            .unwrap_or_else(|| default_kind.to_string())
            .to_ascii_lowercase();
        let kind = if kind.contains("skill") {
            "skill".to_string()
        } else if kind.contains("pack")
            || kind.contains("bundle")
            || kind.contains("combo")
            || kind.contains("组合")
        {
            "pack".to_string()
        } else {
            default_kind.to_string()
        };
        let id = string_field(
            entry,
            &[
                "slug",
                "id",
                "pack_slug",
                "pack_id",
                "skill_id",
                "bundle_id",
            ],
        );
        let Some(id) = id else {
            continue;
        };
        let name = string_field(entry, &["name", "title", "display_name", "label"])
            .unwrap_or_else(|| id.clone());
        let description = string_field(
            entry,
            &["description", "tagline", "summary", "desc", "subtitle"],
        )
        .or_else(|| {
            entry
                .get("scorecard")
                .and_then(|card| string_field(card, &["summary"]))
        })
        .unwrap_or_default();
        let free = detect_free(entry);
        let owned =
            bool_field(entry, &["owned", "purchased", "unlocked", "licensed"]).unwrap_or(free);
        let skill_count = number_field(entry, &["skill_count", "skills_count", "count"])
            .map(|n| n as u32)
            .or_else(|| {
                entry
                    .get("skills")
                    .and_then(Value::as_array)
                    .map(|arr| arr.len() as u32)
            });
        let mut pack_slug = string_field(entry, &["pack_slug", "pack", "bundle_id", "parent_slug"]);
        if kind == "skill" && pack_slug.is_none() {
            pack_slug = Some(id.clone());
        }
        let purchase_url = string_field(
            entry,
            &[
                "purchase_url",
                "checkout_url",
                "buy_url",
                "store_url",
                "url",
            ],
        )
        .unwrap_or_else(|| default_purchase_url(base_url, &kind, &id));
        let version = string_field(entry, &["version", "latest_version"]);
        out.push(TeamupsCatalogItem {
            id,
            kind,
            name,
            description,
            free,
            price_label: price_label(entry, free),
            owned,
            installed: false,
            skill_count,
            pack_slug,
            purchase_url: Some(purchase_url),
            version,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_packs_and_paid_flags() {
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
                    "owned": false,
                    "purchase_url": "https://teamups.vip/checkout/pro"
                }
            ]
        });
        let items = parse_catalog_entries(&raw, "pack", "https://teamups.vip");
        assert_eq!(items.len(), 2);
        assert!(items[0].free);
        assert_eq!(items[0].skill_count, Some(2));
        assert!(!items[1].free);
        assert_eq!(
            items[1].purchase_url.as_deref(),
            Some("https://teamups.vip/checkout/pro")
        );
    }

    #[test]
    fn parses_teamups_vip_public_packs() {
        let raw = json!({
            "packs": [
                {
                    "slug": "ecommerce-cs",
                    "title": "电商客服话术",
                    "priceCents": 12900,
                    "scorecard": { "summary": "客服话术包" }
                },
                {
                    "slug": "weekly-report",
                    "title": "团队周报 / 日报",
                    "priceCents": 9900,
                    "scorecard": { "summary": "周报包" }
                }
            ]
        });
        let items = parse_catalog_entries(&raw, "pack", "https://teamups.vip");
        assert_eq!(items.len(), 2);
        assert!(!items[0].free);
        assert_eq!(items[0].name, "电商客服话术");
        assert_eq!(items[0].description, "客服话术包");
        assert_eq!(items[0].price_label.as_deref(), Some("¥129"));
        assert_eq!(
            items[0].purchase_url.as_deref(),
            Some("https://teamups.vip/packs/ecommerce-cs")
        );
        assert_eq!(items[1].price_label.as_deref(), Some("¥99"));
    }

    #[test]
    fn marks_owned_public_pack_and_appends_missing() {
        let mut items = parse_catalog_entries(
            &json!({
                "packs": [
                    { "slug": "ecommerce-cs", "title": "电商客服话术", "priceCents": 12900 },
                    { "slug": "weekly-report", "title": "团队周报 / 日报", "priceCents": 9900 }
                ]
            }),
            "pack",
            "https://teamups.vip",
        );
        merge_owned_into_catalog(
            &mut items,
            Some(&json!({
                "ok": true,
                "packs": [
                    { "slug": "internal-ops", "title": "内部运营包", "status": "published" }
                ]
            })),
            Some(&json!({ "ok": true, "packs": ["weekly-report", "internal-ops"] })),
            "https://teamups.vip",
        );
        let weekly = items.iter().find(|i| i.id == "weekly-report").unwrap();
        assert!(weekly.owned);
        assert!(!weekly.free);
        let shop = items.iter().find(|i| i.id == "ecommerce-cs").unwrap();
        assert!(!shop.owned);
        let extra = items.iter().find(|i| i.id == "internal-ops").unwrap();
        assert!(extra.owned);
        assert_eq!(extra.name, "内部运营包");
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn expands_title_polish_manifest_without_skills_array() {
        let manifest = manifest_from_doctor_json(
            json!({
                "ok": true,
                "slug": "title-polish",
                "title": "标题润色",
                "version": "0.1.0",
                "artifact": {
                    "url": "https://teamups.vip/api/v1/artifacts/skill/title-polish",
                    "sha256": "84a52f5adfe4370a828503300fb91fdc04aca1866c420e9ec7d570fe998b975d",
                    "version": "0.1.0"
                }
            }),
            "title-polish",
        );
        assert_eq!(manifest.skills.len(), 1);
        assert_eq!(manifest.skills[0].id, "title-polish");
        assert_eq!(
            manifest.skills[0].sha256.as_deref(),
            Some("84a52f5adfe4370a828503300fb91fdc04aca1866c420e9ec7d570fe998b975d")
        );
    }

    #[test]
    fn parses_published_skill_in_packs_list() {
        let items = parse_catalog_entries(
            &json!({
                "packs": [
                    { "slug": "weekly-report", "title": "周报", "priceCents": 9900 },
                    {
                        "slug": "title-polish",
                        "title": "标题润色",
                        "kind": "skill",
                        "tagline": "给草稿标题换几种更抓人的说法",
                        "priceCents": 1900
                    }
                ]
            }),
            "pack",
            "https://teamups.vip",
        );
        assert_eq!(items.len(), 2);
        let skill = items.iter().find(|i| i.id == "title-polish").unwrap();
        assert_eq!(skill.kind, "skill");
        assert_eq!(skill.name, "标题润色");
        assert!(!skill.free);
        assert_eq!(skill.pack_slug.as_deref(), Some("title-polish"));
    }

    #[test]
    fn owned_from_me_strings_only() {
        let mut items = parse_catalog_entries(
            &json!({
                "packs": [{ "slug": "weekly-report", "title": "周报", "priceCents": 9900 }]
            }),
            "pack",
            "https://teamups.vip",
        );
        merge_owned_into_catalog(
            &mut items,
            None,
            Some(&json!({ "packs": ["weekly-report"] })),
            "https://teamups.vip",
        );
        assert!(items[0].owned);
    }
}
