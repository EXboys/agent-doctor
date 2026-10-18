pub mod adapter;
pub mod adapters;
pub mod browser_wire;
pub mod doctor;
pub mod edition;
pub mod evotown;
pub mod exec;
pub mod install;
pub mod knowledge_sync;
pub mod lifecycle;
pub mod presets;
pub mod probe;
pub mod profile;
pub mod prompt_session;
pub mod remote;
pub mod repair;
pub mod runtime;
pub mod session_launch;
pub mod setup;
pub mod skills;
pub mod store;
pub mod usage;
pub mod version_check;
pub mod workspace;

/// Named pipe the desktop app serves. The computer tool connects here on Windows.
pub const COMPUTER_HAND_PIPE: &str = r"\\.\pipe\agent-doctor-computer";

/// MCP config Claude reads only when this app launches it.
///
/// It stays out of the project `.mcp.json`, so Claude the person opens in that
/// folder does not see a computer tool that only works while this app is open.
pub fn claude_computer_mcp_config_path() -> Option<std::path::PathBuf> {
    adapters::util::config_dir().map(|dir| {
        dir.join("agent-doctor")
            .join("ask")
            .join("claude-computer-mcp.json")
    })
}

/// Remove a `computer` server from a project `.mcp.json`. Other servers stay.
/// A file that does not parse is left untouched.
pub fn forget_project_computer_mcp(project_dir: &std::path::Path) -> bool {
    let path = project_dir.join(".mcp.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(mut doc) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    let Some(servers) = doc.get_mut("mcpServers").and_then(|v| v.as_object_mut()) else {
        return false;
    };
    if servers.remove("computer").is_none() {
        return false;
    }
    let Ok(updated) = serde_json::to_string_pretty(&doc) else {
        return false;
    };
    std::fs::write(&path, format!("{updated}\n")).is_ok()
}

/// Probing a broken program (a placeholder `.exe` left by a failed download)
/// makes Windows pop a "16-bit application" dialog over the app. Children
/// inherit this mode, so the start of a failed spawn becomes an error code.
pub fn quiet_system_error_dialogs() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Diagnostics::Debug::{
            SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX,
        };
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX);
    }
}

pub use adapter::{
    AdapterDiscovery, ApplyReport, RuntimeAdapter, RuntimeModelPreset, RuntimeModelState,
    RuntimeProfile,
};
pub use adapters::{
    refresh_managed_runtime_path, CodexAdapter, DeepSeekHarnessAdapter, HermesAdapter,
    HermesSettings, OpenClawAdapter, DEEPSEEK_API_KEY_ENV, DEEPSEEK_BASE_URL_ENV,
    DEEPSEEK_HARNESS_CLI, DEEPSEEK_HARNESS_NPM_PACKAGE, DEEPSEEK_HARNESS_RUNTIME_ID,
    DEEPSEEK_HARNESS_VERSION,
};
pub use browser_wire::{
    install_browser_mcp_wire_backend, BrowserMcpWireBackend, BrowserMcpWireReport,
    BrowserMcpWireResult, WireBrowserMcpOptions, BROWSER_MCP_WIRE_RUNTIMES,
    DEFAULT_BROWSER_MCP_PORT,
};
pub use doctor::{run_doctor, DoctorReport, RuntimeDoctorResult};
pub use edition::{ensure_edition_allows_mode, product_edition, ProductEdition};
pub use evotown::{
    build_inventory_payload, check_evotown_connectivity, evotown_status,
    execute_evotown_onboarding, execute_job, execute_policy_pull, execute_register, execute_sync,
    is_known_dispatch_runtime, known_dispatch_runtimes, list_skills_inventory,
    list_skills_inventory_with_options, load_doctor_node_config, load_evotown_config,
    mount_synced_skills, normalize_runtime, preferred_runtime_status, resolve_preferred_runtime,
    resolve_runtime, run_connect_loop, set_preferred_runtime, skill_mount_runtime_ids,
    unmount_synced_skills, AssignedJob, ConnectOptions, DoctorNodeConfig, EvotownConfig,
    EvotownHealthReport, EvotownStatus, JobResult, OnboardingOptions, OnboardingReport,
    PolicyPullReport, PreferredRuntimeStatus, RegisterOptions, RegisterReport, SkillAgentUsage,
    SkillInventoryItem, SkillMountOptions, SkillMountReport, SkillsInventoryOptions,
    SkillsInventoryReport, SyncOptions, SyncReport,
};
pub use exec::{ExecBackend, ExecOutput, LocalBackend};
pub use install::{
    build_explain_input, execute_install, execute_install_with_progress, needs_binary_install,
    InstallOptions, InstallProgressEvent, InstallReport,
};
pub use knowledge_sync::{sync_knowledge, KnowledgeSyncReport};
pub use lifecycle::{
    deepseek_harness_install_shell_command, deepseek_harness_shell_command,
    deepseek_harness_update_shell_command, hermes_shell_command, openclaw_shell_command,
    run_deepseek_harness_lifecycle, run_hermes_lifecycle, run_openclaw_lifecycle,
    run_shell_command_streaming, submit_install_line, uninstall_runtime, waiting_input_kind,
    DeepSeekHarnessLifecycleAction, HermesLifecycleAction, OpenClawLifecycleAction,
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
pub use prompt_session::{
    ask_image_support, enable_warm_sessions, retire_warm_sessions, run_prompt_session,
    run_prompt_session_with_cancel, shutdown_warm_sessions, AskImageSupport, PlanStep,
    PlanStepState, PromptSessionCancel, PromptSessionControl, PromptSessionEvent,
    PromptSessionOptions, PromptSessionReport, PromptSessionStatus,
};
pub use remote::{
    add_host, add_project, bootstrap_and_add_host, list_hosts, list_projects, load_remote_hosts,
    probe_remote_host, publish_managed_logins, remote_hosts_path, remote_key_path_for,
    remote_keys_dir, remote_root_dir, remove_host, remove_project, run_remote_doctor,
    run_remote_doctor_with_backend, save_remote_hosts, upsert_managed_host,
    write_remote_doctor_report, BootstrapHostOptions, RemoteDoctorOptions, RemoteDoctorReport,
    RemoteHostEntry, RemoteHostProbeReport, RemoteHostsDocument, RemoteProjectEntry,
    RemoteRuntimeDoctorResult, SshBackend, SshTarget,
};
pub use repair::{
    allowed_paths_for_runtime, apply_deepseek_harness_playbook,
    apply_deepseek_harness_playbook_filtered, apply_hermes_playbook,
    apply_hermes_playbook_filtered, build_repair_preview, build_repair_preview_from_bundle,
    diff_probe_checks, execute_repair, execute_repair_loop, explain_runtime,
    list_runtime_backup_ids, mask_secret_value, merge_env_with_vault, probe_health_summary,
    probe_issue_score, restore_runtime_backup, suggest_deepseek_harness_repairs,
    suggest_hermes_repairs, unmask_file_content, AiRepairPlanner, AuditReport, BackupSnapshot,
    CheckChange, CheckDiff, DeterministicPlanner, DiagnosticBundle, DiagnosticFact, ExplainCheck,
    ExplainInput, ExplainInstallFailure, ExplainReport, ExplainSuggestion, HoldBrowserToolsOff,
    LlmConfig, MaskedFileSnippet, MaskedRepairContext, PlannerOptions, PlannerResult,
    PlaybookApplyResult, RedactedFact, RedactionPolicy, Redactor, RepairAction, RepairActionKind,
    RepairExecuteOptions, RepairExecuteReport, RepairLoopOptions, RepairLoopReport,
    RepairLoopRound, RepairPlan, RepairPlanner, RepairRisk, RepairToolCall, RepairToolExecutor,
    RepairToolKind, RepairToolResult, RestoreReport, SecretVault, SensitivityLevel,
    SkippedRepairAction, SnapshotFile, SuggestedRepair,
};
pub use repair::{
    deep_diagnose_llm_config, run_deep_diagnose_chat, run_deep_repair, DeepDiagnoseEvent,
    DeepDiagnoseOptions, DeepDiagnoseReport, DeepDiagnoseTurn, DeepRepairSummary,
    DEEP_DIAGNOSE_CANCELLED, DEEP_DIAGNOSE_NO_PROVIDER, DEEP_DIAGNOSE_UNSUPPORTED_PROTOCOL,
};
pub use runtime::{
    adapter_by_id, all_adapters, all_runtime_ids, apply_runtime_playbook,
    apply_runtime_playbook_filtered, descriptor_by_id, installed_ask_agents, run_runtime_lifecycle,
    runtime_catalog, runtime_supports_lifecycle, runtime_supports_playbook,
    suggest_runtime_repairs, InstalledAskAgent, RuntimeCatalogEntry, RuntimeDescriptor,
    RuntimeLifecycleAction, RuntimeProbeSpec,
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
    is_native_subscription, list_personal_providers, load_mode_status,
    load_personal_provider_status, native_account_status, normalize_personal_gateway_url,
    normalize_protocol, probe_endpoint_bundle, project_bundle, refresh_account_models,
    resign_native_subscription, runtime_strategies, strategy_for, switch_to_personal_mode,
    switch_to_team_mode, upsert_personal_provider, use_current_service, use_native_subscription,
    verify_personal_provider, verify_personal_provider_with_protocol, write_evotown_agent_env,
    BundleProbeReport, EffectorKind, EndpointBundle, ModeStatus, ModeSwitchReport,
    ModeSwitchTarget, NativeAccountRow, NativeAccountStatus, NativeAccountSwitch,
    PersonalProviderListItem, PersonalProviderOptions, PersonalProviderSetupReport,
    PersonalProviderStatus, PersonalProviderVerifyReport, PersonalProvidersDocument,
    RuntimeSetupResult, RuntimeStrategy, SetupOptions, SetupReport, UpsertPersonalProviderOptions,
    WriteSemantics, CODEX_PERSONAL_SLOT, CODEX_TEAM_SLOT, COMPANY_DEFAULT_MODEL,
    HERMES_PERSONAL_SLOT, HERMES_TEAM_SLOT, MODE_PERSONAL, MODE_TEAM, MODE_UNSET,
    OPENCLAW_PERSONAL_SLOT, OPENCLAW_PROVIDER_ID, OPENCLAW_TEAM_SLOT, PROTOCOL_ANTHROPIC,
    PROTOCOL_OPENAI,
};
pub use skills::{
    execute_skills_sync, install_local_skills, install_teamups_mall_item,
    list_teamups_mall_catalog, load_local_skills_layout, parse_teamups_channel_job,
    poll_teamups_login, remove_local_skill, resolve_skills_source, save_skills_source_override,
    set_teamups_channel_enabled, sign_out_teamups, skill_roots, start_teamups_login,
    teamups_account_status, teamups_channel_auth, teamups_channel_enabled, teamups_channel_heard,
    teamups_channel_hello, teamups_channel_reply, teamups_channel_signed_in,
    teamups_channel_ws_url, teamups_voice_ticket, LocalSkillsLayout, ResolvedSkillsSource,
    SkillInstallError, SkillInstallReport, SkillsSyncOptions, TeamupsAccountStatus,
    TeamupsCatalogItem, TeamupsChannelJob, TeamupsClient, TeamupsIatUrl, TeamupsLoginPoll,
    TeamupsLoginPollStatus, TeamupsLoginStart, TeamupsMallCatalog, TeamupsOfficialStatus,
    TeamupsPackManifest, TeamupsSkillEntry, TeamupsVoiceTicket,
};
pub use store::{
    default_skills_cache_dir, default_teamups_base_url, open_settings_store, settings_db_path,
    PersonalProviderRecord, SettingsStore, SkillsSourceKind, SkillsSourceSettings, TeamSettings,
};
pub use version_check::{
    check_runtime_versions, check_runtime_versions_cached, RuntimeVersionStatus,
    VersionCompareStatus,
};
pub use workspace::{
    active_env_path, bash_hook_file_path, browser_configured_runtimes,
    browser_mcp_wire_options_for_active_workspace, diagnose_and_wire_browser_mcp,
    ensure_default_workspace, ensure_stable_agent_doctor_cli, enter_workspace, fish_hook_file_path,
    hook_file_path, init_workspace, install_bash_hook, install_fish_hook, install_powershell_hook,
    install_zsh_hook, installed_browser_mcp_runtime_ids, invalidate_mcp_inventory_cache,
    list_browser_mcp_targets, list_mcp_inventory, list_mcp_inventory_with_doc,
    list_runtime_agent_memory, load_workspaces, match_workspace_for_path,
    migrate_claude_global_mcp_to_project, powershell_hook_file_path, remove_workspace,
    render_direnv_envrc, render_shell_env, render_shell_env_for_name, resolve_agent_doctor_binary,
    save_workspaces, set_workspace_title, use_workspace, use_workspace_with_options,
    wire_browser_mcp_installed, workspace_capability_matrix, workspace_doctor, workspace_fix,
    workspace_hook_status, workspace_show, workspace_status, workspaces_path, write_direnv_envrc,
    AgentMemoryReport, AgentMemorySnippet, BrowserMcpDiagnoseIssue, BrowserMcpDiagnoseWireReport,
    BrowserMcpTargetAction, BrowserMcpTargetStatus, CapabilityCell, CapabilityMatrix,
    ClaudeMcpMigrationReport, EnterWorkspaceReport, GatewayRestartReport, InitWorkspaceReport,
    McpInventoryItem, McpInventoryReport, ShellHookStatus, UseWorkspaceOptions, UseWorkspaceReport,
    WorkspaceCheck, WorkspaceCheckStatus, WorkspaceDoctorReport, WorkspaceEntry,
    WorkspaceFixAction, WorkspaceFixOptions, WorkspaceFixReport, WorkspaceShowReport,
    WorkspaceSnapshotStatus, WorkspaceStatusReport, WorkspacesDocument,
};
