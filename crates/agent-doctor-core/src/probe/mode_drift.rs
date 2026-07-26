//! Cross-cutting mode drift probes attached to each runtime probe report.
//!
//! - `mode.overlay_mismatch` — profile mode URL/kind vs runtime live URL
//! - `runtime.env_stale` — OpenClaw file key vs LaunchAgent/process key
//! - `runtime.model_unroutable` — bare `default` / empty model under team mode

use std::fs;
use std::process::Command;

use crate::adapter::RuntimeAdapter;
use crate::adapters::util::home_join;
use crate::adapters::{configured_base_url, CodexAdapter, HermesAdapter};
use crate::profile::{read_agent_profile, ProviderKind};
use crate::repair::{DiagnosticFact, SensitivityLevel};
use crate::setup::{COMPANY_DEFAULT_MODEL, MODE_PERSONAL, MODE_TEAM};

use super::{ProbeCheck, ProbeSeverity, ProbeStatus};

pub(crate) fn probe_mode_drift(
    runtime_id: &str,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let profile = match read_agent_profile() {
        Ok(Some(p)) => p,
        _ => return,
    };
    let mode = match profile.kind {
        ProviderKind::Personal => MODE_PERSONAL,
        ProviderKind::Company => MODE_TEAM,
        ProviderKind::Unknown => return,
    };
    facts.push(DiagnosticFact::new(
        "mode.active",
        mode,
        SensitivityLevel::Public,
    ));

    let expected_url = profile.gateway_url.as_deref().unwrap_or("").trim();
    if !expected_url.is_empty() {
        facts.push(DiagnosticFact::new(
            "mode.overlay_gateway",
            expected_url,
            SensitivityLevel::ConfigShape,
        ));
    }

    if let Some(live_url) = live_gateway_url(runtime_id) {
        facts.push(DiagnosticFact::new(
            "mode.live_gateway",
            &live_url,
            SensitivityLevel::ConfigShape,
        ));
        if !expected_url.is_empty() {
            let mismatch = urls_conflict(mode, expected_url, &live_url);
            checks.push(ProbeCheck::new(
                "mode.overlay_mismatch",
                "Mode overlay vs live gateway",
                if mismatch {
                    ProbeStatus::Warn
                } else {
                    ProbeStatus::Pass
                },
                if mismatch {
                    ProbeSeverity::Warning
                } else {
                    ProbeSeverity::Info
                },
                if mismatch {
                    format!(
                        "profile mode={mode} expects {expected_url}, but {runtime_id} live is {live_url}"
                    )
                } else {
                    format!("{runtime_id} live gateway matches active mode overlay")
                },
                SensitivityLevel::ConfigShape,
            ));
        }
    }

    if runtime_id == "openclaw" {
        probe_openclaw_env_stale(profile.api_key.as_deref(), checks, facts);
        probe_openclaw_model_unroutable(mode, checks, facts);
    } else if runtime_id == "codex" {
        probe_codex_model_unroutable(mode, checks, facts);
    } else if runtime_id == "hermes" {
        probe_hermes_model_unroutable(mode, checks, facts);
    }
}

fn live_gateway_url(runtime_id: &str) -> Option<String> {
    match runtime_id {
        "openclaw" => {
            let path = home_join(".openclaw/openclaw.json");
            let raw = fs::read_to_string(path).ok()?;
            let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
            configured_base_url(&value)
        }
        "hermes" => HermesAdapter
            .read_profile()
            .ok()
            .and_then(|p| p.gateway_url),
        "codex" => CodexAdapter.read_profile().ok().and_then(|p| p.gateway_url),
        "claude-code" => {
            let path = home_join(".claude/settings.json");
            let raw = fs::read_to_string(path).ok()?;
            let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
            value
                .pointer("/env/ANTHROPIC_BASE_URL")
                .or_else(|| value.get("anthropicBaseUrl"))
                .and_then(|v| v.as_str())
                .filter(|u| !u.trim().is_empty())
                .map(str::to_string)
        }
        _ => None,
    }
}

fn urls_conflict(mode: &str, expected: &str, live: &str) -> bool {
    let exp = normalize_url(expected);
    let liv = normalize_url(live);
    if exp == liv {
        return false;
    }
    // Team overlay should not point live at a personal vendor host, and vice versa.
    let live_is_evotown = looks_like_evotown(&liv);
    match mode {
        MODE_TEAM => !live_is_evotown,
        MODE_PERSONAL => live_is_evotown,
        _ => exp != liv,
    }
}

fn looks_like_evotown(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("/api/gateway/") || u.contains("skilllite.ai") || u.contains("evotown")
}

fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn probe_openclaw_env_stale(
    profile_key: Option<&str>,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let dotenv_key = read_dotenv_openai_key(&home_join(".openclaw/.env"));
    let service_key =
        read_service_env_openai_key(&home_join(".openclaw/service-env/ai.openclaw.gateway.env"));
    let process_key = read_openclaw_gateway_process_key();

    if let Some(ref k) = dotenv_key {
        facts.push(DiagnosticFact::new(
            "openclaw.env.key_hint",
            mask_tail(k),
            SensitivityLevel::Secret,
        ));
    }
    if let Some(ref k) = service_key {
        facts.push(DiagnosticFact::new(
            "openclaw.service_env.key_hint",
            mask_tail(k),
            SensitivityLevel::Secret,
        ));
    }
    if let Some(ref k) = process_key {
        facts.push(DiagnosticFact::new(
            "openclaw.process.key_hint",
            mask_tail(k),
            SensitivityLevel::Secret,
        ));
    }

    let mut stale_reasons = Vec::new();
    if let (Some(a), Some(b)) = (dotenv_key.as_ref(), service_key.as_ref()) {
        if a != b {
            stale_reasons.push(".env and service-env OPENAI_API_KEY differ".to_string());
        }
    }
    if let (Some(a), Some(b)) = (dotenv_key.as_ref(), process_key.as_ref()) {
        if a != b {
            stale_reasons.push(
                "gateway process OPENAI_API_KEY differs from ~/.openclaw/.env (restart needed)"
                    .to_string(),
            );
        }
    }
    if let (Some(profile), Some(dotenv)) = (
        profile_key.map(str::trim).filter(|k| !k.is_empty()),
        dotenv_key.as_ref(),
    ) {
        if profile != dotenv.as_str() {
            stale_reasons.push("profile.env key differs from OpenClaw .env".to_string());
        }
    }

    let stale = !stale_reasons.is_empty();
    checks.push(
        ProbeCheck::new(
            "runtime.env_stale",
            "OpenClaw API key freshness",
            if stale {
                ProbeStatus::Warn
            } else if dotenv_key.is_some() || service_key.is_some() {
                ProbeStatus::Pass
            } else {
                ProbeStatus::NotChecked
            },
            if stale {
                ProbeSeverity::Warning
            } else {
                ProbeSeverity::Info
            },
            if stale {
                "OpenClaw key sources are out of sync — run mode switch or `openclaw gateway restart`"
                    .to_string()
            } else if dotenv_key.is_some() {
                "OpenClaw key files/process look consistent".to_string()
            } else {
                "No OpenClaw OPENAI_API_KEY found to compare".to_string()
            },
            SensitivityLevel::Secret,
        )
        .with_details(stale_reasons),
    );
}

fn probe_openclaw_model_unroutable(
    mode: &str,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let path = home_join(".openclaw/openclaw.json");
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    let primary = value
        .pointer("/agents/defaults/model/primary")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    facts.push(DiagnosticFact::new(
        "openclaw.model.primary",
        primary,
        SensitivityLevel::ConfigShape,
    ));
    let model_id = primary
        .split_once('/')
        .map(|(_, m)| m)
        .unwrap_or(primary)
        .trim();
    push_model_unroutable_check(mode, model_id, "openclaw", primary, checks);
}

fn probe_codex_model_unroutable(
    mode: &str,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let path = home_join(".codex/config.toml");
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    let Ok(value) = toml::from_str::<toml::Value>(&raw) else {
        return;
    };
    let model = value
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let provider = value
        .get("model_provider")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    facts.push(DiagnosticFact::new(
        "codex.model",
        format!("{provider}/{model}"),
        SensitivityLevel::ConfigShape,
    ));
    push_model_unroutable_check(mode, model, "codex", model, checks);
}

fn probe_hermes_model_unroutable(
    mode: &str,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let path = home_join(".hermes/config.yaml");
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(&raw) else {
        return;
    };
    let model = value
        .get("model")
        .and_then(|m| m.get("default"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    facts.push(DiagnosticFact::new(
        "hermes.model.default",
        model,
        SensitivityLevel::ConfigShape,
    ));
    push_model_unroutable_check(mode, model, "hermes", model, checks);
}

fn push_model_unroutable_check(
    mode: &str,
    model_id: &str,
    runtime: &str,
    display: &str,
    checks: &mut Vec<ProbeCheck>,
) {
    let bare_default = model_id.is_empty() || model_id.eq_ignore_ascii_case("default");
    let team_risk = mode == MODE_TEAM && bare_default;
    checks.push(ProbeCheck::new(
        "runtime.model_unroutable",
        "Routable model id",
        if team_risk {
            ProbeStatus::Fail
        } else if bare_default {
            ProbeStatus::Warn
        } else {
            ProbeStatus::Pass
        },
        if team_risk {
            ProbeSeverity::Error
        } else if bare_default {
            ProbeSeverity::Warning
        } else {
            ProbeSeverity::Info
        },
        if team_risk {
            format!(
                "{runtime} model `{display}` is bare/empty under team mode — use e.g. {COMPANY_DEFAULT_MODEL}"
            )
        } else if bare_default {
            format!("{runtime} model `{display}` looks unroutable")
        } else {
            format!("{runtime} model `{display}` looks set")
        },
        SensitivityLevel::ConfigShape,
    ));
}

fn read_dotenv_openai_key(path: &std::path::Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("OPENAI_API_KEY=") {
            let v = value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn read_service_env_openai_key(path: &std::path::Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed
            .strip_prefix("export OPENAI_API_KEY=")
            .or_else(|| trimmed.strip_prefix("OPENAI_API_KEY="))
        {
            let v = rest.trim().trim_matches('\'').trim_matches('"').to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn read_openclaw_gateway_process_key() -> Option<String> {
    let output = Command::new("pgrep")
        .args(["-f", "openclaw/dist/index.js gateway"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let pid = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    if pid.is_empty() {
        return None;
    }
    let ps = Command::new("ps").args(["eww", "-p", &pid]).output().ok()?;
    if !ps.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&ps.stdout);
    for part in text.split(|c: char| c.is_whitespace()) {
        if let Some(value) = part.strip_prefix("OPENAI_API_KEY=") {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn mask_tail(key: &str) -> String {
    if key.len() <= 8 {
        "****".into()
    } else {
        format!("…{}", &key[key.len().saturating_sub(4)..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_live_on_deepseek_is_mismatch() {
        assert!(urls_conflict(
            MODE_TEAM,
            "https://www.skilllite.ai/api/gateway/v1",
            "https://api.deepseek.com/v1"
        ));
    }

    #[test]
    fn personal_live_on_evotown_is_mismatch() {
        assert!(urls_conflict(
            MODE_PERSONAL,
            "https://api.deepseek.com/v1",
            "https://www.skilllite.ai/api/gateway/v1"
        ));
    }

    #[test]
    fn matching_urls_ok() {
        assert!(!urls_conflict(
            MODE_TEAM,
            "https://www.skilllite.ai/api/gateway/v1",
            "https://www.skilllite.ai/api/gateway/v1/"
        ));
    }

    #[test]
    fn bare_default_is_team_risk() {
        assert!(COMPANY_DEFAULT_MODEL.to_ascii_lowercase() != "default");
    }
}
