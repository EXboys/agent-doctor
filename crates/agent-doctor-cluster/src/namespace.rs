//! Account-scoped or pairing-code-scoped cluster namespace & local settings.
//!
//! Devices signed into the same TeamUps account derive the exact same namespace
//! (`ad-acct-<hash12>`) from the account identifier (never the per-device license).
//! Unauthenticated devices generate and persist a 4-digit pairing code so they
//! never fall back to the global `exo` namespace on public Wi-Fi.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use agent_doctor_core::skills::cached_teamups_account_identity;
use agent_doctor_core::store::open_settings_store;

const META_CUSTOM_CODE: &str = "cluster.pairing_code_custom";
const META_AUTO_CODE: &str = "cluster.pairing_code_auto";
const META_REMOTE_ORIGIN: &str = "cluster.remote_origin";
const META_PREFERRED_MODEL: &str = "cluster.preferred_model";
const META_BACKEND: &str = "cluster.backend";
const META_LLAMA_PEERS: &str = "cluster.llama_peers";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamespaceSource {
    /// Explicit pairing code entered by the user.
    CustomCode,
    /// Derived automatically from the signed-in TeamUps account.
    Account,
    /// Auto-generated 4-digit pairing code for unauthenticated local pairing.
    GeneratedCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedNamespace {
    /// Wire namespace passed to `EXO_LIBP2P_NAMESPACE` / `EXO_ZENOH_NAMESPACE`.
    pub namespace: String,
    pub source: NamespaceSource,
    /// Human-readable pairing code when using code mode (`Some("4821")`).
    pub pairing_code: Option<String>,
}

pub fn resolve_cluster_namespace() -> ResolvedNamespace {
    if let Some(custom) = load_meta_non_empty(META_CUSTOM_CODE) {
        let clean = sanitize_code(&custom);
        if !clean.is_empty() {
            return ResolvedNamespace {
                namespace: format!("ad-code-{clean}"),
                source: NamespaceSource::CustomCode,
                pairing_code: Some(clean),
            };
        }
    }

    if let Some((base_url, account_key)) = cached_teamups_account_identity() {
        let hash = account_namespace_hash(&base_url, &account_key);
        return ResolvedNamespace {
            namespace: format!("ad-acct-{hash}"),
            source: NamespaceSource::Account,
            pairing_code: None,
        };
    }

    let auto_code = load_or_create_auto_code();
    ResolvedNamespace {
        namespace: format!("ad-code-{auto_code}"),
        source: NamespaceSource::GeneratedCode,
        pairing_code: Some(auto_code),
    }
}

pub fn set_custom_pairing_code(code: Option<&str>) -> Result<ResolvedNamespace> {
    let store = open_settings_store()?;
    let cleaned = code.map(sanitize_code).unwrap_or_default();
    store.set_meta(META_CUSTOM_CODE, &cleaned)?;
    Ok(resolve_cluster_namespace())
}

pub fn saved_remote_cluster_origin() -> Option<String> {
    load_meta_non_empty(META_REMOTE_ORIGIN)
}

pub fn set_saved_remote_cluster_origin(origin: Option<&str>) -> Result<()> {
    let store = open_settings_store()?;
    let val = origin.map(str::trim).unwrap_or("");
    store.set_meta(META_REMOTE_ORIGIN, val)
}

pub fn saved_preferred_cluster_model() -> Option<String> {
    load_meta_non_empty(META_PREFERRED_MODEL)
}

pub fn set_saved_preferred_cluster_model(model_id: &str) -> Result<()> {
    let store = open_settings_store()?;
    store.set_meta(META_PREFERRED_MODEL, model_id.trim())
}

pub fn saved_cluster_backend() -> Option<crate::ClusterBackendKind> {
    match load_meta_non_empty(META_BACKEND)?.as_str() {
        "exo" => Some(crate::ClusterBackendKind::Exo),
        "llama_cpp" => Some(crate::ClusterBackendKind::LlamaCpp),
        _ => None,
    }
}

pub fn set_saved_cluster_backend(kind: crate::ClusterBackendKind) -> Result<()> {
    let wire = match kind {
        crate::ClusterBackendKind::Exo => "exo",
        crate::ClusterBackendKind::LlamaCpp => "llama_cpp",
        crate::ClusterBackendKind::Ollama => "ollama",
    };
    let store = open_settings_store()?;
    store.set_meta(META_BACKEND, wire)
}

pub fn saved_llama_peer_hosts() -> Option<String> {
    load_meta_non_empty(META_LLAMA_PEERS)
}

pub fn set_saved_llama_peer_hosts(raw: &str) -> Result<()> {
    let store = open_settings_store()?;
    store.set_meta(META_LLAMA_PEERS, raw.trim())
}

pub(crate) fn account_namespace_hash(base_url: &str, account_key: &str) -> String {
    let normalized = format!(
        "{}:{}",
        base_url.trim().trim_end_matches('/').to_ascii_lowercase(),
        account_key.trim().to_ascii_lowercase()
    );
    let digest = Sha256::digest(normalized.as_bytes());
    let mut hex = String::with_capacity(12);
    for byte in &digest[..6] {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

pub(crate) fn sanitize_code(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .map(|c| c.to_ascii_lowercase())
        .take(24)
        .collect()
}

fn load_meta_non_empty(key: &str) -> Option<String> {
    let store = open_settings_store().ok()?;
    store
        .get_meta(key)
        .ok()?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn load_or_create_auto_code() -> String {
    if let Some(existing) = load_meta_non_empty(META_AUTO_CODE) {
        let clean = sanitize_code(&existing);
        if !clean.is_empty() {
            return clean;
        }
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32))
        .unwrap_or(4821);
    let pin = 1000 + (nanos % 9000);
    let code = pin.to_string();
    if let Ok(store) = open_settings_store() {
        let _ = store.set_meta(META_AUTO_CODE, &code);
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_account_produces_identical_namespace_across_devices() {
        let mac_a = account_namespace_hash("https://teamups.vip/", "User_42");
        let mac_b = account_namespace_hash("https://teamups.vip", "user_42");
        assert_eq!(mac_a, mac_b);
        assert_eq!(mac_a.len(), 12);
    }

    #[test]
    fn sanitize_pairing_code_strips_spaces_and_symbols() {
        assert_eq!(sanitize_code(" 8842 "), "8842");
        assert_eq!(sanitize_code("Room A!@#9"), "rooma9");
    }
}
