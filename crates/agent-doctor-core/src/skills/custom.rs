//! Custom HTTP skills source using the same manifest/zip contract as TeamUps.

use anyhow::{bail, Context, Result};
use serde_json::Value;

use super::SkillsSyncSource;
use crate::store::SkillsSourceKind;

pub struct CustomSkillsSource {
    base_url: String,
    token: Option<String>,
    http: reqwest::blocking::Client,
}

impl CustomSkillsSource {
    pub fn new(base_url: &str, token: Option<String>) -> Result<Self> {
        if base_url.trim().is_empty() {
            bail!("custom skills source requires a base URL");
        }
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("failed to build custom skills HTTP client")?;
        Ok(Self {
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            token,
            http,
        })
    }

    fn auth(&self, req: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        match &self.token {
            Some(token) if !token.trim().is_empty() => {
                req.header("Authorization", format!("Bearer {}", token.trim()))
            }
            _ => req,
        }
    }
}

impl SkillsSyncSource for CustomSkillsSource {
    fn kind(&self) -> SkillsSourceKind {
        SkillsSourceKind::Custom
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn fetch_manifest(&self, pack_or_bundle_id: &str, _runtime_target: &str) -> Result<Value> {
        let url = format!("{}/api/v1/packs/{pack_or_bundle_id}/manifest", self.base_url);
        let resp = self
            .auth(self.http.get(&url))
            .send()
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        if !status.is_success() {
            bail!("custom skills manifest failed ({status}): {body}");
        }
        let parsed: Value = serde_json::from_str(&body).context("invalid custom manifest JSON")?;
        if parsed.get("manifest").is_some() {
            Ok(parsed)
        } else {
            Ok(serde_json::json!({ "manifest": parsed }))
        }
    }

    fn download_skill_zip(&self, pack_or_bundle_id: &str, skill_id: &str) -> Result<Vec<u8>> {
        let url = format!(
            "{}/api/v1/packs/{pack_or_bundle_id}/skills/{skill_id}",
            self.base_url
        );
        let resp = self
            .auth(self.http.get(&url))
            .send()
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            bail!("custom skills download failed ({status}): {body}");
        }
        Ok(resp.bytes()?.to_vec())
    }
}
