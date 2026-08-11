import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  applyStaticI18n,
  getLocale,
  setLocale,
  t,
  type Locale,
  type MessageKey,
} from "./i18n";

interface RuntimeDoctorResult {
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

interface DoctorReport {
  profile_env_path: string | null;
  profile_env_exists: boolean;
  active_preset: string | null;
  runtimes: RuntimeDoctorResult[];
}

interface HermesSettings {
  provider: string;
  model: string;
  base_url: string;
  api_key_env: string | null;
  api_key_configured: boolean;
  api_key_hint: string | null;
}

interface ProfileEntry {
  hermes?: Pick<HermesSettings, "provider" | "model" | "base_url">;
  models?: Array<Pick<HermesSettings, "provider" | "model" | "base_url">>;
}

interface ProfilesDocument {
  active: string | null;
  profiles: Record<string, ProfileEntry>;
}

interface WorkspaceEntry {
  path: string;
  hermes_profile: string;
  codex_home: string;
  openclaw_agent_id: string;
  openclaw_workspace: string;
}

interface WorkspacesDocument {
  active: string | null;
  workspaces: Record<string, WorkspaceEntry>;
}

interface UseProfileReport {
  profile: string;
  applied: Array<{
    runtime_id: string;
    config_path: string;
    backup_path: string | null;
    restart_hint: string;
  }>;
  skipped: string[];
}

interface RepairPreviewResponse {
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
  } | null;
}

type RestoreSummary = {
  backup_id: string;
  backup_root: string;
  restored_files: string[];
};

type InstallRuntimeResponse = {
  runtime_id: string;
  install_needed: boolean;
  install_succeeded: boolean;
  install_attempts: number;
  install_log_path: string | null;
  manual_fallback: string[];
  skipped: Array<{ id: string; reason: string }>;
  after_installed: boolean;
};

type InstallProgressEvent = {
  runtime_id: string;
  phase: string;
  message: string;
  percent: number;
};

interface EvotownStatus {
  configured: boolean;
  base_url: string | null;
  api_key_hint: string | null;
  config_source: string | null;
  runtime_target: string | null;
  bundle_id: string | null;
}

interface OnboardingReport {
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

interface OpenSessionReport {
  runtime: string;
  method: "deep-link" | "terminal";
  cwd: string;
  target: string;
  detail: string;
}

interface SyncReport {
  installed: number;
  skipped: number;
  failed: number;
}

interface SkillAgentUsage {
  runtime: string;
  scope: string;
  path: string;
  mounted: boolean;
}

interface SkillInventoryItem {
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

interface SkillsInventoryReport {
  skills_dir: string;
  lock_path: string;
  bundle_id: string | null;
  skills: SkillInventoryItem[];
  remote_stats_ok: boolean;
  remote_stats_error: string | null;
}

interface SkillMountReport {
  mounted: number;
  unmounted: number;
  skipped: number;
  failed: number;
}

interface McpInventoryItem {
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

interface McpInventoryReport {
  workspace_name: string | null;
  workspace_path: string | null;
  servers: McpInventoryItem[];
  total: number;
  healthy: number;
  issues: number;
  browser_configured: boolean;
}

interface BrowserMcpStatus {
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

interface McpModuleStatus {
  browser: BrowserMcpStatus;
  inventory: McpInventoryReport;
  configured_runtimes: string[];
  binary: string;
  config_snippet: unknown;
}

interface McpConfigureReport {
  runtime: string;
  port: number;
  config_path: string;
  binary: string;
}

type ResourceFilter = "all" | "skill" | "mcp" | "issue";
type ResourceRow = {
  kind: "skill" | "mcp";
  name: string;
  sub: string;
  meta: string;
  tone: "ok" | "warn" | "bad" | "muted";
  issue: boolean;
  skillId?: string;
  needsMount?: boolean;
};

interface PersonalProviderStatus {
  configured: boolean;
  gateway_url: string | null;
  model: string | null;
  api_key_hint: string | null;
  profile_env_path: string | null;
  active_id: string | null;
  active_name: string | null;
  protocol: string | null;
}

interface PersonalProviderListItem {
  id: string;
  name: string;
  url: string;
  model: string;
  protocol: string;
  api_key_hint: string;
  active: boolean;
}

interface PersonalProvidersDocument {
  active_id: string | null;
  providers: PersonalProviderListItem[];
  store_path: string;
}

interface PersonalProviderVerifyReport {
  ok: boolean;
  status_code: number | null;
  checked_url: string | null;
  message: string;
  models_sample: string[];
}

interface PersonalProviderSetupReport {
  profile_env_path: string;
  gateway_url: string;
  model: string;
  provider_id: string | null;
  provider_name: string | null;
  runtimes: Array<{ runtime_id: string; applied: boolean }>;
  verify: PersonalProviderVerifyReport | null;
}

const evotownSectionEl = document.querySelector<HTMLElement>("#evotown-section")!;
const evotownStatusEl = document.querySelector<HTMLElement>("#evotown-status")!;
const evotownConnectedEl = document.querySelector<HTMLElement>("#evotown-connected")!;
const evotownConnectedUrlEl = document.querySelector<HTMLElement>("#evotown-connected-url")!;
const evotownConnectedMetaEl = document.querySelector<HTMLElement>("#evotown-connected-meta")!;
const evotownFormEl = document.querySelector<HTMLFormElement>("#evotown-form")!;
const evotownUrlEl = document.querySelector<HTMLInputElement>("#evotown-url")!;
const evotownKeyEl = document.querySelector<HTMLInputElement>("#evotown-key")!;
const evotownConnectEl = document.querySelector<HTMLButtonElement>("#evotown-connect")!;
const evotownResyncEl = document.querySelector<HTMLButtonElement>("#evotown-resync")!;
const evotownHintEl = document.querySelector<HTMLElement>("#evotown-hint")!;
const skillsInventoryEl = document.querySelector<HTMLElement>("#skills-inventory")!;
const skillsRefreshEl = document.querySelector<HTMLButtonElement>("#skills-refresh")!;
const skillsMountAllEl = document.querySelector<HTMLButtonElement>("#skills-mount-all")!;
const skillsDirEl = document.querySelector<HTMLElement>("#skills-dir")!;
const skillsListEl = document.querySelector<HTMLUListElement>("#skills-list")!;
const skillsEmptyEl = document.querySelector<HTMLElement>("#skills-empty")!;
const skillsFootnoteEl = document.querySelector<HTMLElement>("#skills-footnote")!;
const skillsCountEl = document.querySelector<HTMLElement>("#skills-count")!;
const mcpCountEl = document.querySelector<HTMLElement>("#mcp-count")!;

const mcpBrowserBadgeEl = document.querySelector<HTMLElement>("#mcp-browser-badge")!;
const mcpChromeEl = document.querySelector<HTMLElement>("#mcp-chrome")!;
const mcpCdpEl = document.querySelector<HTMLElement>("#mcp-cdp")!;
const mcpConfiguredEl = document.querySelector<HTMLElement>("#mcp-configured")!;
const mcpBinaryEl = document.querySelector<HTMLElement>("#mcp-binary")!;
const mcpShowUiEl = document.querySelector<HTMLInputElement>("#mcp-show-ui")!;
const mcpUserDataDirEl = document.querySelector<HTMLInputElement>("#mcp-user-data-dir")!;
const mcpProfileDirectoryEl = document.querySelector<HTMLInputElement>("#mcp-profile-directory")!;
const mcpProfileSystemEl = document.querySelector<HTMLButtonElement>("#mcp-profile-system")!;
const mcpProfileIsolatedEl = document.querySelector<HTMLButtonElement>("#mcp-profile-isolated")!;
const mcpRefreshEl = document.querySelector<HTMLButtonElement>("#mcp-refresh")!;
const mcpConfigureCodexEl = document.querySelector<HTMLButtonElement>("#mcp-configure-codex")!;
const mcpConfigureClaudeEl = document.querySelector<HTMLButtonElement>("#mcp-configure-claude")!;
const mcpSnippetEl = document.querySelector<HTMLElement>("#mcp-snippet")!;
const mcpFootnoteEl = document.querySelector<HTMLElement>("#mcp-footnote")!;
const MCP_SHOW_UI_KEY = "agent-doctor.mcp.showUi";
const MCP_USER_DATA_DIR_KEY = "agent-doctor.mcp.userDataDir";
const MCP_PROFILE_DIRECTORY_KEY = "agent-doctor.mcp.profileDirectory";
const resourcesRefreshEl = document.querySelector<HTMLButtonElement>("#resources-refresh")!;
const resourcesFiltersEl = document.querySelector<HTMLElement>("#resources-filters")!;
const resourcesListEl = document.querySelector<HTMLUListElement>("#resources-list")!;
const resourcesEmptyEl = document.querySelector<HTMLElement>("#resources-empty")!;
const resourcesFootnoteEl = document.querySelector<HTMLElement>("#resources-footnote")!;

const personalSectionEl = document.querySelector<HTMLElement>("#personal-section")!;
const personalListViewEl = document.querySelector<HTMLElement>("#personal-list-view")!;
const personalFormViewEl = document.querySelector<HTMLElement>("#personal-form-view")!;
const personalStatusEl = document.querySelector<HTMLElement>("#personal-status")!;
const personalConnectedEl = document.querySelector<HTMLElement>("#personal-connected")!;
const personalConnectedUrlEl = document.querySelector<HTMLElement>("#personal-connected-url")!;
const personalConnectedMetaEl = document.querySelector<HTMLElement>("#personal-connected-meta")!;
const personalListEl = document.querySelector<HTMLUListElement>("#personal-list")!;
const personalListHintEl = document.querySelector<HTMLElement>("#personal-list-hint")!;
const personalFormEl = document.querySelector<HTMLFormElement>("#personal-form")!;
const personalFormTitleEl = document.querySelector<HTMLElement>("#personal-form-title")!;
const personalIdEl = document.querySelector<HTMLInputElement>("#personal-id")!;
const personalPresetEl = document.querySelector<HTMLSelectElement>("#personal-preset")!;
const personalProtocolEl = document.querySelector<HTMLSelectElement>("#personal-protocol")!;
const personalNameRowEl = document.querySelector<HTMLElement>("#personal-name-row")!;
const personalNameEl = document.querySelector<HTMLInputElement>("#personal-name")!;
const personalUrlEl = document.querySelector<HTMLInputElement>("#personal-url")!;
const personalKeyEl = document.querySelector<HTMLInputElement>("#personal-key")!;
const personalModelEl = document.querySelector<HTMLInputElement>("#personal-model")!;
const personalModelSuggestionsEl = document.querySelector<HTMLDataListElement>(
  "#personal-model-suggestions",
)!;
const personalAddEl = document.querySelector<HTMLButtonElement>("#personal-add")!;
const personalBackEl = document.querySelector<HTMLButtonElement>("#personal-back")!;
const personalVerifyEl = document.querySelector<HTMLButtonElement>("#personal-verify")!;
const personalSaveEl = document.querySelector<HTMLButtonElement>("#personal-save")!;
const personalApplyEl = document.querySelector<HTMLButtonElement>("#personal-apply")!;
const personalHintEl = document.querySelector<HTMLElement>("#personal-hint")!;

let personalProvidersDoc: PersonalProvidersDocument | null = null;

type ProviderProtocol = "openai" | "anthropic";
type ActiveMode = "personal" | "team" | "unset";

type ModeStatus = {
  mode: ActiveMode | string;
  personal_ready: boolean;
  team_ready: boolean;
  active_label: string | null;
  active_gateway_url: string | null;
  active_key_hint: string | null;
  personal_active_id: string | null;
  personal_active_name: string | null;
  team_base_url: string | null;
};

type ModeSwitchReport = {
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

const modeWithBrowserMcpEl = document.querySelector<HTMLInputElement>("#mode-with-browser-mcp")!;

function wantsBrowserMcp(): boolean {
  return Boolean(modeWithBrowserMcpEl?.checked);
}

function formatBrowserMcpHint(report: ModeSwitchReport): string | null {
  const results = report.browser_mcp?.results;
  if (!results?.length) {
    return null;
  }
  const ok = results.filter((item) => item.ok).length;
  const fail = results.find((item) => !item.ok);
  if (fail) {
    return t("mode.browserMcpFail", { detail: fail.message });
  }
  return t("mode.browserMcpOk", { ok: String(ok), total: String(results.length) });
}

function formatModeSwitchHint(report: ModeSwitchReport): string {
  const parts: string[] = [];
  if (report.probe_ok === false) {
    parts.push(t("mode.probeFailShort"));
  } else if (report.probe_ok === true) {
    parts.push(t("mode.probeOk"));
  } else {
    parts.push(t("mode.switchDone"));
  }
  const applied = report.runtimes.filter((r) => r.applied).length;
  const needRestart = report.runtimes.filter(
    (r) => r.applied && (r.effector === "restart_gateway" || r.effector === "manual_restart"),
  ).length;
  if (needRestart > 0) {
    parts.push(t("mode.effectorHint", { count: String(needRestart), applied: String(applied) }));
  }
  if (report.warnings?.length) {
    parts.push(t("mode.warnings", { count: String(report.warnings.length) }));
  }
  const mcpHint = formatBrowserMcpHint(report);
  if (mcpHint) {
    parts.push(mcpHint);
  }
  return parts.join(" · ");
}

function formatModeSwitchDetail(report: ModeSwitchReport): string {
  const bits = [report.message];
  if (report.probe_detail) bits.push(report.probe_detail);
  if (report.warnings?.length) bits.push(...report.warnings);
  return bits.filter(Boolean).join("\n");
}

const modeMetaEl = document.querySelector<HTMLElement>("#mode-meta")!;
const modeHintEl = document.querySelector<HTMLElement>("#mode-hint")!;
const modeUsePersonalEl = document.querySelector<HTMLButtonElement>("#mode-use-personal")!;
const modeUseTeamEl = document.querySelector<HTMLButtonElement>("#mode-use-team")!;
const wiringModeFootnoteEl = document.querySelector<HTMLElement>("#wiring-mode-footnote")!;
const footerCopyEl = document.querySelector<HTMLElement>("#footer-copy")!;
const agentsWsChipEl = document.querySelector<HTMLElement>("#agents-ws-chip")!;
const agentsWsNameEl = document.querySelector<HTMLElement>("#agents-ws-name")!;
const agentsWsQuickEl = document.querySelector<HTMLButtonElement>("#agents-ws-quick")!;
const agentsWsManageEl = document.querySelector<HTMLButtonElement>("#agents-ws-manage")!;
const agentsWsPickerEl = document.querySelector<HTMLElement>("#agents-ws-picker")!;
const agentsWsListEl = document.querySelector<HTMLUListElement>("#agents-ws-list")!;

let lastModeStatus: ModeStatus | null = null;
let agentsWsPickerOpen = false;

function updateFooterCopy(mode?: string): void {
  const activeMode = mode ?? lastModeStatus?.mode ?? "personal";
  footerCopyEl.textContent = activeMode === "team" ? t("app.footerTeam") : t("app.footer");
}

function updateWiringModeFootnote(mode?: string): void {
  const activeMode = mode ?? lastModeStatus?.mode ?? "personal";
  wiringModeFootnoteEl.textContent =
    activeMode === "team" ? t("wiring.modeTeamFootnote") : t("wiring.modePersonalFootnote");
  wiringModeFootnoteEl.title = t("wiring.modeHint");
}

function closeAgentsWsPicker(): void {
  agentsWsPickerOpen = false;
  agentsWsPickerEl.hidden = true;
}

function toggleAgentsWsPicker(): void {
  if (agentsWsPickerOpen) {
    closeAgentsWsPicker();
    return;
  }
  if (lastWorkspaces) {
    renderAgentsWorkspaceQuickList(lastWorkspaces);
  }
  agentsWsPickerOpen = true;
  agentsWsPickerEl.hidden = false;
}

function updateAgentsWorkspaceChip(doc: WorkspacesDocument): void {
  const active = doc.active ?? selectedWorkspaceName ?? null;
  agentsWsNameEl.textContent = active ?? t("workspaces.noActive");
  renderAgentsWorkspaceQuickList(doc);
}

function renderAgentsWorkspaceQuickList(doc: WorkspacesDocument): void {
  const names = Object.keys(doc.workspaces).sort();
  if (names.length === 0) {
    agentsWsListEl.innerHTML = `
      <li class="ws-quick-item">
        <span class="ws-quick-item-main">
          <strong>${escapeHtml(t("workspaces.none"))}</strong>
          <span>${escapeHtml(t("workspaces.noneHint"))}</span>
        </span>
      </li>
    `;
    agentsWsQuickEl.disabled = true;
    return;
  }

  agentsWsQuickEl.disabled = false;
  const active = doc.active ?? selectedWorkspaceName;
  agentsWsListEl.innerHTML = names
    .map((name) => {
      const entry = doc.workspaces[name];
      const isActive = name === active;
      const meta = entry?.path ?? "";
      return `
        <li class="ws-quick-item ${isActive ? "is-active" : ""}">
          <span class="ws-quick-item-main">
            <strong>${escapeHtml(name)}</strong>
            ${meta ? `<span>${escapeHtml(meta)}</span>` : ""}
          </span>
          ${
            isActive
              ? `<span class="badge ok">${escapeHtml(t("agents.wsActiveBadge"))}</span>`
              : `<button type="button" class="btn-secondary btn-compact" data-agents-ws="${escapeHtml(name)}">${escapeHtml(t("agents.wsUse"))}</button>`
          }
        </li>
      `;
    })
    .join("");
}

async function applyAgentsWorkspaceQuick(name: string): Promise<void> {
  closeAgentsWsPicker();
  agentsWsNameEl.textContent = name;
  try {
    await invoke("use_workspace_command", { name });
    await loadWorkspaces();
  } catch {
    await loadWorkspaces();
  }
}

const PROVIDER_PRESETS: Record<
  string,
  { name: string; url: string; protocol: ProviderProtocol; models: string[] }
> = {
  openai: {
    name: "OpenAI",
    url: "https://api.openai.com/v1",
    protocol: "openai",
    models: ["gpt-4.1-mini", "gpt-4.1", "o4-mini"],
  },
  deepseek: {
    name: "DeepSeek",
    url: "https://api.deepseek.com/v1",
    protocol: "openai",
    models: ["deepseek-v4-flash", "deepseek-v4-pro"],
  },
  moonshot: {
    name: "Moonshot",
    url: "https://api.moonshot.cn/v1",
    protocol: "openai",
    models: ["kimi-k3", "kimi-k2.5"],
  },
  siliconflow: {
    name: "SiliconFlow",
    url: "https://api.siliconflow.cn/v1",
    protocol: "openai",
    models: ["deepseek-ai/DeepSeek-V3.2", "Qwen/Qwen3-235B-A22B"],
  },
  openrouter: {
    name: "OpenRouter",
    url: "https://openrouter.ai/api/v1",
    protocol: "openai",
    models: ["openai/gpt-4.1-mini", "deepseek/deepseek-v4-flash"],
  },
  groq: {
    name: "Groq",
    url: "https://api.groq.com/openai/v1",
    protocol: "openai",
    models: ["llama-3.3-70b-versatile", "openai/gpt-oss-120b"],
  },
  anthropic: {
    name: "Anthropic",
    url: "https://api.anthropic.com",
    protocol: "anthropic",
    models: ["claude-sonnet-4-5", "claude-opus-4-5", "claude-haiku-4-5"],
  },
  "deepseek-anthropic": {
    name: "DeepSeek Claude",
    url: "https://api.deepseek.com/anthropic",
    protocol: "anthropic",
    models: ["deepseek-v4-flash", "deepseek-v4-pro"],
  },
};

function protocolLabel(protocol: string): string {
  return protocol === "anthropic"
    ? t("personal.protocolBadgeClaude")
    : t("personal.protocolBadgeOpenAI");
}

function refreshPresetGroupLabels() {
  personalPresetEl.querySelectorAll("optgroup").forEach((group) => {
    const key = group.getAttribute("data-i18n-label");
    if (key === "personal.groupOpenAI" || key === "personal.groupClaude") {
      group.label = t(key);
    }
  });
}

function setModelSuggestions(models: string[]) {
  personalModelSuggestionsEl.innerHTML = "";
  for (const model of models) {
    const option = document.createElement("option");
    option.value = model;
    personalModelSuggestionsEl.appendChild(option);
  }
}

function matchPresetId(name: string, url: string, protocol?: string): string {
  const normalizedUrl = url.trim().replace(/\/+$/, "");
  for (const [id, preset] of Object.entries(PROVIDER_PRESETS)) {
    const presetUrl = preset.url.replace(/\/+$/, "");
    if (protocol && preset.protocol !== protocol) {
      continue;
    }
    if (
      normalizedUrl === presetUrl ||
      name.trim().toLowerCase() === preset.name.toLowerCase()
    ) {
      return id;
    }
  }
  return "custom";
}

function applyProviderPreset(presetId: string, { forceModel = true } = {}) {
  if (presetId === "custom" || !PROVIDER_PRESETS[presetId]) {
    personalPresetEl.value = "custom";
    personalNameRowEl.classList.remove("is-preset-locked");
    personalNameEl.readOnly = false;
    setModelSuggestions(
      personalProtocolEl.value === "anthropic"
        ? ["claude-sonnet-4-5", "claude-opus-4-5", "deepseek-v4-flash"]
        : ["deepseek-v4-flash", "deepseek-v4-pro", "gpt-4.1-mini"],
    );
    return;
  }
  const preset = PROVIDER_PRESETS[presetId];
  personalPresetEl.value = presetId;
  personalProtocolEl.value = preset.protocol;
  personalNameEl.value = preset.name;
  personalUrlEl.value = preset.url;
  setModelSuggestions(preset.models);
  if (forceModel || !personalModelEl.value.trim()) {
    personalModelEl.value = preset.models[0] ?? "";
  }
  personalNameRowEl.classList.add("is-preset-locked");
  personalNameEl.readOnly = true;
}

const mainTabsEl = document.querySelector<HTMLElement>("#main-tabs")!;
const mainPanels = Array.from(document.querySelectorAll<HTMLElement>("[data-main-panel]"));
const providerPanels = Array.from(document.querySelectorAll<HTMLElement>("[data-provider-panel]"));

type MainTabId = "diagnose" | "resources" | "provider" | "workspace";
type ProviderTabId = "personal" | "evotown";

let lastSkillsInventory: SkillsInventoryReport | null = null;
let lastMcpStatus: McpModuleStatus | null = null;
let resourceFilter: ResourceFilter = "all";
let mcpConfigureInFlight = false;

function setMainTab(tab: MainTabId) {
  mainTabsEl.querySelectorAll<HTMLButtonElement>("[data-main-tab]").forEach((button) => {
    const active = button.dataset.mainTab === tab;
    button.classList.toggle("is-active", active);
    button.setAttribute("aria-selected", active ? "true" : "false");
  });
  for (const panel of mainPanels) {
    const active = panel.dataset.mainPanel === tab;
    panel.classList.toggle("is-active", active);
    panel.hidden = !active;
  }
  if (tab === "resources") {
    void loadResourcesPanel();
  }
}

/** Provider panel follows global mode — no nested Personal/Evotown tabs. */
function syncProviderPanelToMode(mode?: string) {
  const activeMode = mode ?? lastModeStatus?.mode ?? "personal";
  const tab: ProviderTabId = activeMode === "team" ? "evotown" : "personal";
  for (const panel of providerPanels) {
    const active = panel.dataset.providerPanel === tab;
    panel.classList.toggle("is-active", active);
    panel.hidden = !active;
  }
}

function modeDisplayName(mode: string): string {
  if (mode === "personal") return t("mode.personal");
  if (mode === "team") return t("mode.team");
  return t("mode.unset");
}

let modeSwitchInFlight = false;

function renderModeStatus(status: ModeStatus) {
  lastModeStatus = status;
  modeUsePersonalEl.classList.toggle("is-active", status.mode === "personal");
  modeUseTeamEl.classList.toggle("is-active", status.mode === "team");
  // Always visible; disable only when that side has no credentials yet.
  modeUsePersonalEl.hidden = false;
  modeUseTeamEl.hidden = false;
  // Keep clickable so users can jump to the config tab even when not ready.
  // While a mode switch is in flight, keep both buttons disabled.
  modeUsePersonalEl.disabled = modeSwitchInFlight;
  modeUseTeamEl.disabled = modeSwitchInFlight;
  modeUsePersonalEl.classList.toggle("is-unavailable", !status.personal_ready);
  modeUseTeamEl.classList.toggle("is-unavailable", !status.team_ready);
  modeUsePersonalEl.title = status.personal_ready
    ? modeDisplayName("personal")
    : t("mode.personalNotReady");
  modeUseTeamEl.title = status.team_ready ? modeDisplayName("team") : t("mode.teamNotReady");

  const meta = status.active_gateway_url
    ? t("mode.meta", {
        label: status.active_label || modeDisplayName(status.mode),
        url: status.active_gateway_url,
        key: status.active_key_hint || "—",
      })
    : t("mode.metaEmpty");
  modeMetaEl.textContent = meta;
  modeUsePersonalEl.title = status.personal_ready ? meta : t("mode.personalNotReady");
  modeUseTeamEl.title = status.team_ready ? meta : t("mode.teamNotReady");
  syncProviderPanelToMode(status.mode);
  updateWiringModeFootnote(status.mode);
  updateFooterCopy(status.mode);
}

async function loadModeStatus() {
  try {
    const status = await invoke<ModeStatus>("get_mode_status_command");
    renderModeStatus(status);
  } catch (error) {
    modeMetaEl.textContent = String(error);
    modeUsePersonalEl.classList.remove("is-active");
    modeUseTeamEl.classList.remove("is-active");
    modeUsePersonalEl.classList.add("is-unavailable");
    modeUseTeamEl.classList.add("is-unavailable");
  }
}

function showModeHint(text: string, detail?: string) {
  // Keep sr-only #mode-hint for a11y, but also surface on the visible footnote —
  // otherwise mode switch looks "stuck" while buttons are disabled.
  modeHintEl.hidden = !text;
  modeHintEl.textContent = text;
  if (text) {
    wiringModeFootnoteEl.textContent = text;
    wiringModeFootnoteEl.title = detail || text;
  }
}

function setModeSwitchBusy(busy: boolean) {
  modeSwitchInFlight = busy;
  modeUsePersonalEl.disabled = busy;
  modeUseTeamEl.disabled = busy;
  modeUsePersonalEl.classList.toggle("is-busy", busy);
  modeUseTeamEl.classList.toggle("is-busy", busy);
  document.getElementById("mode-banner")?.classList.toggle("is-busy", busy);
}

async function enablePersonalMode() {
  if (modeSwitchInFlight) return;
  if (!lastModeStatus?.personal_ready) {
    showModeHint(t("mode.personalNotReady"));
    syncProviderPanelToMode("personal");
    setMainTab("provider");
    return;
  }
  const wasAlready = lastModeStatus.mode === "personal";
  setModeSwitchBusy(true);
  showModeHint(t("mode.switching"));
  try {
    const report = await invoke<ModeSwitchReport>("switch_to_personal_mode_command", {
      providerId: lastModeStatus.personal_active_id,
      withBrowserMcp: wantsBrowserMcp(),
    });
    await loadModeStatus();
    await loadPersonalProviderStatus();
    const hint = formatModeSwitchHint(report);
    showModeHint(
      t("mode.switchOk", {
        message: wasAlready ? `${t("mode.alreadyPersonal")} · ${hint}` : hint,
      }),
      formatModeSwitchDetail(report),
    );
    // Doctor rescan is secondary — don't block the switch UI on it.
    void refresh();
  } catch (error) {
    showModeHint(t("mode.switchFailed", { error: String(error) }));
    await loadModeStatus();
  } finally {
    setModeSwitchBusy(false);
  }
}

async function enableTeamMode() {
  if (modeSwitchInFlight) return;
  if (!lastModeStatus?.team_ready) {
    showModeHint(t("mode.teamNotReady"));
    syncProviderPanelToMode("team");
    setMainTab("provider");
    return;
  }
  const wasAlready = lastModeStatus.mode === "team";
  setModeSwitchBusy(true);
  showModeHint(t("mode.switching"));
  try {
    const report = await invoke<ModeSwitchReport>("switch_to_team_mode_command", {
      withBrowserMcp: wantsBrowserMcp(),
    });
    await loadModeStatus();
    await loadEvotownStatus();
    const hint = formatModeSwitchHint(report);
    showModeHint(
      t("mode.switchOk", {
        message: wasAlready ? `${t("mode.alreadyTeam")} · ${hint}` : hint,
      }),
      formatModeSwitchDetail(report),
    );
    void refresh();
  } catch (error) {
    showModeHint(t("mode.switchFailed", { error: String(error) }));
    await loadModeStatus();
  } finally {
    setModeSwitchBusy(false);
  }
}

async function rewireCurrentMode(hintEl?: HTMLElement | null) {
  if (modeSwitchInFlight) return;
  const mode = lastModeStatus?.mode;
  if (mode !== "personal" && mode !== "team") {
    setMainTab("provider");
    showModeHint(t("mode.pickSide"));
    if (hintEl) {
      hintEl.hidden = false;
      hintEl.textContent = t("runtime.installWireNext");
    }
    return;
  }
  setModeSwitchBusy(true);
  if (hintEl) {
    hintEl.hidden = false;
    hintEl.textContent = t("mode.switching");
  }
  try {
    const report = await invoke<ModeSwitchReport>("rewire_current_mode_command", {
      withBrowserMcp: wantsBrowserMcp(),
    });
    await loadModeStatus();
    const message = t("mode.switchOk", {
      message: `${t("mode.rewireOk")} · ${formatModeSwitchHint(report)}`,
    });
    showModeHint(message, formatModeSwitchDetail(report));
    if (hintEl) {
      hintEl.textContent = message;
    }
    void refresh();
  } catch (error) {
    const message = t("mode.rewireFailed", { error: String(error) });
    showModeHint(message);
    if (hintEl) {
      hintEl.hidden = false;
      hintEl.textContent = message;
    }
  } finally {
    setModeSwitchBusy(false);
  }
}

const statusEl = document.querySelector<HTMLElement>("#status")!;
const runtimesEl = document.querySelector<HTMLElement>("#runtimes")!;
const runtimeTabsEl = document.querySelector<HTMLElement>("#runtime-tabs")!;
const refreshBtn = document.querySelector<HTMLButtonElement>("#refresh")!;
const spinnerEl = refreshBtn.querySelector<HTMLElement>(".spinner")!;
const installedCountEl = document.querySelector<HTMLElement>("#installed-count")!;
const profileStatusEl = document.querySelector<HTMLElement>("#profile-status")!;
const lastScanEl = document.querySelector<HTMLElement>("#last-scan")!;
const runtimeCountEl = document.querySelector<HTMLElement>("#runtime-count")!;
const presetStatusEl = document.querySelector<HTMLElement>("#preset-status")!;
const presetApplyEl = document.querySelector<HTMLButtonElement>("#preset-apply")!;
const presetHintEl = document.querySelector<HTMLElement>("#preset-hint")!;
const presetPickerEl = document.querySelector<HTMLElement>("#preset-picker")!;
const presetTriggerEl = document.querySelector<HTMLButtonElement>("#preset-trigger")!;
const presetTriggerLabelEl = document.querySelector<HTMLElement>("#preset-trigger-label")!;
const presetMenuEl = document.querySelector<HTMLElement>("#preset-menu")!;
const workspaceStatusEl = document.querySelector<HTMLElement>("#workspace-status")!;
const workspaceListEl = document.querySelector<HTMLUListElement>("#workspace-list")!;
const workspaceChecksEl = document.querySelector<HTMLUListElement>("#workspace-checks")!;
const workspaceHintEl = document.querySelector<HTMLElement>("#workspace-hint")!;
const workspaceRegisterEl = document.querySelector<HTMLButtonElement>("#workspace-register")!;
const remoteStatusEl = document.querySelector<HTMLElement>("#remote-status")!;
const remoteListEl = document.querySelector<HTMLUListElement>("#remote-list")!;
const remoteChecksEl = document.querySelector<HTMLUListElement>("#remote-checks")!;
const remoteHintEl = document.querySelector<HTMLElement>("#remote-hint")!;
const remoteRefreshEl = document.querySelector<HTMLButtonElement>("#remote-refresh")!;
const remoteHostFormEl = document.querySelector<HTMLFormElement>("#remote-host-form")!;
const remoteProjectFormEl = document.querySelector<HTMLFormElement>("#remote-project-form")!;
const remoteHostIdEl = document.querySelector<HTMLInputElement>("#remote-host-id")!;
const remoteSshHostEl = document.querySelector<HTMLInputElement>("#remote-ssh-host")!;
const remoteProjectHostEl = document.querySelector<HTMLSelectElement>("#remote-project-host")!;
const remoteProjectNameEl = document.querySelector<HTMLInputElement>("#remote-project-name")!;
const remoteProjectPathEl = document.querySelector<HTMLInputElement>("#remote-project-path")!;
let remoteBusy = false;
const evotownBadgeEl = document.querySelector<HTMLElement>("#evotown-badge");
const langSwitchEl = document.querySelector<HTMLElement>(".lang-switch")!;
const healthPillEl = document.querySelector<HTMLElement>("#health-pill")!;
const healthLabelEl = document.querySelector<HTMLElement>("#health-label")!;

const PROVIDER_LABELS: Record<string, string> = {
  deepseek: "DeepSeek",
  openai: "OpenAI",
  anthropic: "Claude",
  ollama: "Ollama",
};

const RUNTIME_SHORT: Record<string, string> = {
  openclaw: "OC",
  hermes: "HE",
  "claude-code": "CC",
  codex: "CX",
};

interface HermesModelOption {
  provider: string;
  model: string;
  base_url: string;
  label: string;
  group: "common" | "saved" | "custom";
}

const COMMON_HERMES_MODELS: HermesModelOption[] = [
  {
    provider: "deepseek",
    model: "deepseek-v4-flash",
    base_url: "https://api.deepseek.com/v1",
    label: "DeepSeek · deepseek-v4-flash",
    group: "common",
  },
  {
    provider: "openai",
    model: "gpt-4o",
    base_url: "https://api.openai.com/v1",
    label: "OpenAI · gpt-4o",
    group: "common",
  },
  {
    provider: "openai",
    model: "gpt-4o-mini",
    base_url: "https://api.openai.com/v1",
    label: "OpenAI · gpt-4o-mini",
    group: "common",
  },
  {
    provider: "anthropic",
    model: "claude-sonnet-4-20250514",
    base_url: "https://api.anthropic.com/v1",
    label: "Claude · claude-sonnet-4-20250514",
    group: "common",
  },
  {
    provider: "ollama",
    model: "llama3.2",
    base_url: "http://127.0.0.1:11434/v1",
    label: "Ollama · llama3.2",
    group: "common",
  },
];

const MODEL_PRESET_CUSTOM = "__custom__";

function modelPresetKey(option: Pick<HermesModelOption, "provider" | "model" | "base_url">): string {
  return `${option.provider}|${option.model}|${option.base_url}`;
}

function buildHermesModelOptions(): HermesModelOption[] {
  const seen = new Set<string>();
  const options: HermesModelOption[] = [];

  const push = (option: HermesModelOption) => {
    const key = modelPresetKey(option);
    if (seen.has(key)) {
      return;
    }
    seen.add(key);
    options.push(option);
  };

  for (const option of COMMON_HERMES_MODELS) {
    push(option);
  }

  const activeProfile = lastProfiles?.active;
  const profiles = lastProfiles?.profiles;
  if (activeProfile && profiles?.[activeProfile]) {
    for (const saved of effectiveModels(profiles[activeProfile])) {
      push({
        ...saved,
        label: modelChipLabel(saved),
        group: "saved",
      });
    }
  }

  return options;
}

function findMatchingPreset(
  current: Pick<HermesSettings, "provider" | "model" | "base_url">,
): string {
  const key = modelPresetKey(current);
  for (const option of buildHermesModelOptions()) {
    if (modelPresetKey(option) === key) {
      return key;
    }
  }
  return MODEL_PRESET_CUSTOM;
}

let lastReport: DoctorReport | null = null;
let lastProfiles: ProfilesDocument | null = null;
let lastWorkspaces: WorkspacesDocument | null = null;
let hermesModel: HermesSettings | null = null;
let hermesEditing = false;
let activeRuntimeId: string | null = null;

type RepairStatusFilter = "all" | RepairPreviewResponse["checks"][number]["status"];

const repairPreviewByRuntime = new Map<string, RepairPreviewResponse>();
const repairFilterByRuntime = new Map<string, RepairStatusFilter>();
let selectedPresetName = "";
let selectedWorkspaceName = "";
let presetMenuOpen = false;
let workspaceBusy = false;

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function formatTime(date: Date): string {
  const locale = getLocale() === "zh" ? "zh-CN" : "en-US";
  return date.toLocaleTimeString(locale, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function runtimeClass(id: string): string {
  if (id in RUNTIME_SHORT) {
    return id;
  }
  return "default";
}

function effectiveModels(
  entry: ProfileEntry | undefined,
): Array<Pick<HermesSettings, "provider" | "model" | "base_url">> {
  if (!entry) {
    return [];
  }
  if (entry.models && entry.models.length > 0) {
    return entry.models;
  }
  return entry.hermes ? [entry.hermes] : [];
}

function modelChipLabel(model: Pick<HermesSettings, "provider" | "model">): string {
  const provider = PROVIDER_LABELS[model.provider] ?? model.provider;
  return `${provider} · ${model.model}`;
}

function applyModelPresetToCard(card: HTMLElement, presetKey: string): void {
  if (presetKey === MODEL_PRESET_CUSTOM) {
    return;
  }
  const [provider, model, baseUrl] = presetKey.split("|");
  if (!provider || !model || !baseUrl) {
    return;
  }
  const providerEl = card.querySelector<HTMLInputElement>('[data-field="provider"]');
  const modelEl = card.querySelector<HTMLInputElement>('[data-field="model"]');
  const baseUrlEl = card.querySelector<HTMLInputElement>('[data-field="base_url"]');
  if (providerEl) {
    providerEl.value = provider;
  }
  if (modelEl) {
    modelEl.value = model;
  }
  if (baseUrlEl) {
    baseUrlEl.value = baseUrl;
  }
}

function setStatusBanner(
  kind: "ok" | "warn" | "error" | "neutral",
  message: string,
): void {
  statusEl.textContent = message;
  statusEl.classList.remove("is-ok", "is-warn", "is-error");
  if (kind === "neutral") {
    statusEl.hidden = true;
    return;
  }
  statusEl.hidden = false;
  statusEl.classList.add(`is-${kind}`);
}

function updateHealthStrip(installed: number, total: number, scanning = false): void {
  healthPillEl.classList.remove("is-good", "is-partial", "is-bad", "is-scanning");
  if (scanning) {
    healthPillEl.classList.add("is-scanning");
    healthLabelEl.textContent = t("health.scanning");
    return;
  }
  if (total === 0 || installed === 0) {
    healthPillEl.classList.add("is-bad");
    healthLabelEl.textContent = t("health.bad");
    return;
  }
  if (installed === total) {
    healthPillEl.classList.add("is-good");
    healthLabelEl.textContent = t("health.good");
    return;
  }
  healthPillEl.classList.add("is-partial");
  healthLabelEl.textContent = t("health.partial", {
    installed: String(installed),
    total: String(total),
  });
}

function metaRow(labelKey: Parameters<typeof t>[0], value: string): string {
  return `
    <div class="meta-row">
      <span class="meta-label">${t(labelKey)}</span>
      <p class="meta-value">${escapeHtml(value)}</p>
    </div>
  `;
}

function metaInput(
  labelKey: Parameters<typeof t>[0],
  field: string,
  value: string,
  inputType = "text",
  placeholder = "",
): string {
  return `
    <label class="meta-row meta-row-edit">
      <span class="meta-label">${t(labelKey)}</span>
      <input class="meta-input" data-field="${field}" type="${inputType}" value="${escapeHtml(value)}" placeholder="${escapeHtml(placeholder)}" />
    </label>
  `;
}

function metaSelect(
  labelKey: Parameters<typeof t>[0],
  field: string,
  options: Array<{ value: string; label: string; group?: string }>,
  selectedValue: string,
): string {
  const groups = new Map<string, Array<{ value: string; label: string }>>();
  for (const option of options) {
    const group = option.group ?? "";
    if (!groups.has(group)) {
      groups.set(group, []);
    }
    groups.get(group)!.push(option);
  }

  const body = [...groups.entries()]
    .map(([group, items]) => {
      const opts = items
        .map(
          (item) =>
            `<option value="${escapeHtml(item.value)}" ${item.value === selectedValue ? "selected" : ""}>${escapeHtml(item.label)}</option>`,
        )
        .join("");
      if (!group) {
        return opts;
      }
      const groupLabel =
        group === "common"
          ? t("meta.modelGroupCommon")
          : group === "saved"
            ? t("meta.modelGroupSaved")
            : "";
      if (!groupLabel) {
        return opts;
      }
      return `<optgroup label="${escapeHtml(groupLabel)}">${opts}</optgroup>`;
    })
    .join("");

  return `
    <label class="meta-row meta-row-edit">
      <span class="meta-label">${t(labelKey)}</span>
      <select class="meta-input meta-select" data-field="${field}">${body}</select>
    </label>
  `;
}

function renderHermesModelPresetSelect(
  current: Pick<HermesSettings, "provider" | "model" | "base_url">,
): string {
  const selected = findMatchingPreset(current);
  const options = buildHermesModelOptions().map((option) => ({
    value: modelPresetKey(option),
    label: option.label,
    group: option.group,
  }));
  options.push({
    value: MODEL_PRESET_CUSTOM,
    label: t("meta.modelCustom"),
    group: "custom",
  });

  return metaSelect("meta.modelPreset", "model-preset", options, selected);
}

function renderApiKeyRow(settings: HermesSettings): string {
  if (!settings.api_key_env) {
    return metaRow("meta.apiKey", t("meta.apiKeyOptional"));
  }
  if (settings.api_key_configured && settings.api_key_hint) {
    return metaRow(
      "meta.apiKey",
      t("meta.apiKeySet", { hint: settings.api_key_hint }),
    );
  }
  return metaRow(
    "meta.apiKey",
    t("meta.apiKeyMissing", { env: settings.api_key_env }),
  );
}

function renderRepairSummaryChip(
  filter: RepairStatusFilter,
  count: number,
  className: string,
  label: string,
  activeFilter: RepairStatusFilter,
): string {
  const isActive = activeFilter === filter;
  const disabled = count === 0;
  return `
    <button
      type="button"
      class="repair-chip ${className}${isActive ? " is-active" : ""}"
      data-repair-filter="${filter}"
      aria-pressed="${isActive}"
      ${disabled ? "disabled" : ""}
    >
      ${count} ${label}
    </button>
  `;
}

function renderRepairPreview(
  report: RepairPreviewResponse,
  activeFilter: RepairStatusFilter = "all",
): string {
  const summary = report.summary;
  const visibleChecks =
    activeFilter === "all"
      ? report.checks
      : report.checks.filter((check) => check.status === activeFilter);

  const checks = visibleChecks
    .map((check) => {
      const statusClass = repairStatusClass(check.status);
      const details = check.details.length
        ? `<span class="repair-check-detail">${escapeHtml(check.details[0])}${check.details.length > 1 ? ` +${check.details.length - 1}` : ""}</span>`
        : "";
      return `
        <li class="repair-check">
          <span class="repair-check-status ${statusClass}">${escapeHtml(repairCheckStatusLabel(check.status))}</span>
          <span class="repair-check-body">
            <strong>${escapeHtml(check.title)}</strong>
            <span>${escapeHtml(check.message)}</span>
            ${details}
          </span>
        </li>
      `;
    })
    .join("");

  const summaryChips = [
    { filter: "all" as const, count: report.checks.length, className: "all", label: t("repair.all") },
    { filter: "pass" as const, count: summary.pass, className: "pass", label: t("repair.pass") },
    { filter: "warn" as const, count: summary.warn, className: "warn", label: t("repair.warn") },
    { filter: "fail" as const, count: summary.fail, className: "fail", label: t("repair.fail") },
    {
      filter: "not checked" as const,
      count: summary.not_checked,
      className: "muted",
      label: t("repair.notChecked"),
    },
    {
      filter: "n/a" as const,
      count: summary.not_applicable,
      className: "muted",
      label: t("repair.notApplicable"),
    },
  ]
    .filter((chip) => chip.filter === "all" || chip.count > 0)
    .map((chip) =>
      renderRepairSummaryChip(chip.filter, chip.count, chip.className, chip.label, activeFilter),
    )
    .join("");

  const emptyList =
    visibleChecks.length === 0
      ? `<li class="repair-check repair-check-empty">${escapeHtml(t("repair.noMatches"))}</li>`
      : "";

  const suggested = report.suggested_repairs.length
    ? `
      <div class="repair-suggested">
        <p class="repair-suggested-title">${escapeHtml(t("repair.suggestedTitle"))}</p>
        <ul class="repair-suggested-list">
          ${report.suggested_repairs
            .map(
              (item) => `
            <li class="repair-suggested-item">
              <span class="repair-suggested-badge ${item.auto_fixable ? "ok" : "muted"}">${
                item.auto_fixable ? t("repair.autoFixable") : t("repair.manualOnly")
              }</span>
              <span class="repair-suggested-body">
                <strong>${escapeHtml(item.title)}</strong>
                <span>${escapeHtml(item.description)}</span>
              </span>
            </li>
          `,
            )
            .join("")}
        </ul>
      </div>
    `
    : "";

  const applyButton = report.can_apply_repair
    ? `<button type="button" class="btn-secondary repair-apply-btn" data-action="apply-repair">${t("repair.applyFixes")}</button>`
    : "";

  const rollbackButton =
    report.backup_ids.length > 0
      ? `<button type="button" class="btn-secondary repair-rollback-btn" data-action="rollback-repair">${t("repair.rollback")}</button>`
      : "";

  const executeResult = report.last_execute
    ? renderRepairExecuteResult(report.last_execute)
    : "";

  const planLine = report.last_execute
    ? ""
    : `<p class="repair-plan">${escapeHtml(report.plan_summary)}</p>`;

  return `
    <div class="repair-panel">
      <div class="repair-panel-head">
        <strong>${escapeHtml(report.display_name)}</strong>
        <span>${t("runtime.diagnosisReady")}</span>
      </div>
      <div class="repair-summary" role="tablist" aria-label="${escapeHtml(t("repair.filterLabel"))}">
        ${summaryChips}
      </div>
      <ul class="repair-checks">${checks}${emptyList}</ul>
      ${suggested}
      <div class="repair-panel-actions">${applyButton}${rollbackButton}</div>
      ${executeResult}
      ${planLine}
    </div>
  `;
}

const REPAIR_FIX_LABEL_KEYS: Record<string, string> = {
  "backup-runtime-configs": "repair.fix.backup",
  "fix-hermes-env-permissions": "repair.fix.envPermissions",
  "fix-hermes-api-key-duplicates": "repair.fix.apiKeyDedupe",
  "fix-hermes-api-key-scaffold": "repair.fix.apiKeyScaffold",
  "fix-hermes-config-from-profile": "repair.fix.configFromProfile",
  "fix-claude-code-gateway-from-mode": "repair.fix.claudeGateway",
  "fix-codex-gateway-from-mode": "repair.fix.codexGateway",
  "fix-claude-code-browser-mcp": "repair.fix.claudeBrowserMcp",
  "fix-codex-browser-mcp": "repair.fix.codexBrowserMcp",
};

function repairFixLabel(actionId: string): string {
  const key = REPAIR_FIX_LABEL_KEYS[actionId];
  return key ? t(key as MessageKey) : actionId;
}

function renderRepairExecuteResult(
  execute: NonNullable<RepairPreviewResponse["last_execute"]>,
): string {
  const playbookExecuted = execute.executed.filter((id) => id.startsWith("fix-"));
  const hasBackup = execute.executed.includes("backup-runtime-configs");

  const executedLines = execute.executed.map((id) => repairFixLabel(id));

  const outcome =
    playbookExecuted.length === 0 && execute.skipped.length === 0
      ? `<p class="repair-execute-ok">${escapeHtml(t("repair.nothingToFix"))}</p>`
      : "";

  const executedBlock =
    executedLines.length > 0
      ? `<p><strong>${escapeHtml(t("repair.executed"))}:</strong> ${escapeHtml(executedLines.join("、"))}</p>`
      : "";

  const skippedBlock =
    execute.skipped.length > 0
      ? `<p><strong>${escapeHtml(t("repair.skipped"))}:</strong> ${escapeHtml(
          execute.skipped.map((item) => `${repairFixLabel(item.id)} (${item.reason})`).join("；"),
        )}</p>`
      : "";

  const verify = formatVerificationSummary(execute.verification_summary);

  const guideBlock = execute.guide_path
    ? `<p class="repair-guide"><button type="button" class="btn-link repair-guide-btn" data-action="open-repair-guide" data-guide-path="${encodeURIComponent(execute.guide_path)}">${escapeHtml(t("repair.openGuide"))}</button></p>`
    : "";

  return `
    <div class="repair-execute-result">
      ${
        hasBackup
          ? `<p class="repair-execute-backup">${escapeHtml(t("repair.applyResult", { backup: execute.backup_root }))}</p>`
          : ""
      }
      ${outcome}
      ${executedBlock}
      ${skippedBlock}
      ${guideBlock}
      <p class="repair-verify"><strong>${escapeHtml(t("repair.verifyTitle"))}:</strong> ${escapeHtml(verify)}</p>
    </div>
  `;
}

function formatVerificationSummary(summary: string): string {
  const match = summary.match(/^before:\s*(.+?);\s*after:\s*(.+)$/);
  if (!match) {
    return summary;
  }
  return `${match[1]} → ${match[2]}`;
}

function mountRepairPreview(hint: HTMLElement, report: RepairPreviewResponse): void {
  const runtime = report.runtime_id;
  repairPreviewByRuntime.set(runtime, report);
  const filter = repairFilterByRuntime.get(runtime) ?? "all";
  hint.innerHTML = renderRepairPreview(report, filter);
  const card = runtimesEl.querySelector<HTMLElement>(`[data-runtime="${runtime}"]`);
  if (card) {
    mountRelatedResources(card, runtime);
  }
}

function applyRepairFilter(runtime: string, filter: RepairStatusFilter): void {
  const report = repairPreviewByRuntime.get(runtime);
  const card = runtimesEl.querySelector<HTMLElement>(`[data-runtime="${runtime}"]`);
  const hint = card?.querySelector<HTMLElement>("[data-repair-hint]");
  if (!report || !hint) {
    return;
  }
  const current = repairFilterByRuntime.get(runtime) ?? "all";
  const next = current === filter && filter !== "all" ? "all" : filter;
  repairFilterByRuntime.set(runtime, next);
  hint.innerHTML = renderRepairPreview(report, next);
}

function repairCheckStatusLabel(
  status: RepairPreviewResponse["checks"][number]["status"],
): string {
  switch (status) {
    case "pass":
      return t("repair.pass");
    case "warn":
      return t("repair.warn");
    case "fail":
      return t("repair.fail");
    case "n/a":
      return t("repair.notApplicable");
    case "not checked":
      return t("repair.notChecked");
    default:
      return status;
  }
}

function repairStatusClass(status: RepairPreviewResponse["checks"][number]["status"]): string {
  if (status === "pass") {
    return "pass";
  }
  if (status === "warn") {
    return "warn";
  }
  if (status === "fail") {
    return "fail";
  }
  return "muted";
}

function runtimeHasProblems(runtimeId: string): boolean {
  const report = repairPreviewByRuntime.get(runtimeId);
  if (!report) {
    return false;
  }
  return report.summary.fail > 0 || report.summary.warn > 0;
}

function renderRuntimeCardActions(
  runtime: RuntimeDoctorResult,
  options?: { includeEdit?: boolean },
): string {
  if (!runtime.installed) {
    return `<button type="button" class="btn-primary" data-action="install-runtime">${t("runtime.install")}</button>`;
  }

  const parts: string[] = [];
  if (canOpenSession(runtime.id)) {
    parts.push(
      `<button type="button" class="btn-primary" data-action="open-session">${t("runtime.open")}</button>`,
    );
  }
  if (runtime.id === "claude-code" || runtime.id === "codex") {
    parts.push(
      `<button type="button" class="btn-secondary" data-action="wire-runtime" title="${escapeHtml(t("runtime.wireRuntimeHint"))}">${t("runtime.wireRuntime")}</button>`,
    );
  }
  const diagnoseClass = runtimeHasProblems(runtime.id) ? "btn-secondary" : "btn-ghost";
  parts.push(
    `<button type="button" class="${diagnoseClass}" data-action="diagnose-runtime">${t("runtime.diagnose")}</button>`,
  );
  parts.push(
    `<button type="button" class="btn-ghost" data-action="install-runtime">${t("runtime.install")}</button>`,
  );
  if (options?.includeEdit) {
    parts.push(
      `<button type="button" class="btn-ghost" data-action="edit-hermes">${t("runtime.edit")}</button>`,
    );
  }
  return parts.join("");
}

type RelatedTag = { kind: "skill" | "mcp"; name: string; broken: boolean };

function extractRelatedTags(report: RepairPreviewResponse | undefined): RelatedTag[] {
  if (!report) {
    return [];
  }
  const tags: RelatedTag[] = [];
  const seen = new Set<string>();
  for (const check of report.checks) {
    const blob = `${check.title} ${check.message}`;
    if (!/mcp|skill/i.test(blob)) {
      continue;
    }
    const kind: RelatedTag["kind"] = /skill/i.test(blob) && !/mcp/i.test(check.title)
      ? "skill"
      : "mcp";
    const nameMatch =
      check.message.match(/[`'"]([a-z0-9._/-]+)[`'"]/i) ||
      check.title.match(/(?:mcp|skill)[._-]?([a-z0-9._/-]+)/i);
    const name = (nameMatch?.[1] || check.title).replace(/^(mcp|skill)[._-]?/i, "") || check.title;
    const key = `${kind}:${name}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    tags.push({
      kind,
      name,
      broken: check.status === "fail" || check.status === "warn",
    });
  }
  return tags;
}

function renderRelatedResourcesHtml(runtimeId: string): string {
  const tags = extractRelatedTags(repairPreviewByRuntime.get(runtimeId));
  const tagsHtml = tags.length
    ? tags
        .map((tag) => {
          const label = tag.kind === "skill" ? "Skill" : "MCP";
          const broken = tag.broken ? ` · ${t("agents.relatedMissing")}` : "";
          return `<span class="mini-tag ${tag.kind}${tag.broken ? " broken" : ""}">${label} · ${escapeHtml(tag.name)}${broken}</span>`;
        })
        .join("")
    : `<span class="tone-soft">${escapeHtml(t("agents.relatedEmpty"))}</span>`;
  return `
    <div class="mounted" data-related-resources>
      <h3>${escapeHtml(t("agents.relatedTitle"))}</h3>
      <div class="mini-tags">${tagsHtml}</div>
    </div>
  `;
}

function mountRelatedResources(card: HTMLElement, runtime: string): void {
  const el = card.querySelector<HTMLElement>("[data-related-resources]");
  if (!el) {
    return;
  }
  el.outerHTML = renderRelatedResourcesHtml(runtime);
}

function renderHermesCard(runtime: RuntimeDoctorResult): string {
  const model = hermesModel ?? {
    provider: "",
    model: "",
    base_url: runtime.profile.gateway_url ?? "",
    api_key_env: null,
    api_key_configured: false,
    api_key_hint: null,
  };

  const actionButtons = hermesEditing ? "" : renderRuntimeCardActions(runtime, { includeEdit: true });

  const meta = hermesEditing
    ? [
        renderHermesModelPresetSelect(model),
        metaInput("meta.provider", "provider", model.provider),
        metaInput("meta.model", "model", model.model),
        metaInput("meta.gateway", "base_url", model.base_url),
        model.api_key_env
          ? metaInput(
              "meta.apiKey",
              "api_key",
              "",
              "password",
              t("meta.apiKeyPlaceholder"),
            )
          : "",
      ].join("")
    : [
        model.provider ? metaRow("meta.provider", model.provider) : "",
        model.model ? metaRow("meta.model", model.model) : "",
        model.base_url ? metaRow("meta.gateway", model.base_url) : "",
        renderApiKeyRow(model),
        runtime.profile.key_source
          ? metaRow("meta.secrets", runtime.profile.key_source)
          : "",
        runtime.version ? metaRow("meta.version", runtime.version) : "",
        runtime.binary_path ? metaRow("meta.binary", runtime.binary_path) : "",
        runtime.config_paths.length
          ? metaRow("meta.config", runtime.config_paths.join("\n"))
          : "",
      ]
        .filter(Boolean)
        .join("");

  const actions = hermesEditing
    ? `
      <div class="card-actions">
        <button type="button" class="btn-secondary" data-action="cancel-hermes">${t("runtime.cancel")}</button>
        <button type="button" class="btn-primary" data-action="save-hermes">${t("runtime.save")}</button>
      </div>
      <p class="card-hint" data-hermes-hint>${t("runtime.saveHint")}</p>
    `
    : actionButtons
      ? `<div class="card-actions">${actionButtons}</div>`
      : "";

  return `
    <article class="runtime hermes ${hermesEditing ? "is-editing" : ""}" data-runtime="hermes">
      <div class="section-label runtime-card-label">
        <h2 class="runtime-tab-title">${escapeHtml(runtime.display_name)}</h2>
        <span class="badge ok">${t("runtime.installed")}</span>
      </div>
      ${meta ? `<div class="meta-grid">${meta}</div>` : ""}
      ${actions}
      ${renderRelatedResourcesHtml("hermes")}
      <div class="card-hint repair-hint" data-repair-hint hidden></div>
    </article>
  `;
}

function renderRuntimeCard(runtime: RuntimeDoctorResult): string {
  if (runtime.id === "hermes" && runtime.installed) {
    return renderHermesCard(runtime);
  }

  const state = runtime.installed ? t("runtime.installed") : t("runtime.notInstalled");
  const badgeClass = runtime.installed ? "ok" : "muted";
  const actionButtons = renderRuntimeCardActions(runtime);
  const rows = [
    runtime.version ? metaRow("meta.version", runtime.version) : "",
    runtime.binary_path ? metaRow("meta.binary", runtime.binary_path) : "",
    runtime.config_paths.length ? metaRow("meta.config", runtime.config_paths.join("\n")) : "",
    runtime.profile.gateway_url ? metaRow("meta.gateway", runtime.profile.gateway_url) : "",
    !runtime.installed ? metaRow("meta.status", t("runtime.notDetected")) : "",
  ]
    .filter(Boolean)
    .join("");

  return `
    <article class="runtime ${runtimeClass(runtime.id)}" data-runtime="${runtime.id}">
      <div class="section-label runtime-card-label">
        <h2 class="runtime-tab-title">${escapeHtml(runtime.display_name)}</h2>
        <span class="badge ${badgeClass}">${state}</span>
      </div>
      ${rows ? `<div class="meta-grid">${rows}</div>` : ""}
      ${actionButtons ? `<div class="card-actions">${actionButtons}</div>` : ""}
      ${renderRelatedResourcesHtml(runtime.id)}
      <div class="card-hint repair-hint" data-repair-hint hidden></div>
      ${
        !runtime.installed
          ? `<p class="footnote runtime-install-footnote">${escapeHtml(t("runtime.installHint"))}</p>`
          : ""
      }
    </article>
  `;
}

function canOpenSession(runtimeId: string): boolean {
  return ["claude-code", "codex", "hermes", "openclaw"].includes(runtimeId);
}

function resolveActiveRuntimeId(runtimes: RuntimeDoctorResult[]): string | null {
  if (runtimes.length === 0) {
    return null;
  }
  if (activeRuntimeId && runtimes.some((runtime) => runtime.id === activeRuntimeId)) {
    return activeRuntimeId;
  }
  return runtimes.find((runtime) => runtime.installed)?.id ?? runtimes[0].id;
}

function runtimeTabDotClass(runtime: RuntimeDoctorResult): string {
  if (!runtime.installed) {
    return "off";
  }
  if (runtimeHasProblems(runtime.id)) {
    return "warn";
  }
  return "ok";
}

function renderRuntimeTabs(runtimes: RuntimeDoctorResult[], selectedId: string): string {
  return runtimes
    .map((runtime) => {
      const active = runtime.id === selectedId;
      const shortName =
        runtime.id === "claude-code"
          ? "Claude"
          : runtime.display_name.replace(/\s+Code$/i, "");
      return `
        <button
          type="button"
          class="runtime-tab ${runtimeClass(runtime.id)} ${active ? "is-active" : ""}"
          role="tab"
          aria-selected="${active}"
          data-runtime-tab="${runtime.id}"
        >
          <span class="runtime-tab-dot ${runtimeTabDotClass(runtime)}" aria-hidden="true"></span>
          <span class="runtime-tab-label">${escapeHtml(shortName)}</span>
        </button>
      `;
    })
    .join("");
}

async function loadHermesModel(): Promise<void> {
  try {
    hermesModel = await invoke<HermesSettings>("get_hermes_model_command");
  } catch {
    hermesModel = null;
  }
}

async function renderReport(report: DoctorReport) {
  lastReport = report;
  const installed = report.runtimes.filter((runtime) => runtime.installed).length;
  const total = report.runtimes.length;

  installedCountEl.textContent = `${installed}/${total}`;
  profileStatusEl.textContent = report.active_preset ?? t("status.none");
  lastScanEl.textContent = formatTime(new Date());
  runtimeCountEl.textContent = `${installed}/${total}`;
  updateHealthStrip(installed, total);

  setStatusBanner(
    report.profile_env_exists ? "ok" : "warn",
    report.profile_env_exists ? t("doctor.companyOk") : t("doctor.companyMissing"),
  );

  if (report.runtimes.some((runtime) => runtime.id === "hermes" && runtime.installed)) {
    await loadHermesModel();
  } else {
    hermesModel = null;
    hermesEditing = false;
  }

  if (report.runtimes.length === 0) {
    activeRuntimeId = null;
    runtimeTabsEl.innerHTML = "";
    runtimesEl.innerHTML = `<div class="empty-state">${t("runtimes.empty")}</div>`;
    return;
  }

  const selectedId = resolveActiveRuntimeId(report.runtimes)!;
  activeRuntimeId = selectedId;
  runtimeTabsEl.innerHTML = renderRuntimeTabs(report.runtimes, selectedId);

  const activeRuntime = report.runtimes.find((runtime) => runtime.id === selectedId);
  runtimesEl.innerHTML = activeRuntime ? renderRuntimeCard(activeRuntime) : "";
}

function setPresetTriggerLabel(name: string | null) {
  presetTriggerLabelEl.textContent = name ?? t("presets.noActive");
}

function closePresetMenu() {
  presetMenuOpen = false;
  presetMenuEl.hidden = true;
  presetTriggerEl.setAttribute("aria-expanded", "false");
  presetPickerEl.classList.remove("is-open");
}

function openPresetMenu() {
  if (presetTriggerEl.disabled) {
    return;
  }
  presetMenuOpen = true;
  presetMenuEl.hidden = false;
  presetTriggerEl.setAttribute("aria-expanded", "true");
  presetPickerEl.classList.add("is-open");
}

function togglePresetMenu() {
  if (presetMenuOpen) {
    closePresetMenu();
  } else {
    openPresetMenu();
  }
}

function presetMeta(entry: ProfileEntry | undefined): string {
  const hermes = entry?.hermes;
  if (!hermes) {
    return "";
  }
  if (hermes.provider === "ollama") {
    return t("presets.localMeta", { model: hermes.model });
  }
  return `${hermes.provider} · ${hermes.model}`;
}

function sortPresetNames(names: string[]): string[] {
  return [...names].sort((left, right) => {
    if (left === "local") {
      return -1;
    }
    if (right === "local") {
      return 1;
    }
    return left.localeCompare(right);
  });
}

function renderPresetOptions(
  names: string[],
  active: string | null,
  profiles: Record<string, ProfileEntry>,
) {
  if (names.length === 0) {
    presetMenuEl.innerHTML = "";
    selectedPresetName = "";
    setPresetTriggerLabel(null);
    presetTriggerEl.disabled = true;
    closePresetMenu();
    return;
  }

  selectedPresetName =
    selectedPresetName && names.includes(selectedPresetName)
      ? selectedPresetName
      : (active ?? names[0]);
  setPresetTriggerLabel(selectedPresetName);
  presetTriggerEl.disabled = false;

  presetMenuEl.innerHTML = names
    .map((name) => {
      const activeOption = name === selectedPresetName;
      const meta = presetMeta(profiles[name]);
      return `
        <button
          type="button"
          class="picker-option ${activeOption ? "is-active" : ""}"
          role="option"
          aria-selected="${activeOption}"
          data-preset="${escapeHtml(name)}"
        >
          <span class="picker-option-body">
            <span class="picker-option-label">${escapeHtml(name)}</span>
            ${meta ? `<span class="picker-option-meta">${escapeHtml(meta)}</span>` : ""}
          </span>
          <span class="picker-option-check" aria-hidden="true">✓</span>
        </button>
      `;
    })
    .join("");
}

function renderProfiles(doc: ProfilesDocument) {
  lastProfiles = doc;
  const names = sortPresetNames(Object.keys(doc.profiles));
  presetStatusEl.textContent = "";

  if (names.length === 0) {
    presetApplyEl.disabled = true;
    presetHintEl.textContent = t("presets.noneHint");
    renderPresetOptions([], null, doc.profiles);
    return;
  }

  renderPresetOptions(names, doc.active, doc.profiles);
  presetApplyEl.disabled = false;
  presetHintEl.textContent = doc.active
    ? t("presets.active", { name: doc.active })
    : t("presets.noActive");
}

async function loadEvotownStatus() {
  try {
    const status = await invoke<EvotownStatus>("get_evotown_status_command");
    renderEvotownStatus(status);
  } catch (error) {
    evotownStatusEl.textContent = t("evotown.connectFailed", { error: String(error) });
  }
}

function renderEvotownStatus(status: EvotownStatus) {
  const connected = status.configured && Boolean(status.base_url);
  evotownSectionEl.classList.toggle("is-connected", connected);
  evotownConnectedEl.hidden = !connected;
  evotownStatusEl.hidden = connected;

  if (evotownBadgeEl) {
    if (connected) {
      evotownBadgeEl.hidden = false;
      evotownBadgeEl.className = "badge ok";
      evotownBadgeEl.textContent = t("evotown.connectedBadge");
    } else {
      evotownBadgeEl.hidden = true;
      evotownBadgeEl.textContent = "";
    }
  }

  if (connected && status.base_url) {
    evotownStatusEl.textContent = "";
    evotownConnectedUrlEl.textContent = status.base_url;
    evotownConnectedMetaEl.textContent = t("evotown.meta", {
      runtime: status.runtime_target ?? "openclaw",
      bundle: status.bundle_id ?? "default-agent-skills",
    });
    evotownConnectedMetaEl.title = status.api_key_hint ?? "";
    evotownUrlEl.value = status.base_url;
    evotownResyncEl.hidden = false;
    void loadSkillsInventory();
  } else {
    evotownStatusEl.textContent = t("evotown.notConfigured");
    evotownConnectedMetaEl.textContent = "";
    evotownConnectedMetaEl.title = "";
    evotownResyncEl.hidden = true;
    skillsInventoryEl.hidden = true;
  }
}

function formatRate(value: number | null | undefined): string {
  if (value == null || Number.isNaN(value)) return t("skills.na");
  return `${Math.round(value * 100)}%`;
}

function formatCount(value: number | null | undefined): string {
  if (value == null) return t("skills.na");
  return String(value);
}

const RUNTIME_LABELS: Record<string, string> = {
  hermes: "Hermes",
  openclaw: "OpenClaw",
  "claude-code": "Claude",
  codex: "Codex",
};

function renderSkillsInventory(report: SkillsInventoryReport) {
  skillsInventoryEl.hidden = false;
  skillsDirEl.textContent = t("skills.dir", { dir: report.skills_dir });
  skillsFootnoteEl.textContent = t("skills.footnote");
  skillsListEl.replaceChildren();
  skillsEmptyEl.textContent = t("skills.empty");

  const empty = report.skills.length === 0;
  skillsEmptyEl.hidden = !empty;
  if (empty) return;

  for (const skill of report.skills) {
    const li = document.createElement("li");
    li.className = "skills-item";

    const top = document.createElement("div");
    top.className = "skills-item-top";
    const name = document.createElement("div");
    name.className = "skills-name";
    name.textContent = skill.name || skill.skill_id;
    const ver = document.createElement("div");
    ver.className = "skills-ver";
    ver.textContent =
      skill.download_count != null
        ? `v${skill.version} · ↓${skill.download_count}`
        : `v${skill.version}`;
    top.append(name, ver);

    if (skill.description) {
      const desc = document.createElement("p");
      desc.className = "skills-desc";
      desc.textContent = skill.description;
      li.append(top, desc);
    } else {
      li.append(top);
    }

    const agents = document.createElement("div");
    agents.className = "skills-agents";
    if (skill.agents.length === 0) {
      const none = document.createElement("span");
      none.className = "skills-runtime";
      none.textContent = t("skills.na");
      agents.appendChild(none);
    } else {
      for (const agent of skill.agents) {
        const chip = document.createElement("button");
        chip.type = "button";
        chip.className = agent.mounted ? "skills-runtime is-on" : "skills-runtime";
        const runtimeLabel = RUNTIME_LABELS[agent.runtime] ?? agent.runtime;
        chip.title = agent.mounted
          ? `${t("skills.unmountRuntime", { runtime: runtimeLabel })}\n${agent.path}`
          : `${t("skills.mountRuntime", { runtime: runtimeLabel })}\n${agent.path}`;
        const dot = document.createElement("i");
        dot.className = "skills-runtime-dot";
        const label = document.createElement("span");
        label.textContent = runtimeLabel;
        chip.append(dot, label);
        chip.addEventListener("click", () => {
          void toggleSkillRuntimeMount(chip, skill.skill_id, agent.runtime, agent.mounted);
        });
        agents.appendChild(chip);
      }
    }

    const metrics = document.createElement("div");
    metrics.className = "skills-metrics";
    const metricDefs: Array<[string, string]> = [
      [t("skills.colCalls"), formatCount(skill.call_count)],
      [t("skills.colFirstOk"), formatRate(skill.first_success_rate)],
      [t("skills.colSuccess"), formatRate(skill.success_rate)],
    ];
    for (const [label, value] of metricDefs) {
      const cell = document.createElement("div");
      cell.className = "skills-metric";
      const span = document.createElement("span");
      span.textContent = label;
      const strong = document.createElement("strong");
      strong.textContent = value;
      cell.append(span, strong);
      metrics.appendChild(cell);
    }

    const actions = document.createElement("div");
    actions.className = "skills-item-actions";
    const needsMount = skill.agents.some((a) => !a.mounted);
    if (needsMount) {
      const mountBtn = document.createElement("button");
      mountBtn.type = "button";
      mountBtn.className = "btn-secondary btn-compact";
      mountBtn.textContent = t("skills.mountOne");
      mountBtn.title = t("skills.mountAll");
      mountBtn.addEventListener("click", () => {
        void mountSyncedSkills([skill.skill_id]);
      });
      actions.appendChild(mountBtn);
    }

    li.append(agents, metrics, actions);
    skillsListEl.appendChild(li);
  }
}

async function loadSkillsInventory(opts?: { remoteStats?: boolean }) {
  try {
    const report = await invoke<SkillsInventoryReport>("list_skills_inventory_command", {
      remoteStats: opts?.remoteStats ?? true,
    });
    lastSkillsInventory = report;
    skillsCountEl.textContent = String(report.skills.length);
    renderSkillsInventory(report);
    renderResourcesList();
  } catch (error) {
    lastSkillsInventory = null;
    skillsCountEl.textContent = "—";
    skillsInventoryEl.hidden = false;
    skillsListEl.replaceChildren();
    skillsEmptyEl.hidden = false;
    skillsEmptyEl.textContent = t("skills.loadFailed", { error: String(error) });
    skillsDirEl.textContent = "";
    skillsFootnoteEl.textContent = "";
    renderResourcesList();
  }
}

function scopeLabel(scope: string): string {
  if (scope === "project") return t("resources.scopeProject");
  return t("resources.scopeGlobal");
}

function isShowBrowserUi(): boolean {
  return mcpShowUiEl.checked;
}

function persistShowBrowserUi(show: boolean) {
  try {
    localStorage.setItem(MCP_SHOW_UI_KEY, show ? "1" : "0");
  } catch {
    // ignore quota / private mode
  }
}

function selectedUserDataDir(): string {
  return mcpUserDataDirEl.value.trim();
}

function selectedProfileDirectory(): string {
  return mcpProfileDirectoryEl.value.trim() || "Default";
}

function persistUserDataDir(path: string) {
  try {
    localStorage.setItem(MCP_USER_DATA_DIR_KEY, path.trim());
  } catch {
    // ignore
  }
}

function persistProfileDirectory(name: string) {
  try {
    localStorage.setItem(MCP_PROFILE_DIRECTORY_KEY, name.trim() || "Default");
  } catch {
    // ignore
  }
}

function syncShowBrowserUiPreference(status: McpModuleStatus) {
  try {
    const saved = localStorage.getItem(MCP_SHOW_UI_KEY);
    if (saved === "0" || saved === "1") {
      mcpShowUiEl.checked = saved === "1";
      return;
    }
  } catch {
    // fall through
  }
  const browser = status.inventory.servers.find((server) => server.is_browser);
  if (browser) {
    mcpShowUiEl.checked = !browser.args.includes("--headless");
    return;
  }
  mcpShowUiEl.checked = true;
}

function configuredBrowserArg(status: McpModuleStatus, flag: string): string | null {
  const browser = status.inventory.servers.find((server) => server.is_browser);
  if (!browser) return null;
  const idx = browser.args.findIndex((arg) => arg === flag);
  if (idx >= 0 && browser.args[idx + 1]) {
    return browser.args[idx + 1];
  }
  return null;
}

function syncProfileModeButtons() {
  const dir = selectedUserDataDir();
  const isolated = lastMcpStatus?.browser.isolated_user_data_dir || "";
  const system = lastMcpStatus?.browser.system_user_data_dir || "";
  const isIsolated = Boolean(isolated) && dir === isolated;
  const isSystem = Boolean(system) && dir === system;
  mcpProfileIsolatedEl.classList.toggle("is-active", isIsolated);
  mcpProfileSystemEl.classList.toggle("is-active", isSystem);
}

function syncUserDataDirPreference(status: McpModuleStatus) {
  const chrome = status.browser;
  // Prefer what's actually written to Claude/Codex, then local draft, then isolated.
  const fromConfig = configuredBrowserArg(status, "--user-data-dir");
  let saved: string | null = null;
  try {
    saved = localStorage.getItem(MCP_USER_DATA_DIR_KEY);
  } catch {
    saved = null;
  }
  mcpUserDataDirEl.value =
    (fromConfig && fromConfig.trim()) ||
    (saved && saved.trim()) ||
    chrome.isolated_user_data_dir ||
    chrome.user_data_dir ||
    "";

  const profileFromConfig = configuredBrowserArg(status, "--profile-directory");
  let savedProfile: string | null = null;
  try {
    savedProfile = localStorage.getItem(MCP_PROFILE_DIRECTORY_KEY);
  } catch {
    savedProfile = null;
  }
  mcpProfileDirectoryEl.value =
    (profileFromConfig && profileFromConfig.trim()) ||
    (savedProfile && savedProfile.trim()) ||
    chrome.profile_directory ||
    "Default";

  syncProfileModeButtons();
}

function refreshMcpSnippet() {
  if (!lastMcpStatus) return;
  const port = lastMcpStatus.browser.port;
  const args = ["mcp", "browser", "--port", String(port)];
  if (!isShowBrowserUi()) {
    args.push("--headless");
  }
  const dir = selectedUserDataDir();
  if (dir) {
    args.push("--user-data-dir", dir);
  }
  args.push("--profile-directory", selectedProfileDirectory());
  mcpSnippetEl.textContent = JSON.stringify(
    {
      mcpServers: {
        browser: {
          command: lastMcpStatus.binary,
          args,
        },
      },
    },
    null,
    2,
  );
}

function renderMcpBrowserStatus(status: McpModuleStatus) {
  lastMcpStatus = status;
  mcpCountEl.textContent = String(status.inventory.total);

  const chrome = status.browser;
  mcpChromeEl.textContent = chrome.chrome_found
    ? t("mcp.chromeOk", { version: chrome.version || chrome.binary || "OK" })
    : t("mcp.chromeMissing");
  mcpChromeEl.title = chrome.binary || "";
  mcpCdpEl.textContent = chrome.cdp_connected
    ? t("mcp.cdpConnected", { port: String(chrome.port) })
    : t("mcp.cdpIdle", { port: String(chrome.port) });
  mcpConfiguredEl.textContent =
    status.configured_runtimes.length > 0
      ? t("mcp.configuredList", { list: status.configured_runtimes.join(", ") })
      : t("mcp.configuredNone");
  mcpBinaryEl.textContent = status.binary;
  mcpBinaryEl.title = status.binary;
  syncShowBrowserUiPreference(status);
  syncUserDataDirPreference(status);
  refreshMcpSnippet();

  mcpBrowserBadgeEl.classList.remove("ok", "warn", "muted", "bad");
  if (!chrome.chrome_found) {
    mcpBrowserBadgeEl.textContent = t("mcp.badgeMissing");
    mcpBrowserBadgeEl.classList.add("bad");
  } else if (status.configured_runtimes.length > 0) {
    mcpBrowserBadgeEl.textContent = t("mcp.badgeReady");
    mcpBrowserBadgeEl.classList.add("ok");
  } else {
    mcpBrowserBadgeEl.textContent = t("mcp.badgePartial");
    mcpBrowserBadgeEl.classList.add("warn");
  }

  const canConfigure = chrome.chrome_found && !mcpConfigureInFlight;
  mcpConfigureCodexEl.disabled = !canConfigure;
  mcpConfigureClaudeEl.disabled = !canConfigure;
}

function buildResourceRows(): ResourceRow[] {
  const rows: ResourceRow[] = [];

  for (const server of lastMcpStatus?.inventory.servers ?? []) {
    const scope = `${scopeLabel(server.scope)} · ${server.runtime_hint}`;
    const meta = server.is_browser
      ? t("resources.mcpBrowser")
      : server.healthy
        ? t("resources.mcpHealthy")
        : server.issue || t("resources.mcpIssue");
    rows.push({
      kind: "mcp",
      name: server.name,
      sub: scope,
      meta,
      tone: server.healthy ? (server.is_browser ? "ok" : "ok") : "bad",
      issue: !server.healthy,
    });
  }

  for (const skill of lastSkillsInventory?.skills ?? []) {
    const mounted = skill.agents.filter((a) => a.mounted).length;
    const needsMount = skill.agents.some((a) => !a.mounted);
    const calls = formatCount(skill.call_count);
    const rate = formatRate(skill.first_success_rate);
    rows.push({
      kind: "skill",
      name: skill.name || skill.skill_id,
      sub: t("resources.skillMounted", { count: String(mounted) }),
      meta: t("resources.skillUsage", { calls, rate }),
      tone: needsMount ? "warn" : "ok",
      issue: needsMount,
      skillId: skill.skill_id,
      needsMount,
    });
  }

  return rows;
}

function renderResourcesList() {
  const rows = buildResourceRows().filter((row) => {
    if (resourceFilter === "all") return true;
    if (resourceFilter === "issue") return row.issue;
    return row.kind === resourceFilter;
  });

  resourcesListEl.replaceChildren();
  resourcesEmptyEl.hidden = rows.length > 0;
  resourcesFootnoteEl.textContent =
    lastMcpStatus?.inventory.workspace_name
      ? `${lastMcpStatus.inventory.workspace_name}${
          lastMcpStatus.inventory.workspace_path
            ? ` · ${lastMcpStatus.inventory.workspace_path}`
            : ""
        }`
      : "";

  for (const row of rows) {
    const li = document.createElement("li");
    li.className = "res-item";

    const info = document.createElement("div");
    const strong = document.createElement("strong");
    strong.textContent = row.name;
    const sub = document.createElement("span");
    sub.textContent = `${row.kind === "skill" ? "Skill" : "MCP"} · ${row.sub}`;
    info.append(strong, sub);

    const metaWrap = document.createElement("div");
    metaWrap.className = "res-item-meta";
    const meta = document.createElement("span");
    meta.className = `tone-${row.tone}`;
    meta.textContent = row.meta;
    metaWrap.appendChild(meta);

    if (row.kind === "mcp" && row.issue) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "btn-ghost btn-compact";
      btn.textContent = t("resources.goDiagnose");
      btn.addEventListener("click", () => setMainTab("diagnose"));
      metaWrap.appendChild(btn);
    } else if (row.kind === "skill" && row.needsMount && row.skillId) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "btn-secondary btn-compact";
      btn.textContent = t("resources.mount");
      const skillId = row.skillId;
      btn.addEventListener("click", () => {
        void mountSyncedSkills([skillId]);
      });
      metaWrap.appendChild(btn);
    }

    li.append(info, metaWrap);
    resourcesListEl.appendChild(li);
  }
}

async function loadMcpStatus() {
  try {
    const status = await invoke<McpModuleStatus>("mcp_status_command", { port: null });
    renderMcpBrowserStatus(status);
    renderResourcesList();
  } catch (error) {
    lastMcpStatus = null;
    mcpCountEl.textContent = "—";
    mcpBrowserBadgeEl.textContent = "—";
    mcpBrowserBadgeEl.classList.remove("ok", "warn", "bad");
    mcpBrowserBadgeEl.classList.add("muted");
    mcpFootnoteEl.textContent = t("mcp.loadFailed", { error: String(error) });
    renderResourcesList();
  }
}

async function loadResourcesPanel() {
  await Promise.all([loadMcpStatus(), loadSkillsInventory({ remoteStats: false })]);
}

async function configureBrowserMcp(runtime: "codex" | "claude-code") {
  if (mcpConfigureInFlight) return;
  mcpConfigureInFlight = true;
  mcpConfigureCodexEl.disabled = true;
  mcpConfigureClaudeEl.disabled = true;
  mcpFootnoteEl.textContent = t("mcp.configuring");

  try {
    const showUi = isShowBrowserUi();
    persistShowBrowserUi(showUi);
    const userDataDir = selectedUserDataDir();
    const profileDirectory = selectedProfileDirectory();
    persistUserDataDir(userDataDir);
    persistProfileDirectory(profileDirectory);
    const report = await invoke<McpConfigureReport>("mcp_configure_command", {
      runtime,
      port: null,
      headless: !showUi,
      userDataDir: userDataDir || null,
      profileDirectory,
    });
    mcpFootnoteEl.textContent = t("mcp.configureOk", {
      runtime: report.runtime,
      path: report.config_path,
    });
    await loadMcpStatus();
  } catch (error) {
    mcpFootnoteEl.textContent = t("mcp.configureFailed", { error: String(error) });
  } finally {
    mcpConfigureInFlight = false;
    const chromeOk = lastMcpStatus?.browser.chrome_found ?? false;
    mcpConfigureCodexEl.disabled = !chromeOk;
    mcpConfigureClaudeEl.disabled = !chromeOk;
  }
}

function setSkillsBusy(busy: boolean) {
  skillsInventoryEl.classList.toggle("is-busy", busy);
  skillsMountAllEl.disabled = busy;
  skillsRefreshEl.disabled = busy;
}

async function toggleSkillRuntimeMount(
  chip: HTMLButtonElement,
  skillId: string,
  runtime: string,
  wasMounted: boolean,
) {
  if (chip.classList.contains("is-busy")) return;
  chip.classList.add("is-busy");
  chip.classList.toggle("is-on", !wasMounted);
  skillsFootnoteEl.textContent = wasMounted ? t("skills.unmounting") : t("skills.mounting");
  try {
    const report = wasMounted
      ? await invoke<SkillMountReport>("unmount_synced_skills_command", {
          skillIds: [skillId],
          runtimes: [runtime],
        })
      : await invoke<SkillMountReport>("mount_synced_skills_command", {
          skillIds: [skillId],
          runtimes: [runtime],
        });
    skillsFootnoteEl.textContent = wasMounted
      ? t("skills.unmountOk", {
          unmounted: String(report.unmounted),
          skipped: String(report.skipped),
          failed: String(report.failed),
        })
      : t("skills.mountOk", {
          mounted: String(report.mounted),
          skipped: String(report.skipped),
          failed: String(report.failed),
        });
    // Fast local refresh (skip remote stats) so the click never feels stuck.
    await loadSkillsInventory({ remoteStats: false });
  } catch (error) {
    chip.classList.toggle("is-on", wasMounted);
    skillsFootnoteEl.textContent = t("skills.mountFailed", { error: String(error) });
  } finally {
    chip.classList.remove("is-busy");
  }
}

async function mountSyncedSkills(skillIds?: string[], runtimes?: string[]) {
  setSkillsBusy(true);
  skillsFootnoteEl.textContent = t("skills.mounting");
  try {
    const report = await invoke<SkillMountReport>("mount_synced_skills_command", {
      skillIds: skillIds ?? null,
      runtimes: runtimes ?? null,
    });
    skillsFootnoteEl.textContent = t("skills.mountOk", {
      mounted: String(report.mounted),
      skipped: String(report.skipped),
      failed: String(report.failed),
    });
    await loadSkillsInventory({ remoteStats: false });
  } catch (error) {
    skillsFootnoteEl.textContent = t("skills.mountFailed", { error: String(error) });
  } finally {
    setSkillsBusy(false);
  }
}

async function runEvotownOnboarding() {
  const url = evotownUrlEl.value.trim();
  const key = evotownKeyEl.value.trim();
  if (!url || !key) {
    evotownHintEl.textContent = t("evotown.connectFailed", { error: "URL and API key are required" });
    return;
  }

  evotownConnectEl.disabled = true;
  evotownResyncEl.disabled = true;
  evotownHintEl.textContent = t("evotown.connecting");
  try {
    const report = await invoke<OnboardingReport>("run_evotown_onboarding_command", {
      url,
      key,
      syncSkills: true,
      pullPolicies: true,
    });
    evotownKeyEl.value = "";
    await loadEvotownStatus();
    await loadModeStatus();
    await refresh();
    evotownHintEl.textContent = t("evotown.connectOk", {
      installed: String(report.sync?.installed ?? 0),
      policies: String(report.policy?.policy_count ?? 0),
    });
  } catch (error) {
    evotownHintEl.textContent = t("evotown.connectFailed", { error: String(error) });
  } finally {
    evotownConnectEl.disabled = false;
    evotownResyncEl.disabled = false;
  }
}

async function resyncEvotownSkills() {
  evotownResyncEl.disabled = true;
  evotownHintEl.textContent = t("evotown.resyncRunning");
  try {
    const report = await invoke<SyncReport>("run_sync_command");
    evotownHintEl.textContent = t("evotown.resyncOk", {
      installed: String(report.installed),
      skipped: String(report.skipped),
      failed: String(report.failed),
    });
    await loadSkillsInventory();
  } catch (error) {
    evotownHintEl.textContent = t("evotown.resyncFailed", { error: String(error) });
  } finally {
    evotownResyncEl.disabled = false;
  }
}

async function loadPersonalProviderStatus() {
  try {
    const [status, doc] = await Promise.all([
      invoke<PersonalProviderStatus>("get_personal_provider_status_command"),
      invoke<PersonalProvidersDocument>("list_personal_providers_command"),
    ]);
    personalProvidersDoc = doc;
    renderPersonalProviderStatus(status);
    renderPersonalProviderList(doc);
  } catch (error) {
    personalStatusEl.textContent = t("personal.applyFailed", { error: String(error) });
  }
}

function renderPersonalProviderStatus(status: PersonalProviderStatus) {
  personalSectionEl.classList.toggle("is-configured", status.configured);
  personalConnectedEl.hidden = !status.configured;

  if (status.configured && status.gateway_url) {
    personalStatusEl.textContent = t("personal.configured");
    personalConnectedUrlEl.textContent = status.active_name || status.gateway_url;
    personalConnectedMetaEl.textContent = t("personal.meta", {
      name: status.active_name ?? "—",
      protocol: protocolLabel(status.protocol ?? "openai"),
      model: status.model ?? "—",
      key: status.api_key_hint ?? "…",
    });
  } else {
    personalStatusEl.textContent = t("personal.notConfigured");
    personalConnectedMetaEl.textContent = "";
  }
}

function renderPersonalProviderList(doc: PersonalProvidersDocument) {
  personalListEl.innerHTML = "";
  if (doc.providers.length === 0) {
    const empty = document.createElement("li");
    empty.className = "section-hint";
    empty.textContent = t("personal.emptyList");
    personalListEl.appendChild(empty);
    return;
  }

  const personalModeActive = lastModeStatus?.mode === "personal";
  for (const item of doc.providers) {
    const routingActive = item.active && personalModeActive;
    const li = document.createElement("li");
    li.className = `provider-item${routingActive ? " is-active" : ""}`;
    li.dataset.providerId = item.id;

    const main = document.createElement("div");
    main.className = "provider-item-main";
    const title = document.createElement("p");
    title.className = "provider-item-title";
    title.textContent = item.name;
    if (routingActive) {
      const badge = document.createElement("span");
      badge.className = "provider-badge";
      badge.textContent = t("personal.activeBadge");
      title.appendChild(badge);
    }
    const meta = document.createElement("p");
    meta.className = "provider-item-meta";
    meta.textContent = t("personal.itemMeta", {
      protocol: protocolLabel(item.protocol),
      model: item.model,
      url: item.url,
    });
    main.append(title, meta);

    const actions = document.createElement("div");
    actions.className = "provider-item-actions";

    if (!item.active) {
      const activateBtn = document.createElement("button");
      activateBtn.type = "button";
      activateBtn.className = "btn-primary";
      activateBtn.dataset.action = "activate-provider";
      activateBtn.dataset.providerId = item.id;
      activateBtn.textContent = t("personal.activate");
      actions.appendChild(activateBtn);
    }

    const editBtn = document.createElement("button");
    editBtn.type = "button";
    editBtn.className = "btn-secondary";
    editBtn.dataset.action = "edit-provider";
    editBtn.dataset.providerId = item.id;
    editBtn.textContent = t("personal.edit");
    actions.appendChild(editBtn);

    const deleteBtn = document.createElement("button");
    deleteBtn.type = "button";
    deleteBtn.className = "btn-ghost";
    deleteBtn.dataset.action = "delete-provider";
    deleteBtn.dataset.providerId = item.id;
    deleteBtn.textContent = t("personal.delete");
    actions.appendChild(deleteBtn);

    li.append(main, actions);
    personalListEl.appendChild(li);
  }
}

function showPersonalListView() {
  personalListViewEl.hidden = false;
  personalFormViewEl.hidden = true;
}

function showPersonalFormView(mode: "add" | "edit") {
  personalListViewEl.hidden = true;
  personalFormViewEl.hidden = false;
  personalFormTitleEl.textContent =
    mode === "edit" ? t("personal.formEdit") : t("personal.formAdd");
  personalHintEl.textContent = "";
}

function resetPersonalForm() {
  personalIdEl.value = "";
  personalNameEl.value = "";
  personalUrlEl.value = "";
  personalKeyEl.value = "";
  personalModelEl.value = "";
  personalProtocolEl.value = "openai";
  personalKeyEl.placeholder = "sk-…";
  applyProviderPreset("custom");
  personalPresetEl.value = "custom";
}

function fillPersonalForm(item: PersonalProviderListItem) {
  personalIdEl.value = item.id;
  personalNameEl.value = item.name;
  personalUrlEl.value = item.url;
  personalModelEl.value = item.model;
  personalProtocolEl.value = item.protocol === "anthropic" ? "anthropic" : "openai";
  personalKeyEl.value = "";
  personalKeyEl.placeholder = t("personal.keyKeepHint");
  const presetId = matchPresetId(item.name, item.url, item.protocol);
  if (presetId === "custom") {
    applyProviderPreset("custom");
    personalNameEl.value = item.name;
    personalUrlEl.value = item.url;
    personalProtocolEl.value = item.protocol === "anthropic" ? "anthropic" : "openai";
  } else {
    applyProviderPreset(presetId, { forceModel: false });
    personalNameEl.value = item.name;
    personalUrlEl.value = item.url;
    personalModelEl.value = item.model;
  }
}

function personalFormValues(requireKey: boolean): {
  id: string | null;
  name: string;
  url: string;
  key: string;
  model: string;
  protocol: ProviderProtocol;
} | null {
  const id = personalIdEl.value.trim() || null;
  const name = personalNameEl.value.trim();
  const url = personalUrlEl.value.trim();
  const key = personalKeyEl.value.trim();
  const model = personalModelEl.value.trim();
  const protocol: ProviderProtocol =
    personalProtocolEl.value === "anthropic" ? "anthropic" : "openai";
  if (!name || !url || !model || (requireKey && !key && !id)) {
    personalHintEl.textContent = t("personal.missingFields");
    return null;
  }
  return { id, name, url, key, model, protocol };
}

function setPersonalBusy(busy: boolean) {
  personalVerifyEl.disabled = busy;
  personalSaveEl.disabled = busy;
  personalApplyEl.disabled = busy;
  personalAddEl.disabled = busy;
  personalBackEl.disabled = busy;
}

async function verifyPersonalProvider() {
  const values = personalFormValues(true);
  if (!values) {
    return;
  }
  if (!values.key) {
    personalHintEl.textContent = t("personal.missingFields");
    return;
  }
  setPersonalBusy(true);
  personalHintEl.textContent = t("personal.verifying");
  try {
    const report = await invoke<PersonalProviderVerifyReport>("verify_personal_provider_command", {
      url: values.url,
      key: values.key,
      protocol: values.protocol,
    });
    if (report.ok) {
      const sample =
        report.models_sample.length > 0 ? ` (${report.models_sample.slice(0, 3).join(", ")})` : "";
      personalHintEl.textContent = t("personal.verifyOk", { message: `${report.message}${sample}` });
    } else {
      personalHintEl.textContent = t("personal.verifyFailed", { error: report.message });
    }
  } catch (error) {
    personalHintEl.textContent = t("personal.verifyFailed", { error: String(error) });
  } finally {
    setPersonalBusy(false);
  }
}

async function upsertPersonalProvider(activate: boolean) {
  const editing = Boolean(personalIdEl.value.trim());
  const values = personalFormValues(!editing);
  if (!values) {
    return;
  }
  setPersonalBusy(true);
  personalHintEl.textContent = activate ? t("personal.applying") : t("personal.saving");
  try {
    if (activate) {
      // Save first without activate, then activate for a proper setup report.
      const doc = await invoke<PersonalProvidersDocument>("upsert_personal_provider_command", {
        id: values.id,
        name: values.name,
        url: values.url,
        key: values.key,
        model: values.model,
        protocol: values.protocol,
        activate: false,
      });
      const targetId =
        values.id ??
        doc.providers.find((p) => p.name === values.name && p.url === values.url)?.id ??
        doc.providers[doc.providers.length - 1]?.id;
      if (!targetId) {
        throw new Error("saved provider id missing");
      }
      const report = await invoke<PersonalProviderSetupReport>("activate_personal_provider_command", {
        id: targetId,
      });
      personalKeyEl.value = "";
      await loadPersonalProviderStatus();
      await loadModeStatus();
      await refresh();
      const applied = report.runtimes.filter((item) => item.applied).length;
      personalListHintEl.textContent = t("personal.applyOk", {
        name: report.provider_name ?? values.name,
        count: String(applied),
      });
      resetPersonalForm();
      showPersonalListView();
    } else {
      await invoke<PersonalProvidersDocument>("upsert_personal_provider_command", {
        id: values.id,
        name: values.name,
        url: values.url,
        key: values.key,
        model: values.model,
        protocol: values.protocol,
        activate: false,
      });
      personalKeyEl.value = "";
      await loadPersonalProviderStatus();
      await loadModeStatus();
      personalListHintEl.textContent = t("personal.saveOk", { name: values.name });
      resetPersonalForm();
      showPersonalListView();
    }
  } catch (error) {
    personalHintEl.textContent = t("personal.applyFailed", { error: String(error) });
  } finally {
    setPersonalBusy(false);
  }
}

async function activateProviderById(id: string) {
  setPersonalBusy(true);
  personalListHintEl.textContent = t("personal.applying");
  try {
    const report = await invoke<PersonalProviderSetupReport>("activate_personal_provider_command", {
      id,
    });
    await loadPersonalProviderStatus();
    await loadModeStatus();
    await refresh();
    const applied = report.runtimes.filter((item) => item.applied).length;
    personalListHintEl.textContent = t("personal.applyOk", {
      name: report.provider_name ?? id,
      count: String(applied),
    });
  } catch (error) {
    personalListHintEl.textContent = t("personal.applyFailed", { error: String(error) });
  } finally {
    setPersonalBusy(false);
  }
}

async function deleteProviderById(id: string) {
  setPersonalBusy(true);
  try {
    const doc = await invoke<PersonalProvidersDocument>("delete_personal_provider_command", { id });
    personalProvidersDoc = doc;
    await loadPersonalProviderStatus();
    personalListHintEl.textContent = t("personal.deleteOk");
    showPersonalListView();
  } catch (error) {
    personalListHintEl.textContent = t("personal.applyFailed", { error: String(error) });
  } finally {
    setPersonalBusy(false);
  }
}

async function loadProfiles() {
  try {
    const doc = await invoke<ProfilesDocument>("list_profiles_command");
    renderProfiles(doc);
  } catch (error) {
    presetStatusEl.textContent = t("presets.failed");
    presetHintEl.textContent = String(error);
    presetApplyEl.disabled = true;
  }
}

function renderWorkspaceManageList(doc: WorkspacesDocument): void {
  const names = Object.keys(doc.workspaces).sort();
  selectedWorkspaceName = doc.active ?? selectedWorkspaceName;

  if (names.length === 0) {
    workspaceListEl.innerHTML = `
      <li class="ws-manage-item">
        <div class="ws-manage-main">
          <strong>${escapeHtml(t("workspaces.none"))}</strong>
          <span>${escapeHtml(t("workspaces.noneHint"))}</span>
        </div>
      </li>
    `;
    return;
  }

  workspaceListEl.innerHTML = names
    .map((name) => {
      const entry = doc.workspaces[name];
      const path = entry?.path ?? "";
      const isActive = name === doc.active;
      const pathLabel = isActive
        ? `${path}${path ? " · " : ""}${t("agents.wsActiveBadge")}`
        : path;
      const right = isActive
        ? `
          <div class="ws-manage-right">
            <span class="badge ok">${escapeHtml(t("agents.wsActiveBadge"))}</span>
            <div class="ws-manage-actions">
              <button type="button" class="btn-ghost btn-compact" data-workspace-action="doctor" data-workspace="${escapeHtml(name)}" ${workspaceBusy ? "disabled" : ""}>${escapeHtml(t("workspaces.doctor"))}</button>
              <button type="button" class="btn-ghost btn-compact" data-workspace-action="fix" data-workspace="${escapeHtml(name)}" ${workspaceBusy ? "disabled" : ""}>${escapeHtml(t("workspaces.fix"))}</button>
            </div>
          </div>
        `
        : `
          <div class="ws-manage-right">
            <button type="button" class="btn-secondary btn-compact" data-workspace-action="use" data-workspace="${escapeHtml(name)}" ${workspaceBusy ? "disabled" : ""}>${escapeHtml(t("agents.wsUse"))}</button>
          </div>
        `;
      return `
        <li class="ws-manage-item ${isActive ? "is-active" : ""}">
          <div class="ws-manage-main">
            <strong>${escapeHtml(name)}</strong>
            ${pathLabel ? `<span>${escapeHtml(pathLabel)}</span>` : ""}
          </div>
          ${right}
        </li>
      `;
    })
    .join("");
}

function renderWorkspaces(doc: WorkspacesDocument) {
  lastWorkspaces = doc;
  updateAgentsWorkspaceChip(doc);
  renderWorkspaceManageList(doc);

  if (Object.keys(doc.workspaces).length === 0) {
    workspaceStatusEl.textContent = t("workspaces.none");
    workspaceHintEl.textContent = t("workspaces.noneHint");
    return;
  }

  workspaceStatusEl.textContent = "";
  workspaceHintEl.textContent = "";
}

async function loadWorkspaces() {
  try {
    const doc = await invoke<WorkspacesDocument>("list_workspaces_command");
    renderWorkspaces(doc);
  } catch (error) {
    workspaceStatusEl.textContent = t("workspaces.failed");
    workspaceHintEl.textContent = String(error);
    workspaceListEl.innerHTML = "";
  }
}

async function applyWorkspace(name: string) {
  if (!name || workspaceBusy) {
    return;
  }

  workspaceBusy = true;
  selectedWorkspaceName = name;
  renderWorkspaceManageList(lastWorkspaces ?? { active: null, workspaces: {} });
  workspaceHintEl.textContent = t("workspaces.applying", { name });
  try {
    await invoke("use_workspace_command", { name });
    workspaceHintEl.textContent = t("workspaces.updated", { name });
    await loadWorkspaces();
  } catch (error) {
    workspaceHintEl.textContent = String(error);
  } finally {
    workspaceBusy = false;
    if (lastWorkspaces) {
      renderWorkspaceManageList(lastWorkspaces);
    }
  }
}

async function registerWorkspace() {
  if (workspaceBusy) {
    return;
  }

  let selected: string | string[] | null;
  try {
    selected = await open({
      directory: true,
      multiple: false,
      title: t("workspaces.registerPick"),
    });
  } catch (error) {
    workspaceHintEl.textContent = String(error);
    return;
  }

  const path = Array.isArray(selected) ? selected[0] : selected;
  if (!path) {
    return;
  }

  workspaceBusy = true;
  workspaceRegisterEl.disabled = true;
  workspaceHintEl.textContent = t("workspaces.registering");
  try {
    const report = await invoke<{ name: string }>("init_workspace_command", {
      path,
      name: null,
      gitRoot: true,
    });
    workspaceHintEl.textContent = t("workspaces.registered", { name: report.name });
    await loadWorkspaces();
  } catch (error) {
    workspaceHintEl.textContent = String(error);
  } finally {
    workspaceBusy = false;
    workspaceRegisterEl.disabled = false;
    if (lastWorkspaces) {
      renderWorkspaceManageList(lastWorkspaces);
    }
  }
}

async function doctorWorkspace() {
  if (workspaceBusy) {
    return;
  }
  workspaceBusy = true;
  if (lastWorkspaces) {
    renderWorkspaceManageList(lastWorkspaces);
  }
  workspaceHintEl.textContent = t("workspaces.doctorRunning");
  try {
    const report = await invoke<WorkspaceDoctorReport>("workspace_doctor_command");
    renderWorkspaceChecks(report);
  } catch (error) {
    workspaceChecksEl.hidden = true;
    workspaceChecksEl.innerHTML = "";
    workspaceHintEl.textContent = String(error);
  } finally {
    workspaceBusy = false;
    if (lastWorkspaces) {
      renderWorkspaceManageList(lastWorkspaces);
    }
  }
}

async function fixWorkspace() {
  if (workspaceBusy) {
    return;
  }
  workspaceBusy = true;
  if (lastWorkspaces) {
    renderWorkspaceManageList(lastWorkspaces);
  }
  workspaceHintEl.textContent = t("workspaces.fixRunning");
  try {
    const report = await invoke<WorkspaceFixReport>("workspace_fix_command", {
      migrateClaudeMcp: false,
    });
    const applied = report.actions.filter((action) => action.applied).length;
    workspaceHintEl.textContent = t("workspaces.fixSummary", { count: String(applied) });
    workspaceBusy = false;
    await doctorWorkspace();
  } catch (error) {
    workspaceHintEl.textContent = String(error);
    workspaceBusy = false;
    if (lastWorkspaces) {
      renderWorkspaceManageList(lastWorkspaces);
    }
  }
}

interface WorkspaceCheck {
  id: string;
  title: string;
  status: "pass" | "warn" | "fail";
  detail: string;
}

interface WorkspaceDoctorReport {
  active: string | null;
  checks: WorkspaceCheck[];
}

interface WorkspaceFixReport {
  active: string | null;
  actions: Array<{
    id: string;
    title: string;
    applied: boolean;
    detail: string;
  }>;
}

function workspaceStatusLabel(status: WorkspaceCheck["status"]): string {
  switch (status) {
    case "pass":
      return t("repair.pass");
    case "warn":
      return t("repair.warn");
    case "fail":
      return t("repair.fail");
  }
}

function workspaceStatusClass(status: WorkspaceCheck["status"]): string {
  if (status === "pass") {
    return "pass";
  }
  if (status === "warn") {
    return "warn";
  }
  return "fail";
}

function renderWorkspaceChecks(report: WorkspaceDoctorReport) {
  if (!report.checks.length) {
    workspaceChecksEl.hidden = true;
    workspaceChecksEl.innerHTML = "";
    workspaceHintEl.textContent = t("workspaces.noActive");
    return;
  }

  workspaceChecksEl.hidden = false;
  workspaceChecksEl.innerHTML = report.checks
    .map(
      (check) => `
        <li class="repair-check">
          <span class="repair-check-status ${workspaceStatusClass(check.status)}">${escapeHtml(workspaceStatusLabel(check.status))}</span>
          <span class="repair-check-body">
            <strong>${escapeHtml(check.title)}</strong>
            <span>${escapeHtml(check.detail)}</span>
          </span>
        </li>
      `,
    )
    .join("");

  let pass = 0;
  let warn = 0;
  let fail = 0;
  for (const check of report.checks) {
    if (check.status === "pass") pass += 1;
    else if (check.status === "warn") warn += 1;
    else fail += 1;
  }
  workspaceHintEl.textContent = t("workspaces.doctorSummary", {
    pass: String(pass),
    warn: String(warn),
    fail: String(fail),
  });
}

interface RemoteHostsDocument {
  hosts: Record<
    string,
    {
      ssh_config_host: string;
      projects: Record<string, { path: string; runtimes: string[] }>;
    }
  >;
}

interface RemoteProjectRow {
  host_id: string;
  project_id: string;
  path: string;
  runtimes: string[];
  ssh_config_host: string;
}

interface RemoteProbeCheck {
  id: string;
  title: string;
  status: "pass" | "warn" | "fail" | "not_applicable" | "not_checked";
  severity: string;
  message: string;
  details: string[];
}

interface RemoteDoctorReport {
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

let lastRemoteProjects: RemoteProjectRow[] = [];

function fillRemoteHostSelect(doc: RemoteHostsDocument): void {
  const ids = Object.keys(doc.hosts).sort();
  const previous = remoteProjectHostEl.value;
  if (ids.length === 0) {
    remoteProjectHostEl.innerHTML = `<option value="">${escapeHtml(t("remote.noHosts"))}</option>`;
    remoteProjectHostEl.disabled = true;
    return;
  }
  remoteProjectHostEl.disabled = false;
  remoteProjectHostEl.innerHTML =
    `<option value="">${escapeHtml(t("remote.selectHost"))}</option>` +
    ids
      .map(
        (id) =>
          `<option value="${escapeHtml(id)}">${escapeHtml(id)} (${escapeHtml(doc.hosts[id]?.ssh_config_host ?? id)})</option>`,
      )
      .join("");
  if (previous && ids.includes(previous)) {
    remoteProjectHostEl.value = previous;
  }
}

function renderRemoteList(rows: RemoteProjectRow[]): void {
  if (rows.length === 0) {
    remoteListEl.innerHTML = `
      <li class="ws-manage-item">
        <div class="ws-manage-main">
          <strong>${escapeHtml(t("remote.none"))}</strong>
          <span>${escapeHtml(t("remote.noneHint"))}</span>
        </div>
      </li>
    `;
    return;
  }

  remoteListEl.innerHTML = rows
    .map((row) => {
      const target = `${row.host_id}/${row.project_id}`;
      const runtimes =
        row.runtimes.length === 0 ? "all" : row.runtimes.join(", ");
      return `
        <li class="ws-manage-item">
          <div class="ws-manage-main">
            <strong>${escapeHtml(target)}</strong>
            <span>${escapeHtml(row.path)} · ssh ${escapeHtml(row.ssh_config_host)} · ${escapeHtml(runtimes)}</span>
          </div>
          <div class="ws-manage-right">
            <div class="ws-manage-actions">
              <button type="button" class="btn-ghost btn-compact" data-remote-action="doctor" data-remote-target="${escapeHtml(target)}" ${remoteBusy ? "disabled" : ""}>${escapeHtml(t("remote.doctor"))}</button>
              <button type="button" class="btn-ghost btn-compact" data-remote-action="remove" data-remote-host="${escapeHtml(row.host_id)}" data-remote-project="${escapeHtml(row.project_id)}" ${remoteBusy ? "disabled" : ""}>${escapeHtml(t("remote.remove"))}</button>
            </div>
          </div>
        </li>
      `;
    })
    .join("");
}

function remoteCheckClass(status: RemoteProbeCheck["status"]): string {
  if (status === "pass") return "pass";
  if (status === "warn" || status === "not_checked") return "warn";
  if (status === "fail") return "fail";
  return "pass";
}

function remoteCheckLabel(status: RemoteProbeCheck["status"]): string {
  switch (status) {
    case "pass":
      return t("repair.pass");
    case "warn":
    case "not_checked":
      return t("repair.warn");
    case "fail":
      return t("repair.fail");
    default:
      return "—";
  }
}

function renderRemoteChecks(report: RemoteDoctorReport): void {
  const items: Array<{ title: string; message: string; status: RemoteProbeCheck["status"] }> = [];
  for (const check of report.checks) {
    if (check.status === "not_applicable") continue;
    items.push({
      title: check.title,
      message: check.details.length
        ? `${check.message} · ${check.details.join(" · ")}`
        : check.message,
      status: check.status,
    });
  }
  for (const runtime of report.runtimes) {
    for (const check of runtime.checks) {
      if (check.status === "not_applicable") continue;
      items.push({
        title: `${runtime.display_name}: ${check.title}`,
        message: check.details.length
          ? `${check.message} · ${check.details.join(" · ")}`
          : check.message,
        status: check.status,
      });
    }
  }

  if (items.length === 0) {
    remoteChecksEl.hidden = true;
    remoteChecksEl.innerHTML = "";
    return;
  }

  remoteChecksEl.hidden = false;
  remoteChecksEl.innerHTML = items
    .map(
      (check) => `
        <li class="repair-check">
          <span class="repair-check-status ${remoteCheckClass(check.status)}">${escapeHtml(remoteCheckLabel(check.status))}</span>
          <span class="repair-check-body">
            <strong>${escapeHtml(check.title)}</strong>
            <span>${escapeHtml(check.message)}</span>
          </span>
        </li>
      `,
    )
    .join("");

  let pass = 0;
  let warn = 0;
  let fail = 0;
  for (const check of items) {
    if (check.status === "pass") pass += 1;
    else if (check.status === "fail") fail += 1;
    else warn += 1;
  }
  let summary = t("remote.doctorSummary", {
    pass: String(pass),
    warn: String(warn),
    fail: String(fail),
  });
  if (report.report_path) {
    summary += ` · ${t("remote.reportSaved", { path: report.report_path })}`;
  }
  remoteHintEl.textContent = summary;
}

async function loadRemoteProjects(): Promise<void> {
  try {
    const [hosts, projects] = await Promise.all([
      invoke<RemoteHostsDocument>("list_remote_hosts_command"),
      invoke<RemoteProjectRow[]>("list_remote_projects_command"),
    ]);
    lastRemoteProjects = projects;
    fillRemoteHostSelect(hosts);
    renderRemoteList(projects);
    remoteStatusEl.textContent = "";
  } catch (error) {
    remoteStatusEl.textContent = t("remote.doctorFailed", { error: String(error) });
    remoteListEl.innerHTML = "";
  }
}

async function runRemoteDoctorUi(target: string): Promise<void> {
  if (!target || remoteBusy) return;
  remoteBusy = true;
  renderRemoteList(lastRemoteProjects);
  remoteHintEl.textContent = t("remote.doctorRunning");
  try {
    const report = await invoke<RemoteDoctorReport>("run_remote_doctor_command", {
      target,
      runtime: null,
    });
    renderRemoteChecks(report);
  } catch (error) {
    remoteChecksEl.hidden = true;
    remoteChecksEl.innerHTML = "";
    remoteHintEl.textContent = t("remote.doctorFailed", { error: String(error) });
  } finally {
    remoteBusy = false;
    renderRemoteList(lastRemoteProjects);
  }
}

async function removeRemoteProjectUi(host: string, project: string): Promise<void> {
  if (remoteBusy) return;
  remoteBusy = true;
  try {
    await invoke("remove_remote_project_command", { host, name: project });
    await loadRemoteProjects();
    remoteHintEl.textContent = "";
  } catch (error) {
    remoteHintEl.textContent = t("remote.removeFailed", { error: String(error) });
  } finally {
    remoteBusy = false;
    renderRemoteList(lastRemoteProjects);
  }
}

function setLoading(loading: boolean) {
  refreshBtn.disabled = loading;
  refreshBtn.classList.toggle("is-loading", loading);
  spinnerEl.hidden = !loading;
  runtimesEl.classList.toggle("is-loading", loading);
  runtimeTabsEl.classList.toggle("is-loading", loading);

  if (loading) {
    const installed = lastReport?.runtimes.filter((runtime) => runtime.installed).length ?? 0;
    const total = lastReport?.runtimes.length ?? 0;
    updateHealthStrip(installed, total, true);
    setStatusBanner("neutral", t("doctor.running"));
  }
}

async function refresh() {
  setLoading(true);
  try {
    const report = await invoke<DoctorReport>("run_doctor_command");
    await renderReport(report);
  } catch (error) {
    setStatusBanner("error", t("doctor.failed", { error: String(error) }));
    updateHealthStrip(0, 0);
    runtimesEl.innerHTML = `<div class="empty-state">${t("doctor.empty")}</div>`;
    runtimeTabsEl.innerHTML = "";
    activeRuntimeId = null;
    installedCountEl.textContent = "—";
    profileStatusEl.textContent = t("status.error");
    runtimeCountEl.textContent = "—";
  } finally {
    setLoading(false);
  }
}

async function saveHermesCard(card: HTMLElement) {
  const hint = card.querySelector<HTMLElement>("[data-hermes-hint]");
  const saveBtn = card.querySelector<HTMLButtonElement>('[data-action="save-hermes"]');
  const draft = readHermesDraft(card);

  saveBtn?.setAttribute("disabled", "true");
  if (hint) {
    hint.textContent = t("runtime.saving");
  }

  try {
    await invoke<{ restart_hint: string; backup_path: string | null }>("set_hermes_model_command", {
      provider: draft.provider,
      model: draft.model,
      baseUrl: draft.base_url,
      apiKey: draft.api_key ? draft.api_key : null,
    });

    const activeProfile = lastProfiles?.active;
    if (activeProfile) {
      const profileReport = await invoke<{ restart_hint: string }>("apply_profile_model_command", {
        profile: activeProfile,
        provider: draft.provider,
        model: draft.model,
        baseUrl: draft.base_url,
      });
      if (hint) {
        hint.textContent = profileReport.restart_hint;
      }
    }

    hermesEditing = false;
    await loadProfiles();
    await refresh();
  } catch (error) {
    if (hint) {
      hint.textContent = String(error);
    }
  } finally {
    saveBtn?.removeAttribute("disabled");
  }
}

async function applyPreset() {
  const name = selectedPresetName;
  if (!name) {
    return;
  }

  closePresetMenu();

  presetApplyEl.disabled = true;
  presetHintEl.textContent = t("presets.applying", { name });
  try {
    const report = await invoke<UseProfileReport>("use_profile_command", { name });
    const applied = report.applied.map((item) => item.runtime_id).join(", ");
    presetHintEl.textContent = applied
      ? t("presets.updated", { list: applied })
      : report.skipped.join("; ");
    hermesEditing = false;
    await loadProfiles();
    await refresh();
  } catch (error) {
    presetHintEl.textContent = String(error);
  } finally {
    presetApplyEl.disabled = false;
  }
}

async function rollbackRepairRuntimeCard(card: HTMLElement) {
  const runtime = card.dataset.runtime;
  const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
  const diagnoseButton = card.querySelector<HTMLButtonElement>('[data-action="diagnose-runtime"]');
  const applyButton = card.querySelector<HTMLButtonElement>('[data-action="apply-repair"]');
  const rollbackButton = card.querySelector<HTMLButtonElement>('[data-action="rollback-repair"]');
  if (!runtime || !hint) {
    return;
  }
  diagnoseButton?.setAttribute("disabled", "true");
  applyButton?.setAttribute("disabled", "true");
  rollbackButton?.setAttribute("disabled", "true");
  hint.hidden = false;
  hint.textContent = t("repair.rollingBack");
  try {
    const restore = await invoke<RestoreSummary>("run_repair_rollback_command", {
      runtime,
      backup: null,
    });
    const report = await invoke<RepairPreviewResponse>("run_repair_preview_command", { runtime });
    repairFilterByRuntime.set(runtime, "all");
    mountRepairPreview(hint, report);
    hint.insertAdjacentHTML(
      "afterbegin",
      `<p class="repair-rollback-ok">${escapeHtml(
        t("repair.rollbackDone", { id: restore.backup_id, count: String(restore.restored_files.length) }),
      )}</p>`,
    );
    if (runtime === "hermes") {
      await loadHermesModel();
    }
  } catch (error) {
    hint.textContent = String(error);
  } finally {
    diagnoseButton?.removeAttribute("disabled");
    applyButton?.removeAttribute("disabled");
    rollbackButton?.removeAttribute("disabled");
  }
}

async function openSessionFromCard(card: HTMLElement) {
  const runtime = card.dataset.runtime;
  const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
  const openButton = card.querySelector<HTMLButtonElement>('[data-action="open-session"]');
  if (!runtime) {
    return;
  }
  openButton?.setAttribute("disabled", "true");
  if (hint) {
    hint.hidden = false;
    hint.textContent = t("runtime.opening");
  }
  try {
    const report = await invoke<OpenSessionReport>("open_session_command", {
      runtime,
      cwd: null,
      prompt: null,
      terminal: null,
    });
    const method = report.method === "deep-link" ? "deep-link" : "terminal";
    if (hint) {
      hint.textContent = t("runtime.openOk", { method });
    }
  } catch (error) {
    if (hint) {
      hint.hidden = false;
      hint.textContent = t("runtime.openFailed", { error: String(error) });
    }
  } finally {
    openButton?.removeAttribute("disabled");
  }
}

async function installRuntimeFromCard(card: HTMLElement) {
  const runtime = card.dataset.runtime;
  const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
  const installButton = card.querySelector<HTMLButtonElement>('[data-action="install-runtime"]');
  const diagnoseButton = card.querySelector<HTMLButtonElement>('[data-action="diagnose-runtime"]');
  if (!runtime) {
    return;
  }
  installButton?.setAttribute("disabled", "true");
  diagnoseButton?.setAttribute("disabled", "true");
  if (hint) {
    hint.hidden = false;
    hint.innerHTML = `
      <div class="install-progress" data-install-progress>
        <div class="install-progress-head">
          <span data-install-status>${escapeHtml(t("runtime.installing"))}</span>
          <span data-install-percent>0%</span>
        </div>
        <div class="install-progress-track" aria-hidden="true">
          <div class="install-progress-fill is-indeterminate" data-install-fill></div>
        </div>
        <pre class="install-progress-log" data-install-log></pre>
      </div>
    `;
  }
  const statusEl = hint?.querySelector<HTMLElement>("[data-install-status]");
  const percentEl = hint?.querySelector<HTMLElement>("[data-install-percent]");
  const fillEl = hint?.querySelector<HTMLElement>("[data-install-fill]");
  const logEl = hint?.querySelector<HTMLElement>("[data-install-log]");
  const logLines: string[] = [];

  const unlisten = await listen<InstallProgressEvent>("install-progress", (event) => {
    if (event.payload.runtime_id !== runtime) {
      return;
    }
    const { phase, message, percent } = event.payload;
    if (statusEl) {
      statusEl.textContent =
        phase === "done"
          ? t("runtime.installOk")
          : phase === "verifying"
            ? t("runtime.installVerifying")
            : t("runtime.installing");
    }
    if (percentEl) {
      percentEl.textContent = `${Math.min(100, Math.max(0, percent))}%`;
    }
    if (fillEl) {
      fillEl.style.width = `${Math.min(100, Math.max(0, percent))}%`;
      fillEl.classList.toggle("is-indeterminate", phase === "installing" || phase === "output");
    }
    if (logEl && message.trim()) {
      logLines.push(message);
      while (logLines.length > 40) {
        logLines.shift();
      }
      logEl.textContent = logLines.join("\n");
      logEl.scrollTop = logEl.scrollHeight;
    }
  });

  try {
    const report = await invoke<InstallRuntimeResponse>("install_runtime_command", { runtime });
    if (hint) {
      if (!report.install_needed) {
        hint.textContent = t("runtime.installAlready");
      } else if (report.install_succeeded || report.after_installed) {
        const last = logLines.slice(-3).join("\n");
        hint.innerHTML = `<div class="install-progress-done">${escapeHtml(t("runtime.installOk"))}${
          last ? `<pre class="install-progress-log">${escapeHtml(last)}</pre>` : ""
        }</div>`;
      } else {
        const detail =
          report.skipped.map((item) => item.reason).find(Boolean) ||
          report.manual_fallback[0] ||
          t("runtime.installFailed");
        hint.textContent = `${t("runtime.installFailed")} ${detail}`;
      }
    }
    await refresh();
  } catch (error) {
    if (hint) {
      hint.hidden = false;
      hint.textContent = String(error);
    }
  } finally {
    unlisten();
    installButton?.removeAttribute("disabled");
    diagnoseButton?.removeAttribute("disabled");
  }
}

async function openRepairGuide(path: string) {
  await invoke("open_path_command", { path });
}

async function applyRepairRuntimeCard(card: HTMLElement) {
  const runtime = card.dataset.runtime;
  const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
  const diagnoseButton = card.querySelector<HTMLButtonElement>('[data-action="diagnose-runtime"]');
  const applyButton = card.querySelector<HTMLButtonElement>('[data-action="apply-repair"]');
  if (!runtime || !hint) {
    return;
  }
  diagnoseButton?.setAttribute("disabled", "true");
  applyButton?.setAttribute("disabled", "true");
  hint.hidden = false;
  hint.textContent = t("repair.applying");
  try {
    const report = await invoke<RepairPreviewResponse>("run_repair_execute_command", { runtime });
    repairFilterByRuntime.set(runtime, "all");
    mountRepairPreview(hint, report);
    if (runtime === "hermes") {
      await loadHermesModel();
    }
  } catch (error) {
    hint.textContent = String(error);
  } finally {
    diagnoseButton?.removeAttribute("disabled");
    applyButton?.removeAttribute("disabled");
  }
}

async function diagnoseRuntimeCard(card: HTMLElement) {
  const runtime = card.dataset.runtime;
  const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
  const button = card.querySelector<HTMLButtonElement>('[data-action="diagnose-runtime"]');
  if (!runtime || !hint) {
    return;
  }
  button?.setAttribute("disabled", "true");
  hint.hidden = false;
  hint.textContent = t("runtime.diagnosing");
  try {
    const report = await invoke<RepairPreviewResponse>("run_repair_preview_command", { runtime });
    repairFilterByRuntime.set(runtime, "all");
    mountRepairPreview(hint, report);
  } catch (error) {
    hint.textContent = String(error);
  } finally {
    button?.removeAttribute("disabled");
  }
}

function readHermesDraft(card: HTMLElement): {
  provider: string;
  model: string;
  base_url: string;
  api_key: string;
} {
  const read = (field: string) =>
    card.querySelector<HTMLInputElement>(`[data-field="${field}"]`)?.value.trim() ?? "";
  return {
    provider: read("provider"),
    model: read("model"),
    base_url: read("base_url"),
    api_key: read("api_key"),
  };
}

function updateLangButtons() {
  const current = getLocale();
  langSwitchEl.querySelectorAll<HTMLButtonElement>(".lang-btn").forEach((button) => {
    const active = button.dataset.lang === current;
    button.classList.toggle("is-active", active);
    button.setAttribute("aria-pressed", String(active));
  });
}

async function switchLocale(next: Locale) {
  if (next === getLocale()) {
    return;
  }
  setLocale(next);
  applyStaticI18n();
  refreshPresetGroupLabels();
  updateLangButtons();
  if (lastProfiles) {
    renderProfiles(lastProfiles);
  }
  if (lastWorkspaces) {
    renderWorkspaces(lastWorkspaces);
  }
  updateFooterCopy();
  updateWiringModeFootnote();
  if (lastReport) {
    await renderReport(lastReport);
  } else {
    setStatusBanner("neutral", t("doctor.loading"));
    presetStatusEl.textContent = t("presets.loading");
    healthLabelEl.textContent = t("health.ready");
  }
  await loadEvotownStatus();
  await loadPersonalProviderStatus();
  await loadModeStatus();
}

runtimeTabsEl.addEventListener("click", (event) => {
  const tab = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-runtime-tab]");
  const runtimeId = tab?.dataset.runtimeTab;
  if (!runtimeId || runtimeId === activeRuntimeId) {
    return;
  }
  activeRuntimeId = runtimeId;
  hermesEditing = false;
  if (lastReport) {
    void renderReport(lastReport);
  }
});

runtimesEl.addEventListener("change", (event) => {
  const target = event.target as HTMLElement;
  if (target instanceof HTMLSelectElement && target.dataset.field === "model-preset") {
    const card = target.closest<HTMLElement>('[data-runtime="hermes"]');
    if (card) {
      applyModelPresetToCard(card, target.value);
    }
  }
});

runtimesEl.addEventListener("click", (event) => {
  const target = event.target as HTMLElement;
  const filterBtn = target.closest<HTMLButtonElement>("[data-repair-filter]");
  if (filterBtn && !filterBtn.disabled) {
    const card = filterBtn.closest<HTMLElement>("[data-runtime]");
    const runtime = card?.dataset.runtime;
    const filter = filterBtn.dataset.repairFilter as RepairStatusFilter | undefined;
    if (runtime && filter) {
      applyRepairFilter(runtime, filter);
    }
    return;
  }

  const action = target.closest<HTMLElement>("[data-action]")?.dataset.action;
  if (!action) {
    return;
  }

  const runtimeCard = target.closest<HTMLElement>("[data-runtime]");
  if (action === "diagnose-runtime" && runtimeCard) {
    void diagnoseRuntimeCard(runtimeCard);
    return;
  }

  if (action === "open-session" && runtimeCard) {
    void openSessionFromCard(runtimeCard);
    return;
  }

  if (action === "install-runtime" && runtimeCard) {
    void installRuntimeFromCard(runtimeCard);
    return;
  }

  if (action === "go-provider") {
    setMainTab("provider");
    return;
  }

  if (action === "wire-runtime" && runtimeCard) {
    const hint = runtimeCard.querySelector<HTMLElement>("[data-repair-hint]");
    void rewireCurrentMode(hint);
    return;
  }

  if (action === "apply-repair" && runtimeCard) {
    void applyRepairRuntimeCard(runtimeCard);
    return;
  }

  if (action === "rollback-repair" && runtimeCard) {
    void rollbackRepairRuntimeCard(runtimeCard);
    return;
  }

  const guideBtn = target.closest<HTMLButtonElement>('[data-action="open-repair-guide"]');
  if (guideBtn?.dataset.guidePath) {
    void openRepairGuide(decodeURIComponent(guideBtn.dataset.guidePath));
    return;
  }

  const card = target.closest<HTMLElement>('[data-runtime="hermes"]');
  if (!card) {
    return;
  }

  if (action === "edit-hermes") {
    hermesEditing = true;
    activeRuntimeId = "hermes";
    if (lastReport) {
      void renderReport(lastReport);
    }
    return;
  }

  if (action === "cancel-hermes") {
    hermesEditing = false;
    if (lastReport) {
      void renderReport(lastReport);
    }
    return;
  }

  if (action === "save-hermes") {
    void saveHermesCard(card);
  }
});

langSwitchEl.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>(".lang-btn");
  const lang = button?.dataset.lang;
  if (lang === "en" || lang === "zh") {
    void switchLocale(lang);
  }
});

mainTabsEl.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-main-tab]");
  const tab = button?.dataset.mainTab;
  if (tab === "diagnose" || tab === "resources" || tab === "provider" || tab === "workspace") {
    setMainTab(tab);
  }
});

mcpRefreshEl.addEventListener("click", () => {
  void loadMcpStatus();
});

mcpShowUiEl.addEventListener("change", () => {
  persistShowBrowserUi(mcpShowUiEl.checked);
  refreshMcpSnippet();
});

mcpUserDataDirEl.addEventListener("change", () => {
  persistUserDataDir(selectedUserDataDir());
  refreshMcpSnippet();
  syncProfileModeButtons();
});

mcpUserDataDirEl.addEventListener("input", () => {
  refreshMcpSnippet();
  syncProfileModeButtons();
});

mcpProfileDirectoryEl.addEventListener("change", () => {
  persistProfileDirectory(selectedProfileDirectory());
  refreshMcpSnippet();
});

mcpProfileDirectoryEl.addEventListener("input", () => {
  refreshMcpSnippet();
});

mcpProfileSystemEl.addEventListener("click", () => {
  const path =
    lastMcpStatus?.browser.system_user_data_dir ||
    lastMcpStatus?.browser.user_data_dir ||
    "";
  mcpUserDataDirEl.value = path;
  mcpProfileDirectoryEl.value = "Default";
  persistUserDataDir(path);
  persistProfileDirectory("Default");
  refreshMcpSnippet();
  syncProfileModeButtons();
});

mcpProfileIsolatedEl.addEventListener("click", () => {
  const path = lastMcpStatus?.browser.isolated_user_data_dir || "";
  mcpUserDataDirEl.value = path;
  mcpProfileDirectoryEl.value = "Default";
  persistUserDataDir(path);
  persistProfileDirectory("Default");
  refreshMcpSnippet();
  syncProfileModeButtons();
});

resourcesRefreshEl.addEventListener("click", () => {
  void loadResourcesPanel();
});

mcpConfigureCodexEl.addEventListener("click", () => {
  void configureBrowserMcp("codex");
});

mcpConfigureClaudeEl.addEventListener("click", () => {
  void configureBrowserMcp("claude-code");
});

resourcesFiltersEl.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-res-filter]");
  const filter = button?.dataset.resFilter as ResourceFilter | undefined;
  if (!filter) return;
  resourceFilter = filter;
  resourcesFiltersEl.querySelectorAll<HTMLButtonElement>("[data-res-filter]").forEach((chip) => {
    chip.classList.toggle("is-active", chip.dataset.resFilter === filter);
  });
  renderResourcesList();
});

evotownFormEl.addEventListener("submit", (event) => {
  event.preventDefault();
  void runEvotownOnboarding();
});

evotownResyncEl.addEventListener("click", () => {
  void resyncEvotownSkills();
});

skillsRefreshEl.addEventListener("click", () => {
  void loadSkillsInventory();
});

skillsMountAllEl.addEventListener("click", () => {
  void mountSyncedSkills();
});

personalAddEl.addEventListener("click", () => {
  resetPersonalForm();
  showPersonalFormView("add");
});

personalBackEl.addEventListener("click", () => {
  resetPersonalForm();
  showPersonalListView();
});

personalFormEl.addEventListener("submit", (event) => {
  event.preventDefault();
  void upsertPersonalProvider(true);
});

personalSaveEl.addEventListener("click", () => {
  void upsertPersonalProvider(false);
});

personalVerifyEl.addEventListener("click", () => {
  void verifyPersonalProvider();
});

personalPresetEl.addEventListener("change", () => {
  applyProviderPreset(personalPresetEl.value, { forceModel: true });
  if (personalPresetEl.value === "custom") {
    personalNameEl.focus();
  } else {
    personalKeyEl.focus();
  }
});

personalProtocolEl.addEventListener("change", () => {
  const presetId = personalPresetEl.value;
  if (presetId !== "custom" && PROVIDER_PRESETS[presetId]) {
    if (PROVIDER_PRESETS[presetId].protocol !== personalProtocolEl.value) {
      const keptName = personalNameEl.value;
      const keptUrl = personalUrlEl.value;
      const keptModel = personalModelEl.value;
      applyProviderPreset("custom");
      personalNameEl.value = keptName;
      personalUrlEl.value = keptUrl;
      personalModelEl.value = keptModel;
    }
  } else {
    setModelSuggestions(
      personalProtocolEl.value === "anthropic"
        ? ["claude-sonnet-4-5", "claude-opus-4-5", "deepseek-v4-flash"]
        : ["deepseek-v4-flash", "deepseek-v4-pro", "gpt-4.1-mini"],
    );
  }
});

personalUrlEl.addEventListener("change", () => {
  const presetId = personalPresetEl.value;
  if (presetId !== "custom" && PROVIDER_PRESETS[presetId]) {
    const presetUrl = PROVIDER_PRESETS[presetId].url.replace(/\/+$/, "");
    const currentUrl = personalUrlEl.value.trim().replace(/\/+$/, "");
    if (currentUrl && currentUrl !== presetUrl) {
      const keptName = personalNameEl.value;
      const keptProtocol = personalProtocolEl.value;
      applyProviderPreset("custom");
      personalNameEl.value = keptName;
      personalProtocolEl.value = keptProtocol;
    }
  }
});

personalListEl.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-action]");
  const action = button?.dataset.action;
  const id = button?.dataset.providerId;
  if (!action || !id) {
    return;
  }
  if (action === "activate-provider") {
    void activateProviderById(id);
    return;
  }
  if (action === "edit-provider") {
    const item = personalProvidersDoc?.providers.find((p) => p.id === id);
    if (item) {
      fillPersonalForm(item);
      showPersonalFormView("edit");
      personalHintEl.textContent = t("personal.keyKeepHint");
    }
    return;
  }
  if (action === "delete-provider") {
    void deleteProviderById(id);
  }
});

refreshBtn.addEventListener("click", () => {
  void refresh();
});

presetApplyEl.addEventListener("click", () => {
  void applyPreset();
});

workspaceRegisterEl.addEventListener("click", () => {
  void registerWorkspace();
});

workspaceListEl.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-workspace-action]");
  if (!button) {
    return;
  }
  const name = button.dataset.workspace;
  const action = button.dataset.workspaceAction;
  if (!name || !action) {
    return;
  }
  if (action === "use") {
    void applyWorkspace(name);
    return;
  }
  if (action === "doctor") {
    void doctorWorkspace();
    return;
  }
  if (action === "fix") {
    void fixWorkspace();
  }
});

remoteRefreshEl.addEventListener("click", () => {
  void loadRemoteProjects();
});

remoteListEl.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-remote-action]");
  if (!button) {
    return;
  }
  const action = button.dataset.remoteAction;
  if (action === "doctor") {
    const target = button.dataset.remoteTarget;
    if (target) {
      void runRemoteDoctorUi(target);
    }
    return;
  }
  if (action === "remove") {
    const host = button.dataset.remoteHost;
    const project = button.dataset.remoteProject;
    if (host && project) {
      void removeRemoteProjectUi(host, project);
    }
  }
});

remoteHostFormEl.addEventListener("submit", (event) => {
  event.preventDefault();
  const id = remoteHostIdEl.value.trim();
  const ssh = remoteSshHostEl.value.trim();
  if (!id || !ssh || remoteBusy) {
    return;
  }
  remoteBusy = true;
  void (async () => {
    try {
      await invoke("add_remote_host_command", { id, sshConfigHost: ssh });
      remoteHostIdEl.value = "";
      remoteSshHostEl.value = "";
      remoteHintEl.textContent = t("remote.hostSaved", { id });
      await loadRemoteProjects();
      const addHost = document.querySelector<HTMLDetailsElement>("#remote-add-host");
      if (addHost) {
        addHost.open = false;
      }
    } catch (error) {
      remoteHintEl.textContent = t("remote.hostFailed", { error: String(error) });
    } finally {
      remoteBusy = false;
    }
  })();
});

remoteProjectFormEl.addEventListener("submit", (event) => {
  event.preventDefault();
  const host = remoteProjectHostEl.value.trim();
  const name = remoteProjectNameEl.value.trim();
  const path = remoteProjectPathEl.value.trim();
  if (!host || !name || !path || remoteBusy) {
    return;
  }
  remoteBusy = true;
  void (async () => {
    try {
      await invoke("add_remote_project_command", {
        host,
        name,
        path,
        runtimes: [],
      });
      const target = `${host}/${name}`;
      remoteProjectNameEl.value = "";
      remoteProjectPathEl.value = "";
      remoteHintEl.textContent = t("remote.projectSaved", { target });
      await loadRemoteProjects();
      const addProject = document.querySelector<HTMLDetailsElement>("#remote-add-project");
      if (addProject) {
        addProject.open = false;
      }
    } catch (error) {
      remoteHintEl.textContent = t("remote.projectFailed", { error: String(error) });
    } finally {
      remoteBusy = false;
    }
  })();
});

presetTriggerEl.addEventListener("click", () => {
  togglePresetMenu();
});

presetMenuEl.addEventListener("click", (event) => {
  const option = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-preset]");
  const name = option?.dataset.preset;
  if (!name || !lastProfiles) {
    return;
  }
  selectedPresetName = name;
  renderPresetOptions(
    sortPresetNames(Object.keys(lastProfiles.profiles)),
    lastProfiles.active,
    lastProfiles.profiles,
  );
  closePresetMenu();
});

document.addEventListener("click", (event) => {
  const target = event.target as Node;
  if (presetMenuOpen && !presetPickerEl.contains(target)) {
    closePresetMenu();
  }
  if (
    agentsWsPickerOpen &&
    !agentsWsChipEl.contains(target) &&
    !agentsWsPickerEl.contains(target)
  ) {
    closeAgentsWsPicker();
  }
});

agentsWsQuickEl.addEventListener("click", () => {
  toggleAgentsWsPicker();
});

agentsWsManageEl.addEventListener("click", () => {
  closeAgentsWsPicker();
  setMainTab("workspace");
});

agentsWsListEl.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-agents-ws]");
  const name = button?.dataset.agentsWs;
  if (name) {
    void applyAgentsWorkspaceQuick(name);
  }
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    closePresetMenu();
    closeAgentsWsPicker();
  }
});

void listen<DoctorReport>("doctor-report", (event) => {
  void renderReport(event.payload);
});

void listen("workspace-changed", () => {
  void loadWorkspaces();
  void loadMcpStatus();
});

void listen<WorkspaceDoctorReport>("workspace-doctor-report", (event) => {
  renderWorkspaceChecks(event.payload);
});

setLocale(getLocale());
applyStaticI18n();
updateFooterCopy();
updateWiringModeFootnote();
updateLangButtons();
refreshPresetGroupLabels();
applyProviderPreset("custom");
showPersonalListView();
modeUsePersonalEl.addEventListener("click", () => {
  void enablePersonalMode();
});
modeUseTeamEl.addEventListener("click", () => {
  void enableTeamMode();
});

void loadProfiles();
void loadWorkspaces();
void loadRemoteProjects();
void loadEvotownStatus();
void loadPersonalProviderStatus();
void loadModeStatus();
void loadMcpStatus();
void refresh();
