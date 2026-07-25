use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::profile::{
    agent_profile_path, company_baseline_path, read_company_baseline,
    read_env_map as read_profile_env_map,
};
use crate::setup::{
    default_evotown_skills_dir, evotown_agent_env_path, evotown_base_from_gateway,
    DEFAULT_EVOTOWN_BUNDLE_ID, DEFAULT_EVOTOWN_RUNTIME, EVOTOWN_API_KEY_ENV, EVOTOWN_BUNDLE_ID_ENV,
    EVOTOWN_RUNTIME_ENV, EVOTOWN_SKILLS_DIR_ENV, EVOTOWN_URL_ENV,
};

pub const DEFAULT_BUNDLE_ID: &str = DEFAULT_EVOTOWN_BUNDLE_ID;
pub const DEFAULT_RUNTIME_TARGET: &str = DEFAULT_EVOTOWN_RUNTIME;

#[derive(Debug, Clone)]
pub struct EvotownConfig {
    pub base_url: String,
    pub api_key: String,
    pub runtime_target: String,
    pub bundle_id: String,
    pub skills_dir: PathBuf,
    pub skills_lock_path: PathBuf,
    pub policy_cache_path: PathBuf,
    pub config_source: String,
}

pub fn evotown_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|base| base.join("evotown"))
}

pub fn default_skills_dir() -> PathBuf {
    default_evotown_skills_dir()
}

pub fn default_skills_lock_path() -> Option<PathBuf> {
    evotown_config_dir().map(|base| base.join("skills-lock.json"))
}

pub fn default_policy_cache_path() -> Option<PathBuf> {
    evotown_config_dir().map(|base| base.join("policies-cache.json"))
}

pub fn load_evotown_config() -> Result<EvotownConfig> {
    // Prefer dedicated Evotown env so a personal provider overlay cannot hijack team connect.
    if let Some(path) = evotown_agent_env_path() {
        if path.exists() {
            return load_from_env_file(&path, path.display().to_string());
        }
    }
    if let Some(from_profile) = load_from_company_profile()? {
        return Ok(from_profile);
    }
    bail!(
        "Evotown is not configured — run `agent-doctor setup --url <evotown-url> --key evk_...` \
         or create ~/.config/evotown/evotown.agent.env"
    )
}

fn load_from_company_profile() -> Result<Option<EvotownConfig>> {
    // Durable team baseline only — never the personal provider overlay.
    let Some(profile) = read_company_baseline()? else {
        return Ok(None);
    };
    let Some(gateway_url) = profile.gateway_url.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let Some(api_key) = profile.api_key.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };

    let evotown_url = company_baseline_path()
        .filter(|path| path.exists())
        .and_then(|path| read_profile_env_map(&path).ok())
        .and_then(|env| env.get("AGENT_DOCTOR_EVOTOWN_URL").cloned())
        .filter(|value| !value.trim().is_empty());

    let base_url = evotown_url.unwrap_or_else(|| evotown_base_from_gateway(&gateway_url));
    let agent_env = evotown_agent_env_path();
    let file_overrides = agent_env
        .as_ref()
        .filter(|path| path.exists())
        .map(|path| load_env_map(path))
        .transpose()?
        .unwrap_or_default();

    let source = company_baseline_path()
        .filter(|path| path.exists())
        .or_else(agent_profile_path)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "company-profile.env".to_string());

    Ok(Some(build_config(
        base_url,
        api_key,
        file_overrides,
        source,
    )?))
}

fn load_from_env_file(path: &Path, source: String) -> Result<EvotownConfig> {
    let file_values = load_env_map(path)?;
    let base_url = file_values
        .get(EVOTOWN_URL_ENV)
        .cloned()
        .or_else(|| std::env::var(EVOTOWN_URL_ENV).ok())
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .context(format!("{EVOTOWN_URL_ENV} is required in {source}"))?;

    let api_key = file_values
        .get(EVOTOWN_API_KEY_ENV)
        .cloned()
        .or_else(|| std::env::var(EVOTOWN_API_KEY_ENV).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .context(format!("{EVOTOWN_API_KEY_ENV} is required in {source}"))?;

    build_config(base_url, api_key, file_values, source)
}

fn build_config(
    base_url: String,
    api_key: String,
    overrides: std::collections::HashMap<String, String>,
    config_source: String,
) -> Result<EvotownConfig> {
    validate_evotown_api_key(&api_key)?;

    let runtime_target = overrides
        .get(EVOTOWN_RUNTIME_ENV)
        .cloned()
        .or_else(|| std::env::var(EVOTOWN_RUNTIME_ENV).ok())
        .unwrap_or_else(|| DEFAULT_RUNTIME_TARGET.to_string());

    let bundle_id = overrides
        .get(EVOTOWN_BUNDLE_ID_ENV)
        .cloned()
        .or_else(|| std::env::var(EVOTOWN_BUNDLE_ID_ENV).ok())
        .unwrap_or_else(|| DEFAULT_BUNDLE_ID.to_string());

    let skills_dir = overrides
        .get(EVOTOWN_SKILLS_DIR_ENV)
        .map(|value| expand_path(value))
        .or_else(|| {
            std::env::var(EVOTOWN_SKILLS_DIR_ENV)
                .ok()
                .map(|value| expand_path(&value))
        })
        .unwrap_or_else(default_skills_dir);

    let skills_lock_path = default_skills_lock_path().context("could not resolve config dir")?;
    let policy_cache_path = default_policy_cache_path().context("could not resolve config dir")?;

    Ok(EvotownConfig {
        base_url: base_url.trim_end_matches('/').to_string(),
        api_key,
        runtime_target,
        bundle_id,
        skills_dir,
        skills_lock_path,
        policy_cache_path,
        config_source,
    })
}

pub fn validate_evotown_api_key(api_key: &str) -> Result<()> {
    if api_key.starts_with("evk_") {
        return Ok(());
    }
    bail!("Evotown API key must start with `evk_` (employee key from Evotown control plane)")
}

fn load_env_map(path: &Path) -> Result<std::collections::HashMap<String, String>> {
    let raw = std::fs::read_to_string(path)?;
    let mut values = std::collections::HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let assignment = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = assignment.split_once('=') else {
            continue;
        };
        values.insert(
            key.trim().to_string(),
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string(),
        );
    }
    Ok(values)
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

pub fn evotown_status() -> Result<EvotownStatus> {
    match load_evotown_config() {
        Ok(config) => Ok(EvotownStatus {
            configured: true,
            base_url: Some(config.base_url),
            api_key_hint: Some(mask_key(&config.api_key)),
            config_source: Some(config.config_source),
            runtime_target: Some(config.runtime_target),
            bundle_id: Some(config.bundle_id),
        }),
        Err(_) => Ok(EvotownStatus {
            configured: false,
            base_url: read_company_baseline().ok().flatten().and_then(|profile| {
                profile
                    .gateway_url
                    .map(|url| evotown_base_from_gateway(&url))
            }),
            api_key_hint: read_company_baseline()
                .ok()
                .flatten()
                .and_then(|profile| profile.api_key.map(|key| mask_key(&key))),
            config_source: None,
            runtime_target: None,
            bundle_id: None,
        }),
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvotownStatus {
    pub configured: bool,
    pub base_url: Option<String>,
    pub api_key_hint: Option<String>,
    pub config_source: Option<String>,
    pub runtime_target: Option<String>,
    pub bundle_id: Option<String>,
}

fn mask_key(key: &str) -> String {
    if key.len() <= 12 {
        return "evk_…".to_string();
    }
    format!("{}…", &key[..12])
}
