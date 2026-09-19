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

    fn auth_header(&self, req: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        match &self.license {
            Some(token) if !token.trim().is_empty() => {
                req.header("Authorization", format!("Bearer {}", token.trim()))
            }
            _ => req,
        }
    }

    pub fn list_packs(&self) -> Result<Value> {
        let url = format!("{}/api/v1/packs", self.base_url);
        let req = self.auth_header(self.http.get(&url));
        let resp = req.send().with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        if !status.is_success() {
            bail!("TeamUps packs list failed ({status}): {body}");
        }
        serde_json::from_str(&body).context("invalid TeamUps packs JSON")
    }

    pub fn fetch_pack_manifest(&self, slug: &str) -> Result<TeamupsPackManifest> {
        let url = format!("{}/api/v1/packs/{slug}/manifest", self.base_url);
        let req = self.auth_header(self.http.get(&url));
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
        serde_json::from_str(&body).context("invalid TeamUps manifest JSON")
    }

    pub fn download_skill(&self, slug: &str, skill_id: &str) -> Result<Vec<u8>> {
        let url = format!(
            "{}/api/v1/packs/{}/skills/{}",
            self.base_url, slug, skill_id
        );
        let req = self.auth_header(self.http.get(&url));
        let resp = req.send().with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            bail!(
                "TeamUps skill `{skill_id}` requires a license — configure TeamUps license in Provider"
            );
        }
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            bail!("TeamUps skill download failed ({status}): {body}");
        }
        let bytes = resp.bytes().context("failed to read skill zip bytes")?;
        Ok(bytes.to_vec())
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
