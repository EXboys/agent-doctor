//! Exclusive personal vs team (Evotown) mode for runtime LLM wiring.
//!
//! - **personal**: runtimes use the active personal provider endpoint + key
//! - **team**: runtimes use Evotown / company gateway only
//!
//! Modes are mutually exclusive for gateway URL + API key. Control-plane files
//! (`evotown.agent.env`, `company-profile.env`, `personal-providers.json`) are kept
//! so you can switch back without re-entering credentials.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::profile::{
    company_baseline_path, read_agent_profile, read_company_baseline, read_env_map, ProviderKind,
};
use crate::repair::mask_secret_value;
use super::personal::{
    activate_personal_provider, list_personal_providers, PersonalProviderSetupReport,
    PersonalProvidersDocument,
};
use super::{
    evotown_agent_env_path, evotown_base_from_gateway, execute_setup, gateway_url_from_evotown_base,
    RuntimeSetupResult, SetupOptions, SetupReport, EVOTOWN_API_KEY_ENV, EVOTOWN_URL_ENV,
};

pub const MODE_PERSONAL: &str = "personal";
pub const MODE_TEAM: &str = "team";
pub const MODE_UNSET: &str = "unset";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeStatus {
    /// `personal` | `team` | `unset`
    pub mode: String,
    pub personal_ready: bool,
    pub team_ready: bool,
    pub active_label: Option<String>,
    pub active_gateway_url: Option<String>,
    pub active_key_hint: Option<String>,
    pub personal_active_id: Option<String>,
    pub personal_active_name: Option<String>,
    pub team_base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeSwitchReport {
    pub mode: String,
    pub active_label: Option<String>,
    pub active_gateway_url: Option<String>,
    pub runtimes: Vec<RuntimeSetupResult>,
    pub message: String,
    pub personal: Option<PersonalProviderSetupReport>,
    pub team_setup: Option<SetupReport>,
}

pub fn load_mode_status() -> Result<ModeStatus> {
    let personal_doc = list_personal_providers().unwrap_or(PersonalProvidersDocument {
        active_id: None,
        providers: Vec::new(),
        store_path: String::new(),
    });
    let personal_ready = !personal_doc.providers.is_empty();
    let personal_active = personal_doc
        .active_id
        .as_ref()
        .and_then(|id| personal_doc.providers.iter().find(|p| &p.id == id));

    let team = resolve_team_credentials().ok();
    let team_ready = team.is_some();
    let team_base_url = team.as_ref().map(|t| t.base_url.clone());

    let active = read_agent_profile().ok().flatten();

    // Profile kind is the source of truth for which LLM path is active.
    let mode = match active.as_ref().map(|p| p.kind) {
        Some(ProviderKind::Personal) => MODE_PERSONAL.to_string(),
        Some(ProviderKind::Company) => MODE_TEAM.to_string(),
        Some(ProviderKind::Unknown) | None => {
            if team_ready && active.as_ref().and_then(|p| p.gateway_url.as_ref()).is_some() {
                MODE_TEAM.to_string()
            } else if personal_active.is_some()
                && active
                    .as_ref()
                    .is_some_and(|p| p.kind == ProviderKind::Personal)
            {
                MODE_PERSONAL.to_string()
            } else if active.is_none() && !team_ready && !personal_ready {
                MODE_UNSET.to_string()
            } else if personal_active.is_some() && !team_ready {
                MODE_PERSONAL.to_string()
            } else if team_ready && personal_active.is_none() {
                MODE_TEAM.to_string()
            } else {
                MODE_UNSET.to_string()
            }
        }
    };

    let (active_label, active_gateway_url, active_key_hint) = match mode.as_str() {
        MODE_PERSONAL => (
            personal_active
                .map(|p| p.name.clone())
                .or_else(|| Some("Personal".to_string())),
            active
                .as_ref()
                .and_then(|p| p.gateway_url.clone())
                .or_else(|| personal_active.map(|p| p.url.clone())),
            active
                .as_ref()
                .and_then(|p| p.api_key.as_deref())
                .map(mask_secret_value)
                .or_else(|| personal_active.map(|p| p.api_key_hint.clone())),
        ),
        MODE_TEAM => (
            Some("Evotown".to_string()),
            active
                .as_ref()
                .and_then(|p| p.gateway_url.clone())
                .or_else(|| {
                    team.as_ref()
                        .map(|t| gateway_url_from_evotown_base(&t.base_url))
                }),
            active
                .as_ref()
                .and_then(|p| p.api_key.as_deref())
                .map(mask_secret_value)
                .or_else(|| team.as_ref().map(|t| mask_secret_value(&t.api_key))),
        ),
        _ => (None, None, None),
    };

    Ok(ModeStatus {
        mode,
        personal_ready,
        team_ready,
        active_label,
        active_gateway_url,
        active_key_hint,
        personal_active_id: personal_active.map(|p| p.id.clone()),
        personal_active_name: personal_active.map(|p| p.name.clone()),
        team_base_url,
    })
}

/// Switch runtime wiring to personal provider (exclusive).
///
/// `provider_id` selects which saved provider; `None` uses the last active personal provider.
pub fn switch_to_personal_mode(provider_id: Option<&str>) -> Result<ModeSwitchReport> {
    let doc = list_personal_providers()?;
    if doc.providers.is_empty() {
        bail!("no personal providers saved — add one under Personal first");
    }
    let id = provider_id
        .map(str::to_string)
        .or(doc.active_id.clone())
        .or_else(|| doc.providers.first().map(|p| p.id.clone()))
        .context("no personal provider id")?;

    let personal = activate_personal_provider(&id)?;
    let applied = personal.runtimes.iter().filter(|r| r.applied).count();

    Ok(ModeSwitchReport {
        mode: MODE_PERSONAL.to_string(),
        active_label: personal.provider_name.clone(),
        active_gateway_url: Some(personal.gateway_url.clone()),
        runtimes: personal.runtimes.clone(),
        message: format!(
            "personal mode — {applied} runtime(s) wired to personal provider (Evotown gateway not used for LLM)"
        ),
        personal: Some(personal),
        team_setup: None,
    })
}

/// Switch runtime wiring to Evotown / company gateway (exclusive).
pub fn switch_to_team_mode() -> Result<ModeSwitchReport> {
    let team = resolve_team_credentials().context(
        "team mode not configured — connect Evotown first (URL + evk_ key)",
    )?;

    let gateway_url = gateway_url_from_evotown_base(&team.base_url);
    let setup = execute_setup(&SetupOptions {
        gateway_url: gateway_url.clone(),
        api_key: team.api_key.clone(),
        hermes_provider: "openai".to_string(),
    })?;
    let applied = setup.runtimes.iter().filter(|r| r.applied).count();

    Ok(ModeSwitchReport {
        mode: MODE_TEAM.to_string(),
        active_label: Some("Evotown".to_string()),
        active_gateway_url: Some(gateway_url),
        runtimes: setup.runtimes.clone(),
        message: format!(
            "team mode — {applied} runtime(s) wired to Evotown gateway (personal provider not used for LLM)"
        ),
        personal: None,
        team_setup: Some(setup),
    })
}

struct TeamCredentials {
    base_url: String,
    api_key: String,
}

fn resolve_team_credentials() -> Result<TeamCredentials> {
    // Prefer dedicated Evotown agent env (survives personal overlay).
    if let Some(path) = evotown_agent_env_path() {
        if path.exists() {
            let env = read_env_map(&path)?;
            let base_url = env
                .get(EVOTOWN_URL_ENV)
                .cloned()
                .filter(|u| !u.trim().is_empty());
            let api_key = env
                .get(EVOTOWN_API_KEY_ENV)
                .cloned()
                .filter(|k| !k.trim().is_empty());
            if let (Some(base_url), Some(api_key)) = (base_url, api_key) {
                return Ok(TeamCredentials {
                    base_url: base_url.trim().trim_end_matches('/').to_string(),
                    api_key,
                });
            }
        }
    }

    let baseline = read_company_baseline()?.context("no company baseline")?;
    let gateway = baseline
        .gateway_url
        .filter(|u| !u.trim().is_empty())
        .context("company baseline missing gateway URL")?;
    let api_key = baseline
        .api_key
        .filter(|k| !k.trim().is_empty())
        .context("company baseline missing API key")?;

    let base_url = company_baseline_path()
        .filter(|path| path.exists())
        .and_then(|path| read_env_map(&path).ok())
        .and_then(|env| env.get("AGENT_DOCTOR_EVOTOWN_URL").cloned())
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| evotown_base_from_gateway(&gateway));

    Ok(TeamCredentials { base_url, api_key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_constants_are_stable() {
        assert_eq!(MODE_PERSONAL, "personal");
        assert_eq!(MODE_TEAM, "team");
        assert_eq!(MODE_UNSET, "unset");
    }

    #[test]
    fn load_mode_status_does_not_panic() {
        let status = load_mode_status().expect("status");
        assert!(
            status.mode == MODE_PERSONAL
                || status.mode == MODE_TEAM
                || status.mode == MODE_UNSET
        );
    }
}
