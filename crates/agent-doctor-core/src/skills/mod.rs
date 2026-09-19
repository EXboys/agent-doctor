//! Pluggable skills download sources (TeamUps default, Evotown / custom / local).

mod custom;
mod evotown_source;
pub(crate) mod local;
mod resolve;
mod sync_router;
mod teamups;

pub use resolve::{
    load_local_skills_layout, resolve_skills_source, save_skills_source_override,
    LocalSkillsLayout, ResolvedSkillsSource,
};
pub use sync_router::{execute_skills_sync, SkillsSyncOptions};
pub use teamups::{TeamupsClient, TeamupsPackManifest, TeamupsSkillEntry};

use anyhow::Result;
use serde_json::Value;

/// Common interface for remote (or local-noop) skills sync backends.
pub trait SkillsSyncSource: Send + Sync {
    fn kind(&self) -> crate::store::SkillsSourceKind;
    fn base_url(&self) -> &str;
    fn fetch_manifest(&self, pack_or_bundle_id: &str, runtime_target: &str) -> Result<Value>;
    fn download_skill_zip(&self, pack_or_bundle_id: &str, skill_id: &str) -> Result<Vec<u8>>;
}
