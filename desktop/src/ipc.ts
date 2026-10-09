//! Desktop command contract. This is the only module that names a Tauri command.
//!
//! Argument fields use the camelCase Tauri sends. Result fields stay in the
//! serde shape declared in `types.ts`. `desktop/src-tauri` checks this list
//! against `generate_handler!`.

import { invoke } from "@tauri-apps/api/core";
import type { PromptSessionReport } from "./chat/types";
import type {
  BrowserMcpDiagnoseWireReport,
  DoctorReport,
  EngineRegisterStatus,
  EvotownStatus,
  HermesSettings,
  InstallRuntimeResponse,
  McpConfigureReport,
  McpInventoryReport,
  McpModuleStatus,
  ModeStatus,
  ModeSwitchReport,
  OnboardingReport,
  OpenSessionReport,
  PersonalProviderSetupReport,
  PersonalProviderStatus,
  PersonalProviderVerifyReport,
  PersonalProvidersDocument,
  ProfilesDocument,
  RegisterReport,
  RemoteDoctorReport,
  RemoteHostProbeReport,
  RemoteHostRow,
  RemoteHostsDocument,
  RemoteProjectRow,
  RepairPreviewResponse,
  RestoreSummary,
  RuntimeVersionStatus,
  SkillMountReport,
  SkillsInventoryReport,
  SyncReport,
  TeamupsAccountStatus,
  TeamupsLoginPoll,
  TeamupsLoginStart,
  TeamupsMallCatalog,
  UseProfileReport,
  WindowSizeReport,
  WorkspaceDoctorReport,
  WorkspaceFixReport,
  WorkspacesDocument,
} from "./types";

export type ApplyReport = {
  runtime_id: string;
  config_path: string;
  backup_path: string | null;
  restart_hint: string;
};

export type BrowserMcpWireReport = {
  results: Array<{
    runtime: string;
    ok: boolean;
    config_path: string | null;
    message: string;
  }>;
};

export type SpeechCapability = {
  available: boolean;
  backend: string;
  reason?: string | null;
};

export type SpeechResult = {
  text: string;
  confidence: number;
  isFinal: boolean;
};

export type ImageTextReport = {
  readings: { name: string; text: string; ok: boolean }[];
};

export type DeepDiagnoseReport = {
  answer: string;
  model: string;
  tool_calls: number;
  open_issues: number;
};

export type DeepRepairSummary = {
  runtime_id: string;
  backup_id: string;
  issue_score_before: number;
  issue_score_after: number;
  executed: string[];
  skipped: { id: string; reason: string }[];
  fixed: RepairCheckChange[];
  new_issues: RepairCheckChange[];
  rolled_back_issues: RepairCheckChange[];
};

export type RepairCheckChange = {
  id: string;
  title: string;
  status: string;
  message: string;
};

export type WorkspaceStatusReport = {
  active: string | null;
  cwd: string;
  matched_workspace: string | null;
  runtimes: Array<{
    runtime_id: string;
    isolation_tier: string;
    expected: string;
    actual: string;
    aligned: boolean;
    hint: string;
  }>;
};

export type StartPromptSessionArgs = {
  runtime: string;
  prompt: string;
  cwd?: string | null;
  workspaceName?: string | null;
  timeoutSec?: number | null;
  dangerouslySkipPermissions?: boolean | null;
  fullAuto?: boolean | null;
  resumeThreadId?: string | null;
  selectedMcps?: string[] | null;
  imagePaths?: string[] | null;
};

export function getEvotownStatus(): Promise<EvotownStatus> {
  return invoke("get_evotown_status_command");
}

export function runEvotownOnboarding(args: {
  url: string;
  key: string;
  syncSkills: boolean;
  pullPolicies: boolean;
}): Promise<OnboardingReport> {
  return invoke("run_evotown_onboarding_command", args);
}

export function runSync(): Promise<SyncReport> {
  return invoke("run_sync_command");
}

export function getEngineRegisterStatus(): Promise<EngineRegisterStatus> {
  return invoke("get_engine_register_status_command");
}

export function runEngineRegister(args: {
  bootstrapToken: string;
  engineId: string | null;
  rotate: boolean;
}): Promise<RegisterReport> {
  return invoke("run_engine_register_command", args);
}

export function listSkillsInventory(args: {
  remoteStats?: boolean | null;
}): Promise<SkillsInventoryReport> {
  return invoke("list_skills_inventory_command", args);
}

export function skillMountRuntimeIds(): Promise<string[]> {
  return invoke("skill_mount_runtime_ids_command");
}

export function listTeamupsMallCatalog(): Promise<TeamupsMallCatalog> {
  return invoke("list_teamups_mall_catalog_command");
}

export function startTeamupsLogin(): Promise<TeamupsLoginStart> {
  return invoke("start_teamups_login_command");
}

export function pollTeamupsLogin(args: { deviceCode: string }): Promise<TeamupsLoginPoll> {
  return invoke("poll_teamups_login_command", args);
}

export function teamupsAccountStatus(): Promise<TeamupsAccountStatus> {
  return invoke("teamups_account_status_command");
}

export function signOutTeamups(): Promise<TeamupsAccountStatus> {
  return invoke("sign_out_teamups_command");
}

export function installTeamupsMallItem(args: {
  kind: string;
  id: string;
  packSlug?: string | null;
}): Promise<SyncReport> {
  return invoke("install_teamups_mall_item_command", args);
}

export function listMcpInventory(): Promise<McpInventoryReport> {
  return invoke("list_mcp_inventory_command");
}

export function mcpStatus(args: {
  port?: number | null;
  probeChrome?: boolean | null;
  discoverChrome?: boolean | null;
}): Promise<McpModuleStatus> {
  return invoke("mcp_status_command", args);
}

export function mcpConfigure(args: {
  runtime: string;
  port?: number | null;
  headless?: boolean | null;
  userDataDir?: string | null;
  profileDirectory?: string | null;
}): Promise<McpConfigureReport> {
  return invoke("mcp_configure_command", args);
}

export function mcpDiagnoseWire(args: {
  port?: number | null;
  headless?: boolean | null;
  userDataDir?: string | null;
  profileDirectory?: string | null;
}): Promise<BrowserMcpDiagnoseWireReport> {
  return invoke("mcp_diagnose_wire_command", args);
}

export function mountSyncedSkills(args: {
  skillIds?: string[] | null;
  runtimes?: string[] | null;
}): Promise<SkillMountReport> {
  return invoke("mount_synced_skills_command", args);
}

export function unmountSyncedSkills(args: {
  skillIds?: string[] | null;
  runtimes?: string[] | null;
}): Promise<SkillMountReport> {
  return invoke("unmount_synced_skills_command", args);
}

export function getPersonalProviderStatus(): Promise<PersonalProviderStatus> {
  return invoke("get_personal_provider_status_command");
}

export function listPersonalProviders(): Promise<PersonalProvidersDocument> {
  return invoke("list_personal_providers_command");
}

export function upsertPersonalProvider(args: {
  id: string | null;
  name: string;
  url: string;
  key: string;
  model: string;
  protocol: string;
  activate: boolean;
}): Promise<PersonalProvidersDocument> {
  return invoke("upsert_personal_provider_command", args);
}

export function deletePersonalProvider(args: { id: string }): Promise<PersonalProvidersDocument> {
  return invoke("delete_personal_provider_command", args);
}

export function activatePersonalProvider(args: { id: string }): Promise<PersonalProviderSetupReport> {
  return invoke("activate_personal_provider_command", args);
}

export function verifyPersonalProvider(args: {
  url: string;
  key: string;
  protocol: string;
}): Promise<PersonalProviderVerifyReport> {
  return invoke("verify_personal_provider_command", args);
}

export function applyPersonalProvider(args: {
  url: string;
  key: string;
  model: string;
  protocol: string;
}): Promise<PersonalProviderSetupReport> {
  return invoke("apply_personal_provider_command", args);
}

export function getModeStatus(): Promise<ModeStatus> {
  return invoke("get_mode_status_command");
}

export function getProductEdition(): Promise<string> {
  return invoke("get_product_edition_command");
}

export function switchToPersonalMode(args: {
  providerId?: string | null;
  withBrowserMcp?: boolean | null;
}): Promise<ModeSwitchReport> {
  return invoke("switch_to_personal_mode_command", args);
}

export function switchToTeamMode(args: {
  withBrowserMcp?: boolean | null;
}): Promise<ModeSwitchReport> {
  return invoke("switch_to_team_mode_command", args);
}

export function wireBrowserMcp(): Promise<BrowserMcpWireReport> {
  return invoke("wire_browser_mcp_command");
}

export function rewireCurrentMode(args: {
  withBrowserMcp?: boolean | null;
}): Promise<ModeSwitchReport> {
  return invoke("rewire_current_mode_command", args);
}

export function runDoctor(): Promise<DoctorReport> {
  return invoke("run_doctor_command");
}

export function checkRuntimeVersions(args: {
  installed: Array<{ runtimeId: string; version: string | null }>;
}): Promise<RuntimeVersionStatus[]> {
  return invoke("check_runtime_versions_command", args);
}

export function listProfiles(): Promise<ProfilesDocument> {
  return invoke("list_profiles_command");
}

export function listWorkspaces(): Promise<WorkspacesDocument> {
  return invoke("list_workspaces_command");
}

export type WorkspaceDirEntry = {
  name: string;
  relativePath: string;
  isDir: boolean;
};

export type WorkspaceFilePayload = {
  relativePath: string;
  name: string;
  content: string;
  language: string;
  editable: boolean;
  sizeBytes: number;
};

export function listWorkspaceDir(args: {
  root: string;
  relative?: string | null;
}): Promise<WorkspaceDirEntry[]> {
  return invoke("list_workspace_dir_command", args);
}

export function readWorkspaceFile(args: {
  root: string;
  relative: string;
}): Promise<WorkspaceFilePayload> {
  return invoke("read_workspace_file_command", args);
}

export function writeWorkspaceFile(args: {
  root: string;
  relative: string;
  content: string;
}): Promise<void> {
  return invoke("write_workspace_file_command", args);
}

export function initWorkspace(args: {
  path: string;
  name?: string | null;
  gitRoot: boolean;
  /** When true (default), switch global default project after register — main window flow. */
  activate?: boolean | null;
}): Promise<{ name: string }> {
  return invoke("init_workspace_command", args);
}

export function useWorkspace(args: { name: string }): Promise<void> {
  return invoke("use_workspace_command", args);
}

export function workspaceStatus(): Promise<WorkspaceStatusReport> {
  return invoke("workspace_status_command");
}

export function workspaceDoctor(): Promise<WorkspaceDoctorReport> {
  return invoke("workspace_doctor_command");
}

export function workspaceFix(args: { migrateClaudeMcp: boolean }): Promise<WorkspaceFixReport> {
  return invoke("workspace_fix_command", args);
}

export function listRemoteHosts(): Promise<RemoteHostsDocument> {
  return invoke("list_remote_hosts_command");
}

export function listRemoteHostRows(): Promise<RemoteHostRow[]> {
  return invoke("list_remote_host_rows_command");
}

export function listRemoteProjects(): Promise<RemoteProjectRow[]> {
  return invoke("list_remote_projects_command");
}

export function bootstrapRemoteHost(args: {
  id: string;
  hostname: string;
  user: string;
  port: number;
  password: string;
}): Promise<RemoteHostsDocument> {
  return invoke("bootstrap_remote_host_command", args);
}

export function addRemoteHost(args: {
  id: string;
  sshConfigHost: string;
}): Promise<RemoteHostsDocument> {
  return invoke("add_remote_host_command", args);
}

export function addRemoteProject(args: {
  host: string;
  name: string;
  path: string;
  runtimes: string[];
}): Promise<void> {
  return invoke("add_remote_project_command", args);
}

export function removeRemoteHost(args: { id: string }): Promise<void> {
  return invoke("remove_remote_host_command", args);
}

export function removeRemoteProject(args: { host: string; name: string }): Promise<void> {
  return invoke("remove_remote_project_command", args);
}

export function probeRemoteHost(args: { id: string }): Promise<RemoteHostProbeReport> {
  return invoke("probe_remote_host_command", args);
}

export function runRemoteDoctor(args: {
  target: string;
  runtime?: string | null;
}): Promise<RemoteDoctorReport> {
  return invoke("run_remote_doctor_command", args);
}

export function useProfile(args: { name: string }): Promise<UseProfileReport> {
  return invoke("use_profile_command", args);
}

export function getHermesModel(): Promise<HermesSettings> {
  return invoke("get_hermes_model_command");
}

export function setHermesModel(args: {
  provider: string;
  model: string;
  baseUrl: string;
  apiKey?: string | null;
}): Promise<ApplyReport> {
  return invoke("set_hermes_model_command", args);
}

export function applyProfileModel(args: {
  profile: string;
  provider: string;
  model: string;
  baseUrl: string;
}): Promise<ApplyReport> {
  return invoke("apply_profile_model_command", args);
}

export function runRepairPreview(args: { runtime: string }): Promise<RepairPreviewResponse> {
  return invoke("run_repair_preview_command", args);
}

export function runRepairExecute(args: { runtime: string }): Promise<RepairPreviewResponse> {
  return invoke("run_repair_execute_command", args);
}

export function deepDiagnoseChat(args: {
  runtime: string;
  question: string;
  history?: unknown;
  locale?: string | null;
}): Promise<DeepDiagnoseReport> {
  return invoke("deep_diagnose_chat_command", args);
}

export function cancelDeepDiagnose(): Promise<boolean> {
  return invoke("cancel_deep_diagnose_command");
}

export function deepRepair(args: { runtime: string }): Promise<DeepRepairSummary> {
  return invoke("deep_repair_command", args);
}

export function runBrowserSmoke(): Promise<{ ok: boolean; detail: string }> {
  return invoke("run_browser_smoke_command");
}

export function runRepairRollback(args: {
  runtime: string;
  backup?: string | null;
}): Promise<RestoreSummary> {
  return invoke("run_repair_rollback_command", args);
}

export function installRuntime(args: {
  runtime: string;
  force?: boolean | null;
}): Promise<InstallRuntimeResponse> {
  return invoke("install_runtime_command", args);
}

export function uninstallRuntime(args: { runtime: string }): Promise<void> {
  return invoke("uninstall_runtime_command", args);
}

export function openPath(args: { path: string }): Promise<void> {
  return invoke("open_path_command", args);
}

export function fetchRuntimeCatalog(): Promise<
  Array<{
    id: string;
    label: string;
    ask: boolean;
    opens_app: boolean;
    browser_mcp: boolean;
    skill_mount: boolean;
  }>
> {
  return invoke("runtime_catalog_command");
}

export function openSession(args: {
  runtime: string;
  cwd?: string | null;
  prompt?: string | null;
  terminal?: boolean | null;
}): Promise<OpenSessionReport> {
  return invoke("open_session_command", args);
}

export function openAskWindow(args: { runtime?: string | null }): Promise<void> {
  return invoke("open_ask_window_command", args);
}

export function closeAskWindow(args: { destroy?: boolean | null }): Promise<void> {
  return invoke("close_ask_window_command", args);
}

export function openResourcesWindow(args: { section?: string | null }): Promise<void> {
  return invoke("open_resources_window_command", args);
}

export function closeResourcesWindow(args: { destroy?: boolean | null }): Promise<void> {
  return invoke("close_resources_window_command", args);
}

export function openDiagnoseWindow(args: { runtime?: string | null }): Promise<void> {
  return invoke("open_diagnose_window_command", args);
}

export function closeDiagnoseWindow(args: { destroy?: boolean | null }): Promise<void> {
  return invoke("close_diagnose_window_command", args);
}

export function focusMainTab(args: { tab?: string | null }): Promise<void> {
  return invoke("focus_main_tab_command", args);
}

export function resizeMainWindow(args: {
  width?: number | null;
  height?: number | null;
}): Promise<WindowSizeReport> {
  return invoke("resize_main_window_command", args);
}

export function startPromptSession(args: StartPromptSessionArgs): Promise<PromptSessionReport> {
  return invoke("start_prompt_session_command", args);
}

export function cancelPromptSession(): Promise<boolean> {
  return invoke("cancel_prompt_session_command");
}

export function resolvePermissionSession(args: {
  sessionId: string;
  requestId: string;
  allow: boolean;
  text?: string | null;
}): Promise<boolean> {
  return invoke("resolve_permission_session_command", args);
}

export function submitInstallInput(line: string): Promise<void> {
  return invoke("submit_install_input_command", { line });
}

export function speechCapability(): Promise<SpeechCapability> {
  return invoke("speech_capability_command");
}

export function speechDictate(args: { language?: string | null }): Promise<SpeechResult> {
  return invoke("speech_dictate_command", args);
}

export function speechCancelDictation(): Promise<void> {
  return invoke("speech_cancel_dictation_command");
}

export function voiceListenStart(args: { language?: string | null }): Promise<void> {
  return invoke("voice_listen_start_command", args);
}

export function voiceListenStop(): Promise<void> {
  return invoke("voice_listen_stop_command");
}

export function voiceSpeak(args: { text: string; language?: string | null }): Promise<void> {
  return invoke("voice_speak_command", args);
}

export function voiceSpeakStop(): Promise<void> {
  return invoke("voice_speak_stop_command");
}

export function voiceHostedReduce<S, E>(args: {
  state: S;
  input: unknown;
}): Promise<{ state: S; effects: E[] }> {
  return invoke("voice_hosted_reduce_command", args);
}

export function voiceTurnEnd(args: {
  text: string;
}): Promise<{ thinking: boolean; holdMs: number; textToSend: string }> {
  return invoke("voice_turn_end_command", args);
}

export function readImageTexts(args: { paths: string[] }): Promise<ImageTextReport> {
  return invoke("read_image_texts_command", args);
}

export type AskImageSupport = {
  runtime: string;
  sees_images: boolean;
  model: string | null;
  formats: string[];
};

export function askImageSupport(args: { runtime: string }): Promise<AskImageSupport> {
  return invoke("ask_image_support_command", args);
}

export function publishIslandSnapshot(snapshot: import("./island/track").IslandSnapshot): Promise<void> {
  return invoke("publish_island_snapshot_command", { snapshot });
}

export function islandSetHover(hovering: boolean, sticky = false): Promise<void> {
  return invoke("island_set_hover_command", { hovering, sticky });
}

export function islandRestore(): Promise<void> {
  return invoke("island_restore_command");
}

export function islandPin(): Promise<void> {
  return invoke("island_pin_command");
}

export function islandOpenSession(sessionId: string): Promise<void> {
  return invoke("island_open_session_command", { sessionId });
}

export function islandSendText(sessionId: string, text: string): Promise<void> {
  return invoke("island_send_text_command", { sessionId, text });
}

export function islandSetReading(reading: boolean): Promise<void> {
  return invoke("island_set_reading_command", { reading });
}

export function islandSetContentHeight(height: number): Promise<void> {
  return invoke("island_set_content_height_command", { height });
}

export function islandHideWhenIdle(): Promise<boolean> {
  return invoke("island_hide_when_idle_command");
}

export function islandSetHideWhenIdle(hide: boolean): Promise<void> {
  return invoke("island_set_hide_when_idle_command", { hide });
}

export function islandClaimKeyboard(): Promise<void> {
  return invoke("island_claim_keyboard_command");
}

export function currentIslandView(): Promise<import("./island/track").IslandView> {
  return invoke("current_island_view_command");
}
