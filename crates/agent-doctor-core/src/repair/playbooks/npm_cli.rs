//! Claude Code / Codex repair playbooks (deep v1).
//!
//! Shared with Hermes/OpenClaw:
//! - Install missing binary via npm lifecycle
//! - Rewrite gateway / provider slot from active Personal/Team mode
//! - Browser MCP wire/repair
//!
//! Claude-specific:
//! - Scaffold empty `ANTHROPIC_API_KEY` + local guide (no secret fill)
//! - Tighten `~/.claude/settings.json` permissions when it holds a key
//! - Heal `env` / `anthropicBaseUrl` mismatch via mode rewire
//!
//! Codex-specific:
//! - Patch `wire_api=responses` + top-level `openai_base_url`
//! - Clear placeholder / empty apikey `auth.json`
//! - Fill bare `default` model from active mode
//! - Guide for missing `env_key` secret (no secret fill)

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value as JsonValue};

use agent_doctor_mcp::{wire_browser_mcp, WireBrowserMcpOptions};

use crate::adapters::util::home_join;
use crate::adapters::CodexAdapter;
use crate::lifecycle::{run_claude_code_lifecycle, run_codex_lifecycle, NpmCliLifecycleAction};
use crate::probe::{ProbeStatus, RuntimeProbeReport};
use crate::repair::{SkippedRepairAction, SuggestedRepair};
use crate::setup::{
    adopt_live_gateway_as_overlay, clear_codex_placeholder_auth, load_mode_status,
    mode_overlay_ready_from_profile, switch_to_personal_mode, switch_to_team_mode, MODE_PERSONAL,
    MODE_TEAM,
};
use crate::workspace::resolve_agent_doctor_binary;

use super::should_run;
use super::PlaybookApplyResult;

pub fn suggest_claude_code_repairs(probe: &RuntimeProbeReport) -> Vec<SuggestedRepair> {
    suggest_npm_cli_repairs("claude-code", "Claude Code", probe)
}

pub fn suggest_codex_repairs(probe: &RuntimeProbeReport) -> Vec<SuggestedRepair> {
    suggest_npm_cli_repairs("codex", "Codex", probe)
}

fn suggest_npm_cli_repairs(
    runtime_id: &str,
    display: &str,
    probe: &RuntimeProbeReport,
) -> Vec<SuggestedRepair> {
    let mut items = Vec::new();
    // Prefer profile.env so suggest/preview does not open the OS keychain.
    let mode_ready = mode_overlay_ready_from_profile();

    for check in &probe.checks {
        if check.id == "binary.exists" && check.status == ProbeStatus::Fail {
            items.push(SuggestedRepair {
                id: format!("fix-{runtime_id}-install"),
                title: format!("Install {display}"),
                description:
                    "Install via `npm install -g` (official package). Requires Node.js/npm and network."
                        .to_string(),
                auto_fixable: true,
            });
        }

        if check.id.starts_with("config.exists:")
            && check.status == ProbeStatus::Warn
            && (check.id.contains("settings.json") || check.id.contains("config.toml"))
        {
            items.push(SuggestedRepair {
                id: format!("fix-{runtime_id}-create-config"),
                title: format!("Create {display} config from active mode"),
                description: if mode_ready {
                    "Create config by re-applying Personal/Team overlay.".to_string()
                } else {
                    "Configure a Personal Provider or connect Evotown, then switch mode."
                        .to_string()
                },
                auto_fixable: mode_ready,
            });
        }

        if check.id == "mode.overlay_mismatch"
            && matches!(check.status, ProbeStatus::Warn | ProbeStatus::Fail)
        {
            let live = probe
                .facts
                .iter()
                .find(|f| f.key == "mode.live_gateway")
                .map(|f| f.value.as_str())
                .unwrap_or("live runtime gateway");
            let expected = probe
                .facts
                .iter()
                .find(|f| f.key == "mode.overlay_gateway")
                .map(|f| f.value.as_str())
                .unwrap_or("Agent Doctor overlay");

            items.push(SuggestedRepair {
                id: format!("fix-{runtime_id}-gateway-from-mode"),
                title: format!("Align {display} → Agent Doctor overlay"),
                description: if mode_ready {
                    format!(
                        "Rewrite this runtime to match overlay ({expected}). Use when overlay is the source of truth."
                    )
                } else {
                    "Configure a Personal Provider or connect Evotown, then switch mode.".to_string()
                },
                auto_fixable: mode_ready,
            });

            items.push(SuggestedRepair {
                id: format!("fix-{runtime_id}-adopt-live-gateway"),
                title: format!("Keep {display} live → update Agent Doctor overlay"),
                description: format!(
                    "Leave runtime at {live}; update personal overlay to match. \
                     Does not rewrite runtime configs. Personal mode only. \
                     On full --apply this is skipped unless selected alone (二选一)."
                ),
                auto_fixable: mode_ready,
            });
        }

        if check.id == "runtime.model_unroutable"
            && matches!(check.status, ProbeStatus::Warn | ProbeStatus::Fail)
        {
            items.push(SuggestedRepair {
                id: format!("fix-{runtime_id}-model-from-mode"),
                title: format!("Set routable {display} model from active mode"),
                description: if mode_ready {
                    "Re-apply mode wiring so model is a gateway-routable id (not bare `default`)."
                        .to_string()
                } else {
                    "Configure Personal/Team mode first, then repair.".to_string()
                },
                auto_fixable: mode_ready,
            });
        }

        if check.id.starts_with("config.schema:")
            && check.status == ProbeStatus::Warn
            && (check.message.contains("model_provider")
                || check.message.contains("ANTHROPIC_BASE_URL")
                || check.message.contains("base_url")
                || check.message.contains("env_key")
                || check.message.contains("openai_base_url"))
        {
            items.push(SuggestedRepair {
                id: format!("fix-{runtime_id}-config-from-mode"),
                title: format!("Heal {display} provider schema from active mode"),
                description: if mode_ready {
                    "Re-apply Personal/Team overlay to fill provider / base URL / env_key fields."
                        .to_string()
                } else {
                    "Configure Personal/Team mode first.".to_string()
                },
                auto_fixable: mode_ready,
            });
        }

        if (check.id == "claude.schema.base_url_mismatch"
            || check.id == "codex.schema.openai_base_url_missing")
            && check.status == ProbeStatus::Warn
        {
            items.push(SuggestedRepair {
                id: format!("fix-{runtime_id}-config-from-mode"),
                title: format!("Heal {display} base URL wiring from active mode"),
                description: if mode_ready {
                    "Re-apply mode overlay so base URL fields stay consistent.".to_string()
                } else {
                    "Configure Personal/Team mode first.".to_string()
                },
                auto_fixable: mode_ready,
            });
        }
    }

    if runtime_id == "claude-code" {
        items.extend(suggest_claude_specific(probe, mode_ready));
        items.extend(suggest_claude_global_mcp_bleed());
    }
    if runtime_id == "codex" {
        items.extend(suggest_codex_specific(probe, mode_ready));
    }

    items.extend(suggest_browser_mcp_repairs(runtime_id, display, probe));
    dedupe_repairs(items)
}

fn suggest_claude_specific(probe: &RuntimeProbeReport, mode_ready: bool) -> Vec<SuggestedRepair> {
    let mut items = Vec::new();
    for check in &probe.checks {
        if check.id == "claude.api_key.configured" && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: if mode_ready {
                    "fix-claude-code-gateway-from-mode".to_string()
                } else {
                    "fix-claude-code-api-key-scaffold".to_string()
                },
                title: if mode_ready {
                    "Apply active mode API key into Claude settings".to_string()
                } else {
                    "Prepare Claude settings for API key".to_string()
                },
                description: if mode_ready {
                    "Re-apply Personal/Team overlay (writes ANTHROPIC_API_KEY from wiring)."
                        .to_string()
                } else {
                    "Create empty ANTHROPIC_API_KEY placeholder + local guide (secret not auto-filled)."
                        .to_string()
                },
                auto_fixable: true,
            });
        }

        if check.id.starts_with("claude.settings.permissions:") && check.status == ProbeStatus::Warn
        {
            items.push(SuggestedRepair {
                id: "fix-claude-code-settings-permissions".to_string(),
                title: "Tighten ~/.claude/settings.json permissions".to_string(),
                description: "Set settings.json to mode 600 (contains API key).".to_string(),
                auto_fixable: cfg!(unix),
            });
        }
    }
    items
}

fn suggest_claude_global_mcp_bleed() -> Vec<SuggestedRepair> {
    let summary = crate::workspace::backends::claude_global_mcp_summary();
    let total = summary.user_scope_servers + summary.claude_json_servers;
    if total == 0 {
        return Vec::new();
    }
    vec![SuggestedRepair {
        id: "review-claude-global-mcp".to_string(),
        title: "Review user-scoped Claude MCP (project bleed risk)".to_string(),
        description: format!(
            "{total} user-scoped MCP server(s) in ~/.claude/settings.json and/or ~/.claude.json. \
             Prefer project .mcp.json — run `agent-doctor workspace fix` \
             (add --migrate-claude-mcp to merge; globals are not auto-deleted)."
        ),
        auto_fixable: false,
    }]
}

fn suggest_codex_specific(probe: &RuntimeProbeReport, mode_ready: bool) -> Vec<SuggestedRepair> {
    let mut items = Vec::new();
    for check in &probe.checks {
        if (check.id == "codex.schema.wire_api_missing" || check.id == "codex.schema.wire_api")
            && check.status == ProbeStatus::Warn
        {
            items.push(SuggestedRepair {
                id: "fix-codex-wire-api".to_string(),
                title: "Set Codex wire_api to responses".to_string(),
                description: "Patch active model_providers.*.wire_api = \"responses\".".to_string(),
                auto_fixable: true,
            });
        }

        if check.id == "codex.auth.placeholder" && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: "fix-codex-clear-placeholder-auth".to_string(),
                title: "Remove placeholder Codex auth.json".to_string(),
                description: "Delete empty/placeholder auth.json so env_key auth can work."
                    .to_string(),
                auto_fixable: true,
            });
        }

        if check.id == "codex.api_key.configured" && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: if mode_ready {
                    "fix-codex-gateway-from-mode".to_string()
                } else {
                    "fix-codex-api-key-scaffold".to_string()
                },
                title: if mode_ready {
                    "Re-apply mode wiring for Codex env_key".to_string()
                } else {
                    "Prepare Codex API key guide".to_string()
                },
                description: if mode_ready {
                    "Re-apply Personal/Team overlay so env_key + openai_base_url match wiring."
                        .to_string()
                } else {
                    "Write a local guide for the required env_key (secret is never auto-filled)."
                        .to_string()
                },
                auto_fixable: true,
            });
        }
    }
    items
}

fn dedupe_repairs(items: Vec<SuggestedRepair>) -> Vec<SuggestedRepair> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.id.clone()))
        .collect()
}

/// Suggest Browser MCP write/repair for any supported Ask runtime.
pub(crate) fn suggest_browser_mcp_repairs(
    runtime_id: &str,
    display: &str,
    probe: &RuntimeProbeReport,
) -> Vec<SuggestedRepair> {
    let mut items = Vec::new();
    for check in &probe.checks {
        if check.id == "mcp.browser.configured"
            && matches!(check.status, ProbeStatus::Warn | ProbeStatus::Fail)
        {
            items.push(SuggestedRepair {
                id: format!("fix-{runtime_id}-browser-mcp"),
                title: format!("Write Browser MCP into {display} config"),
                description:
                    "Upsert mcpServers/mcp_servers.browser → agent-doctor (keeps other MCP entries)."
                        .to_string(),
                auto_fixable: true,
            });
        }

        if check.id == "mcp.browser.healthy" && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: format!("fix-{runtime_id}-browser-mcp"),
                title: format!("Repair Browser MCP path for {display}"),
                description: "Rewrite the browser MCP command to the real agent-doctor CLI binary."
                    .to_string(),
                auto_fixable: true,
            });
        }
    }
    items
}

pub fn apply_claude_code_playbook(probe: &RuntimeProbeReport) -> Result<PlaybookApplyResult> {
    apply_claude_code_playbook_filtered(probe, None)
}

pub fn apply_claude_code_playbook_filtered(
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
) -> Result<PlaybookApplyResult> {
    apply_npm_cli_playbook("claude-code", probe, only_ids)
}

pub fn apply_codex_playbook(probe: &RuntimeProbeReport) -> Result<PlaybookApplyResult> {
    apply_codex_playbook_filtered(probe, None)
}

pub fn apply_codex_playbook_filtered(
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
) -> Result<PlaybookApplyResult> {
    apply_npm_cli_playbook("codex", probe, only_ids)
}

fn apply_npm_cli_playbook(
    runtime_id: &str,
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
) -> Result<PlaybookApplyResult> {
    let mut result = PlaybookApplyResult::default();
    let install_id = format!("fix-{runtime_id}-install");
    let create_id = format!("fix-{runtime_id}-create-config");
    let gateway_id = format!("fix-{runtime_id}-gateway-from-mode");
    let adopt_id = format!("fix-{runtime_id}-adopt-live-gateway");
    let model_id = format!("fix-{runtime_id}-model-from-mode");
    let config_id = format!("fix-{runtime_id}-config-from-mode");
    let browser_id = format!("fix-{runtime_id}-browser-mcp");

    if should_run(&install_id, only_ids) && needs_install(probe) {
        match run_install(runtime_id) {
            Ok(()) => result.executed.push(install_id),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: install_id,
                reason: error.to_string(),
            }),
        }
    }

    // Mode drift is 二选一: align runtime→overlay vs keep live→update overlay.
    let adopt_selected = only_ids.is_some_and(|ids| ids.iter().any(|id| id == &adopt_id));
    let align_selected = only_ids
        .map(|ids| ids.iter().any(|id| id == &gateway_id))
        .unwrap_or(true);
    if needs_gateway_rewire(probe) && adopt_selected && align_selected && only_ids.is_some() {
        result.skipped.push(SkippedRepairAction {
            id: adopt_id.clone(),
            reason: "pick one: align runtime to overlay OR keep live and update overlay".into(),
        });
        result.skipped.push(SkippedRepairAction {
            id: gateway_id.clone(),
            reason: "pick one: align runtime to overlay OR keep live and update overlay".into(),
        });
    } else if needs_gateway_rewire(probe) && adopt_selected && !align_selected {
        match adopt_live_from_probe(probe) {
            Ok(()) => result.executed.push(adopt_id),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: adopt_id,
                reason: error.to_string(),
            }),
        }
    } else if needs_gateway_rewire(probe) && !adopt_selected && only_ids.is_none() {
        // Full --apply: default to align; leave adopt as an explicit choice.
        result.skipped.push(SkippedRepairAction {
            id: adopt_id,
            reason: "skipped on full --apply — select this action alone to keep live gateway"
                .into(),
        });
    } else if needs_gateway_rewire(probe) && adopt_selected && only_ids.is_none() {
        // unreachable: adopt_selected requires only_ids
    }

    let needs_mode_rewire = (needs_gateway_rewire(probe) && !adopt_selected)
        || needs_create_config(probe)
        || needs_model_from_mode(probe)
        || needs_config_from_mode(probe)
        || (runtime_id == "claude-code" && needs_claude_key_from_mode(probe))
        || (runtime_id == "codex" && needs_codex_key_from_mode(probe));

    // When adopt won the 二选一, do not also rewire runtime from mode.
    let skip_align_for_adopt = adopt_selected && !align_selected && needs_gateway_rewire(probe);

    let mode_action_ids = [
        gateway_id.as_str(),
        create_id.as_str(),
        model_id.as_str(),
        config_id.as_str(),
    ];
    let should_mode = !skip_align_for_adopt
        && (mode_action_ids.iter().any(|id| should_run(id, only_ids))
            || (runtime_id == "claude-code"
                && should_run("fix-claude-code-gateway-from-mode", only_ids)
                && needs_claude_key_from_mode(probe))
            || (runtime_id == "codex"
                && should_run("fix-codex-gateway-from-mode", only_ids)
                && needs_codex_key_from_mode(probe)));

    if should_mode && needs_mode_rewire {
        let primary_id = if needs_create_config(probe) && should_run(&create_id, only_ids) {
            create_id
        } else if needs_gateway_rewire(probe)
            && should_run(&gateway_id, only_ids)
            && !adopt_selected
        {
            gateway_id
        } else if needs_model_from_mode(probe) && should_run(&model_id, only_ids) {
            model_id
        } else if needs_config_from_mode(probe) && should_run(&config_id, only_ids) {
            config_id
        } else if runtime_id == "claude-code" && needs_claude_key_from_mode(probe) {
            "fix-claude-code-gateway-from-mode".to_string()
        } else if runtime_id == "codex" && needs_codex_key_from_mode(probe) {
            "fix-codex-gateway-from-mode".to_string()
        } else {
            gateway_id
        };
        match rewire_gateway_from_active_mode() {
            Ok(()) => result.executed.push(primary_id),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: primary_id,
                reason: error.to_string(),
            }),
        }
    }

    if runtime_id == "claude-code" {
        apply_claude_specific(probe, only_ids, &mut result)?;
    }
    if runtime_id == "codex" {
        apply_codex_specific(probe, only_ids, &mut result)?;
    }

    if should_run(&browser_id, only_ids) && needs_browser_mcp_rewire(probe) {
        match wire_browser_mcp_for_runtime(runtime_id) {
            Ok(()) => result.executed.push(browser_id),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: browser_id,
                reason: error.to_string(),
            }),
        }
    }

    Ok(result)
}

fn apply_claude_specific(
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
    result: &mut PlaybookApplyResult,
) -> Result<()> {
    if should_run("fix-claude-code-api-key-scaffold", only_ids)
        && needs_claude_api_key_scaffold(probe)
    {
        match scaffold_claude_api_key() {
            Ok(guide_path) => {
                result
                    .executed
                    .push("fix-claude-code-api-key-scaffold".to_string());
                result.guide_path = Some(guide_path);
            }
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: "fix-claude-code-api-key-scaffold".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    if should_run("fix-claude-code-settings-permissions", only_ids)
        && needs_claude_permissions(probe)
    {
        match tighten_claude_settings_permissions() {
            Ok(()) => result
                .executed
                .push("fix-claude-code-settings-permissions".to_string()),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: "fix-claude-code-settings-permissions".to_string(),
                reason: error.to_string(),
            }),
        }
    }
    Ok(())
}

fn apply_codex_specific(
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
    result: &mut PlaybookApplyResult,
) -> Result<()> {
    if should_run("fix-codex-wire-api", only_ids) && needs_codex_wire_api(probe) {
        match patch_codex_wire_api() {
            Ok(()) => result.executed.push("fix-codex-wire-api".to_string()),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: "fix-codex-wire-api".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    if should_run("fix-codex-clear-placeholder-auth", only_ids)
        && needs_codex_placeholder_auth(probe)
    {
        match clear_codex_placeholder_auth() {
            Ok(()) => result
                .executed
                .push("fix-codex-clear-placeholder-auth".to_string()),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: "fix-codex-clear-placeholder-auth".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    if should_run("fix-codex-api-key-scaffold", only_ids) && needs_codex_api_key_scaffold(probe) {
        match scaffold_codex_api_key_guide(probe) {
            Ok(guide_path) => {
                result
                    .executed
                    .push("fix-codex-api-key-scaffold".to_string());
                result.guide_path = Some(guide_path);
            }
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: "fix-codex-api-key-scaffold".to_string(),
                reason: error.to_string(),
            }),
        }
    }
    Ok(())
}

/// Apply Browser MCP wire when probe says configured/healthy is bad.
pub(crate) fn apply_browser_mcp_repair(
    runtime_id: &str,
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
) -> Result<PlaybookApplyResult> {
    let mut result = PlaybookApplyResult::default();
    let browser_id = format!("fix-{runtime_id}-browser-mcp");
    if should_run(&browser_id, only_ids) && needs_browser_mcp_rewire(probe) {
        match wire_browser_mcp_for_runtime(runtime_id) {
            Ok(()) => result.executed.push(browser_id),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: browser_id,
                reason: error.to_string(),
            }),
        }
    }
    Ok(result)
}

fn needs_install(probe: &RuntimeProbeReport) -> bool {
    probe
        .checks
        .iter()
        .any(|check| check.id == "binary.exists" && check.status == ProbeStatus::Fail)
}

fn needs_create_config(probe: &RuntimeProbeReport) -> bool {
    probe.checks.iter().any(|check| {
        check.id.starts_with("config.exists:")
            && check.status == ProbeStatus::Warn
            && (check.id.contains("settings.json") || check.id.contains("config.toml"))
    })
}

fn needs_gateway_rewire(probe: &RuntimeProbeReport) -> bool {
    probe.checks.iter().any(|check| {
        check.id == "mode.overlay_mismatch"
            && matches!(check.status, ProbeStatus::Warn | ProbeStatus::Fail)
    })
}

fn needs_model_from_mode(probe: &RuntimeProbeReport) -> bool {
    probe.checks.iter().any(|check| {
        check.id == "runtime.model_unroutable"
            && matches!(check.status, ProbeStatus::Warn | ProbeStatus::Fail)
    })
}

fn needs_config_from_mode(probe: &RuntimeProbeReport) -> bool {
    probe.checks.iter().any(|check| {
        (check.id.starts_with("config.schema:")
            && check.status == ProbeStatus::Warn
            && (check.message.contains("model_provider")
                || check.message.contains("ANTHROPIC_BASE_URL")
                || check.message.contains("base_url")
                || check.message.contains("env_key")
                || check.message.contains("openai_base_url")))
            || (check.id == "claude.schema.base_url_mismatch" && check.status == ProbeStatus::Warn)
            || (check.id == "codex.schema.openai_base_url_missing"
                && check.status == ProbeStatus::Warn)
    })
}

fn needs_browser_mcp_rewire(probe: &RuntimeProbeReport) -> bool {
    probe.checks.iter().any(|check| {
        (check.id == "mcp.browser.configured" || check.id == "mcp.browser.healthy")
            && matches!(check.status, ProbeStatus::Warn | ProbeStatus::Fail)
    })
}

fn needs_claude_key_from_mode(probe: &RuntimeProbeReport) -> bool {
    mode_is_ready()
        && probe.checks.iter().any(|check| {
            check.id == "claude.api_key.configured" && check.status == ProbeStatus::Warn
        })
}

fn needs_claude_api_key_scaffold(probe: &RuntimeProbeReport) -> bool {
    !mode_is_ready()
        && probe.checks.iter().any(|check| {
            check.id == "claude.api_key.configured" && check.status == ProbeStatus::Warn
        })
}

fn needs_claude_permissions(probe: &RuntimeProbeReport) -> bool {
    probe.checks.iter().any(|check| {
        check.id.starts_with("claude.settings.permissions:") && check.status == ProbeStatus::Warn
    })
}

fn needs_codex_wire_api(probe: &RuntimeProbeReport) -> bool {
    probe.checks.iter().any(|check| {
        (check.id == "codex.schema.wire_api_missing" || check.id == "codex.schema.wire_api")
            && check.status == ProbeStatus::Warn
    })
}

fn needs_codex_placeholder_auth(probe: &RuntimeProbeReport) -> bool {
    probe
        .checks
        .iter()
        .any(|check| check.id == "codex.auth.placeholder" && check.status == ProbeStatus::Warn)
}

fn needs_codex_key_from_mode(probe: &RuntimeProbeReport) -> bool {
    mode_is_ready()
        && probe.checks.iter().any(|check| {
            check.id == "codex.api_key.configured" && check.status == ProbeStatus::Warn
        })
}

fn needs_codex_api_key_scaffold(probe: &RuntimeProbeReport) -> bool {
    !mode_is_ready()
        && probe.checks.iter().any(|check| {
            check.id == "codex.api_key.configured" && check.status == ProbeStatus::Warn
        })
}

fn mode_is_ready() -> bool {
    mode_overlay_ready_from_profile()
}

fn adopt_live_from_probe(probe: &RuntimeProbeReport) -> Result<()> {
    let live = probe
        .facts
        .iter()
        .find(|f| f.key == "mode.live_gateway")
        .map(|f| f.value.as_str())
        .filter(|u| !u.is_empty())
        .context("probe is missing mode.live_gateway fact")?;
    adopt_live_gateway_as_overlay(live)
}

fn run_install(runtime_id: &str) -> Result<()> {
    match runtime_id {
        "claude-code" => run_claude_code_lifecycle(NpmCliLifecycleAction::Install),
        "codex" => run_codex_lifecycle(NpmCliLifecycleAction::Install),
        other => bail!("no npm install playbook for {other}"),
    }
}

fn rewire_gateway_from_active_mode() -> Result<()> {
    let status = load_mode_status().context("load mode status")?;
    match status.mode.as_str() {
        MODE_PERSONAL => {
            let report = switch_to_personal_mode(status.personal_active_id.as_deref())?;
            if report.runtimes.iter().all(|r| !r.applied) {
                bail!(
                    "mode switch produced no applied runtimes: {}",
                    report.message
                );
            }
            Ok(())
        }
        MODE_TEAM => {
            let report = switch_to_team_mode()?;
            if report.runtimes.iter().all(|r| !r.applied) {
                bail!(
                    "mode switch produced no applied runtimes: {}",
                    report.message
                );
            }
            Ok(())
        }
        _ => bail!("no active Personal/Team mode — configure a provider first, then switch mode"),
    }
}

fn wire_browser_mcp_for_runtime(runtime_id: &str) -> Result<()> {
    let discovery = agent_doctor_mcp::discover_chrome().context("discover Chrome")?;
    let resolved = resolve_agent_doctor_binary().context("resolve agent-doctor binary")?;
    let binary = crate::workspace::ensure_stable_agent_doctor_cli(&resolved)?;
    let mut options = WireBrowserMcpOptions::with_binary(binary);
    options.runtimes = vec![runtime_id.to_string()];
    if let Ok(doc) = crate::workspace::load_workspaces() {
        if let Some(active) = doc.active.as_ref() {
            if let Some(entry) = doc.workspaces.get(active) {
                options.project_path = Some(entry.path.clone());
                options.codex_home = Some(entry.codex_home.clone());
                options.hermes_home = Some(
                    crate::adapters::util::home_join(".hermes/profiles")
                        .join(&entry.hermes_profile),
                );
                options.openclaw_workspace = Some(entry.openclaw_workspace.clone());
            }
        }
    }
    let report = wire_browser_mcp(&discovery, &options);
    let Some(item) = report.results.into_iter().find(|r| r.runtime == runtime_id) else {
        bail!("browser MCP wire returned no result for {runtime_id}");
    };
    if !item.ok {
        bail!("{}", item.message);
    }
    Ok(())
}

fn scaffold_claude_api_key() -> Result<PathBuf> {
    let path = home_join(".claude/settings.json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut root: JsonValue = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    let env = root
        .as_object_mut()
        .context("Claude settings root must be an object")?
        .entry("env")
        .or_insert_with(|| json!({}));
    if let Some(env_obj) = env.as_object_mut() {
        if !env_obj
            .get("ANTHROPIC_API_KEY")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.trim().is_empty())
        {
            env_obj.insert("ANTHROPIC_API_KEY".to_string(), json!(""));
        }
    }
    fs::write(&path, serde_json::to_string_pretty(&root)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }

    let guide_path = api_key_guide_path("claude-api-key-ANTHROPIC_API_KEY.md")?;
    let guide = "# Claude Code API key setup\n\n\
         Agent Doctor prepared `~/.claude/settings.json` with an empty `env.ANTHROPIC_API_KEY`.\n\n\
         ## Steps\n\n\
         1. Open `~/.claude/settings.json`.\n\
         2. Set `env.ANTHROPIC_API_KEY` to your key (or configure Personal/Team mode in Agent Doctor).\n\
         3. Optionally set `env.ANTHROPIC_BASE_URL` / `anthropicBaseUrl` for a custom gateway.\n\
         4. Run `agent-doctor repair claude-code` again to verify.\n\n\
         Secrets stay on this machine. Agent Doctor does not upload API keys.\n";
    fs::write(&guide_path, guide)?;
    Ok(guide_path)
}

#[cfg(unix)]
fn tighten_claude_settings_permissions() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = home_join(".claude/settings.json");
    if !path.exists() {
        bail!("Claude settings.json not found");
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to chmod {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn tighten_claude_settings_permissions() -> Result<()> {
    bail!("settings permission repair is only supported on Unix")
}

fn patch_codex_wire_api() -> Result<()> {
    let path = CodexAdapter::config_path();
    if !path.exists() {
        bail!("Codex config.toml not found at {}", path.display());
    }
    let raw = fs::read_to_string(&path)?;
    let mut doc = raw
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("failed to parse {}", path.display()))?;

    let provider = doc
        .get("model_provider")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .context("model_provider is missing")?;

    let providers = doc["model_providers"]
        .as_table_mut()
        .context("model_providers must be a table")?;
    let entry = providers
        .get_mut(&provider)
        .with_context(|| format!("model_providers.{provider} is missing"))?
        .as_table_mut()
        .with_context(|| format!("model_providers.{provider} must be a table"))?;
    entry["wire_api"] = toml_edit::value("responses");
    if entry.get("supports_websockets").is_none() {
        entry["supports_websockets"] = toml_edit::value(false);
    }

    // Keep top-level openai_base_url aligned with the active provider base_url when present.
    if let Some(base) = entry.get("base_url").and_then(|v| v.as_str()) {
        if !base.trim().is_empty() {
            doc["openai_base_url"] = toml_edit::value(base);
        }
    }

    fs::write(&path, doc.to_string())?;
    Ok(())
}

fn scaffold_codex_api_key_guide(probe: &RuntimeProbeReport) -> Result<PathBuf> {
    let env_key = probe
        .facts
        .iter()
        .find(|f| f.key == "codex.api_key.env")
        .map(|f| f.value.clone())
        .unwrap_or_else(|| "OPENAI_API_KEY".to_string());
    let guide_path = api_key_guide_path(&format!("codex-api-key-{env_key}.md"))?;
    let guide = format!(
        "# Codex API key setup\n\n\
         Agent Doctor detected that `{env_key}` is not available for Codex.\n\n\
         ## Steps\n\n\
         1. Prefer configuring a Personal Provider or Evotown Team mode in Agent Doctor (Wiring tab).\n\
         2. Or export `{env_key}=your_api_key_here` in your shell profile.\n\
         3. Ensure `model_providers.<slot>.env_key` in Codex config points at `{env_key}`.\n\
         4. Run `agent-doctor repair codex` again to verify.\n\n\
         Secrets stay on this machine. Agent Doctor does not auto-fill or upload API keys.\n"
    );
    fs::write(&guide_path, guide)?;
    Ok(guide_path)
}

fn api_key_guide_path(file_name: &str) -> Result<PathBuf> {
    let root = dirs::config_dir()
        .map(|dir| dir.join("agent-doctor").join("guides"))
        .context("could not resolve config directory")?;
    fs::create_dir_all(&root)?;
    Ok(root.join(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{ProbeCheck, ProbeSeverity};
    use crate::repair::SensitivityLevel;

    fn probe_with(runtime_id: &str, checks: Vec<ProbeCheck>) -> RuntimeProbeReport {
        RuntimeProbeReport {
            runtime_id: runtime_id.into(),
            display_name: runtime_id.into(),
            binary_name: runtime_id.into(),
            checks,
            facts: Vec::new(),
        }
    }

    #[test]
    fn suggests_gateway_and_browser_fixes() {
        let probe = probe_with(
            "codex",
            vec![
                ProbeCheck::new(
                    "mode.overlay_mismatch",
                    "drift",
                    ProbeStatus::Warn,
                    ProbeSeverity::Warning,
                    "mismatch",
                    SensitivityLevel::ConfigShape,
                ),
                ProbeCheck::new(
                    "mcp.browser.configured",
                    "browser",
                    ProbeStatus::Warn,
                    ProbeSeverity::Warning,
                    "missing",
                    SensitivityLevel::ConfigShape,
                ),
            ],
        );
        let items = suggest_codex_repairs(&probe);
        assert!(items.iter().any(|i| i.id == "fix-codex-gateway-from-mode"));
        assert!(items.iter().any(|i| i.id == "fix-codex-browser-mcp"));
    }

    #[test]
    fn suggests_deep_codex_fixes() {
        let probe = probe_with(
            "codex",
            vec![
                ProbeCheck::new(
                    "binary.exists",
                    "bin",
                    ProbeStatus::Fail,
                    ProbeSeverity::Error,
                    "missing",
                    SensitivityLevel::Public,
                ),
                ProbeCheck::new(
                    "codex.schema.wire_api_missing",
                    "wire",
                    ProbeStatus::Warn,
                    ProbeSeverity::Warning,
                    "missing wire_api",
                    SensitivityLevel::ConfigShape,
                ),
                ProbeCheck::new(
                    "codex.auth.placeholder",
                    "auth",
                    ProbeStatus::Warn,
                    ProbeSeverity::Warning,
                    "placeholder",
                    SensitivityLevel::ConfigShape,
                ),
                ProbeCheck::new(
                    "runtime.model_unroutable",
                    "model",
                    ProbeStatus::Fail,
                    ProbeSeverity::Error,
                    "bare default",
                    SensitivityLevel::ConfigShape,
                ),
            ],
        );
        let items = suggest_codex_repairs(&probe);
        assert!(items.iter().any(|i| i.id == "fix-codex-install"));
        assert!(items.iter().any(|i| i.id == "fix-codex-wire-api"));
        assert!(items
            .iter()
            .any(|i| i.id == "fix-codex-clear-placeholder-auth"));
        assert!(items.iter().any(|i| i.id == "fix-codex-model-from-mode"));
    }

    #[test]
    fn suggests_deep_claude_fixes() {
        let probe = probe_with(
            "claude-code",
            vec![
                ProbeCheck::new(
                    "claude.api_key.configured",
                    "key",
                    ProbeStatus::Warn,
                    ProbeSeverity::Warning,
                    "missing",
                    SensitivityLevel::ConfigShape,
                ),
                ProbeCheck::new(
                    "claude.settings.permissions:/tmp/settings.json",
                    "perms",
                    ProbeStatus::Warn,
                    ProbeSeverity::Warning,
                    "too open",
                    SensitivityLevel::LocalPath,
                ),
                ProbeCheck::new(
                    "claude.schema.base_url_mismatch",
                    "url",
                    ProbeStatus::Warn,
                    ProbeSeverity::Warning,
                    "disagree",
                    SensitivityLevel::ConfigShape,
                ),
            ],
        );
        let items = suggest_claude_code_repairs(&probe);
        assert!(items.iter().any(|i| {
            i.id == "fix-claude-code-api-key-scaffold"
                || i.id == "fix-claude-code-gateway-from-mode"
        }));
        assert!(items
            .iter()
            .any(|i| i.id == "fix-claude-code-settings-permissions"));
        assert!(items
            .iter()
            .any(|i| i.id == "fix-claude-code-config-from-mode"));
    }

    #[test]
    fn suggests_mode_drift_as_two_way_choice() {
        let probe = RuntimeProbeReport {
            runtime_id: "claude-code".into(),
            display_name: "Claude Code".into(),
            binary_name: "claude".into(),
            checks: vec![ProbeCheck::new(
                "mode.overlay_mismatch",
                "drift",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                "mismatch",
                SensitivityLevel::ConfigShape,
            )],
            facts: vec![
                crate::repair::DiagnosticFact::new(
                    "mode.live_gateway",
                    "https://live.example/v1",
                    SensitivityLevel::ConfigShape,
                ),
                crate::repair::DiagnosticFact::new(
                    "mode.overlay_gateway",
                    "https://overlay.example/v1",
                    SensitivityLevel::ConfigShape,
                ),
            ],
        };
        let items = suggest_claude_code_repairs(&probe);
        assert!(items
            .iter()
            .any(|i| i.id == "fix-claude-code-gateway-from-mode"));
        assert!(items
            .iter()
            .any(|i| i.id == "fix-claude-code-adopt-live-gateway"));
        assert!(items.iter().any(|i| i.title.contains("Keep")));
    }
}
