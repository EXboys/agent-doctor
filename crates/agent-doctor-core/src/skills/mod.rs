//! Pluggable skills download sources (TeamUps default, Evotown / custom / local).

mod custom;
mod evotown_source;
pub(crate) mod local;
mod mall;
mod resolve;
mod sync_router;
mod teamups;
mod teamups_auth;

pub use mall::{install_teamups_mall_item, list_teamups_mall_catalog};
pub use resolve::{
    load_local_skills_layout, resolve_skills_source, save_skills_source_override,
    LocalSkillsLayout, ResolvedSkillsSource,
};
pub use sync_router::{execute_skills_sync, SkillsSyncOptions};
pub use teamups::{
    TeamupsCatalogItem, TeamupsClient, TeamupsMallCatalog, TeamupsPackManifest, TeamupsSkillEntry,
};
pub use teamups_auth::{
    poll_teamups_login, sign_out_teamups, start_teamups_login, teamups_account_status,
    TeamupsAccountStatus, TeamupsLoginPoll, TeamupsLoginPollStatus, TeamupsLoginStart,
};

use anyhow::Result;
use serde_json::Value;

/// Common interface for remote (or local-noop) skills sync backends.
pub trait SkillsSyncSource: Send + Sync {
    fn kind(&self) -> crate::store::SkillsSourceKind;
    fn base_url(&self) -> &str;
    fn fetch_manifest(&self, pack_or_bundle_id: &str, runtime_target: &str) -> Result<Value>;
    fn download_skill_zip(&self, pack_or_bundle_id: &str, skill_id: &str) -> Result<Vec<u8>>;
}
