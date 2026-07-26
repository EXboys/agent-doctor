//! Personal (non-Evotown) OpenAI-compatible provider setup.
//!
//! Supports multiple named providers with one-click activate/switch.
//! Active provider is written into runtime configs + `profile.env`.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::profile::{
    agent_profile_path, ensure_company_baseline_snapshot_from_active, COMPANY_API_KEY_ENV,
    GATEWAY_URL_ENV, PROVIDER_KIND_ENV, PROVIDER_KIND_PERSONAL,
};
use crate::repair::mask_secret_value;
use crate::runtime::all_adapters;
use crate::setup::merge::{self, clear_codex_placeholder_auth};
use crate::setup::{normalize_gateway_url, RuntimeSetupResult};

pub const MODEL_ENV: &str = "AGENT_DOCTOR_MODEL";
pub const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";
pub const ACTIVE_PROVIDER_ID_ENV: &str = "AGENT_DOCTOR_PROVIDER_ID";
pub const PROVIDER_PROTOCOL_ENV: &str = "AGENT_DOCTOR_PROVIDER_PROTOCOL";
pub const PROTOCOL_OPENAI: &str = "openai";
pub const PROTOCOL_ANTHROPIC: &str = "anthropic";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalProviderOptions {
    pub url: String,
    pub api_key: String,
    pub model: String,
    /// `openai` (Codex/Hermes) or `anthropic` (Claude Code).
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

fn default_protocol() -> String {
    PROTOCOL_OPENAI.to_string()
}

pub fn normalize_protocol(protocol: &str) -> String {
    match protocol.trim().to_ascii_lowercase().as_str() {
        PROTOCOL_ANTHROPIC | "claude" | "anthropic-messages" => PROTOCOL_ANTHROPIC.to_string(),
        _ => PROTOCOL_OPENAI.to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalProviderStatus {
    pub configured: bool,
    pub gateway_url: Option<String>,
    pub model: Option<String>,
    pub api_key_hint: Option<String>,
    pub profile_env_path: Option<String>,
    pub active_id: Option<String>,
    pub active_name: Option<String>,
    pub protocol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalProviderVerifyReport {
    pub ok: bool,
    pub status_code: Option<u16>,
    pub checked_url: Option<String>,
    pub message: String,
    pub models_sample: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalProviderSetupReport {
    pub profile_env_path: String,
    pub gateway_url: String,
    pub model: String,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub runtimes: Vec<RuntimeSetupResult>,
    pub verify: Option<PersonalProviderVerifyReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalProviderEntry {
    pub id: String,
    pub name: String,
    pub url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalProviderListItem {
    pub id: String,
    pub name: String,
    pub url: String,
    pub model: String,
    pub protocol: String,
    pub api_key_hint: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalProvidersDocument {
    pub active_id: Option<String>,
    pub providers: Vec<PersonalProviderListItem>,
    pub store_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertPersonalProviderOptions {
    /// `None` creates a new provider.
    pub id: Option<String>,
    pub name: String,
    pub url: String,
    /// Empty on update keeps the existing key.
    pub api_key: String,
    pub model: String,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    /// When true, save then activate (verify + apply to runtimes).
    pub activate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersonalProvidersStore {
    active_id: Option<String>,
    providers: Vec<PersonalProviderEntry>,
}

pub fn personal_providers_path() -> Option<PathBuf> {
    dirs::config_dir().map(|base| base.join("agent-doctor").join("providers.json"))
}

pub fn normalize_personal_gateway_url(url: &str) -> Result<String> {
    // Keep the user's path intact (including `/v1`); do NOT append Evotown suffixes.
    normalize_gateway_url(url)
}

pub fn list_personal_providers() -> Result<PersonalProvidersDocument> {
    let path = personal_providers_path().context("could not resolve config directory")?;
    let mut store = load_store(&path)?;
    migrate_from_profile_if_empty(&mut store, &path)?;
    Ok(document_from_store(&store, &path))
}

pub fn upsert_personal_provider(
    options: &UpsertPersonalProviderOptions,
) -> Result<PersonalProvidersDocument> {
    let path = personal_providers_path().context("could not resolve config directory")?;
    let mut store = load_store(&path)?;
    migrate_from_profile_if_empty(&mut store, &path)?;

    let name = options.name.trim();
    if name.is_empty() {
        bail!("provider name must not be empty");
    }
    let url = normalize_personal_gateway_url(&options.url)?;
    let model = options.model.trim();
    if model.is_empty() {
        bail!("model must not be empty");
    }
    let protocol = normalize_protocol(&options.protocol);

    let id = if let Some(existing_id) = options
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let Some(entry) = store.providers.iter_mut().find(|p| p.id == existing_id) else {
            bail!("provider not found: {existing_id}");
        };
        let api_key = options.api_key.trim();
        if !api_key.is_empty() {
            entry.api_key = api_key.to_string();
        } else if entry.api_key.trim().is_empty() {
            bail!("API key must not be empty");
        }
        entry.name = name.to_string();
        entry.url = url;
        entry.model = model.to_string();
        entry.protocol = protocol;
        existing_id.to_string()
    } else {
        let api_key = options.api_key.trim();
        if api_key.is_empty() {
            bail!("API key must not be empty");
        }
        let id = new_provider_id();
        store.providers.push(PersonalProviderEntry {
            id: id.clone(),
            name: name.to_string(),
            url,
            api_key: api_key.to_string(),
            model: model.to_string(),
            protocol,
        });
        id
    };

    save_store(&path, &store)?;

    if options.activate {
        activate_personal_provider(&id)?;
        store = load_store(&path)?;
    }

    Ok(document_from_store(&store, &path))
}

pub fn delete_personal_provider(id: &str) -> Result<PersonalProvidersDocument> {
    let path = personal_providers_path().context("could not resolve config directory")?;
    let mut store = load_store(&path)?;
    let before = store.providers.len();
    store.providers.retain(|p| p.id != id);
    if store.providers.len() == before {
        bail!("provider not found: {id}");
    }
    if store.active_id.as_deref() == Some(id) {
        store.active_id = None;
    }
    save_store(&path, &store)?;
    Ok(document_from_store(&store, &path))
}

pub fn activate_personal_provider(id: &str) -> Result<PersonalProviderSetupReport> {
    let switch = crate::setup::pipeline::apply_mode_switch(
        crate::setup::pipeline::ModeSwitchTarget::Personal {
            provider_id: Some(id.to_string()),
        },
    )?;
    switch.personal.ok_or_else(|| {
        anyhow::anyhow!("personal mode switch did not return a personal setup report")
    })
}

/// Mark which personal provider is active without projecting runtimes.
pub(crate) fn set_active_personal_provider_id(id: &str) -> Result<()> {
    let path = personal_providers_path().context("could not resolve config directory")?;
    let mut store = load_store(&path)?;
    if !store.providers.iter().any(|p| p.id == id) {
        bail!("provider not found: {id}");
    }
    store.active_id = Some(id.to_string());
    save_store(&path, &store)
}

pub(crate) fn load_personal_provider_entry(id: &str) -> Result<PersonalProviderEntry> {
    let path = personal_providers_path().context("could not resolve config directory")?;
    let mut store = load_store(&path)?;
    migrate_from_profile_if_empty(&mut store, &path)?;
    store
        .providers
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .with_context(|| format!("provider not found: {id}"))
}

pub fn load_personal_provider_status() -> Result<PersonalProviderStatus> {
    let profile_path = agent_profile_path();
    let store_path = personal_providers_path();
    let store = store_path
        .as_ref()
        .and_then(|path| load_store(path).ok())
        .unwrap_or_default();

    let active = store
        .active_id
        .as_ref()
        .and_then(|id| store.providers.iter().find(|p| &p.id == id));

    if let Some(entry) = active {
        return Ok(PersonalProviderStatus {
            configured: true,
            gateway_url: Some(entry.url.clone()),
            model: Some(entry.model.clone()),
            api_key_hint: Some(mask_secret_value(&entry.api_key)),
            profile_env_path: profile_path.map(|p| p.display().to_string()),
            active_id: Some(entry.id.clone()),
            active_name: Some(entry.name.clone()),
            protocol: Some(normalize_protocol(&entry.protocol)),
        });
    }

    let Some(path) = profile_path else {
        return Ok(PersonalProviderStatus {
            configured: false,
            gateway_url: None,
            model: None,
            api_key_hint: None,
            profile_env_path: None,
            active_id: None,
            active_name: None,
            protocol: None,
        });
    };
    if !path.exists() {
        return Ok(PersonalProviderStatus {
            configured: false,
            gateway_url: None,
            model: None,
            api_key_hint: None,
            profile_env_path: Some(path.display().to_string()),
            active_id: None,
            active_name: None,
            protocol: None,
        });
    }

    let env = read_env_file(&path)?;
    let kind = env.get(PROVIDER_KIND_ENV).map(String::as_str).unwrap_or("");
    let gateway_url = env.get(GATEWAY_URL_ENV).cloned();
    let model = env.get(MODEL_ENV).cloned();
    let api_key = env
        .get(OPENAI_API_KEY_ENV)
        .or_else(|| env.get(COMPANY_API_KEY_ENV))
        .cloned();

    let configured = kind == PROVIDER_KIND_PERSONAL
        && gateway_url.as_deref().is_some_and(|u| !u.trim().is_empty())
        && api_key.as_deref().is_some_and(|k| !k.trim().is_empty());

    Ok(PersonalProviderStatus {
        configured,
        gateway_url: if configured { gateway_url } else { None },
        model: if configured { model } else { None },
        api_key_hint: if configured {
            api_key.as_deref().map(mask_secret_value)
        } else {
            None
        },
        profile_env_path: Some(path.display().to_string()),
        active_id: env.get(ACTIVE_PROVIDER_ID_ENV).cloned(),
        active_name: None,
        protocol: if configured {
            Some(normalize_protocol(
                env.get(PROVIDER_PROTOCOL_ENV)
                    .map(String::as_str)
                    .unwrap_or(PROTOCOL_OPENAI),
            ))
        } else {
            None
        },
    })
}

pub fn verify_personal_provider(url: &str, api_key: &str) -> Result<PersonalProviderVerifyReport> {
    verify_personal_provider_with_protocol(url, api_key, PROTOCOL_OPENAI)
}

pub fn verify_personal_provider_with_protocol(
    url: &str,
    api_key: &str,
    protocol: &str,
) -> Result<PersonalProviderVerifyReport> {
    let gateway_url = normalize_personal_gateway_url(url)?;
    let api_key = api_key.trim();
    if api_key.is_empty() {
        bail!("API key must not be empty");
    }
    let protocol = normalize_protocol(protocol);

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("failed to build HTTP client")?;

    if protocol == PROTOCOL_ANTHROPIC {
        return verify_anthropic_endpoint(&client, &gateway_url, api_key);
    }

    let mut last_error = String::from("no models endpoint responded");
    for checked in models_endpoint_candidates(&gateway_url) {
        let response = match client
            .get(&checked)
            .header("Authorization", format!("Bearer {api_key}"))
            .send()
        {
            Ok(resp) => resp,
            Err(err) => {
                last_error = format!("request failed for {checked}: {err}");
                continue;
            }
        };
        let status = response.status();
        let status_code = status.as_u16();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            last_error = format!(
                "{checked} → HTTP {status_code}: {}",
                body.chars().take(180).collect::<String>()
            );
            continue;
        }

        let body = response.text().unwrap_or_default();
        let models_sample = extract_model_ids(&body);
        return Ok(PersonalProviderVerifyReport {
            ok: true,
            status_code: Some(status_code),
            checked_url: Some(checked),
            message: if models_sample.is_empty() {
                "OpenAI-compatible gateway reachable.".to_string()
            } else {
                format!(
                    "OpenAI-compatible gateway reachable — saw {} model(s).",
                    models_sample.len()
                )
            },
            models_sample,
        });
    }

    Ok(PersonalProviderVerifyReport {
        ok: false,
        status_code: None,
        checked_url: None,
        message: last_error,
        models_sample: Vec::new(),
    })
}

pub fn execute_personal_provider_setup(
    options: &PersonalProviderOptions,
) -> Result<PersonalProviderSetupReport> {
    let gateway_url = normalize_personal_gateway_url(&options.url)?;
    let api_key = options.api_key.trim();
    if api_key.is_empty() {
        bail!("API key must not be empty");
    }
    let model = options.model.trim();
    if model.is_empty() {
        bail!("model must not be empty");
    }
    let protocol = normalize_protocol(&options.protocol);

    let verify = verify_personal_provider_with_protocol(&gateway_url, api_key, &protocol)?;
    if !verify.ok {
        bail!("provider connectivity check failed: {}", verify.message);
    }

    let profile_path = agent_profile_path().context("could not resolve config directory")?;
    write_personal_profile(
        &profile_path,
        &gateway_url,
        api_key,
        model,
        &protocol,
        None,
        None,
    )?;
    let _ = clear_codex_placeholder_auth();

    let mut runtimes = Vec::new();
    for adapter in all_adapters() {
        let result = match (protocol.as_str(), adapter.id()) {
            (PROTOCOL_OPENAI, "openclaw") => {
                merge::apply_openclaw(&gateway_url, api_key, Some(model))
            }
            (PROTOCOL_OPENAI, "hermes") => {
                merge::apply_hermes(&gateway_url, api_key, "custom", Some(model))
            }
            (PROTOCOL_OPENAI, "codex") => merge::apply_codex(&gateway_url, api_key, Some(model)),
            (PROTOCOL_OPENAI, "claude-code") => Ok(RuntimeSetupResult {
                runtime_id: "claude-code".to_string(),
                display_name: "Claude Code".to_string(),
                applied: false,
                message: "skipped — this provider uses OpenAI protocol; Claude Code needs Anthropic/Claude protocol"
                    .to_string(),
                ..Default::default()
            }),
            (PROTOCOL_ANTHROPIC, "claude-code") => {
                merge::apply_claude_code(&gateway_url, api_key)
            }
            (PROTOCOL_ANTHROPIC, other) => Ok(RuntimeSetupResult {
                runtime_id: other.to_string(),
                display_name: adapter.display_name().to_string(),
                applied: false,
                message: format!(
                    "skipped — this provider uses Claude/Anthropic protocol; {other} needs OpenAI-compatible API"
                ),
                ..Default::default()
            }),
            (_, other) => Ok(RuntimeSetupResult {
                runtime_id: other.to_string(),
                display_name: adapter.display_name().to_string(),
                applied: false,
                message: "no personal provider merge for this runtime yet".to_string(),
                ..Default::default()
            }),
        }?;
        runtimes.push(result);
    }

    Ok(PersonalProviderSetupReport {
        profile_env_path: profile_path.display().to_string(),
        gateway_url,
        model: model.to_string(),
        provider_id: None,
        provider_name: None,
        runtimes,
        verify: Some(verify),
    })
}

fn document_from_store(store: &PersonalProvidersStore, path: &Path) -> PersonalProvidersDocument {
    let active_id = store.active_id.clone();
    PersonalProvidersDocument {
        active_id: active_id.clone(),
        providers: store
            .providers
            .iter()
            .map(|entry| PersonalProviderListItem {
                id: entry.id.clone(),
                name: entry.name.clone(),
                url: entry.url.clone(),
                model: entry.model.clone(),
                protocol: normalize_protocol(&entry.protocol),
                api_key_hint: mask_secret_value(&entry.api_key),
                active: active_id.as_deref() == Some(entry.id.as_str()),
            })
            .collect(),
        store_path: path.display().to_string(),
    }
}

fn load_store(path: &Path) -> Result<PersonalProvidersStore> {
    if !path.exists() {
        return Ok(PersonalProvidersStore::default());
    }
    let raw = fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(PersonalProvidersStore::default());
    }
    serde_json::from_str(&raw).context("failed to parse providers.json")
}

fn save_store(path: &Path, store: &PersonalProvidersStore) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(store)?;
    fs::write(path, raw + "\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn migrate_from_profile_if_empty(
    store: &mut PersonalProvidersStore,
    store_path: &Path,
) -> Result<()> {
    if !store.providers.is_empty() {
        return Ok(());
    }
    let Some(profile_path) = agent_profile_path() else {
        return Ok(());
    };
    if !profile_path.exists() {
        return Ok(());
    }
    let env = read_env_file(&profile_path)?;
    if env.get(PROVIDER_KIND_ENV).map(String::as_str) != Some(PROVIDER_KIND_PERSONAL) {
        return Ok(());
    }
    let Some(url) = env
        .get(GATEWAY_URL_ENV)
        .cloned()
        .filter(|u| !u.trim().is_empty())
    else {
        return Ok(());
    };
    let Some(api_key) = env
        .get(OPENAI_API_KEY_ENV)
        .or_else(|| env.get(COMPANY_API_KEY_ENV))
        .cloned()
        .filter(|k| !k.trim().is_empty())
    else {
        return Ok(());
    };
    let model = env
        .get(MODEL_ENV)
        .cloned()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| "gpt-4o-mini".to_string());
    let id = env
        .get(ACTIVE_PROVIDER_ID_ENV)
        .cloned()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(new_provider_id);

    let protocol = normalize_protocol(
        env.get(PROVIDER_PROTOCOL_ENV)
            .map(String::as_str)
            .unwrap_or(PROTOCOL_OPENAI),
    );
    store.providers.push(PersonalProviderEntry {
        id: id.clone(),
        name: "Imported".to_string(),
        url,
        api_key,
        model,
        protocol,
    });
    store.active_id = Some(id);
    save_store(store_path, store)?;
    Ok(())
}

fn verify_anthropic_endpoint(
    client: &reqwest::blocking::Client,
    gateway_url: &str,
    api_key: &str,
) -> Result<PersonalProviderVerifyReport> {
    let checked = format!("{}/v1/messages", gateway_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": "claude-sonnet-4-5",
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "ping"}]
    });
    let response = client
        .post(&checked)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        // Some Anthropic-compatible gateways also accept Bearer.
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send();

    match response {
        Ok(resp) => {
            let status_code = resp.status().as_u16();
            // 200 OK or 400 with auth-accepted validation errors both prove reachability+auth.
            // 401/403 mean bad key.
            if status_code == 401 || status_code == 403 {
                let text = resp.text().unwrap_or_default();
                return Ok(PersonalProviderVerifyReport {
                    ok: false,
                    status_code: Some(status_code),
                    checked_url: Some(checked),
                    message: format!(
                        "Anthropic endpoint rejected key: {}",
                        text.chars().take(180).collect::<String>()
                    ),
                    models_sample: Vec::new(),
                });
            }
            if (200..500).contains(&status_code) {
                return Ok(PersonalProviderVerifyReport {
                    ok: true,
                    status_code: Some(status_code),
                    checked_url: Some(checked),
                    message: "Claude/Anthropic endpoint reachable.".to_string(),
                    models_sample: Vec::new(),
                });
            }
            let text = resp.text().unwrap_or_default();
            Ok(PersonalProviderVerifyReport {
                ok: false,
                status_code: Some(status_code),
                checked_url: Some(checked),
                message: format!(
                    "Anthropic endpoint error: {}",
                    text.chars().take(180).collect::<String>()
                ),
                models_sample: Vec::new(),
            })
        }
        Err(err) => Ok(PersonalProviderVerifyReport {
            ok: false,
            status_code: None,
            checked_url: Some(checked),
            message: format!("Anthropic request failed: {err}"),
            models_sample: Vec::new(),
        }),
    }
}

pub(crate) fn write_personal_profile(
    path: &Path,
    gateway_url: &str,
    api_key: &str,
    model: &str,
    protocol: &str,
    provider_id: Option<&str>,
    provider_name: Option<&str>,
) -> Result<()> {
    // Keep team baseline intact when switching the active overlay to personal.
    ensure_company_baseline_snapshot_from_active()?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(path).context("failed to create profile.env")?;
    writeln!(
        file,
        "# Agent Doctor personal provider — written by agent-doctor (not Evotown)"
    )?;
    writeln!(
        file,
        "# Source before running agents: set -a && source \"{}\" && set +a",
        path.display()
    )?;
    writeln!(file, "{PROVIDER_KIND_ENV}={PROVIDER_KIND_PERSONAL}")?;
    writeln!(
        file,
        "{PROVIDER_PROTOCOL_ENV}={}",
        normalize_protocol(protocol)
    )?;
    if let Some(id) = provider_id {
        writeln!(file, "{ACTIVE_PROVIDER_ID_ENV}={id}")?;
    }
    if let Some(name) = provider_name {
        writeln!(file, "AGENT_DOCTOR_PROVIDER_NAME={name}")?;
    }
    writeln!(file, "{GATEWAY_URL_ENV}={gateway_url}")?;
    writeln!(file, "{MODEL_ENV}={model}")?;
    writeln!(file, "{COMPANY_API_KEY_ENV}={api_key}")?;
    writeln!(file, "COMPANY_API_KEY={api_key}")?;
    writeln!(file, "{OPENAI_API_KEY_ENV}={api_key}")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn models_endpoint_candidates(base: &str) -> Vec<String> {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1") {
        vec![format!("{base}/models")]
    } else {
        vec![format!("{base}/models"), format!("{base}/v1/models")]
    }
}

fn extract_model_ids(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(data) = value.get("data").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    data.iter()
        .filter_map(|item| {
            item.get("id")
                .and_then(|id| id.as_str())
                .map(str::to_string)
        })
        .take(8)
        .collect()
}

fn read_env_file(path: &Path) -> Result<HashMap<String, String>> {
    let raw = fs::read_to_string(path)?;
    let mut map = HashMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        map.insert(key.trim().to_string(), value.trim().to_string());
    }
    Ok(map)
}

fn new_provider_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("pp_{nanos:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_url_keeps_v1_suffix() {
        assert_eq!(
            normalize_personal_gateway_url("https://api.example.com/v1/").unwrap(),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn models_candidates_for_bare_base() {
        assert_eq!(
            models_endpoint_candidates("https://api.example.com"),
            vec![
                "https://api.example.com/models".to_string(),
                "https://api.example.com/v1/models".to_string(),
            ]
        );
    }

    #[test]
    fn models_candidates_for_v1_base() {
        assert_eq!(
            models_endpoint_candidates("https://api.example.com/v1"),
            vec!["https://api.example.com/v1/models".to_string()]
        );
    }

    #[test]
    fn extract_model_ids_from_openai_shape() {
        let ids = extract_model_ids(
            r#"{"object":"list","data":[{"id":"gpt-4o-mini"},{"id":"deepseek-chat"}]}"#,
        );
        assert_eq!(ids, vec!["gpt-4o-mini", "deepseek-chat"]);
    }

    #[test]
    fn store_roundtrip_marks_active() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("providers.json");
        let store = PersonalProvidersStore {
            active_id: Some("a".into()),
            providers: vec![
                PersonalProviderEntry {
                    id: "a".into(),
                    name: "One".into(),
                    url: "https://a.example/v1".into(),
                    api_key: "sk-abcdefghijklmnop".into(),
                    model: "m1".into(),
                    protocol: PROTOCOL_OPENAI.into(),
                },
                PersonalProviderEntry {
                    id: "b".into(),
                    name: "Two".into(),
                    url: "https://b.example/anthropic".into(),
                    api_key: "sk-bbbbbbbbbbbbbbbb".into(),
                    model: "claude-sonnet-4-5".into(),
                    protocol: PROTOCOL_ANTHROPIC.into(),
                },
            ],
        };
        save_store(&path, &store).unwrap();
        let loaded = load_store(&path).unwrap();
        let doc = document_from_store(&loaded, &path);
        assert_eq!(doc.providers.len(), 2);
        assert!(doc.providers[0].active);
        assert!(!doc.providers[1].active);
        assert_eq!(doc.providers[0].api_key_hint, "sk-***nop");
    }
}
