export interface RuntimeDoctorResult {
  id: string;
  display_name: string;
  installed: boolean;
  version: string | null;
  binary_path: string | null;
  config_paths: string[];
  profile: {
    gateway_url: string | null;
    key_source: string | null;
  };
}

export interface DoctorReport {
  profile_env_path: string | null;
  profile_env_exists: boolean;
  active_preset: string | null;
  runtimes: RuntimeDoctorResult[];
}

export interface HermesSettings {
  provider: string;
  model: string;
  base_url: string;
  api_key_env: string | null;
  api_key_configured: boolean;
  api_key_hint: string | null;
}

export interface ProfileEntry {
  hermes?: Pick<HermesSettings, "provider" | "model" | "base_url">;
  models?: Array<Pick<HermesSettings, "provider" | "model" | "base_url">>;
}

export interface ProfilesDocument {
  active: string | null;
  profiles: Record<string, ProfileEntry>;
}

export interface WorkspaceEntry {
  path: string;
  hermes_profile: string;
  codex_home: string;
  openclaw_agent_id: string;
  openclaw_workspace: string;
}

export interface WorkspacesDocument {
  active: string | null;
  workspaces: Record<string, WorkspaceEntry>;
}

export interface UseProfileReport {
  profile: string;
  applied: Array<{
    runtime_id: string;
    config_path: string;
    backup_path: string | null;
    restart_hint: string;
  }>;
  skipped: string[];
}

export interface RepairPreviewResponse {
  runtime_id: string;
  display_name: string;
  summary: {
    pass: number;
    warn: number;
    fail: number;
    not_applicable: number;
    not_checked: number;
  };
  checks: Array<{
    title: string;
    status: "pass" | "warn" | "fail" | "n/a" | "not checked";
    message: string;
    details: string[];
  }>;
  plan_summary: string;
  suggested_repairs: Array<{
    id: string;
    title: string;
    description: string;
    auto_fixable: boolean;
  }>;
  can_apply_repair: boolean;
  backup_ids: string[];
  last_execute: {
    backup_id: string;
    backup_root: string;
    executed: string[];
    skipped: Array<{ id: string; reason: string }>;
    verification_summary: string;
    rollback_hint: string;
    guide_path: string | null;
    browser_smoke?: { ok: boolean; detail: string } | null;
  } | null;
}

export type RestoreSummary = {
  backup_id: string;
  backup_root: string;
  restored_files: string[];
};

export type InstallRuntimeResponse = {
  runtime_id: string;
  install_needed: boolean;
  install_succeeded: boolean;
  install_attempts: number;
  install_log_path: string | null;
  manual_fallback: string[];
  skipped: Array<{ id: string; reason: string }>;
  after_installed: boolean;
};

export type InstallProgressEvent = {
  runtime_id: string;
  phase: string;
  message: string;
  percent: number;
};

export interface EvotownStatus {
  configured: boolean;
  base_url: string | null;
  api_key_hint: string | null;
  config_source: string | null;
  runtime_target: string | null;
  bundle_id: string | null;
}

export interface EngineRegisterStatus {
  registered: boolean;
  engine_id: string | null;
  env_path: string | null;
}

export interface RegisterReport {
  base_url: string;
  engine_id: string;
  engine_type: string;
  ingest_token_issued: boolean;
  ingest_token: string | null;
  saved_to: string | null;
  rotated: boolean;
  detail: string;
}

export interface OnboardingReport {
  setup: {
    gateway_url: string;
    evotown_base_url: string;
    profile_env_path: string;
  };
  sync: {
    installed: number;
    skipped: number;
    failed: number;
  } | null;
  policy: {
    policy_count: number;
  } | null;
}

export interface OpenSessionReport {
  runtime: string;
  method: "deep-link" | "terminal";
  cwd: string;
  target: string;
  detail: string;
}

export interface SyncReport {
  installed: number;
  skipped: number;
  failed: number;
}

export interface SkillAgentUsage {
  runtime: string;
  scope: string;
  path: string;
  mounted: boolean;
}

export interface SkillInventoryItem {
  skill_id: string;
  name: string;
  version: string;
  description: string | null;
  installed_path: string;
  agents: SkillAgentUsage[];
  call_count: number | null;
  success_count: number | null;
  success_rate: number | null;
  first_success_rate: number | null;
  download_count: number | null;
  metrics_source: string;
}

export interface SkillsInventoryReport {
  skills_dir: string;
  lock_path: string;
  bundle_id: string | null;
  skills: SkillInventoryItem[];
  remote_stats_ok: boolean;
  remote_stats_error: string | null;
}

export interface SkillMountReport {
  mounted: number;
  unmounted: number;
  skipped: number;
  failed: number;
}

export interface McpInventoryItem {
  name: string;
  scope: string;
  config_path: string;
  command: string | null;
  args: string[];
  healthy: boolean;
  issue: string | null;
  is_browser: boolean;
  runtime_hint: string;
}

export interface McpInventoryReport {
  workspace_name: string | null;
  workspace_path: string | null;
  servers: McpInventoryItem[];
  total: number;
  healthy: number;
  issues: number;
  browser_configured: boolean;
}

export interface BrowserMcpStatus {
  chrome_found: boolean;
  binary: string | null;
  version: string | null;
  user_data_dir: string | null;
  profile_directory: string;
  system_user_data_dir: string;
  isolated_user_data_dir: string;
  cdp_connected: boolean;
  ws_endpoint: string | null;
  port: number;
}

export interface BrowserMcpTargetStatus {
  runtime_id: string;
  display_name: string;
  installed: boolean;
  configured: boolean;
}

export interface BrowserMcpDiagnoseIssue {
  code: string;
  message: string;
}

export interface BrowserMcpTargetAction {
  runtime_id: string;
  display_name: string;
  installed: boolean;
  configured_before: boolean;
  action: string;
  ok: boolean;
  config_path: string | null;
  message: string;
}

export interface BrowserMcpDiagnoseWireReport {
  chrome_ok: boolean;
  cli_ok: boolean;
  issues: BrowserMcpDiagnoseIssue[];
  targets: BrowserMcpTargetAction[];
  wrote: number;
  failed: number;
  skipped: number;
}

export interface McpModuleStatus {
  browser: BrowserMcpStatus;
  inventory: McpInventoryReport;
  configured_runtimes: string[];
  targets: BrowserMcpTargetStatus[];
  binary: string;
  config_snippet: unknown;
}

export interface McpConfigureReport {
  runtime: string;
  port: number;
  config_path: string;
  binary: string;
}

export type ResourceFilter = "all" | "skill" | "mcp" | "issue";
export type ResourceRow = {
  kind: "skill" | "mcp";
  name: string;
  sub: string;
  meta: string;
  tone: "ok" | "warn" | "bad" | "muted";
  issue: boolean;
  skillId?: string;
  needsMount?: boolean;
};

export interface PersonalProviderStatus {
  configured: boolean;
  gateway_url: string | null;
  model: string | null;
  api_key_hint: string | null;
  profile_env_path: string | null;
  active_id: string | null;
  active_name: string | null;
  protocol: string | null;
}

export interface PersonalProviderListItem {
  id: string;
  name: string;
  url: string;
  model: string;
  protocol: string;
  api_key_hint: string;
  active: boolean;
}

export interface PersonalProvidersDocument {
  active_id: string | null;
  providers: PersonalProviderListItem[];
  store_path: string;
}

export interface PersonalProviderVerifyReport {
  ok: boolean;
  status_code: number | null;
  checked_url: string | null;
  message: string;
  models_sample: string[];
}

export interface PersonalProviderSetupReport {
  profile_env_path: string;
  gateway_url: string;
  model: string;
  provider_id: string | null;
  provider_name: string | null;
  runtimes: Array<{ runtime_id: string; applied: boolean }>;
  verify: PersonalProviderVerifyReport | null;
}

export type ProviderProtocol = "openai" | "anthropic";
export type ActiveMode = "personal" | "team" | "unset";

export type ModeStatus = {
  mode: ActiveMode | string;
  /** Build edition: personal | team (locked package, not a runtime switch). */
  edition?: string;
  personal_ready: boolean;
  team_ready: boolean;
  active_label: string | null;
  active_gateway_url: string | null;
  active_key_hint: string | null;
  personal_active_id: string | null;
  personal_active_name: string | null;
  team_base_url: string | null;
};

export type ModeSwitchReport = {
  mode: string;
  active_label: string | null;
  active_gateway_url: string | null;
  message: string;
  model?: string | null;
  source_id?: string | null;
  probe_ok?: boolean | null;
  probe_detail?: string | null;
  warnings?: string[];
  runtimes: Array<{
    runtime_id: string;
    applied: boolean;
    effector?: string | null;
    effector_ok?: boolean | null;
    probe_ok?: boolean | null;
  }>;
  browser_mcp?: {
    results: Array<{
      runtime: string;
      ok: boolean;
      config_path?: string | null;
      message: string;
    }>;
  } | null;
};

export type MainTabId = "diagnose" | "resources" | "provider" | "workspace";
export type ProviderTabId = "personal" | "evotown";

export type RepairStatusFilter = "all" | RepairPreviewResponse["checks"][number]["status"];

export type WindowSizeReport = { width: number; height: number };

export type RelatedTag = { kind: "skill" | "mcp"; name: string; broken: boolean };

export interface WorkspaceCheck {
  id: string;
  title: string;
  status: "pass" | "warn" | "fail";
  detail: string;
}

export interface WorkspaceDoctorReport {
  active: string | null;
  checks: WorkspaceCheck[];
}

export interface WorkspaceFixReport {
  active: string | null;
  actions: Array<{
    id: string;
    title: string;
    applied: boolean;
    detail: string;
  }>;
}

export interface RemoteHostsDocument {
  hosts: Record<
    string,
    {
      ssh_config_host: string;
      hostname?: string | null;
      user?: string | null;
      port?: number | null;
      identity_file?: string | null;
      projects: Record<string, { path: string; runtimes: string[] }>;
    }
  >;
}

export interface RemoteHostRow {
  host_id: string;
  target: string;
  managed: boolean;
  project_count: number;
}

export type RemoteHostProbeStatus = "unknown" | "probing" | "ok" | "fail";

export interface RemoteHostProbeReport {
  host_id: string;
  target: string;
  ok: boolean;
  message: string;
}

export interface RemoteProjectRow {
  host_id: string;
  project_id: string;
  path: string;
  runtimes: string[];
  ssh_config_host: string;
  managed?: boolean;
}

export interface RemoteProbeCheck {
  id: string;
  title: string;
  status: "pass" | "warn" | "fail" | "not_applicable" | "not_checked";
  severity: string;
  message: string;
  details: string[];
}

export interface RemoteDoctorReport {
  host_id: string;
  ssh_config_host: string;
  project_id: string;
  project_path: string;
  remote_home: string | null;
  connectivity_ok: boolean;
  checks: RemoteProbeCheck[];
  runtimes: Array<{
    runtime_id: string;
    display_name: string;
    binary_name: string;
    checks: RemoteProbeCheck[];
  }>;
  report_path: string | null;
}
