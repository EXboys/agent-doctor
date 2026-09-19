//! Evotown SkillHub adapter wrapping existing EvotownClient paths.
//! Available for callers that need the SkillsSyncSource trait; the sync router
//! currently delegates Evotown installs to `execute_sync` for package URL/signature parity.

#![allow(dead_code)]

use anyhow::Result;
use serde_json::Value;

use super::SkillsSyncSource;
use crate::evotown::EvotownClient;
use crate::store::SkillsSourceKind;

pub struct EvotownSkillsSource {
    client: EvotownClient,
    base_url: String,
}

impl EvotownSkillsSource {
    pub fn new(base_url: &str, api_key: &str) -> Result<Self> {
        let client = EvotownClient::new(base_url, api_key)?;
        Ok(Self {
            client,
            base_url: base_url.trim().trim_end_matches('/').to_string(),
        })
    }
}

impl SkillsSyncSource for EvotownSkillsSource {
    fn kind(&self) -> SkillsSourceKind {
        SkillsSourceKind::Evotown
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn fetch_manifest(&self, pack_or_bundle_id: &str, runtime_target: &str) -> Result<Value> {
        let path = format!(
            "/api/v1/market/bundles/{pack_or_bundle_id}/manifest?runtime_target={runtime_target}"
        );
        self.client.get_json(&path)
    }

    fn download_skill_zip(&self, _pack_or_bundle_id: &str, skill_id: &str) -> Result<Vec<u8>> {
        let path = format!("/api/v1/market/skills/{skill_id}/download");
        self.client.get_bytes(&path)
    }
}
