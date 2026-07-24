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
pub mod setup;
pub mod workspace;

pub use adapter::{
    AdapterDiscovery, ApplyReport, RuntimeAdapter, RuntimeModelPreset, RuntimeModelState,
    RuntimeProfile,
};
pub use adapters::{CodexAdapter, HermesAdapter, HermesSettings, OpenClawAdapter};
pub use doctor::{run_doctor, DoctorReport, RuntimeDoctorResult};
pub use evotown::{
    build_inventory_payload, check_evotown_connectivity, execute_job, evotown_status,
    execute_evotown_onboarding, execute_policy_pull, execute_sync, load_doctor_node_config,
    load_evotown_config, resolve_runtime, run_connect_loop, AssignedJob, ConnectOptions,
    DoctorNodeConfig, EvotownConfig, EvotownHealthReport, EvotownStatus, JobResult,
    OnboardingOptions, OnboardingReport, PolicyPullReport, SyncOptions, SyncReport,
};
pub use install::{
    build_explain_input, execute_install, needs_binary_install, InstallOptions, InstallReport,
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
pub use profile::{agent_profile_path, read_company_profile, CompanyProfile};
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
pub use setup::{
    evotown_agent_env_path, evotown_base_from_gateway, execute_setup,
    gateway_url_from_evotown_base, write_evotown_agent_env, RuntimeSetupResult, SetupOptions,
    SetupReport,
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
