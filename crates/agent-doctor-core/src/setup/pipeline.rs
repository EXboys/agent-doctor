//! Unified mode-switch pipeline: resolve → project → effector metadata.
//!
//! The live chat probe stays available as [`probe_endpoint_bundle`], but a switch
//! does not wait on it. That wait used to freeze the desktop provider click.
//!
//! All personal/team LLM wiring should enter through [`apply_mode_switch`].

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::profile::{
    agent_profile_path, company_baseline_path, read_company_baseline, read_env_map,
};
use crate::runtime::all_adapters;
use crate::setup::coding_plan::plan_endpoints;
use crate::setup::merge::{self, clear_codex_placeholder_auth, COMPANY_DEFAULT_MODEL};
use crate::setup::personal::{
    align_coding_plan_url, list_personal_providers, load_personal_provider_entry,
    normalize_personal_gateway_url, normalize_protocol, persist_personal_provider_model,
    persist_personal_provider_url, set_active_personal_provider_id, write_personal_profile,
    PersonalProviderSetupReport, PROTOCOL_ANTHROPIC, PROTOCOL_OPENAI,
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

pub fn runtime_strategies() -> Vec<RuntimeStrategy> {
    crate::runtime::all_runtime_ids()
        .filter_map(strategy_for)
        .collect()
}

pub fn strategy_for(runtime_id: &str) -> Option<RuntimeStrategy> {
    let entry = crate::runtime::descriptor_by_id(runtime_id)?;
    let wiring = entry.wiring()?;
    Some(RuntimeStrategy {
        runtime_id: entry.id,
        write_semantics: wiring.write_semantics,
        effector: wiring.effector,
        openai_compatible: wiring.openai_compatible,
        anthropic_compatible: wiring.anthropic_compatible,
    })
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
    match &target {
        ModeSwitchTarget::Personal { .. } => {
            crate::edition::ensure_edition_allows_mode(MODE_PERSONAL)?;
        }
        ModeSwitchTarget::Team => {
            crate::edition::ensure_edition_allows_mode(MODE_TEAM)?;
        }
    }

    let bundle = match &target {
        ModeSwitchTarget::Personal { provider_id } => {
            resolve_personal_bundle(provider_id.as_deref())?
        }
        ModeSwitchTarget::Team => resolve_team_bundle()?,
    };

    // Skip the live model check. Pre-verify can take ~20s, and the chat probe
    // waits up to 8s. The desktop "使用" click cannot finish until this returns.
    // Adding a provider still has its own check.
    write_overlay_for_bundle(&bundle)?;
    let _ = clear_codex_placeholder_auth();

    let runtimes = project_bundle(&bundle)?;

    let applied = runtimes.iter().filter(|r| r.applied).count();
    let mut warnings = Vec::new();
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
            "personal mode — {applied} runtime(s) wired to {} (model {})",
            bundle.label, bundle.model,
        ),
        MODE_TEAM => format!(
            "team mode — {applied} runtime(s) wired to Evotown (model {})",
            bundle.model,
        ),
        other => format!("mode {other} — {applied} runtime(s)"),
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
            verify: None,
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
        probe_ok: None,
        probe_detail: None,
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
        (_, "claude-code") => project_claude_code(bundle, display_name),
        (PROTOCOL_ANTHROPIC, other) => Ok(RuntimeSetupResult {
            runtime_id: other.into(),
            display_name: display_name.into(),
            applied: false,
            message: format!("skipped — Anthropic protocol; {other} needs OpenAI-compatible API"),
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

/// Claude Code needs an Anthropic Messages endpoint.
/// Team Evotown still uses OpenAI protocol for Codex/Hermes, but also exposes
/// `/api/gateway/anthropic` — do not skip Claude just because protocol is openai.
fn claude_code_target_url(bundle: &EndpointBundle) -> Option<&str> {
    if let Some(url) = bundle
        .anthropic_gateway_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        return Some(url);
    }
    if bundle.protocol == PROTOCOL_ANTHROPIC {
        let url = bundle.gateway_url.trim();
        if !url.is_empty() {
            return Some(url);
        }
    }
    None
}

fn project_claude_code(bundle: &EndpointBundle, display_name: &str) -> Result<RuntimeSetupResult> {
    let Some(url) = claude_code_target_url(bundle) else {
        return Ok(RuntimeSetupResult {
            runtime_id: "claude-code".into(),
            display_name: display_name.into(),
            applied: false,
            message: "skipped — no Anthropic-compatible gateway for Claude Code".into(),
            effector: Some(effector_label(EffectorKind::None).into()),
            ..Default::default()
        });
    };
    merge::apply_claude_code_with_model(url, &bundle.api_key, Some(&bundle.model))
}

pub fn probe_endpoint_bundle(bundle: &EndpointBundle) -> BundleProbeReport {
    if bundle.protocol == PROTOCOL_ANTHROPIC {
        return probe_anthropic_bundle(bundle);
    }
    probe_openai_chat_bundle(bundle)
}

fn probe_openai_chat_bundle(bundle: &EndpointBundle) -> BundleProbeReport {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
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
        .timeout(Duration::from_secs(8))
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

    let mut gateway_url = normalize_personal_gateway_url(&entry.url)?;
    let stored_model = entry.model.trim();
    if stored_model.is_empty() {
        bail!("personal provider model must not be empty");
    }
    let model = if let Some(fixed) = coerce_gateway_model(&gateway_url, stored_model) {
        let _ = persist_personal_provider_model(&entry.id, &fixed);
        fixed
    } else {
        stored_model.to_string()
    };
    let model = model.as_str();
    if model.eq_ignore_ascii_case("default") {
        bail!("personal provider model must not be bare \"default\"");
    }
    let protocol = normalize_protocol(&entry.protocol);
    let api_key = entry.api_key.trim();
    if api_key.is_empty() {
        bail!("personal provider API key must not be empty");
    }

    set_active_personal_provider_id(&entry.id)?;

    // Coding Plan keys (Pro / Lite / Max) fail with "no balance" on the pay-as-you-go
    // address. One models request picks the line this key actually belongs to.
    if let Some(resolved) = align_coding_plan_url(&gateway_url, api_key) {
        let resolved = normalize_personal_gateway_url(&resolved).unwrap_or(resolved);
        if resolved.trim_end_matches('/') != gateway_url.trim_end_matches('/') {
            let _ = persist_personal_provider_url(&entry.id, &resolved);
            gateway_url = resolved;
        }
    }

    let mut anthropic_gateway_url = if protocol == PROTOCOL_ANTHROPIC {
        Some(gateway_url.clone())
    } else {
        None
    };
    let mut protocol = protocol;

    // Known dual-protocol hosts (same key): auto-route OpenAI agents + Claude Code.
    if let Some(dual) = dual_protocol_endpoints(&gateway_url) {
        gateway_url = dual.openai_url;
        anthropic_gateway_url = Some(dual.anthropic_url);
        // Primary path stays OpenAI so Codex / Hermes / OpenClaw all write;
        // Claude Code reads anthropic_gateway_url.
        protocol = PROTOCOL_OPENAI.to_string();
    }

    Ok(EndpointBundle {
        mode: MODE_PERSONAL.to_string(),
        label: entry.name.clone(),
        gateway_url,
        api_key: api_key.to_string(),
        model: model.to_string(),
        protocol,
        source_id: format!("personal:{}", entry.id),
        hermes_provider: "custom".to_string(),
        anthropic_gateway_url,
        personal_provider_id: Some(entry.id.clone()),
        personal_provider_name: Some(entry.name.clone()),
    })
}

/// Providers that expose both OpenAI-compatible and Anthropic Messages APIs with one key.
struct DualProtocolEndpoints {
    openai_url: String,
    anthropic_url: String,
}

/// TeamUps 官方：OpenAI 在 `/api/v1/official`，Claude Code 在旁边的 Anthropic 入口。
/// Claude 自己会在这个地址后面加 `/v1/messages`。
fn teamups_official_endpoints(url: &str) -> Option<DualProtocolEndpoints> {
    let trimmed = url.trim().trim_end_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    let marker = "/api/v1/official";
    let idx = lower.find(marker)?;
    let origin = trimmed[..idx].trim_end_matches('/');
    if origin.is_empty() || !origin.contains("://") {
        return None;
    }
    Some(DualProtocolEndpoints {
        openai_url: format!("{origin}/api/v1/official"),
        anthropic_url: format!("{origin}/api/v1/official/anthropic"),
    })
}

/// Saved model id from another vendor (DeepSeek id on a GLM address, and the reverse).
/// Custom ids are left alone. `Some` is the model this gateway should actually be called with.
pub fn coerce_gateway_model(url: &str, model: &str) -> Option<String> {
    let host = url.to_ascii_lowercase();
    let model_l = model.trim().to_ascii_lowercase();
    if model_l.is_empty() {
        return None;
    }
    let (fits, default) = if host.contains("open.bigmodel.cn") || host.contains("api.z.ai") {
        (model_l.starts_with("glm"), "glm-5.3")
    } else if host.contains("api.deepseek.com") {
        (model_l.starts_with("deepseek"), "deepseek-v4-flash")
    } else if host.contains("dashscope") {
        (model_l.starts_with("qwen"), "qwen3.7-plus")
    } else if host.contains("api.kimi.") || host.contains("moonshot.") {
        (
            model_l.starts_with("kimi") || model_l.starts_with("moonshot"),
            "kimi-k2.6",
        )
    } else if host.contains("minimax") {
        (model_l.starts_with("minimax"), "MiniMax-M3")
    } else {
        return None;
    };
    if fits || !model_is_foreign_vendor(&model_l) {
        None
    } else {
        Some(default.to_string())
    }
}

fn model_is_foreign_vendor(model_l: &str) -> bool {
    model_l.starts_with("glm")
        || model_l.starts_with("deepseek")
        || model_l.starts_with("qwen")
        || model_l.starts_with("kimi")
        || model_l.starts_with("moonshot")
        || model_l.starts_with("minimax")
        || model_l.starts_with("doubao")
        || model_l.starts_with("claude")
        || model_l.starts_with("gpt-")
}

/// The OpenAI-compatible address the runtimes are wired to for this saved address.
pub(crate) fn openai_gateway_for_provider_url(url: &str) -> (String, bool) {
    match dual_protocol_endpoints(url) {
        Some(dual) => (dual.openai_url, true),
        None => (url.trim().trim_end_matches('/').to_string(), false),
    }
}

/// Providers that expose both OpenAI-compatible and Anthropic Messages APIs with one key.
pub fn anthropic_gateway_for_provider_url(url: &str) -> Option<String> {
    if let Some(dual) = dual_protocol_endpoints(url) {
        return Some(dual.anthropic_url);
    }
    let trimmed = url.trim().trim_end_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("/anthropic") || lower.contains("api.anthropic.com") {
        return Some(trimmed.to_string());
    }
    None
}

fn dual_protocol_endpoints(url: &str) -> Option<DualProtocolEndpoints> {
    if let Some(official) = teamups_official_endpoints(url) {
        return Some(official);
    }
    let lower = url.trim().to_ascii_lowercase();
    // DeepSeek: /v1 (OpenAI) + /anthropic (Claude Code)
    if lower.contains("api.deepseek.com") {
        return Some(DualProtocolEndpoints {
            openai_url: "https://api.deepseek.com/v1".into(),
            anthropic_url: "https://api.deepseek.com/anthropic".into(),
        });
    }
    // MiniMax: China (minimax.cn) and international (minimaxi.com / minimax.io).
    // Subscription keys and pay-as-you-go keys share the path; the host must stay.
    if lower.contains("api.minimax.cn") {
        return Some(DualProtocolEndpoints {
            openai_url: "https://api.minimax.cn/v1".into(),
            anthropic_url: "https://api.minimax.cn/anthropic".into(),
        });
    }
    if lower.contains("api.minimaxi.com") {
        return Some(DualProtocolEndpoints {
            openai_url: "https://api.minimaxi.com/v1".into(),
            anthropic_url: "https://api.minimaxi.com/anthropic".into(),
        });
    }
    if lower.contains("api.minimax.io") {
        return Some(DualProtocolEndpoints {
            openai_url: "https://api.minimax.io/v1".into(),
            anthropic_url: "https://api.minimax.io/anthropic".into(),
        });
    }
    // GLM, Qwen, and Kimi each sell a coding subscription on its own address.
    if let Some(plan) = plan_endpoints(&lower) {
        return Some(DualProtocolEndpoints {
            openai_url: plan.openai_url,
            anthropic_url: plan.anthropic_url,
        });
    }
    // SiliconFlow: OpenAI /v1 + Anthropic Messages on same host root
    if lower.contains("api.siliconflow.cn") {
        return Some(DualProtocolEndpoints {
            openai_url: "https://api.siliconflow.cn/v1".into(),
            anthropic_url: "https://api.siliconflow.cn".into(),
        });
    }
    if lower.contains("api.siliconflow.com") {
        return Some(DualProtocolEndpoints {
            openai_url: "https://api.siliconflow.com/v1".into(),
            anthropic_url: "https://api.siliconflow.com".into(),
        });
    }
    // OpenRouter: /api/v1 (OpenAI) + /api (Claude Code → /api/v1/messages)
    if lower.contains("openrouter.ai") {
        return Some(DualProtocolEndpoints {
            openai_url: "https://openrouter.ai/api/v1".into(),
            anthropic_url: "https://openrouter.ai/api".into(),
        });
    }
    None
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
            let _ =
                write_evotown_agent_env(&evotown_base, &bundle.api_key, DEFAULT_EVOTOWN_RUNTIME);
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
    fn strategy_table_covers_wired_runtimes_only() {
        use crate::runtime::all_runtime_ids;

        for id in all_runtime_ids() {
            let wired = matches!(id, "openclaw" | "hermes" | "codex" | "claude-code");
            assert_eq!(strategy_for(id).is_some(), wired, "{id}");
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
        assert!(strategy_for("deepseek-harness").is_none());
        assert!(strategy_for("qoder").is_none());
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

    fn sample_bundle(protocol: &str, anthropic_gateway_url: Option<&str>) -> EndpointBundle {
        EndpointBundle {
            mode: MODE_TEAM.to_string(),
            label: "test".into(),
            gateway_url: "https://www.skilllite.ai/api/gateway/v1".into(),
            api_key: "sk-test".into(),
            model: "gpt-4o".into(),
            protocol: protocol.into(),
            source_id: "team:evotown".into(),
            hermes_provider: "openai".into(),
            anthropic_gateway_url: anthropic_gateway_url.map(str::to_string),
            personal_provider_id: None,
            personal_provider_name: None,
        }
    }

    #[test]
    fn team_openai_still_targets_evotown_anthropic_for_claude() {
        let bundle = sample_bundle(
            PROTOCOL_OPENAI,
            Some("https://www.skilllite.ai/api/gateway/anthropic"),
        );
        assert_eq!(
            claude_code_target_url(&bundle),
            Some("https://www.skilllite.ai/api/gateway/anthropic")
        );
    }

    #[test]
    fn official_openai_also_targets_claude() {
        let dual =
            dual_protocol_endpoints("https://teamups.vip/api/v1/official").expect("official");
        assert_eq!(dual.openai_url, "https://teamups.vip/api/v1/official");
        assert_eq!(
            dual.anthropic_url,
            "https://teamups.vip/api/v1/official/anthropic"
        );
        let dual_anthropic =
            dual_protocol_endpoints("https://teamups.vip/api/v1/official/anthropic")
                .expect("official anthropic");
        assert_eq!(dual_anthropic.openai_url, dual.openai_url);
        assert_eq!(dual_anthropic.anthropic_url, dual.anthropic_url);

        let mut bundle = sample_bundle(PROTOCOL_OPENAI, None);
        bundle.mode = MODE_PERSONAL.to_string();
        bundle.gateway_url = dual.openai_url;
        bundle.anthropic_gateway_url = Some(dual.anthropic_url.clone());
        assert_eq!(
            claude_code_target_url(&bundle),
            Some(dual.anthropic_url.as_str())
        );
    }

    #[test]
    fn personal_openai_without_anthropic_gateway_skips_claude() {
        let mut bundle = sample_bundle(PROTOCOL_OPENAI, None);
        bundle.mode = MODE_PERSONAL.to_string();
        bundle.gateway_url = "https://api.openai.com/v1".into();
        assert!(claude_code_target_url(&bundle).is_none());
    }

    #[test]
    fn deepseek_dual_endpoints_cover_openai_and_anthropic() {
        let dual = dual_protocol_endpoints("https://api.deepseek.com/v1").expect("deepseek");
        assert_eq!(dual.openai_url, "https://api.deepseek.com/v1");
        assert_eq!(dual.anthropic_url, "https://api.deepseek.com/anthropic");

        let dual2 = dual_protocol_endpoints("https://api.deepseek.com/anthropic")
            .expect("deepseek anthropic");
        assert_eq!(dual2.openai_url, "https://api.deepseek.com/v1");
        assert_eq!(dual2.anthropic_url, "https://api.deepseek.com/anthropic");

        let or = dual_protocol_endpoints("https://openrouter.ai/api/v1").expect("openrouter");
        assert_eq!(or.openai_url, "https://openrouter.ai/api/v1");
        assert_eq!(or.anthropic_url, "https://openrouter.ai/api");
    }

    #[test]
    fn glm_and_minimax_dual_endpoints() {
        let glm = dual_protocol_endpoints("https://open.bigmodel.cn/api/paas/v4").expect("glm");
        assert_eq!(glm.openai_url, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(glm.anthropic_url, "https://open.bigmodel.cn/api/anthropic");

        let coding =
            dual_protocol_endpoints("https://open.bigmodel.cn/api/coding/paas/v4").expect("coding");
        assert_eq!(
            coding.openai_url,
            "https://open.bigmodel.cn/api/coding/paas/v4"
        );
        assert_eq!(
            coding.anthropic_url,
            "https://open.bigmodel.cn/api/anthropic"
        );

        let zai = dual_protocol_endpoints("https://api.z.ai/api/coding/paas/v4").expect("zai");
        assert_eq!(zai.openai_url, "https://api.z.ai/api/coding/paas/v4");

        let qwen_coding = dual_protocol_endpoints("https://coding.dashscope.aliyuncs.com/v1")
            .expect("qwen coding");
        assert_eq!(
            qwen_coding.openai_url,
            "https://coding.dashscope.aliyuncs.com/v1"
        );
        let kimi_coding =
            dual_protocol_endpoints("https://api.kimi.com/coding/v1").expect("kimi coding");
        assert_eq!(kimi_coding.openai_url, "https://api.kimi.com/coding/v1");
        assert_eq!(kimi_coding.anthropic_url, "https://api.kimi.com/coding");

        let mm_cn = dual_protocol_endpoints("https://api.minimax.cn/v1").expect("minimax cn");
        assert_eq!(mm_cn.openai_url, "https://api.minimax.cn/v1");
        assert_eq!(mm_cn.anthropic_url, "https://api.minimax.cn/anthropic");

        let mm = dual_protocol_endpoints("https://api.minimaxi.com/v1").expect("minimax intl");
        assert_eq!(mm.openai_url, "https://api.minimaxi.com/v1");
        assert_eq!(mm.anthropic_url, "https://api.minimaxi.com/anthropic");

        let mm_io = dual_protocol_endpoints("https://api.minimax.io/v1").expect("minimax intl");
        assert_eq!(mm_io.openai_url, "https://api.minimax.io/v1");
        assert_eq!(mm_io.anthropic_url, "https://api.minimax.io/anthropic");
    }

    #[test]
    fn deepseek_model_on_glm_address_is_replaced() {
        assert_eq!(
            coerce_gateway_model(
                "https://open.bigmodel.cn/api/coding/paas/v4",
                "deepseek-v4-flash"
            )
            .as_deref(),
            Some("glm-5.3")
        );
        assert_eq!(
            coerce_gateway_model("https://open.bigmodel.cn/api/paas/v4", "glm-5.3"),
            None
        );
        assert_eq!(
            coerce_gateway_model("https://api.deepseek.com/v1", "deepseek-v4-flash"),
            None
        );
        assert_eq!(
            coerce_gateway_model("https://qianfan.baidubce.com/v2", "deepseek-v3.2"),
            None
        );
    }

    #[test]
    fn qwen_moonshot_siliconflow_dual_endpoints() {
        let qwen = dual_protocol_endpoints("https://dashscope.aliyuncs.com/compatible-mode/v1")
            .expect("qwen");
        assert_eq!(
            qwen.openai_url,
            "https://dashscope.aliyuncs.com/compatible-mode/v1"
        );
        assert_eq!(
            qwen.anthropic_url,
            "https://dashscope.aliyuncs.com/apps/anthropic"
        );

        let kimi = dual_protocol_endpoints("https://api.moonshot.cn/v1").expect("moonshot");
        assert_eq!(kimi.openai_url, "https://api.moonshot.cn/v1");
        assert_eq!(kimi.anthropic_url, "https://api.moonshot.cn/anthropic");

        let sf = dual_protocol_endpoints("https://api.siliconflow.cn/v1").expect("siliconflow");
        assert_eq!(sf.openai_url, "https://api.siliconflow.cn/v1");
        assert_eq!(sf.anthropic_url, "https://api.siliconflow.cn");
    }

    #[test]
    fn personal_deepseek_openai_also_targets_claude() {
        let mut bundle = sample_bundle(PROTOCOL_OPENAI, None);
        bundle.mode = MODE_PERSONAL.to_string();
        bundle.gateway_url = "https://api.deepseek.com/v1".into();
        if let Some(dual) = dual_protocol_endpoints(&bundle.gateway_url) {
            bundle.gateway_url = dual.openai_url;
            bundle.anthropic_gateway_url = Some(dual.anthropic_url);
        }
        assert_eq!(
            claude_code_target_url(&bundle),
            Some("https://api.deepseek.com/anthropic")
        );
    }

    #[test]
    fn personal_anthropic_uses_gateway_url_when_slot_missing() {
        let mut bundle = sample_bundle(PROTOCOL_ANTHROPIC, None);
        bundle.mode = MODE_PERSONAL.to_string();
        bundle.gateway_url = "https://api.anthropic.com".into();
        assert_eq!(
            claude_code_target_url(&bundle),
            Some("https://api.anthropic.com")
        );
    }
}
