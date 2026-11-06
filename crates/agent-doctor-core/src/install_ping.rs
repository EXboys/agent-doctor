//! Anonymous install ping so the site can count machines running the client.
//!
//! One random UUID per installation is generated on first launch and stored in
//! the local settings DB. Each launch sends a single tiny POST with that ID,
//! the app version, and the platform. No user data, no hardware fingerprint,
//! no account link — the server stores one row per ID with first/last-seen
//! times, and the ID itself carries nothing personal.
//!
//! Server contract (agentdoctor.cn): `POST /api/v1/doctor/install/ping`,
//! JSON body, any 2xx accepted, failures ignored by the client.
//! Override the URL with `AGENT_DOCTOR_INSTALL_PING_URL` for local testing.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use uuid::Uuid;

use crate::edition::product_edition;
use crate::store::{open_settings_store, SettingsStore};

pub const INSTALL_PING_URL: &str = "https://agentdoctor.cn/api/v1/doctor/install/ping";

const INSTALL_ID_KEY: &str = "install_id";
const INSTALL_PING_OPT_OUT_KEY: &str = "install_ping_opt_out";

/// Install counting is on unless the user opted out in the app.
pub fn install_ping_enabled(store: &SettingsStore) -> Result<bool> {
    Ok(store.meta_get(INSTALL_PING_OPT_OUT_KEY)?.as_deref() != Some("1"))
}

pub fn set_install_ping_enabled(store: &SettingsStore, enabled: bool) -> Result<()> {
    store.meta_set(INSTALL_PING_OPT_OUT_KEY, if enabled { "0" } else { "1" })?;
    Ok(())
}

/// `Ok((id, true))` when the ID was just created for this installation —
/// that launch is the install event.
fn ensure_install_id(store: &SettingsStore) -> Result<(String, bool)> {
    if let Some(id) = store.meta_get(INSTALL_ID_KEY)? {
        let id = id.trim().to_string();
        if !id.is_empty() {
            return Ok((id, false));
        }
    }
    let id = Uuid::new_v4().to_string();
    store.meta_set(INSTALL_ID_KEY, &id)?;
    Ok((id, true))
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct InstallPing {
    pub install_id: String,
    pub version: &'static str,
    /// "macos" | "windows" | "linux" (std::env::consts::OS).
    pub os: &'static str,
    /// std::env::consts::ARCH, e.g. "aarch64" | "x86_64".
    pub arch: &'static str,
    /// "personal" | "team".
    pub edition: &'static str,
    /// Which shipped surface sent it ("desktop", later maybe "cli").
    pub client: &'static str,
    /// True only on the launch that created the ID.
    pub first_run: bool,
}

/// Ping endpoint, overridable for local testing.
pub fn install_ping_url() -> String {
    std::env::var("AGENT_DOCTOR_INSTALL_PING_URL")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| INSTALL_PING_URL.to_string())
}

/// Build and send the ping for this installation. Best effort: every failure
/// (store, opt-out check, network) is reported to the caller but startup code
/// is expected to drop it.
pub fn send_install_ping() -> Result<Option<InstallPing>> {
    let store = open_settings_store()?;
    if !install_ping_enabled(&store)? {
        return Ok(None);
    }
    let (install_id, first_run) = ensure_install_id(&store)?;
    let ping = InstallPing {
        install_id,
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        edition: product_edition().as_str(),
        client: "desktop",
        first_run,
    };
    post_install_ping(&install_ping_url(), &ping)?;
    Ok(Some(ping))
}

fn post_install_ping(url: &str, ping: &InstallPing) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build install ping HTTP client")?;
    // The body matters to nobody on a failed send — drop the result.
    let _ = client.post(url).json(ping).send()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::secrets::MemorySecretBackend;
    use std::path::Path;
    use std::sync::Arc;

    fn open_store(path: &Path) -> SettingsStore {
        SettingsStore::open_at(path, Arc::new(MemorySecretBackend::new())).unwrap()
    }

    #[test]
    fn install_id_is_stable_and_flags_first_run_once() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("settings.db");

        let store = open_store(&db);
        let (first_id, first_run) = ensure_install_id(&store).unwrap();
        assert!(first_run);
        drop(store);

        let store = open_store(&db);
        let (second_id, second_run) = ensure_install_id(&store).unwrap();
        assert!(!second_run);
        assert_eq!(first_id, second_id);
        assert!(Uuid::parse_str(&first_id).is_ok());
    }

    #[test]
    fn ping_is_enabled_by_default_and_opt_out_persists() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("settings.db");
        let store = open_store(&db);
        assert!(install_ping_enabled(&store).unwrap());

        set_install_ping_enabled(&store, false).unwrap();
        assert!(!install_ping_enabled(&store).unwrap());
        drop(store);

        let store = open_store(&db);
        assert!(!install_ping_enabled(&store).unwrap());
        set_install_ping_enabled(&store, true).unwrap();
        assert!(install_ping_enabled(&store).unwrap());
    }

    #[test]
    fn ping_payload_carries_only_anonymous_fields() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("settings.db");
        let store = open_store(&db);
        let (install_id, first_run) = ensure_install_id(&store).unwrap();
        let ping = InstallPing {
            install_id,
            version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            edition: product_edition().as_str(),
            client: "desktop",
            first_run,
        };
        let json = serde_json::to_value(&ping).unwrap();
        let keys = json.as_object().unwrap();
        let mut names: Vec<_> = keys.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "arch",
                "client",
                "edition",
                "first_run",
                "install_id",
                "os",
                "version"
            ]
        );
    }
}
