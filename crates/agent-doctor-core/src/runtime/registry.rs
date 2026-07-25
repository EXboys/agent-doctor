use std::path::Path;

use anyhow::{Context, Result};

use crate::adapter::RuntimeAdapter;
use crate::adapters::{ClaudeCodeAdapter, CodexAdapter, HermesAdapter, OpenClawAdapter};
use crate::lifecycle::{
    run_claude_code_lifecycle, run_codex_lifecycle, run_hermes_lifecycle, run_openclaw_lifecycle,
    HermesLifecycleAction, NpmCliLifecycleAction, OpenClawLifecycleAction,
};
use crate::probe::runtimes::{
    openclaw_probe_deep, probe_deep, schema_claude_code, schema_codex, schema_hermes,
    schema_openclaw,
};
use crate::probe::ParsedConfig;
use crate::probe::{ProbeCheck, ProbeStatus, RuntimeProbeReport};
use crate::repair::{
    apply_hermes_playbook, apply_hermes_playbook_filtered, apply_openclaw_playbook,
    apply_openclaw_playbook_filtered, suggest_hermes_repairs, suggest_openclaw_repairs,
    PlaybookApplyResult, SuggestedRepair,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigFormat {
    Json,
    Yaml,
    Toml,
    Env,
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeProbeSpec {
    pub binary_name: &'static str,
    pub config_format: ConfigFormat,
    pub env_keywords: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeLifecycleAction {
    Install,
    Update,
}

type AdapterFactory = fn() -> Box<dyn RuntimeAdapter>;
type DeepProbeFn = fn(&mut Vec<ProbeCheck>, &mut Vec<DiagnosticFact>);
type SchemaProbeFn = fn(&Path, &ParsedConfig, &mut Vec<ProbeCheck>, &mut Vec<DiagnosticFact>);
type SuggestRepairsFn = fn(&RuntimeProbeReport) -> Vec<SuggestedRepair>;
type ApplyPlaybookFn = fn(&RuntimeProbeReport) -> Result<PlaybookApplyResult>;
type RunLifecycleFn = fn(RuntimeLifecycleAction) -> Result<()>;

use crate::repair::DiagnosticFact;

#[derive(Clone, Copy)]
pub struct RuntimeDescriptor {
    pub id: &'static str,
    pub probe: RuntimeProbeSpec,
    create_adapter: AdapterFactory,
    schema_probe: Option<SchemaProbeFn>,
    deep_probe: Option<DeepProbeFn>,
    suggest_repairs: Option<SuggestRepairsFn>,
    apply_playbook: Option<ApplyPlaybookFn>,
    run_lifecycle: Option<RunLifecycleFn>,
}

impl RuntimeDescriptor {
    pub fn create_adapter(&self) -> Box<dyn RuntimeAdapter> {
        (self.create_adapter)()
    }

    pub(crate) fn run_schema_probe(
        &self,
        path: &Path,
        parsed: &ParsedConfig,
        checks: &mut Vec<ProbeCheck>,
        facts: &mut Vec<DiagnosticFact>,
    ) {
        if let Some(schema_probe) = self.schema_probe {
            schema_probe(path, parsed, checks, facts);
        }
    }

    pub(crate) fn run_deep_probe(
        &self,
        checks: &mut Vec<ProbeCheck>,
        facts: &mut Vec<DiagnosticFact>,
    ) {
        if let Some(deep_probe) = self.deep_probe {
            deep_probe(checks, facts);
        }
    }
}

const OPENCLAW_ENV: &[&str] = &["OPENCLAW", "EVOTOWN", "OPENAI", "ANTHROPIC"];
const CLAUDE_ENV: &[&str] = &["ANTHROPIC", "CLAUDE"];
const CODEX_ENV: &[&str] = &["OPENAI", "CODEX"];
const HERMES_ENV: &[&str] = &["HERMES", "OPENAI", "ANTHROPIC", "DEEPSEEK", "GOOGLE"];

fn openclaw_adapter() -> Box<dyn RuntimeAdapter> {
    Box::new(OpenClawAdapter)
}

fn claude_code_adapter() -> Box<dyn RuntimeAdapter> {
    Box::new(ClaudeCodeAdapter)
}

fn codex_adapter() -> Box<dyn RuntimeAdapter> {
    Box::new(CodexAdapter)
}

fn hermes_adapter() -> Box<dyn RuntimeAdapter> {
    Box::new(HermesAdapter)
}

fn run_openclaw_lifecycle_action(action: RuntimeLifecycleAction) -> Result<()> {
    let action = match action {
        RuntimeLifecycleAction::Install => OpenClawLifecycleAction::Install,
        RuntimeLifecycleAction::Update => OpenClawLifecycleAction::Update,
    };
    run_openclaw_lifecycle(action)
}

fn run_hermes_lifecycle_action(action: RuntimeLifecycleAction) -> Result<()> {
    let action = match action {
        RuntimeLifecycleAction::Install => HermesLifecycleAction::Install,
        RuntimeLifecycleAction::Update => HermesLifecycleAction::Update,
    };
    run_hermes_lifecycle(action)
}

fn run_claude_code_lifecycle_action(action: RuntimeLifecycleAction) -> Result<()> {
    let action = match action {
        RuntimeLifecycleAction::Install => NpmCliLifecycleAction::Install,
        RuntimeLifecycleAction::Update => NpmCliLifecycleAction::Update,
    };
    run_claude_code_lifecycle(action)
}

fn run_codex_lifecycle_action(action: RuntimeLifecycleAction) -> Result<()> {
    let action = match action {
        RuntimeLifecycleAction::Install => NpmCliLifecycleAction::Install,
        RuntimeLifecycleAction::Update => NpmCliLifecycleAction::Update,
    };
    run_codex_lifecycle(action)
}

static RUNTIME_REGISTRY: &[RuntimeDescriptor] = &[
    RuntimeDescriptor {
        id: "openclaw",
        probe: RuntimeProbeSpec {
            binary_name: "openclaw",
            config_format: ConfigFormat::Json,
            env_keywords: OPENCLAW_ENV,
        },
        create_adapter: openclaw_adapter,
        schema_probe: Some(schema_openclaw),
        deep_probe: Some(openclaw_probe_deep),
        suggest_repairs: Some(suggest_openclaw_repairs),
        apply_playbook: Some(apply_openclaw_playbook),
        run_lifecycle: Some(run_openclaw_lifecycle_action),
    },
    RuntimeDescriptor {
        id: "hermes",
        probe: RuntimeProbeSpec {
            binary_name: "hermes",
            config_format: ConfigFormat::Yaml,
            env_keywords: HERMES_ENV,
        },
        create_adapter: hermes_adapter,
        schema_probe: Some(schema_hermes),
        deep_probe: Some(probe_deep),
        suggest_repairs: Some(suggest_hermes_repairs),
        apply_playbook: Some(apply_hermes_playbook),
        run_lifecycle: Some(run_hermes_lifecycle_action),
    },
    RuntimeDescriptor {
        id: "claude-code",
        probe: RuntimeProbeSpec {
            binary_name: "claude",
            config_format: ConfigFormat::Json,
            env_keywords: CLAUDE_ENV,
        },
        create_adapter: claude_code_adapter,
        schema_probe: Some(schema_claude_code),
        deep_probe: None,
        suggest_repairs: None,
        apply_playbook: None,
        run_lifecycle: Some(run_claude_code_lifecycle_action),
    },
    RuntimeDescriptor {
        id: "codex",
        probe: RuntimeProbeSpec {
            binary_name: "codex",
            config_format: ConfigFormat::Toml,
            env_keywords: CODEX_ENV,
        },
        create_adapter: codex_adapter,
        schema_probe: Some(schema_codex),
        deep_probe: None,
        suggest_repairs: None,
        apply_playbook: None,
        run_lifecycle: Some(run_codex_lifecycle_action),
    },
];

pub fn all_runtime_ids() -> impl Iterator<Item = &'static str> {
    RUNTIME_REGISTRY.iter().map(|entry| entry.id)
}

pub fn descriptor_by_id(runtime_id: &str) -> Option<&'static RuntimeDescriptor> {
    RUNTIME_REGISTRY.iter().find(|entry| entry.id == runtime_id)
}

pub fn all_adapters() -> Vec<Box<dyn RuntimeAdapter>> {
    RUNTIME_REGISTRY
        .iter()
        .map(|entry| entry.create_adapter())
        .collect()
}

pub fn adapter_by_id(runtime_id: &str) -> Option<Box<dyn RuntimeAdapter>> {
    descriptor_by_id(runtime_id).map(|entry| entry.create_adapter())
}

pub fn suggest_runtime_repairs(
    runtime_id: &str,
    probe: &RuntimeProbeReport,
) -> Vec<SuggestedRepair> {
    let mut items = descriptor_by_id(runtime_id)
        .and_then(|entry| entry.suggest_repairs)
        .map(|suggest| suggest(probe))
        .unwrap_or_default();

    if probe_needs_binary_install(probe) && !items.iter().any(|item| item.id.ends_with("-install"))
    {
        let title = adapter_by_id(runtime_id)
            .map(|adapter| adapter.display_name().to_string())
            .unwrap_or_else(|| runtime_id.to_string());
        let has_rules = runtime_supports_lifecycle(runtime_id);
        items.insert(
            0,
            SuggestedRepair {
                id: format!("fix-{runtime_id}-install"),
                title: format!("Install {title}"),
                description: if has_rules {
                    "Install via official rule-based script; AI repair may retry on failure."
                        .to_string()
                } else {
                    "No rule installer registered; AI repair uses allowlisted install commands."
                        .to_string()
                },
                auto_fixable: true,
            },
        );
    }

    items
}

fn probe_needs_binary_install(probe: &RuntimeProbeReport) -> bool {
    probe
        .checks
        .iter()
        .any(|check| check.id == "binary.exists" && check.status == ProbeStatus::Fail)
}

pub fn apply_runtime_playbook(
    runtime_id: &str,
    probe: &RuntimeProbeReport,
) -> Result<PlaybookApplyResult> {
    apply_runtime_playbook_filtered(runtime_id, probe, None)
}

pub fn apply_runtime_playbook_filtered(
    runtime_id: &str,
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
) -> Result<PlaybookApplyResult> {
    if runtime_id == "openclaw" {
        return apply_openclaw_playbook_filtered(probe, only_ids);
    }
    if runtime_id == "hermes" {
        return apply_hermes_playbook_filtered(probe, only_ids);
    }
    let apply = descriptor_by_id(runtime_id)
        .and_then(|entry| entry.apply_playbook)
        .with_context(|| format!("runtime '{runtime_id}' has no repair playbook"))?;
    if only_ids.is_some() {
        anyhow::bail!("runtime '{runtime_id}' does not support filtered playbook execution yet");
    }
    apply(probe)
}

pub fn run_runtime_lifecycle(runtime_id: &str, action: RuntimeLifecycleAction) -> Result<()> {
    let run = descriptor_by_id(runtime_id)
        .and_then(|entry| entry.run_lifecycle)
        .with_context(|| format!("runtime '{runtime_id}' has no install/update hooks"))?;
    run(action)
}

pub fn runtime_supports_playbook(runtime_id: &str) -> bool {
    descriptor_by_id(runtime_id).is_some_and(|entry| entry.apply_playbook.is_some())
}

pub fn runtime_supports_lifecycle(runtime_id: &str) -> bool {
    descriptor_by_id(runtime_id).is_some_and(|entry| entry.run_lifecycle.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{ProbeCheck, ProbeSeverity};

    #[test]
    fn registry_has_unique_ids_in_stable_order() {
        let ids: Vec<_> = RUNTIME_REGISTRY.iter().map(|entry| entry.id).collect();
        assert_eq!(ids, vec!["openclaw", "hermes", "claude-code", "codex"]);
        let unique: std::collections::HashSet<_> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn adapter_by_id_matches_registry() {
        for entry in RUNTIME_REGISTRY {
            let adapter = adapter_by_id(entry.id).expect("adapter");
            assert_eq!(adapter.id(), entry.id);
            assert_eq!(adapter.discover().installed, adapter.discover().installed);
        }
    }

    #[test]
    fn openclaw_entry_wires_playbook_and_lifecycle() {
        let openclaw = descriptor_by_id("openclaw").expect("openclaw");
        assert!(openclaw.schema_probe.is_some());
        assert!(openclaw.deep_probe.is_some());
        assert!(openclaw.suggest_repairs.is_some());
        assert!(openclaw.apply_playbook.is_some());
        assert!(openclaw.run_lifecycle.is_some());
    }

    #[test]
    fn hermes_entry_wires_playbook_and_lifecycle() {
        let hermes = descriptor_by_id("hermes").expect("hermes");
        assert!(hermes.schema_probe.is_some());
        assert!(hermes.suggest_repairs.is_some());
        assert!(hermes.apply_playbook.is_some());
        assert!(hermes.run_lifecycle.is_some());
        assert!(hermes.deep_probe.is_some());
    }

    #[test]
    fn claude_and_codex_wire_lifecycle() {
        assert!(descriptor_by_id("claude-code")
            .expect("claude-code")
            .run_lifecycle
            .is_some());
        assert!(descriptor_by_id("codex")
            .expect("codex")
            .run_lifecycle
            .is_some());
        assert!(runtime_supports_lifecycle("claude-code"));
        assert!(runtime_supports_lifecycle("codex"));
    }

    #[test]
    fn suggests_generic_install_when_binary_missing() {
        let probe = RuntimeProbeReport {
            runtime_id: "claude-code".to_string(),
            display_name: "Claude Code".to_string(),
            binary_name: "claude".to_string(),
            checks: vec![ProbeCheck::new(
                "binary.exists",
                "Binary on PATH",
                ProbeStatus::Fail,
                ProbeSeverity::Error,
                "missing",
                crate::repair::SensitivityLevel::Public,
            )],
            facts: Vec::new(),
        };
        let items = suggest_runtime_repairs("claude-code", &probe);
        assert!(items
            .iter()
            .any(|item| item.id == "fix-claude-code-install"));
    }

    #[test]
    fn every_runtime_has_schema_probe() {
        for entry in RUNTIME_REGISTRY {
            assert!(
                entry.schema_probe.is_some(),
                "{} missing schema_probe",
                entry.id
            );
        }
    }
}
