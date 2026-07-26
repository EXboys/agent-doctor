//! Unified mode-switch pipeline: resolve → project → effector metadata → probe.
//!
//! All personal/team LLM wiring should enter through [`apply_mode_switch`].

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::profile::{
    agent_profile_path, company_baseline_path, read_company_baseline, read_env_map,
};
use crate::runtime::all_adapters;
use crate::setup::merge::{self, clear_codex_placeholder_auth, COMPANY_DEFAULT_MODEL};
use crate::setup::personal::{
    load_personal_provider_entry, normalize_personal_gateway_url, normalize_protocol,
    set_active_personal_provider_id, verify_personal_provider_with_protocol, write_personal_profile,
    list_personal_providers, PersonalProviderSetupReport, PersonalProviderVerifyReport,
    PROTOCOL_ANTHROPIC, PROTOCOL_OPENAI,
};
use crate::setup::{
    anthropic_gateway_url_from_evotown_base, evotown_agent_env_path, evotown_base_from_gateway,
    gateway_url_from_evotown_base, normalize_gateway_url, write_company_profile_with_gateway,
    write_evotown_agent_env, ModeSwitchReport, RuntimeSetupResult, SetupReport,
    DEFAULT_EVOTOWN_RUNTIME, EVOTOWN_API_KEY_ENV, EVOTOWN_URL_ENV, MODE_PERSONAL, MODE_TEAM,
};

/// How a runtime should treat provider entries when projecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteSemantics {
    /// Single current provider overwrites live config (Claude/Codex style).
    Exclusive,
    /// Providers coexist; current is a pointer (OpenClaw direction for P1).
    Additive,
}

/// Post-write action required for the new key/URL to take effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectorKind {
    None,
    /// Process env must be reloaded (OpenClaw LaunchAgent).
    RestartGateway,
    /// User must restart the client/terminal (Codex).
    ManualRestart,
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeStrategy {
    pub runtime_id: &'static str,
    pub write_semantics: WriteSemantics,
    pub effector: EffectorKind,
    pub openai_compatible: bool,
    pub anthropic_compatible: bool,
}

pub fn runtime_strategies() -> &'static [RuntimeStrategy] {
    &[
        RuntimeStrategy {
            runtime_id: "openclaw",
            write_semantics: WriteSemantics::Additive,
            effector: EffectorKind::RestartGateway,
            openai_compatible: true,
            anthropic_compatible: false,
        },
        RuntimeStrategy {
            runtime_id: "hermes",
            write_semantics: WriteSemantics::Additive,
            effector: EffectorKind::RestartGateway,
            openai_compatible: true,
            anthropic_compatible: false,
        },
        RuntimeStrategy {
            runtime_id: "codex",
            write_semantics: WriteSemantics::Additive,
            effector: EffectorKind::ManualRestart,
            openai_compatible: true,
            anthropic_compatible: false,
        },
        RuntimeStrategy {
            runtime_id: "claude-code",
            write_semantics: WriteSemantics::Exclusive,
            effector: EffectorKind::None,
            openai_compatible: false,
            anthropic_compatible: true,
        },
    ]
}

pub fn strategy_for(runtime_id: &str) -> Option<&'static RuntimeStrategy> {
    runtime_strategies()
        .iter()
        .find(|s| s.runtime_id == runtime_id)
}

pub fn effector_label(kind: EffectorKind) -> &'static str {
    match kind {
        EffectorKind::None => "none",
        EffectorKind::RestartGateway => "restart_gateway",
        EffectorKind::ManualRestart => "manual_restart",
    }
}

/// Resolved credentials + model for the active mode (never mix personal/team).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointBundle {
    pub mode: String,
    pub label: String,
    pub gateway_url: String,
    pub api_key: String,
    pub model: String,
    pub protocol: String,
    pub source_id: String,
    pub hermes_provider: String,
    pub anthropic_gateway_url: Option<String>,
    pub personal_provider_id: Option<String>,
    pub personal_provider_name: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ModeSwitchTarget {
    Personal { provider_id: Option<String> },
    Team,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleProbeReport {
    pub ok: bool,
    pub detail: String,
    pub checked_url: Option<String>,
    pub status_code: Option<u16>,
}

/// Single entry for personal/team mode switches.
pub fn apply_mode_switch(target: ModeSwitchTarget) -> Result<ModeSwitchReport> {
    let bundle = match &target {
        ModeSwitchTarget::Personal { provider_id } => {
            resolve_personal_bundle(provider_id.as_deref())?
        }
        ModeSwitchTarget::Team => resolve_team_bundle()?,
    };

    let mut verify_report = None;
    if bundle.mode == MODE_PERSONAL {
        let verify = verify_personal_provider_with_protocol(
            &bundle.gateway_url,
            &bundle.api_key,
            &bundle.protocol,
        )?;
        if !verify.ok {
            bail!("provider connectivity check failed: {}", verify.message);
        }
        verify_report = Some(verify);
    }

    write_overlay_for_bundle(&bundle)?;
    let _ = clear_codex_placeholder_auth();

    let mut runtimes = project_bundle(&bundle)?;
    let probe = probe_endpoint_bundle(&bundle);
    annotate_runtimes_with_strategy_and_probe(&mut runtimes, &probe);

    let applied = runtimes.iter().filter(|r| r.applied).count();
    let mut warnings = Vec::new();
    if !probe.ok {
        warnings.push(format!("llm_probe_failed: {}", probe.detail));
    }
    for runtime in &runtimes {
        if runtime.applied {
            if let Some(false) = runtime.effector_ok {
                if let Some(detail) = &runtime.effector_detail {
                    warnings.push(format!("{}: {}", runtime.runtime_id, detail));
                }
            }
        }
    }

    let message = match bundle.mode.as_str() {
        MODE_PERSONAL => format!(
            "personal mode — {applied} runtime(s) wired to {} (model {}); probe={}",
            bundle.label,
            bundle.model,
            if probe.ok { "ok" } else { "fail" }
        ),
        MODE_TEAM => format!(
            "team mode — {applied} runtime(s) wired to Evotown (model {}); probe={}",
            bundle.model,
            if probe.ok { "ok" } else { "fail" }
        ),
        other => format!("mode {other} — {applied} runtime(s); probe={}", probe.ok),
    };

    let personal = if bundle.mode == MODE_PERSONAL {
        Some(PersonalProviderSetupReport {
            provider_id: bundle.personal_provider_id.clone(),
            provider_name: bundle.personal_provider_name.clone(),
            profile_env_path: agent_profile_path()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            gateway_url: bundle.gateway_url.clone(),
            model: bundle.model.clone(),
            runtimes: runtimes.clone(),
            verify: verify_report.or(Some(PersonalProviderVerifyReport {
                ok: probe.ok,
                status_code: probe.status_code,
                checked_url: probe.checked_url.clone(),
                message: probe.detail.clone(),
                models_sample: Vec::new(),
            })),
        })
    } else {
        None
    };

    let team_setup = if bundle.mode == MODE_TEAM {
        Some(SetupReport {
            profile_env_path: agent_profile_path()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            gateway_url: bundle.gateway_url.clone(),
            evotown_base_url: evotown_base_from_gateway(&bundle.gateway_url),
            evotown_agent_env_path: evotown_agent_env_path().map(|p| p.display().to_string()),
            runtimes: runtimes.clone(),
        })
    } else {
        None
    };

    Ok(ModeSwitchReport {
        mode: bundle.mode.clone(),
        active_label: Some(bundle.label.clone()),
        active_gateway_url: Some(bundle.gateway_url.clone()),
        runtimes,
        message,
        personal,
        team_setup,
        model: Some(bundle.model),
        source_id: Some(bundle.source_id),
        probe_ok: Some(probe.ok),
        probe_detail: Some(probe.detail),
        warnings,
    })
}

/// Project an already-resolved bundle onto installed runtimes (no overlay rewrite).
pub fn project_bundle(bundle: &EndpointBundle) -> Result<Vec<RuntimeSetupResult>> {
    let mut runtimes = Vec::new();
    for adapter in all_adapters() {
        let mut result = project_one_runtime(bundle, adapter.id(), adapter.display_name())?;
        if let Some(strategy) = strategy_for(adapter.id()) {
            result.effector = Some(effector_label(strategy.effector).to_string());
            if strategy.effector == EffectorKind::RestartGateway && result.applied {
                let restarted = result.message.contains("restarted OpenClaw gateway");
                let skipped = result.message.contains("gateway restart skipped");
                if strategy.runtime_id == "openclaw" {
                    result.effector_ok = Some(restarted || !skipped);
                    if skipped {
                        result.effector_detail = Some(result.message.clone());
                    } else if restarted {
                        result.effector_detail = Some("restarted".into());
                    } else {
                        result.effector_detail =
                            Some("key synced; restart may have been skipped".into());
                    }
                } else {
                    result.effector_ok = Some(true);
                    result.effector_detail =
                        Some("restart Hermes gateway if it was already running".into());
                }
            } else if strategy.effector == EffectorKind::ManualRestart && result.applied {
                result.effector_ok = Some(true);
                result.effector_detail = Some("restart Codex client/terminal to apply".into());
            } else if result.applied {
                result.effector_ok = Some(true);
                result.effector_detail = Some("none".into());
            }
        }
        runtimes.push(result);
    }
    Ok(runtimes)
}

fn project_one_runtime(
    bundle: &EndpointBundle,
    runtime_id: &str,
    display_name: &str,
) -> Result<RuntimeSetupResult> {
    let protocol = bundle.protocol.as_str();
    match (protocol, runtime_id) {
        (PROTOCOL_OPENAI, "openclaw") => {
            let slot = if bundle.mode == MODE_TEAM {
                merge::OPENCLAW_TEAM_SLOT
            } else {
                merge::OPENCLAW_PERSONAL_SLOT
            };
            merge::apply_openclaw_slot(
                &bundle.gateway_url,
                &bundle.api_key,
                Some(&bundle.model),
                Some(slot),
            )
        }
        (PROTOCOL_OPENAI, "hermes") => {
            let slot = if bundle.mode == MODE_TEAM {
                merge::HERMES_TEAM_SLOT
            } else {
                merge::HERMES_PERSONAL_SLOT
            };
            merge::apply_hermes_slot(
                &bundle.gateway_url,
                &bundle.api_key,
                &bundle.hermes_provider,
                Some(&bundle.model),
                Some(slot),
            )
        }
        (PROTOCOL_OPENAI, "codex") => {
            let slot = if bundle.mode == MODE_TEAM {
                merge::CODEX_TEAM_SLOT
            } else {
                merge::CODEX_PERSONAL_SLOT
            };
            merge::apply_codex_slot(
                &bundle.gateway_url,
                &bundle.api_key,
                Some(&bundle.model),
                Some(slot),
            )
        }
        (PROTOCOL_OPENAI, "claude-code") => Ok(RuntimeSetupResult {
            runtime_id: "claude-code".into(),
            display_name: display_name.into(),
            applied: false,
            message: "skipped — OpenAI protocol; Claude Code needs Anthropic protocol".into(),
            effector: Some(effector_label(EffectorKind::None).into()),
            ..Default::default()
        }),
        (PROTOCOL_ANTHROPIC, "claude-code") => {
            let url = bundle
                .anthropic_gateway_url
                .as_deref()
                .unwrap_or(&bundle.gateway_url);
            merge::apply_claude_code(url, &bundle.api_key)
        }
        (PROTOCOL_ANTHROPIC, other) => Ok(RuntimeSetupResult {
            runtime_id: other.into(),
            display_name: display_name.into(),
            applied: false,
            message: format!(
                "skipped — Anthropic protocol; {other} needs OpenAI-compatible API"
            ),
            effector: strategy_for(other).map(|s| effector_label(s.effector).into()),
            ..Default::default()
        }),
        (_, other) => Ok(RuntimeSetupResult {
            runtime_id: other.into(),
            display_name: display_name.into(),
            applied: false,
            message: "no projector for this runtime/protocol yet".into(),
            ..Default::default()
        }),
    }
}

fn annotate_runtimes_with_strategy_and_probe(
    runtimes: &mut [RuntimeSetupResult],
    probe: &BundleProbeReport,
) {
    for runtime in runtimes.iter_mut() {
        if runtime.applied {
            runtime.probe_ok = Some(probe.ok);
            runtime.probe_detail = Some(probe.detail.clone());
        }
    }
}

pub fn probe_endpoint_bundle(bundle: &EndpointBundle) -> BundleProbeReport {
    if bundle.protocol == PROTOCOL_ANTHROPIC {
        return probe_anthropic_bundle(bundle);
    }
    probe_openai_chat_bundle(bundle)
}

fn probe_openai_chat_bundle(bundle: &EndpointBundle) -> BundleProbeReport {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(25))
        .build()
    {
        Ok(c) => c,
        Err(err) => {
            return BundleProbeReport {
                ok: false,
                detail: format!("http client: {err}"),
                checked_url: None,
                status_code: None,
            };
        }
    };

    let url = format!(
        "{}/chat/completions",
        bundle.gateway_url.trim_end_matches('/')
    );
    let body = serde_json::json!({
        "model": bundle.model,
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 8,
        "stream": false
    });

    match client
        .post(&url)
        .header("Authorization", format!("Bearer {}", bundle.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
    {
        Ok(resp) => {
            let status = resp.status();
            let code = status.as_u16();
            let text = resp.text().unwrap_or_default();
            if status.is_success() {
                BundleProbeReport {
                    ok: true,
                    detail: format!("chat/completions HTTP {code} model={}", bundle.model),
                    checked_url: Some(url),
                    status_code: Some(code),
                }
            } else {
                BundleProbeReport {
                    ok: false,
                    detail: format!(
                        "chat/completions HTTP {code}: {}",
                        text.chars().take(220).collect::<String>()
                    ),
                    checked_url: Some(url),
                    status_code: Some(code),
                }
            }
        }
        Err(err) => BundleProbeReport {
            ok: false,
            detail: format!("chat/completions request failed: {err}"),
            checked_url: Some(url),
            status_code: None,
        },
    }
}

fn probe_anthropic_bundle(bundle: &EndpointBundle) -> BundleProbeReport {
    let base = bundle
        .anthropic_gateway_url
        .as_deref()
        .unwrap_or(&bundle.gateway_url)
        .trim_end_matches('/');
    let url = format!("{base}/v1/models");
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(err) => {
            return BundleProbeReport {
                ok: false,
                detail: format!("http client: {err}"),
                checked_url: None,
                status_code: None,
            };
        }
    };
    match client
        .get(&url)
        .header("x-api-key", &bundle.api_key)
        .header("anthropic-version", "2023-06-01")
        .send()
    {
        Ok(resp) => {
            let code = resp.status().as_u16();
            let ok = resp.status().is_success();
            let text = resp.text().unwrap_or_default();
            BundleProbeReport {
                ok,
                detail: if ok {
                    format!("anthropic models HTTP {code}")
                } else {
                    format!(
                        "anthropic models HTTP {code}: {}",
                        text.chars().take(180).collect::<String>()
                    )
                },
                checked_url: Some(url),
                status_code: Some(code),
            }
        }
        Err(err) => BundleProbeReport {
            ok: false,
            detail: format!("anthropic probe failed: {err}"),
            checked_url: Some(url),
            status_code: None,
        },
    }
}

fn resolve_personal_bundle(provider_id: Option<&str>) -> Result<EndpointBundle> {
    let doc = list_personal_providers()?;
    if doc.providers.is_empty() {
        bail!("no personal providers saved — add one under Personal first");
    }
    let id = provider_id
        .map(str::to_string)
        .or(doc.active_id.clone())
        .or_else(|| doc.providers.first().map(|p| p.id.clone()))
        .context("no personal provider id")?;
    let entry = load_personal_provider_entry(&id)?;

    let gateway_url = normalize_personal_gateway_url(&entry.url)?;
    let model = entry.model.trim();
    if model.is_empty() {
        bail!("personal provider model must not be empty");
    }
    if model.eq_ignore_ascii_case("default") {
        bail!("personal provider model must not be bare \"default\"");
    }
    let protocol = normalize_protocol(&entry.protocol);
    let api_key = entry.api_key.trim();
    if api_key.is_empty() {
        bail!("personal provider API key must not be empty");
    }

    set_active_personal_provider_id(&entry.id)?;

    Ok(EndpointBundle {
        mode: MODE_PERSONAL.to_string(),
        label: entry.name.clone(),
        gateway_url,
        api_key: api_key.to_string(),
        model: model.to_string(),
        protocol,
        source_id: format!("personal:{}", entry.id),
        hermes_provider: "custom".to_string(),
        anthropic_gateway_url: None,
        personal_provider_id: Some(entry.id.clone()),
        personal_provider_name: Some(entry.name.clone()),
    })
}

fn resolve_team_bundle() -> Result<EndpointBundle> {
    let team = resolve_team_credentials()?;
    let gateway_url = normalize_gateway_url(&gateway_url_from_evotown_base(&team.base_url))?;
    let evotown_base = evotown_base_from_gateway(&gateway_url);
    let model = COMPANY_DEFAULT_MODEL.to_string();

    Ok(EndpointBundle {
        mode: MODE_TEAM.to_string(),
        label: "Evotown".to_string(),
        gateway_url: gateway_url.clone(),
        api_key: team.api_key,
        model,
        protocol: PROTOCOL_OPENAI.to_string(),
        source_id: "team:evotown".to_string(),
        hermes_provider: "openai".to_string(),
        anthropic_gateway_url: Some(anthropic_gateway_url_from_evotown_base(&evotown_base)),
        personal_provider_id: None,
        personal_provider_name: None,
    })
}

fn write_overlay_for_bundle(bundle: &EndpointBundle) -> Result<()> {
    let profile_path = agent_profile_path().context("could not resolve config directory")?;
    match bundle.mode.as_str() {
        MODE_PERSONAL => {
            write_personal_profile(
                &profile_path,
                &bundle.gateway_url,
                &bundle.api_key,
                &bundle.model,
                &bundle.protocol,
                bundle.personal_provider_id.as_deref(),
                bundle.personal_provider_name.as_deref(),
            )?;
        }
        MODE_TEAM => {
            let evotown_base = evotown_base_from_gateway(&bundle.gateway_url);
            write_company_profile_with_gateway(
                &profile_path,
                &bundle.gateway_url,
                &bundle.api_key,
                &evotown_base,
            )?;
            let _ = write_evotown_agent_env(&evotown_base, &bundle.api_key, DEFAULT_EVOTOWN_RUNTIME);
        }
        other => bail!("unsupported mode for overlay write: {other}"),
    }
    Ok(())
}

struct TeamCredentials {
    base_url: String,
    api_key: String,
}

fn resolve_team_credentials() -> Result<TeamCredentials> {
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
    fn strategy_table_covers_core_runtimes() {
        for id in ["openclaw", "hermes", "codex", "claude-code"] {
            assert!(strategy_for(id).is_some(), "missing strategy for {id}");
        }
        let oc = strategy_for("openclaw").unwrap();
        assert_eq!(oc.effector, EffectorKind::RestartGateway);
        assert_eq!(oc.write_semantics, WriteSemantics::Additive);
        assert_eq!(
            strategy_for("hermes").unwrap().write_semantics,
            WriteSemantics::Additive
        );
        assert_eq!(
            strategy_for("codex").unwrap().write_semantics,
            WriteSemantics::Additive
        );
        let cd = strategy_for("claude-code").unwrap();
        assert!(cd.anthropic_compatible);
        assert!(!cd.openai_compatible);
    }

    #[test]
    fn company_default_model_is_not_bare_default() {
        assert_ne!(COMPANY_DEFAULT_MODEL.to_ascii_lowercase(), "default");
        assert!(!COMPANY_DEFAULT_MODEL.is_empty());
    }

    #[test]
    fn effector_labels_are_stable() {
        assert_eq!(effector_label(EffectorKind::None), "none");
        assert_eq!(
            effector_label(EffectorKind::RestartGateway),
            "restart_gateway"
        );
        assert_eq!(
            effector_label(EffectorKind::ManualRestart),
            "manual_restart"
        );
    }

}
