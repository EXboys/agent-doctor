//! Durable Agent Doctor settings: SQLite (non-secrets) + OS credential store (secrets).
//!
//! Secrets backends: macOS Keychain, Windows Credential Manager (DPAPI file fallback),
//! Linux Secret Service — see [`secrets::platform_secret_store_name`].

pub mod db;
pub mod migrate;
pub mod secrets;

pub use db::{
    agent_doctor_config_dir, default_skills_cache_dir, default_skills_lock_path,
    default_teamups_base_url, legacy_evotown_skills_dir, open_settings_store,
    open_settings_store_at, require_team_credentials, settings_db_path, team_configured,
    PersonalProviderRecord, SettingsStore, SkillsSourceKind, SkillsSourceSettings, TeamSettings,
    SCHEMA_VERSION,
};
pub use secrets::{
    default_secret_backend, personal_api_key_account, platform_secret_store_name,
    CompositeSecretBackend, KeyringBackend, MemorySecretBackend, SecretBackend, KEYRING_SERVICE,
    SECRET_CUSTOM_SKILLS_TOKEN, SECRET_OVERLAY_API_KEY, SECRET_TEAMUPS_LICENSE,
    SECRET_TEAM_API_KEY, SECRET_TEAM_ENGINE_INGEST,
};
