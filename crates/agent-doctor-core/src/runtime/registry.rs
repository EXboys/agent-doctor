use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::adapter::RuntimeAdapter;
use crate::adapters::{
    ClaudeCodeAdapter, CodexAdapter, CursorAdapter, DeepSeekHarnessAdapter, HermesAdapter,
    OpenClawAdapter, QoderAdapter, WorkbuddyAdapter,
};
use crate::evotown::{
    run_claude_cli, run_codex_cli, run_hermes_hook, run_openclaw_hook, AssignedJob, JobResult,
};
use crate::lifecycle::{
    run_claude_code_lifecycle, run_codex_lifecycle, run_cursor_lifecycle,
    run_deepseek_harness_lifecycle, run_hermes_lifecycle, run_openclaw_lifecycle,
    run_qoder_lifecycle, run_workbuddy_lifecycle, CursorLifecycleAction,
    DeepSeekHarnessLifecycleAction, HermesLifecycleAction, NpmCliLifecycleAction,
    OpenClawLifecycleAction, QoderLifecycleAction, WorkbuddyLifecycleAction,
};
use crate::probe::runtimes::{
    claude_code_probe_deep, codex_probe_deep, deepseek_harness_probe_deep, openclaw_probe_deep,
    probe_deep, probe_deep_noop, schema_claude_code, schema_codex, schema_cursor,
    schema_deepseek_harness, schema_hermes, schema_openclaw, schema_qoder, schema_workbuddy,
};
use crate::probe::ParsedConfig;
use crate::probe::{ProbeCheck, ProbeStatus, RuntimeProbeReport};
use crate::prompt_session::{
    AskBackend, ClaudeAskBackend, CodexAskBackend, DeepSeekHarnessAskBackend, HermesAskBackend,
    OpenClawAskBackend,
};
use crate::repair::{
    apply_claude_code_playbook_filtered, apply_codex_playbook_filtered, apply_config_syntax_repair,
    apply_cursor_playbook_filtered, apply_deepseek_harness_playbook_filtered,
    apply_hermes_playbook_filtered, apply_openclaw_playbook_filtered,
    apply_qoder_playbook_filtered, apply_workbuddy_playbook_filtered, suggest_claude_code_repairs,
    suggest_codex_repairs, suggest_config_syntax_repairs, suggest_cursor_repairs,
    suggest_deepseek_harness_repairs, suggest_hermes_repairs, suggest_openclaw_repairs,
    suggest_qoder_repairs, suggest_workbuddy_repairs, PlaybookApplyResult, SuggestedRepair,
};
use crate::session_launch::{
    open_claude_code, open_codex, open_cursor_app, open_in_terminal, OpenSessionReport,
};
use crate::setup::{EffectorKind, WriteSemantics};
use crate::workspace::backends::{
    bind_claude_code, bind_codex_for_project, bind_hermes, bind_openclaw, RuntimeBindReport,
};
use crate::workspace::repairs::{
    repair_claude_project_mcp, repair_codex_home, repair_hermes_gateway, repair_hermes_profile,
    repair_openclaw_agent_env, repair_openclaw_workspace, WorkspaceRepairFn,
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
type ApplyPlaybookFn = fn(&RuntimeProbeReport, Option<&[String]>) -> Result<PlaybookApplyResult>;
type RunLifecycleFn = fn(RuntimeLifecycleAction) -> Result<()>;
type WorkspaceBindFn = for<'a> fn(&WorkspaceBindInput<'a>) -> Result<RuntimeBindReport>;
type AskSession = &'static (dyn AskBackend + Sync);
type OpenSessionFn = fn(&Path, Option<&str>, bool) -> Result<OpenSessionReport>;
type DispatchJobFn = fn(&AssignedJob) -> Result<JobResult>;

/// How an interactive session is opened. `None` means launch is unsupported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenSessionKind {
    Terminal,
    DeepLink,
    App,
}

use crate::repair::DiagnosticFact;

/// Gateway projection for one runtime. `None` on the descriptor means wiring is unsupported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WiringSpec {
    pub write_semantics: WriteSemantics,
    pub effector: EffectorKind,
    pub openai_compatible: bool,
    pub anthropic_compatible: bool,
}

/// Inputs shared by per-runtime workspace binders.
pub struct WorkspaceBindInput<'a> {
    pub project_path: &'a Path,
    pub hermes_profile: &'a str,
    pub codex_home: &'a Path,
    pub openclaw_agent_id: &'a str,
    pub openclaw_workspace: &'a Path,
}

/// One doctor check that `workspace fix` can repair for this runtime.
pub struct WorkspaceCheckRepair {
    pub check_id: &'static str,
    /// Replaces the doctor detail in the plan. `None` keeps the doctor detail.
    pub preview: Option<&'static str>,
    repair: WorkspaceRepairFn,
}

impl WorkspaceCheckRepair {
    pub(crate) fn run(
        &self,
        input: &crate::workspace::repairs::WorkspaceRepairInput,
    ) -> Result<Option<crate::workspace::repairs::WorkspaceRepairOutcome>> {
        (self.repair)(input)
    }
}

const CLAUDE_MCP_PREVIEW: &str =
    "Will restore/scaffold .mcp.json and write .agent-doctor/claude-mcp-isolation.md \
                 (globals kept — pass --migrate-claude-mcp to merge into project .mcp.json)";
const HERMES_GATEWAY_PREVIEW: &str =
    "Will attempt Hermes/OpenClaw gateway restart (pass --restart-gateways)";

const OPENCLAW_REPAIRS: &[WorkspaceCheckRepair] = &[
    WorkspaceCheckRepair {
        check_id: "workspace.openclaw.workspace",
        preview: None,
        repair: repair_openclaw_workspace,
    },
    WorkspaceCheckRepair {
        check_id: "workspace.openclaw.routing.default",
        preview: None,
        repair: repair_openclaw_workspace,
    },
    WorkspaceCheckRepair {
        check_id: "workspace.openclaw.routing.defaults_workspace",
        preview: None,
        repair: repair_openclaw_workspace,
    },
    WorkspaceCheckRepair {
        check_id: "workspace.openclaw.agent_env",
        preview: None,
        repair: repair_openclaw_agent_env,
    },
];

const HERMES_REPAIRS: &[WorkspaceCheckRepair] = &[
    WorkspaceCheckRepair {
        check_id: "workspace.hermes.profile",
        preview: None,
        repair: repair_hermes_profile,
    },
    WorkspaceCheckRepair {
        check_id: "workspace.hermes.gateway_mismatch",
        preview: Some(HERMES_GATEWAY_PREVIEW),
        repair: repair_hermes_gateway,
    },
];

const CLAUDE_REPAIRS: &[WorkspaceCheckRepair] = &[
    WorkspaceCheckRepair {
        check_id: "workspace.claude.project_mcp",
        preview: Some(CLAUDE_MCP_PREVIEW),
        repair: repair_claude_project_mcp,
    },
    WorkspaceCheckRepair {
        check_id: "workspace.claude.global_mcp",
        preview: Some(CLAUDE_MCP_PREVIEW),
        repair: repair_claude_project_mcp,
    },
];

const CODEX_REPAIRS: &[WorkspaceCheckRepair] = &[
    WorkspaceCheckRepair {
        check_id: "workspace.codex.home",
        preview: None,
        repair: repair_codex_home,
    },
    WorkspaceCheckRepair {
        check_id: "workspace.codex.global_memory",
        preview: None,
        repair: repair_codex_home,
    },
    WorkspaceCheckRepair {
        check_id: "workspace.codex.isolation_marker",
        preview: None,
        repair: repair_codex_home,
    },
    WorkspaceCheckRepair {
        check_id: "workspace.codex.shared_global_home",
        preview: None,
        repair: repair_codex_home,
    },
];

#[derive(Clone, Copy)]
pub struct RuntimeDescriptor {
    pub id: &'static str,
    pub probe: RuntimeProbeSpec,
    create_adapter: AdapterFactory,
    schema_probe: Option<SchemaProbeFn>,
    deep_probe: Option<DeepProbeFn>,
    suggest_repairs: Option<SuggestRepairsFn>,
    /// Filtered repair. `None` means this runtime has no playbook.
    apply_playbook: Option<ApplyPlaybookFn>,
    run_lifecycle: Option<RunLifecycleFn>,
    /// `None` means personal/team gateway wiring is not supported.
    wiring: Option<WiringSpec>,
    /// `None` means Ask is not supported.
    ask: Option<AskSession>,
    /// `None` means project isolation is not supported.
    workspace_bind: Option<WorkspaceBindFn>,
    /// Doctor checks this runtime can repair. Empty means workspace fix skips it.
    workspace_repairs: &'static [WorkspaceCheckRepair],
    /// Short name for desktop chips. Full product name stays on the adapter.
    label: &'static str,
    open_session: Option<OpenSessionFn>,
    open_session_kind: Option<OpenSessionKind>,
    /// Evotown `job.assign`. `None` means this runtime is not dispatched.
    dispatch: Option<DispatchJobFn>,
    /// Skills can be mounted onto this runtime.
    skill_mount: bool,
    /// Browser MCP can be written into this runtime's config.
    browser_mcp: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeCatalogEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub ask: bool,
    pub opens_app: bool,
    pub browser_mcp: bool,
    pub skill_mount: bool,
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

    pub(crate) fn wiring(&self) -> Option<WiringSpec> {
        self.wiring
    }

    pub(crate) fn supports_workspace_bind(&self) -> bool {
        self.workspace_bind.is_some()
    }
}

const OPENCLAW_ENV: &[&str] = &["OPENCLAW", "EVOTOWN", "OPENAI", "ANTHROPIC"];
const CLAUDE_ENV: &[&str] = &["ANTHROPIC", "CLAUDE"];
const CODEX_ENV: &[&str] = &["OPENAI", "CODEX"];
const HERMES_ENV: &[&str] = &["HERMES", "OPENAI", "ANTHROPIC", "DEEPSEEK", "GOOGLE"];
const DEEPSEEK_HARNESS_ENV: &[&str] = &["DSH", "DEEPSEEK"];
const QODER_ENV: &[&str] = &["QODER"];
const WORKBUDDY_ENV: &[&str] = &["CODEBUDDY", "WORKBUDDY", "TENCENT"];
const CURSOR_ENV: &[&str] = &["CURSOR"];

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

fn deepseek_harness_adapter() -> Box<dyn RuntimeAdapter> {
    Box::new(DeepSeekHarnessAdapter)
}

fn qoder_adapter() -> Box<dyn RuntimeAdapter> {
    Box::new(QoderAdapter)
}

fn workbuddy_adapter() -> Box<dyn RuntimeAdapter> {
    Box::new(WorkbuddyAdapter)
}

fn cursor_adapter() -> Box<dyn RuntimeAdapter> {
    Box::new(CursorAdapter)
}

const WIRE_GATEWAY: WiringSpec = WiringSpec {
    write_semantics: WriteSemantics::Additive,
    effector: EffectorKind::RestartGateway,
    openai_compatible: true,
    anthropic_compatible: false,
};

const WIRE_CODEX: WiringSpec = WiringSpec {
    write_semantics: WriteSemantics::Additive,
    effector: EffectorKind::ManualRestart,
    openai_compatible: true,
    anthropic_compatible: false,
};

const WIRE_CLAUDE: WiringSpec = WiringSpec {
    write_semantics: WriteSemantics::Exclusive,
    effector: EffectorKind::None,
    openai_compatible: false,
    anthropic_compatible: true,
};

static OPENCLAW_ASK: OpenClawAskBackend = OpenClawAskBackend;
static HERMES_ASK: HermesAskBackend = HermesAskBackend;
static DEEPSEEK_HARNESS_ASK: DeepSeekHarnessAskBackend = DeepSeekHarnessAskBackend;
static CLAUDE_ASK: ClaudeAskBackend = ClaudeAskBackend;
static CODEX_ASK: CodexAskBackend = CodexAskBackend;

fn bind_openclaw_workspace(input: &WorkspaceBindInput) -> Result<RuntimeBindReport> {
    bind_openclaw(input.openclaw_agent_id, input.openclaw_workspace)
}

fn bind_hermes_workspace(input: &WorkspaceBindInput) -> Result<RuntimeBindReport> {
    bind_hermes(input.hermes_profile, input.project_path)
}

fn bind_claude_workspace(input: &WorkspaceBindInput) -> Result<RuntimeBindReport> {
    bind_claude_code(input.project_path)
}

fn bind_codex_workspace(input: &WorkspaceBindInput) -> Result<RuntimeBindReport> {
    bind_codex_for_project(input.codex_home, Some(input.project_path))
}

fn open_openclaw_session(
    cwd: &Path,
    prompt: Option<&str>,
    _prefer_deep_link: bool,
) -> Result<OpenSessionReport> {
    open_in_terminal("openclaw", &["openclaw", "tui"], cwd, prompt)
}

fn open_hermes_session(
    cwd: &Path,
    prompt: Option<&str>,
    _prefer_deep_link: bool,
) -> Result<OpenSessionReport> {
    open_in_terminal("hermes", &["hermes"], cwd, prompt)
}

fn open_deepseek_session(
    cwd: &Path,
    _prompt: Option<&str>,
    _prefer_deep_link: bool,
) -> Result<OpenSessionReport> {
    open_in_terminal("deepseek-harness", &["dsh", "web"], cwd, None)
}

fn open_claude_session(
    cwd: &Path,
    prompt: Option<&str>,
    prefer_deep_link: bool,
) -> Result<OpenSessionReport> {
    open_claude_code(cwd, prompt, prefer_deep_link)
}

fn open_codex_session(
    cwd: &Path,
    prompt: Option<&str>,
    _prefer_deep_link: bool,
) -> Result<OpenSessionReport> {
    open_codex(cwd, prompt)
}

fn open_qoder_session(
    cwd: &Path,
    prompt: Option<&str>,
    _prefer_deep_link: bool,
) -> Result<OpenSessionReport> {
    open_in_terminal("qoder", &["qoder"], cwd, prompt)
}

fn open_workbuddy_session(
    cwd: &Path,
    prompt: Option<&str>,
    _prefer_deep_link: bool,
) -> Result<OpenSessionReport> {
    open_in_terminal("workbuddy", &["codebuddy"], cwd, prompt)
}

fn open_cursor_session(
    cwd: &Path,
    _prompt: Option<&str>,
    _prefer_deep_link: bool,
) -> Result<OpenSessionReport> {
    open_cursor_app(cwd)
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

fn run_deepseek_harness_lifecycle_action(action: RuntimeLifecycleAction) -> Result<()> {
    let action = match action {
        RuntimeLifecycleAction::Install => DeepSeekHarnessLifecycleAction::Install,
        RuntimeLifecycleAction::Update => DeepSeekHarnessLifecycleAction::Update,
    };
    run_deepseek_harness_lifecycle(action)
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

fn run_qoder_lifecycle_action(action: RuntimeLifecycleAction) -> Result<()> {
    let action = match action {
        RuntimeLifecycleAction::Install => QoderLifecycleAction::Install,
        RuntimeLifecycleAction::Update => QoderLifecycleAction::Update,
    };
    run_qoder_lifecycle(action)
}

fn run_workbuddy_lifecycle_action(action: RuntimeLifecycleAction) -> Result<()> {
    let action = match action {
        RuntimeLifecycleAction::Install => WorkbuddyLifecycleAction::Install,
        RuntimeLifecycleAction::Update => WorkbuddyLifecycleAction::Update,
    };
    run_workbuddy_lifecycle(action)
}

fn run_cursor_lifecycle_action(action: RuntimeLifecycleAction) -> Result<()> {
    let action = match action {
        RuntimeLifecycleAction::Install => CursorLifecycleAction::Install,
        RuntimeLifecycleAction::Update => CursorLifecycleAction::Update,
    };
    run_cursor_lifecycle(action)
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
        apply_playbook: Some(apply_openclaw_playbook_filtered),
        run_lifecycle: Some(run_openclaw_lifecycle_action),
        wiring: Some(WIRE_GATEWAY),
        ask: Some(&OPENCLAW_ASK),
        workspace_bind: Some(bind_openclaw_workspace),
        workspace_repairs: OPENCLAW_REPAIRS,
        label: "OpenClaw",
        open_session: Some(open_openclaw_session),
        open_session_kind: Some(OpenSessionKind::Terminal),
        dispatch: Some(run_openclaw_hook),
        skill_mount: true,
        browser_mcp: true,
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
        apply_playbook: Some(apply_hermes_playbook_filtered),
        run_lifecycle: Some(run_hermes_lifecycle_action),
        wiring: Some(WIRE_GATEWAY),
        ask: Some(&HERMES_ASK),
        workspace_bind: Some(bind_hermes_workspace),
        workspace_repairs: HERMES_REPAIRS,
        label: "Hermes",
        open_session: Some(open_hermes_session),
        open_session_kind: Some(OpenSessionKind::Terminal),
        dispatch: Some(run_hermes_hook),
        skill_mount: true,
        browser_mcp: true,
    },
    RuntimeDescriptor {
        id: "deepseek-harness",
        probe: RuntimeProbeSpec {
            binary_name: "dsh",
            config_format: ConfigFormat::Yaml,
            env_keywords: DEEPSEEK_HARNESS_ENV,
        },
        create_adapter: deepseek_harness_adapter,
        schema_probe: Some(schema_deepseek_harness),
        deep_probe: Some(deepseek_harness_probe_deep),
        suggest_repairs: Some(suggest_deepseek_harness_repairs),
        apply_playbook: Some(apply_deepseek_harness_playbook_filtered),
        run_lifecycle: Some(run_deepseek_harness_lifecycle_action),
        wiring: None,
        ask: Some(&DEEPSEEK_HARNESS_ASK),
        workspace_bind: None,
        workspace_repairs: &[],
        label: "DeepSeek",
        open_session: Some(open_deepseek_session),
        open_session_kind: Some(OpenSessionKind::Terminal),
        dispatch: None,
        skill_mount: true,
        browser_mcp: true,
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
        deep_probe: Some(claude_code_probe_deep),
        suggest_repairs: Some(suggest_claude_code_repairs),
        apply_playbook: Some(apply_claude_code_playbook_filtered),
        run_lifecycle: Some(run_claude_code_lifecycle_action),
        wiring: Some(WIRE_CLAUDE),
        ask: Some(&CLAUDE_ASK),
        workspace_bind: Some(bind_claude_workspace),
        workspace_repairs: CLAUDE_REPAIRS,
        label: "Claude",
        open_session: Some(open_claude_session),
        open_session_kind: Some(OpenSessionKind::DeepLink),
        dispatch: Some(run_claude_cli),
        skill_mount: true,
        browser_mcp: true,
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
        deep_probe: Some(codex_probe_deep),
        suggest_repairs: Some(suggest_codex_repairs),
        apply_playbook: Some(apply_codex_playbook_filtered),
        run_lifecycle: Some(run_codex_lifecycle_action),
        wiring: Some(WIRE_CODEX),
        ask: Some(&CODEX_ASK),
        workspace_bind: Some(bind_codex_workspace),
        workspace_repairs: CODEX_REPAIRS,
        label: "Codex",
        open_session: Some(open_codex_session),
        open_session_kind: Some(OpenSessionKind::Terminal),
        dispatch: Some(run_codex_cli),
        skill_mount: true,
        browser_mcp: true,
    },
    RuntimeDescriptor {
        id: "qoder",
        probe: RuntimeProbeSpec {
            binary_name: "qoder",
            config_format: ConfigFormat::Json,
            env_keywords: QODER_ENV,
        },
        create_adapter: qoder_adapter,
        schema_probe: Some(schema_qoder),
        deep_probe: Some(probe_deep_noop),
        suggest_repairs: Some(suggest_qoder_repairs),
        apply_playbook: Some(apply_qoder_playbook_filtered),
        run_lifecycle: Some(run_qoder_lifecycle_action),
        wiring: None,
        ask: None,
        workspace_bind: None,
        workspace_repairs: &[],
        label: "Qoder",
        open_session: Some(open_qoder_session),
        open_session_kind: Some(OpenSessionKind::Terminal),
        dispatch: None,
        skill_mount: false,
        browser_mcp: false,
    },
    RuntimeDescriptor {
        id: "workbuddy",
        probe: RuntimeProbeSpec {
            binary_name: "codebuddy",
            config_format: ConfigFormat::Json,
            env_keywords: WORKBUDDY_ENV,
        },
        create_adapter: workbuddy_adapter,
        schema_probe: Some(schema_workbuddy),
        deep_probe: Some(probe_deep_noop),
        suggest_repairs: Some(suggest_workbuddy_repairs),
        apply_playbook: Some(apply_workbuddy_playbook_filtered),
        run_lifecycle: Some(run_workbuddy_lifecycle_action),
        wiring: None,
        ask: None,
        workspace_bind: None,
        workspace_repairs: &[],
        label: "WorkBuddy",
        open_session: Some(open_workbuddy_session),
        open_session_kind: Some(OpenSessionKind::Terminal),
        dispatch: None,
        skill_mount: false,
        browser_mcp: false,
    },
    RuntimeDescriptor {
        id: "cursor",
        probe: RuntimeProbeSpec {
            binary_name: "agent",
            config_format: ConfigFormat::Json,
            env_keywords: CURSOR_ENV,
        },
        create_adapter: cursor_adapter,
        schema_probe: Some(schema_cursor),
        deep_probe: Some(probe_deep_noop),
        suggest_repairs: Some(suggest_cursor_repairs),
        apply_playbook: Some(apply_cursor_playbook_filtered),
        run_lifecycle: Some(run_cursor_lifecycle_action),
        wiring: None,
        ask: None,
        workspace_bind: None,
        workspace_repairs: &[],
        label: "Cursor",
        open_session: Some(open_cursor_session),
        open_session_kind: Some(OpenSessionKind::App),
        dispatch: None,
        skill_mount: true,
        browser_mcp: false,
    },
];

pub fn all_runtime_ids() -> impl Iterator<Item = &'static str> {
    RUNTIME_REGISTRY.iter().map(|entry| entry.id)
}

pub fn descriptor_by_id(runtime_id: &str) -> Option<&'static RuntimeDescriptor> {
    let runtime_id = match runtime_id.trim().to_ascii_lowercase().as_str() {
        "dsh" | "deepseek" | "deepseek_harness" => "deepseek-harness",
        "cursor-cli" | "cursor-agent" => "cursor",
        _ => runtime_id,
    };
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
    let mut items = suggest_config_syntax_repairs(probe);
    items.extend(
        descriptor_by_id(runtime_id)
            .and_then(|entry| entry.suggest_repairs)
            .map(|suggest| suggest(probe))
            .unwrap_or_default(),
    );

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

    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(item.id.clone()));
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
    let descriptor = descriptor_by_id(runtime_id);
    let runtime_id = descriptor.map(|entry| entry.id).unwrap_or(runtime_id);
    let apply = descriptor
        .and_then(|entry| entry.apply_playbook)
        .with_context(|| format!("runtime '{runtime_id}' has no repair playbook"))?;
    let mut result = apply_config_syntax_repair(probe, only_ids);
    let runtime = apply(probe, only_ids)?;
    result.executed.extend(runtime.executed);
    result.skipped.extend(runtime.skipped);
    result.guide_path = runtime.guide_path;
    Ok(result)
}

pub(crate) fn ask_backend(runtime_id: &str) -> Option<AskSession> {
    descriptor_by_id(runtime_id).and_then(|entry| entry.ask)
}

pub(crate) fn ask_runtime_ids() -> Vec<&'static str> {
    RUNTIME_REGISTRY
        .iter()
        .filter(|entry| entry.ask.is_some())
        .map(|entry| entry.id)
        .collect()
}

pub(crate) fn find_workspace_repair(check_id: &str) -> Option<&'static WorkspaceCheckRepair> {
    RUNTIME_REGISTRY.iter().find_map(|entry| {
        entry
            .workspace_repairs
            .iter()
            .find(|repair| repair.check_id == check_id)
    })
}

pub(crate) fn bind_workspace_runtimes(
    input: &WorkspaceBindInput,
) -> Result<Vec<RuntimeBindReport>> {
    let mut reports = Vec::new();
    for entry in RUNTIME_REGISTRY {
        if let Some(bind) = entry.workspace_bind {
            reports.push(bind(input)?);
        }
    }
    Ok(reports)
}

pub(crate) fn open_session(runtime_id: &str) -> Option<OpenSessionFn> {
    descriptor_by_id(runtime_id).and_then(|entry| entry.open_session)
}

pub(crate) fn dispatch_job(runtime_id: &str) -> Option<DispatchJobFn> {
    descriptor_by_id(runtime_id).and_then(|entry| entry.dispatch)
}

pub(crate) fn dispatch_runtime_ids() -> Vec<&'static str> {
    RUNTIME_REGISTRY
        .iter()
        .filter(|entry| entry.dispatch.is_some())
        .map(|entry| entry.id)
        .collect()
}

pub(crate) fn skill_mount_runtime_ids() -> Vec<&'static str> {
    RUNTIME_REGISTRY
        .iter()
        .filter(|entry| entry.skill_mount)
        .map(|entry| entry.id)
        .collect()
}

pub fn runtime_catalog() -> Vec<RuntimeCatalogEntry> {
    RUNTIME_REGISTRY
        .iter()
        .map(|entry| RuntimeCatalogEntry {
            id: entry.id,
            label: entry.label,
            ask: entry.ask.is_some(),
            opens_app: entry.open_session_kind == Some(OpenSessionKind::App),
            browser_mcp: entry.browser_mcp,
            skill_mount: entry.skill_mount,
        })
        .collect()
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
        assert_eq!(
            ids,
            vec![
                "openclaw",
                "hermes",
                "deepseek-harness",
                "claude-code",
                "codex",
                "qoder",
                "workbuddy",
                "cursor"
            ]
        );
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
    fn deepseek_harness_entry_is_fully_wired_and_aliases_resolve() {
        let runtime = descriptor_by_id("deepseek-harness").expect("deepseek-harness");
        assert_eq!(runtime.probe.binary_name, "dsh");
        assert!(runtime.schema_probe.is_some());
        assert!(runtime.deep_probe.is_some());
        assert!(runtime.suggest_repairs.is_some());
        assert!(runtime.apply_playbook.is_some());
        assert!(runtime.run_lifecycle.is_some());
        assert_eq!(
            descriptor_by_id("dsh").map(|item| item.id),
            Some(runtime.id)
        );
    }

    #[test]
    fn claude_and_codex_wire_lifecycle_playbook_and_deep_probe() {
        let claude = descriptor_by_id("claude-code").expect("claude-code");
        let codex = descriptor_by_id("codex").expect("codex");
        assert!(claude.run_lifecycle.is_some());
        assert!(codex.run_lifecycle.is_some());
        assert!(claude.deep_probe.is_some());
        assert!(codex.deep_probe.is_some());
        assert!(runtime_supports_lifecycle("claude-code"));
        assert!(runtime_supports_lifecycle("codex"));
        assert!(runtime_supports_playbook("claude-code"));
        assert!(runtime_supports_playbook("codex"));
        assert!(claude.suggest_repairs.is_some());
        assert!(codex.apply_playbook.is_some());
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

    fn json_parse_fail_probe(runtime_id: &str, path: &Path) -> RuntimeProbeReport {
        RuntimeProbeReport {
            runtime_id: runtime_id.to_string(),
            display_name: runtime_id.to_string(),
            binary_name: runtime_id.to_string(),
            checks: vec![ProbeCheck::new(
                format!("config.parse:{}", path.display()),
                "Config parse",
                ProbeStatus::Fail,
                ProbeSeverity::Error,
                "invalid JSON: trailing comma",
                crate::repair::SensitivityLevel::SensitiveLog,
            )],
            facts: Vec::new(),
        }
    }

    #[test]
    fn every_runtime_suggests_shared_config_syntax_fix() {
        for entry in RUNTIME_REGISTRY {
            let probe = json_parse_fail_probe(entry.id, Path::new("/x/settings.json"));
            let items = suggest_runtime_repairs(entry.id, &probe);
            assert!(
                items
                    .iter()
                    .any(|item| item.id == "fix-config-syntax" && item.auto_fixable),
                "{} missing fix-config-syntax",
                entry.id
            );
        }
    }

    #[test]
    fn registry_apply_runs_shared_config_syntax_fix() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("settings.json");
        std::fs::write(&path, "{\"model\": \"x\",}").unwrap();
        let probe = json_parse_fail_probe("claude-code", &path);
        let only = vec!["fix-config-syntax".to_string()];
        let result = apply_runtime_playbook_filtered("claude-code", &probe, Some(&only)).unwrap();
        assert!(result.executed.iter().any(|id| id == "fix-config-syntax"));
        let fixed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(fixed, serde_json::json!({"model": "x"}));
    }

    #[test]
    fn every_runtime_has_schema_probe() {
        for entry in RUNTIME_REGISTRY {
            assert!(
                entry.schema_probe.is_some(),
                "{} missing schema_probe",
                entry.id
            );
            assert!(
                entry.apply_playbook.is_some(),
                "{} missing filtered playbook",
                entry.id
            );
        }
    }

    #[test]
    fn optional_capabilities_live_on_the_descriptor() {
        let wired: Vec<_> = RUNTIME_REGISTRY
            .iter()
            .filter(|entry| entry.wiring.is_some())
            .map(|entry| entry.id)
            .collect();
        assert_eq!(wired, vec!["openclaw", "hermes", "claude-code", "codex"]);
        let ask: Vec<_> = RUNTIME_REGISTRY
            .iter()
            .filter(|entry| entry.ask.is_some())
            .map(|entry| entry.id)
            .collect();
        assert_eq!(
            ask,
            vec![
                "openclaw",
                "hermes",
                "deepseek-harness",
                "claude-code",
                "codex"
            ]
        );
        let workspace: Vec<_> = RUNTIME_REGISTRY
            .iter()
            .filter(|entry| entry.workspace_bind.is_some())
            .map(|entry| entry.id)
            .collect();
        assert_eq!(
            workspace,
            vec!["openclaw", "hermes", "claude-code", "codex"]
        );
        assert!(descriptor_by_id("qoder").unwrap().wiring.is_none());
        assert!(descriptor_by_id("workbuddy").unwrap().ask.is_none());
        assert!(descriptor_by_id("cursor").unwrap().workspace_bind.is_none());
        let mut repair_ids: Vec<_> = RUNTIME_REGISTRY
            .iter()
            .flat_map(|entry| entry.workspace_repairs.iter().map(|repair| repair.check_id))
            .collect();
        repair_ids.sort_unstable();
        let mut unique = repair_ids.clone();
        unique.dedup();
        assert_eq!(
            repair_ids, unique,
            "workspace repair check ids must be unique"
        );
        assert!(RUNTIME_REGISTRY.iter().all(|entry| {
            entry.workspace_bind.is_some() == !entry.workspace_repairs.is_empty()
        }));
        assert!(find_workspace_repair("workspace.cwd.mismatch").is_none());
        assert!(find_workspace_repair("workspace.codex.home").is_some());
        for entry in RUNTIME_REGISTRY {
            let marker = if entry.id == "claude-code" {
                ".claude.".to_string()
            } else {
                format!(".{}.", entry.id)
            };
            for repair in entry.workspace_repairs {
                assert!(
                    repair.check_id.contains(&marker),
                    "{} owns a repair for another runtime: {}",
                    entry.id,
                    repair.check_id
                );
            }
        }
        assert!(descriptor_by_id("deepseek-harness")
            .unwrap()
            .wiring
            .is_none());
        assert_eq!(
            dispatch_runtime_ids(),
            vec!["openclaw", "hermes", "claude-code", "codex"]
        );
        assert!(RUNTIME_REGISTRY
            .iter()
            .all(|entry| entry.open_session.is_some()));
        assert_eq!(
            descriptor_by_id("cursor").unwrap().open_session_kind,
            Some(OpenSessionKind::App)
        );
        assert_eq!(descriptor_by_id("claude-code").unwrap().label, "Claude");
        let catalog = runtime_catalog();
        assert_eq!(catalog.len(), RUNTIME_REGISTRY.len());
        assert!(catalog
            .iter()
            .any(|entry| entry.id == "codex" && entry.browser_mcp && entry.ask));
        let mut browser_ids: Vec<_> = RUNTIME_REGISTRY
            .iter()
            .filter(|entry| entry.browser_mcp)
            .map(|entry| entry.id)
            .collect();
        let mut listed = crate::BROWSER_MCP_WIRE_RUNTIMES.to_vec();
        browser_ids.sort_unstable();
        listed.sort_unstable();
        assert_eq!(browser_ids, listed);
        assert!(catalog
            .iter()
            .any(|entry| entry.id == "qoder" && !entry.skill_mount && !entry.ask));
    }
}
