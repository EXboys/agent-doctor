pub mod adapter;
pub mod adapters;
pub mod doctor;
pub mod evotown;
pub mod install;
pub mod lifecycle;
pub mod presets;
pub mod probe;
pub mod profile;
pub mod repair;
pub mod runtime;
pub mod session_launch;
pub mod setup;
pub mod workspace;

pub use adapter::{
    AdapterDiscovery, ApplyReport, RuntimeAdapter, RuntimeModelPreset, RuntimeModelState,
    RuntimeProfile,
};
pub use adapters::{CodexAdapter, HermesAdapter, HermesSettings, OpenClawAdapter};
pub use doctor::{run_doctor, DoctorReport, RuntimeDoctorResult};
pub use evotown::{
    build_inventory_payload, check_evotown_connectivity, evotown_status,
    execute_evotown_onboarding, execute_job, execute_policy_pull, execute_sync,
    is_known_dispatch_runtime, known_dispatch_runtimes, load_doctor_node_config,
    load_evotown_config, normalize_runtime, preferred_runtime_status, resolve_preferred_runtime,
    resolve_runtime, run_connect_loop, set_preferred_runtime, AssignedJob, ConnectOptions,
    DoctorNodeConfig, EvotownConfig, EvotownHealthReport, EvotownStatus, JobResult,
    OnboardingOptions, OnboardingReport, PolicyPullReport, PreferredRuntimeStatus, SyncOptions,
    SyncReport,
};
pub use install::{
    build_explain_input, execute_install, execute_install_with_progress, needs_binary_install,
    InstallOptions, InstallProgressEvent, InstallReport,
};
pub use lifecycle::{
    hermes_shell_command, openclaw_shell_command, run_hermes_lifecycle, run_openclaw_lifecycle,
    HermesLifecycleAction, OpenClawLifecycleAction,
};
pub use presets::{
    apply_profile_model, default_local_hermes_preset, default_work_models, effective_models,
    init_example_profiles, load_profiles, merge_builtin_profiles, profiles_path, set_runtime_model,
    show_config, use_profile, HermesProfilePreset, ProfileEntry, ProfilesDocument,
    UseProfileReport,
};
pub use probe::{
    probe_all_runtimes, probe_runtime, ProbeCheck, ProbeSeverity, ProbeStatus, RuntimeProbeReport,
};
pub use profile::{
    agent_profile_path, company_baseline_path, read_agent_profile, read_company_baseline,
    read_company_profile, AgentProfile, CompanyProfile, ProviderKind,
};
pub use repair::{
    allowed_paths_for_runtime, apply_hermes_playbook, apply_hermes_playbook_filtered,
    build_repair_preview, build_repair_preview_from_bundle, execute_repair, execute_repair_loop,
    explain_runtime, list_runtime_backup_ids, mask_secret_value, merge_env_with_vault,
    probe_health_summary, probe_issue_score, restore_runtime_backup, suggest_hermes_repairs,
    unmask_file_content, AiRepairPlanner, AuditReport, BackupSnapshot, DeterministicPlanner,
    DiagnosticBundle, DiagnosticFact, ExplainCheck, ExplainInput, ExplainInstallFailure,
    ExplainReport, ExplainSuggestion, LlmConfig, MaskedFileSnippet, MaskedRepairContext,
    PlannerOptions, PlannerResult, PlaybookApplyResult, RedactedFact, RedactionPolicy, Redactor,
    RepairAction, RepairActionKind, RepairExecuteOptions, RepairExecuteReport, RepairLoopOptions,
    RepairLoopReport, RepairLoopRound, RepairPlan, RepairPlanner, RepairRisk, RepairToolCall,
    RepairToolExecutor, RepairToolKind, RepairToolResult, RestoreReport, SecretVault,
    SensitivityLevel, SkippedRepairAction, SnapshotFile, SuggestedRepair,
};
pub use runtime::{adapter_by_id, all_adapters};
pub use runtime::{
    all_runtime_ids, apply_runtime_playbook, apply_runtime_playbook_filtered, descriptor_by_id,
    run_runtime_lifecycle, runtime_supports_lifecycle, runtime_supports_playbook,
    suggest_runtime_repairs, RuntimeDescriptor, RuntimeLifecycleAction, RuntimeProbeSpec,
};
pub use session_launch::{
    claude_cli_deep_link, open_interactive_session, resolve_session_cwd, OpenSessionMethod,
    OpenSessionOptions, OpenSessionReport,
};
pub use setup::{
    activate_personal_provider, anthropic_gateway_url_from_evotown_base, apply_codex_slot,
    apply_hermes_slot, apply_mode_switch, apply_openclaw_slot, clear_codex_placeholder_auth,
    delete_personal_provider, effector_label, evotown_agent_env_path, evotown_base_from_gateway,
    execute_personal_provider_setup, execute_setup, gateway_url_from_evotown_base,
    list_personal_providers, load_mode_status, load_personal_provider_status,
    normalize_personal_gateway_url, normalize_protocol, probe_endpoint_bundle, project_bundle,
    runtime_strategies, strategy_for, switch_to_personal_mode, switch_to_team_mode,
    upsert_personal_provider, verify_personal_provider, verify_personal_provider_with_protocol,
    write_evotown_agent_env, BundleProbeReport, EffectorKind, EndpointBundle, ModeStatus,
    ModeSwitchReport, ModeSwitchTarget, PersonalProviderListItem, PersonalProviderOptions,
    PersonalProviderSetupReport, PersonalProviderStatus, PersonalProviderVerifyReport,
    PersonalProvidersDocument, RuntimeSetupResult, RuntimeStrategy, SetupOptions, SetupReport,
    UpsertPersonalProviderOptions, WriteSemantics, CODEX_PERSONAL_SLOT, CODEX_TEAM_SLOT,
    COMPANY_DEFAULT_MODEL, HERMES_PERSONAL_SLOT, HERMES_TEAM_SLOT, MODE_PERSONAL, MODE_TEAM,
    MODE_UNSET, OPENCLAW_PERSONAL_SLOT, OPENCLAW_PROVIDER_ID, OPENCLAW_TEAM_SLOT,
    PROTOCOL_ANTHROPIC, PROTOCOL_OPENAI,
};
pub use workspace::{
    active_env_path, bash_hook_file_path, enter_workspace, fish_hook_file_path, hook_file_path,
    init_workspace, install_bash_hook, install_fish_hook, install_powershell_hook,
    install_zsh_hook, load_workspaces, match_workspace_for_path,
    migrate_claude_global_mcp_to_project, powershell_hook_file_path, remove_workspace,
    render_direnv_envrc, render_shell_env, render_shell_env_for_name, save_workspaces,
    use_workspace, use_workspace_with_options, workspace_capability_matrix, workspace_doctor,
    workspace_fix, workspace_hook_status, workspace_show, workspace_status, workspaces_path,
    write_direnv_envrc, CapabilityCell, CapabilityMatrix, ClaudeMcpMigrationReport,
    EnterWorkspaceReport, GatewayRestartReport, InitWorkspaceReport, ShellHookStatus,
    UseWorkspaceOptions, UseWorkspaceReport, WorkspaceCheck, WorkspaceCheckStatus,
    WorkspaceDoctorReport, WorkspaceEntry, WorkspaceFixAction, WorkspaceFixOptions,
    WorkspaceFixReport, WorkspaceShowReport, WorkspaceSnapshotStatus, WorkspaceStatusReport,
    WorkspacesDocument,
};
