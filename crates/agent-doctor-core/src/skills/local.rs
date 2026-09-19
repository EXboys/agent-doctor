//! Local-only skills source (no remote sync).

#![allow(dead_code)]

use anyhow::{bail, Result};
use serde_json::Value;

use super::SkillsSyncSource;
use crate::store::SkillsSourceKind;

pub struct LocalSkillsSource;

impl SkillsSyncSource for LocalSkillsSource {
    fn kind(&self) -> SkillsSourceKind {
        SkillsSourceKind::Local
    }

    fn base_url(&self) -> &str {
        ""
    }

    fn fetch_manifest(&self, _pack_or_bundle_id: &str, _runtime_target: &str) -> Result<Value> {
        bail!("local skills source does not sync remotely — use mount on existing ~/.agent-doctor/skills")
    }

    fn download_skill_zip(&self, _pack_or_bundle_id: &str, _skill_id: &str) -> Result<Vec<u8>> {
        bail!("local skills source does not download packages")
    }
}
