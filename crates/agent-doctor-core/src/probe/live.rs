//! Live probe: one minimal real call against an agent's configured gateway, so
//! a "待验证" credential has a path to confirmed-healthy / confirmed-fault.
//! Static checks never make network calls; this runs only on explicit request.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::profile::COMPANY_API_KEY_ENV;
use crate::runtime::adapter_by_id;
use crate::session_env::collect_overlay_env;
use crate::setup::{verify_personal_provider_with_protocol, PROTOCOL_ANTHROPIC};

/// How the minimal real call ended. `model_ok` is the only healthy outcome;
/// everything else is evidence for a confirmed fault or a still-unverified gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveProbeOutcome {
    /// The gateway answered a real model request.
    ModelOk,
    /// 401/403 — the credential was rejected.
    AuthFailed,
    /// The endpoint or model was not found.
    NotFound,
    /// Network-level failure (DNS/TCP/TLS/timeout).
    Unreachable,
    /// The runtime has no gateway configured (subscription login, probably).
    NoGateway,
    /// No usable key found in the app wiring or the process environment.
    NoKey,
    /// Something else went wrong; see `detail`.
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeLiveProbeReport {
    pub runtime_id: String,
    pub ok: bool,
    pub outcome: LiveProbeOutcome,
    /// Human-readable evidence; never contains the key itself.
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_url: Option<String>,
}

/// Env vars that may carry a usable key for the runtime, most specific first.
fn key_env_candidates(runtime_id: &str) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = match runtime_id {
        "hermes" => vec!["CUSTOM_API_KEY"],
        "deepseek-harness" => vec!["DEEPSEEK_API_KEY"],
        "claude-code" => vec!["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"],
        "codex" => vec!["OPENAI_API_KEY"],
        "openclaw" => vec!["OPENCLAW_API_KEY"],
        _ => Vec::new(),
    };
    names.extend(["OPENAI_API_KEY", "ANTHROPIC_API_KEY", COMPANY_API_KEY_ENV]);
    names
}

/// First non-empty value from the app wiring, then the process environment.
fn resolve_probe_api_key(runtime_id: &str) -> Option<String> {
    let overlay = collect_overlay_env();
    for name in key_env_candidates(runtime_id) {
        if let Some(value) = overlay.get(name).filter(|v| !v.trim().is_empty()) {
            return Some(value.clone());
        }
    }
    for name in key_env_candidates(runtime_id) {
        if let Ok(value) = std::env::var(name) {
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// Anthropic-protocol gateways answer `/v1/messages`; everything else is
/// verified through the OpenAI-compatible path.
fn guess_protocol(gateway_url: &str) -> &'static str {
    if gateway_url.contains("anthropic") || gateway_url.contains("/claude") {
        PROTOCOL_ANTHROPIC
    } else {
        "openai"
    }
}

/// Classify a verify failure into the evidence outcome.
fn outcome_from_message(message: &str) -> LiveProbeOutcome {
    if message.contains("401") || message.contains("403") {
        LiveProbeOutcome::AuthFailed
    } else if message.contains("404") || message.contains("model") {
        LiveProbeOutcome::NotFound
    } else if message.contains("timed out")
        || message.contains("unreachable")
        || message.contains("request failed")
        || message.contains("DNS")
    {
        LiveProbeOutcome::Unreachable
    } else {
        LiveProbeOutcome::Error
    }
}

pub fn probe_runtime_live(runtime_id: &str) -> Result<RuntimeLiveProbeReport> {
    let adapter =
        adapter_by_id(runtime_id).with_context(|| format!("unknown runtime: {runtime_id}"))?;
    let profile = adapter
        .read_profile()
        .context("failed to read gateway profile")?;
    let gateway = profile
        .gateway_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_string);
    let Some(gateway) = gateway else {
        return Ok(RuntimeLiveProbeReport {
            runtime_id: runtime_id.to_string(),
            ok: false,
            outcome: LiveProbeOutcome::NoGateway,
            detail: "no gateway/base_url configured — this agent likely uses a subscription login, so a live call does not apply".into(),
            resolved_url: None,
        });
    };

    let Some(api_key) = resolve_probe_api_key(runtime_id) else {
        return Ok(RuntimeLiveProbeReport {
            runtime_id: runtime_id.to_string(),
            ok: false,
            outcome: LiveProbeOutcome::NoKey,
            detail: "no usable key found in the app wiring or the process environment".into(),
            resolved_url: None,
        });
    };

    let protocol = guess_protocol(&gateway);
    let report = match verify_personal_provider_with_protocol(&gateway, &api_key, protocol) {
        Ok(report) => report,
        Err(error) => {
            return Ok(RuntimeLiveProbeReport {
                runtime_id: runtime_id.to_string(),
                ok: false,
                outcome: LiveProbeOutcome::Error,
                detail: format!("{error:#}"),
                resolved_url: None,
            })
        }
    };

    let outcome = if report.ok {
        LiveProbeOutcome::ModelOk
    } else {
        outcome_from_message(&report.message)
    };
    Ok(RuntimeLiveProbeReport {
        runtime_id: runtime_id.to_string(),
        ok: report.ok,
        outcome,
        detail: report.message,
        resolved_url: report.resolved_url.or(report.checked_url),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_gateways_use_the_anthropic_protocol() {
        assert_eq!(
            guess_protocol("https://open.bigmodel.cn/api/anthropic"),
            "anthropic"
        );
        assert_eq!(guess_protocol("https://api.minimax.io/v1"), "openai");
    }

    #[test]
    fn verify_failures_classify_into_evidence_outcomes() {
        assert_eq!(
            outcome_from_message("HTTP 401 at /v1/models"),
            LiveProbeOutcome::AuthFailed
        );
        assert_eq!(outcome_from_message("HTTP 404"), LiveProbeOutcome::NotFound);
        assert_eq!(
            outcome_from_message("request failed for https://x: timeout"),
            LiveProbeOutcome::Unreachable
        );
        assert_eq!(
            outcome_from_message("plan mismatch"),
            LiveProbeOutcome::Error
        );
    }

    #[test]
    fn key_candidates_prefer_the_runtime_specific_var() {
        let names = key_env_candidates("hermes");
        assert_eq!(names[0], "CUSTOM_API_KEY");
        assert!(names.contains(&"OPENAI_API_KEY"));
        let names = key_env_candidates("unknown-runtime");
        assert_eq!(names[0], "OPENAI_API_KEY");
    }
}
