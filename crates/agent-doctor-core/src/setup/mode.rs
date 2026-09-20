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

use super::personal::{
    list_personal_providers, PersonalProviderSetupReport, PersonalProvidersDocument,
};
use super::pipeline::{apply_mode_switch, ModeSwitchTarget};
use super::{
    evotown_agent_env_path, evotown_base_from_gateway, gateway_url_from_evotown_base,
    RuntimeSetupResult, SetupReport, EVOTOWN_API_KEY_ENV, EVOTOWN_URL_ENV,
};
use crate::profile::{
    agent_profile_path, company_baseline_path, read_agent_profile, read_company_baseline,
    read_env_map, ProviderKind,
};
use crate::repair::mask_secret_value;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
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
            if team_ready
                && active
                    .as_ref()
                    .and_then(|p| p.gateway_url.as_ref())
                    .is_some()
            {
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
    apply_mode_switch(ModeSwitchTarget::Personal {
        provider_id: provider_id.map(str::to_string),
    })
}

/// Switch runtime wiring to Evotown / company gateway (exclusive).
pub fn switch_to_team_mode() -> Result<ModeSwitchReport> {
    apply_mode_switch(ModeSwitchTarget::Team)
}

/// Fast overlay readiness check that only reads `profile.env` (no OS keychain).
pub fn mode_overlay_ready_from_profile() -> bool {
    matches!(
        read_agent_profile().ok().flatten().map(|p| p.kind),
        Some(ProviderKind::Personal | ProviderKind::Company)
    )
}

/// Keep the runtime's live gateway; update Agent Doctor **personal** overlay to match.
///
/// Does **not** rewrite runtime configs. Refused in team mode so company baseline
/// stays authoritative.
pub fn adopt_live_gateway_as_overlay(live_url: &str) -> Result<()> {
    use super::personal::{
        write_personal_profile, ACTIVE_PROVIDER_ID_ENV, MODEL_ENV, PROTOCOL_ANTHROPIC,
        PROTOCOL_OPENAI, PROVIDER_PROTOCOL_ENV,
    };

    let live = live_url.trim().trim_end_matches('/');
    if live.is_empty() {
        bail!("live gateway URL is empty");
    }
    if !(live.starts_with("http://") || live.starts_with("https://")) {
        bail!("live gateway URL must start with http:// or https://");
    }

    let path = agent_profile_path().context("could not resolve profile.env path")?;
    let active = read_agent_profile()?.context("no active Agent Doctor overlay")?;
    if active.kind != ProviderKind::Personal {
        bail!(
            "keeping live gateway is only supported in personal mode \
             (team/company baseline must stay authoritative)"
        );
    }
    let api_key = active
        .api_key
        .filter(|k| !k.trim().is_empty())
        .context("personal overlay is missing an API key in profile.env")?;

    let env = read_env_map(&path)?;
    let model = env
        .get(MODEL_ENV)
        .cloned()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| super::COMPANY_DEFAULT_MODEL.to_string());
    let mut protocol = env
        .get(PROVIDER_PROTOCOL_ENV)
        .cloned()
        .unwrap_or_else(|| PROTOCOL_OPENAI.to_string());
    // Claude Anthropic-Messages gateways often end with /anthropic.
    if live.contains("/anthropic") {
        protocol = PROTOCOL_ANTHROPIC.to_string();
    }
    let provider_id = env.get(ACTIVE_PROVIDER_ID_ENV).map(String::as_str);
    let provider_name = env.get("AGENT_DOCTOR_PROVIDER_NAME").map(String::as_str);

    write_personal_profile(
        &path,
        live,
        &api_key,
        &model,
        &protocol,
        provider_id,
        provider_name,
    )?;

    // Best-effort: keep settings.db personal provider URL in sync (SQLite only).
    if let Ok(store) = crate::store::open_settings_store() {
        if let Ok(records) = store.list_personal_providers() {
            for record in records {
                if record.active || provider_id == Some(record.id.as_str()) {
                    let mut updated = record;
                    updated.url = live.to_string();
                    let _ = store.upsert_personal_provider(&updated);
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Team credentials for status checks (overlay readiness).
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

struct TeamCredentials {
    base_url: String,
    api_key: String,
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
            status.mode == MODE_PERSONAL || status.mode == MODE_TEAM || status.mode == MODE_UNSET
        );
    }
}
