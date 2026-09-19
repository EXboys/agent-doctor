//! SQLite settings store for Agent Doctor (non-secret state).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::secrets::{
    personal_api_key_account, SecretBackend, SECRET_CUSTOM_SKILLS_TOKEN, SECRET_OVERLAY_API_KEY,
    SECRET_TEAMUPS_LICENSE, SECRET_TEAM_API_KEY, SECRET_TEAM_ENGINE_INGEST,
};

pub const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillsSourceKind {
    #[default]
    Teamups,
    Evotown,
    Custom,
    Local,
}

impl SkillsSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Teamups => "teamups",
            Self::Evotown => "evotown",
            Self::Custom => "custom",
            Self::Local => "local",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "teamups" | "team-ups" | "default" => Some(Self::Teamups),
            "evotown" => Some(Self::Evotown),
            "custom" => Some(Self::Custom),
            "local" => Some(Self::Local),
            "" => None,
            _ => None,
        }
    }
}

/// Built-in TeamUps base URL (can be overridden by channel / env).
pub fn default_teamups_base_url() -> String {
    std::env::var("AGENT_DOCTOR_TEAMUPS_URL")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "https://www.teamups.ai".to_string())
}

pub fn default_skills_cache_dir() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".agent-doctor").join("skills"))
        .unwrap_or_else(|| PathBuf::from(".agent-doctor/skills"))
}

pub fn legacy_evotown_skills_dir() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".evotown").join("skills"))
        .unwrap_or_else(|| PathBuf::from(".evotown/skills"))
}

pub fn agent_doctor_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|base| base.join("agent-doctor"))
}

pub fn settings_db_path() -> Option<PathBuf> {
    agent_doctor_config_dir().map(|base| base.join("settings.db"))
}

pub fn default_skills_lock_path() -> Option<PathBuf> {
    agent_doctor_config_dir().map(|base| base.join("skills-lock.json"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamSettings {
    pub base_url: Option<String>,
    pub runtime: Option<String>,
    pub bundle_id: Option<String>,
    pub skills_dir: Option<String>,
    pub engine_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsSourceSettings {
    /// `None` means "use product default (TeamUps)".
    pub source: Option<SkillsSourceKind>,
    pub base_url: Option<String>,
    pub pack_slug: Option<String>,
}

impl Default for SkillsSourceSettings {
    fn default() -> Self {
        Self {
            source: None,
            base_url: None,
            pack_slug: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalProviderRecord {
    pub id: String,
    pub name: String,
    pub url: String,
    pub model: String,
    pub protocol: String,
    pub active: bool,
}

pub struct SettingsStore {
    conn: Connection,
    secrets: Arc<dyn SecretBackend>,
    path: PathBuf,
}

impl SettingsStore {
    pub fn open_default() -> Result<Self> {
        let path = settings_db_path().context("could not resolve agent-doctor config dir")?;
        Self::open_at(&path, super::secrets::default_secret_backend())
    }

    pub fn open_at(path: &Path, secrets: Arc<dyn SecretBackend>) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open settings db at {}", path.display()))?;
        let store = Self {
            conn,
            secrets,
            path: path.to_path_buf(),
        };
        store.migrate_schema()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn secrets(&self) -> &Arc<dyn SecretBackend> {
        &self.secrets
    }

    fn migrate_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS team_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                base_url TEXT,
                runtime TEXT,
                bundle_id TEXT,
                skills_dir TEXT,
                engine_id TEXT
            );
            CREATE TABLE IF NOT EXISTS skills_source_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                source TEXT,
                base_url TEXT,
                pack_slug TEXT
            );
            CREATE TABLE IF NOT EXISTS mode_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                active_mode TEXT
            );
            CREATE TABLE IF NOT EXISTS personal_providers (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                url TEXT NOT NULL,
                model TEXT NOT NULL,
                protocol TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 0
            );
            "#,
        )?;
        let version: i64 = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| {
                    let raw: String = row.get(0)?;
                    Ok(raw.parse::<i64>().unwrap_or(0))
                },
            )
            .optional()?
            .unwrap_or(0);
        if version < SCHEMA_VERSION {
            self.conn.execute(
                "INSERT INTO meta(key, value) VALUES('schema_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![SCHEMA_VERSION.to_string()],
            )?;
        }
        Ok(())
    }

    pub fn migration_done(&self) -> Result<bool> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'legacy_migration_done'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.as_deref() == Some("1"))
    }

    pub fn set_migration_done(&self) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES('legacy_migration_done', '1')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )?;
        Ok(())
    }

    pub fn get_team_settings(&self) -> Result<TeamSettings> {
        let row = self
            .conn
            .query_row(
                "SELECT base_url, runtime, bundle_id, skills_dir, engine_id
                 FROM team_settings WHERE id = 1",
                [],
                |row| {
                    Ok(TeamSettings {
                        base_url: row.get(0)?,
                        runtime: row.get(1)?,
                        bundle_id: row.get(2)?,
                        skills_dir: row.get(3)?,
                        engine_id: row.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(row.unwrap_or_default())
    }

    pub fn set_team_settings(&self, settings: &TeamSettings) -> Result<()> {
        self.conn.execute(
            "INSERT INTO team_settings(id, base_url, runtime, bundle_id, skills_dir, engine_id)
             VALUES(1, ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
               base_url = excluded.base_url,
               runtime = excluded.runtime,
               bundle_id = excluded.bundle_id,
               skills_dir = excluded.skills_dir,
               engine_id = excluded.engine_id",
            params![
                settings.base_url,
                settings.runtime,
                settings.bundle_id,
                settings.skills_dir,
                settings.engine_id
            ],
        )?;
        Ok(())
    }

    pub fn get_skills_source_settings(&self) -> Result<SkillsSourceSettings> {
        let row = self
            .conn
            .query_row(
                "SELECT source, base_url, pack_slug FROM skills_source_settings WHERE id = 1",
                [],
                |row| {
                    let source_raw: Option<String> = row.get(0)?;
                    Ok(SkillsSourceSettings {
                        source: source_raw.as_deref().and_then(SkillsSourceKind::parse),
                        base_url: row.get(1)?,
                        pack_slug: row.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(row.unwrap_or_default())
    }

    pub fn set_skills_source_settings(&self, settings: &SkillsSourceSettings) -> Result<()> {
        self.conn.execute(
            "INSERT INTO skills_source_settings(id, source, base_url, pack_slug)
             VALUES(1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               source = excluded.source,
               base_url = excluded.base_url,
               pack_slug = excluded.pack_slug",
            params![
                settings.source.map(|s| s.as_str().to_string()),
                settings.base_url,
                settings.pack_slug
            ],
        )?;
        Ok(())
    }

    /// Clear explicit skills source so product default (TeamUps) applies.
    pub fn clear_skills_source_override(&self) -> Result<()> {
        self.set_skills_source_settings(&SkillsSourceSettings::default())
    }

    pub fn get_active_mode(&self) -> Result<Option<String>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT active_mode FROM mode_settings WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.filter(|v| !v.trim().is_empty()))
    }

    pub fn set_active_mode(&self, mode: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO mode_settings(id, active_mode) VALUES(1, ?1)
             ON CONFLICT(id) DO UPDATE SET active_mode = excluded.active_mode",
            params![mode],
        )?;
        Ok(())
    }

    pub fn list_personal_providers(&self) -> Result<Vec<PersonalProviderRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, url, model, protocol, active FROM personal_providers ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PersonalProviderRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                url: row.get(2)?,
                model: row.get(3)?,
                protocol: row.get(4)?,
                active: row.get::<_, i64>(5)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn upsert_personal_provider(&self, record: &PersonalProviderRecord) -> Result<()> {
        if record.active {
            self.conn
                .execute("UPDATE personal_providers SET active = 0", [])?;
        }
        self.conn.execute(
            "INSERT INTO personal_providers(id, name, url, model, protocol, active)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               url = excluded.url,
               model = excluded.model,
               protocol = excluded.protocol,
               active = excluded.active",
            params![
                record.id,
                record.name,
                record.url,
                record.model,
                record.protocol,
                if record.active { 1 } else { 0 }
            ],
        )?;
        Ok(())
    }

    pub fn delete_personal_provider(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM personal_providers WHERE id = ?1", params![id])?;
        let _ = self.secrets.delete(&personal_api_key_account(id));
        Ok(())
    }

    pub fn set_team_api_key(&self, key: &str) -> Result<()> {
        self.secrets.set(SECRET_TEAM_API_KEY, key)
    }

    pub fn get_team_api_key(&self) -> Result<Option<String>> {
        self.secrets.get(SECRET_TEAM_API_KEY)
    }

    pub fn set_team_engine_ingest_token(&self, token: &str) -> Result<()> {
        self.secrets.set(SECRET_TEAM_ENGINE_INGEST, token)
    }

    pub fn get_team_engine_ingest_token(&self) -> Result<Option<String>> {
        self.secrets.get(SECRET_TEAM_ENGINE_INGEST)
    }

    pub fn set_teamups_license(&self, license: &str) -> Result<()> {
        self.secrets.set(SECRET_TEAMUPS_LICENSE, license)
    }

    pub fn get_teamups_license(&self) -> Result<Option<String>> {
        self.secrets.get(SECRET_TEAMUPS_LICENSE)
    }

    pub fn set_custom_skills_token(&self, token: &str) -> Result<()> {
        self.secrets.set(SECRET_CUSTOM_SKILLS_TOKEN, token)
    }

    pub fn get_custom_skills_token(&self) -> Result<Option<String>> {
        self.secrets.get(SECRET_CUSTOM_SKILLS_TOKEN)
    }

    pub fn set_overlay_api_key(&self, key: &str) -> Result<()> {
        self.secrets.set(SECRET_OVERLAY_API_KEY, key)
    }

    pub fn get_overlay_api_key(&self) -> Result<Option<String>> {
        self.secrets.get(SECRET_OVERLAY_API_KEY)
    }

    pub fn set_personal_api_key(&self, provider_id: &str, key: &str) -> Result<()> {
        self.secrets
            .set(&personal_api_key_account(provider_id), key)
    }

    pub fn get_personal_api_key(&self, provider_id: &str) -> Result<Option<String>> {
        self.secrets.get(&personal_api_key_account(provider_id))
    }

    pub fn resolved_skills_dir(&self) -> Result<PathBuf> {
        let team = self.get_team_settings()?;
        if let Some(dir) = team
            .skills_dir
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Ok(expand_path(dir));
        }
        let modern = default_skills_cache_dir();
        if modern.is_dir() {
            return Ok(modern);
        }
        let legacy = legacy_evotown_skills_dir();
        if legacy.is_dir() {
            return Ok(legacy);
        }
        Ok(modern)
    }
}

fn expand_path(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        dirs::home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(value))
    } else {
        PathBuf::from(value)
    }
}

/// Open store and run one-shot legacy migration if needed.
pub fn open_settings_store() -> Result<SettingsStore> {
    let store = SettingsStore::open_default()?;
    if !store.migration_done()? {
        super::migrate::migrate_legacy_into(&store)?;
        store.set_migration_done()?;
    }
    Ok(store)
}

pub fn open_settings_store_at(
    path: &Path,
    secrets: Arc<dyn SecretBackend>,
    run_legacy_migrate: bool,
) -> Result<SettingsStore> {
    let store = SettingsStore::open_at(path, secrets)?;
    if run_legacy_migrate && !store.migration_done()? {
        super::migrate::migrate_legacy_into(&store)?;
        store.set_migration_done()?;
    }
    Ok(store)
}

pub fn team_configured(store: &SettingsStore) -> Result<bool> {
    let settings = store.get_team_settings()?;
    let key = store.get_team_api_key()?;
    Ok(settings
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some()
        && key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .is_some())
}

pub fn require_team_credentials(store: &SettingsStore) -> Result<(String, String)> {
    let settings = store.get_team_settings()?;
    let Some(base_url) = settings
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.trim_end_matches('/').to_string())
    else {
        bail!(
            "Team / Evotown is not configured — open Agent Doctor → Provider to connect your team, \
             or run `agent-doctor setup --url <evotown-url> --key evk_...`"
        );
    };
    let Some(api_key) = store
        .get_team_api_key()?
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    else {
        bail!(
            "Team / Evotown API key missing from {} — open Agent Doctor → Provider to reconnect",
            crate::store::platform_secret_store_name()
        );
    };
    Ok((base_url, api_key))
}
