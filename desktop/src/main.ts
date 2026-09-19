import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import {
  applyStaticI18n,
  getLocale,
  setLocale,
  t,
  type Locale,
} from "./i18n";
import { escapeHtml, formatTime, formatRate, formatCount } from "./format";
import {
  isAskRuntimeId,
  renderRuntimeCard,
  renderRuntimeCardActions,
  renderRuntimeTabs,
  resolveActiveRuntimeId,
  runtimeAdvancedMeta,
  supportsBrowserMcp,
  type RuntimeCardActionContext,
} from "./agents-ui";
import {
  preferredRepairFilter,
  renderDiagnosePendingHtml,
  renderRelatedResourcesHtml,
  renderRepairPreview,
} from "./repair-ui";
import type {
  DoctorReport,
  EngineRegisterStatus,
  EvotownStatus,
  HermesSettings,
  InstallProgressEvent,
  InstallRuntimeResponse,
  MainTabId,
  McpModuleStatus,
  ModeStatus,
  ModeSwitchReport,
  OnboardingReport,
  OpenSessionReport,
  PersonalProviderListItem,
  PersonalProviderSetupReport,
  PersonalProvidersDocument,
  PersonalProviderStatus,
  PersonalProviderVerifyReport,
  ProfileEntry,
  ProfilesDocument,
  ProviderProtocol,
  ProviderTabId,
  RegisterReport,
  RemoteDoctorReport,
  RemoteHostsDocument,
  RemoteProjectRow,
  RemoteProbeCheck,
  RepairPreviewResponse,
  RepairStatusFilter,
  RestoreSummary,
  RuntimeDoctorResult,
  SkillMountReport,
  SkillsInventoryReport,
  SyncReport,
  UseProfileReport,
  WindowSizeReport,
  WorkspaceCheck,
  WorkspaceDoctorReport,
  WorkspaceFixReport,
  WorkspacesDocument,
} from "./types";

if (navigator.userAgent.includes("Windows")) {
  document.documentElement.classList.add("is-opaque-shell");
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
const evotownEngineEl = document.querySelector<HTMLElement>("#evotown-engine")!;
const evotownEngineBadgeEl = document.querySelector<HTMLElement>("#evotown-engine-badge")!;
const evotownEngineStatusEl = document.querySelector<HTMLElement>("#evotown-engine-status")!;
const evotownEngineFormEl = document.querySelector<HTMLFormElement>("#evotown-engine-form")!;
const evotownBootstrapEl = document.querySelector<HTMLInputElement>("#evotown-bootstrap")!;
const evotownEngineIdEl = document.querySelector<HTMLInputElement>("#evotown-engine-id")!;
const evotownEngineRotateEl = document.querySelector<HTMLInputElement>("#evotown-engine-rotate")!;
const evotownEngineRegisterEl =
  document.querySelector<HTMLButtonElement>("#evotown-engine-register")!;
const evotownEngineHintEl = document.querySelector<HTMLElement>("#evotown-engine-hint")!;
const skillsInventoryEl = document.querySelector<HTMLElement>("#skills-inventory")!;
const skillsRefreshEl = document.querySelector<HTMLButtonElement>("#skills-refresh")!;
const skillsMountAllEl = document.querySelector<HTMLButtonElement>("#skills-mount-all")!;
const skillsDirEl = document.querySelector<HTMLElement>("#skills-dir")!;
const skillsListEl = document.querySelector<HTMLUListElement>("#skills-list")!;
const skillsEmptyEl = document.querySelector<HTMLElement>("#skills-empty")!;
const skillsFootnoteEl = document.querySelector<HTMLElement>("#skills-footnote")!;
const skillsCountEl = document.querySelector<HTMLElement>("#skills-count")!;
const mcpCountEl = document.querySelector<HTMLElement>("#mcp-count")!;

const hubMcpBadgeEl = document.querySelector<HTMLElement>("#hub-mcp-badge")!;
const hubBrowserStatusEl = document.querySelector<HTMLElement>("#hub-browser-status")!;
const hubSkillsCountEl = document.querySelector<HTMLElement>("#hub-skills-count")!;
const hubMcpCountEl = document.querySelector<HTMLElement>("#hub-mcp-count")!;
const hubConfiguredEl = document.querySelector<HTMLElement>("#hub-configured")!;
const openResourcesWindowEl = document.querySelector<HTMLButtonElement>("#open-resources-window")!;
const openResourcesBrowserEl = document.querySelector<HTMLButtonElement>("#open-resources-browser")!;
const toolbarResourcesEl = document.querySelector<HTMLButtonElement>("#toolbar-resources");

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
  agentsWsChipEl.classList.toggle("is-empty", !doc.active);
  agentsWsChipEl.classList.toggle("needs-attention", !doc.active);
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
    // Refresh CTA now that workspace is active.
    if (activeRuntimeId && lastReport) {
      const card = runtimesEl.querySelector<HTMLElement>(`[data-runtime="${activeRuntimeId}"]`);
      if (card) {
        refreshRuntimeCardActions(card, activeRuntimeId);
      }
    }
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

let lastSkillsInventory: SkillsInventoryReport | null = null;
let lastMcpStatus: McpModuleStatus | null = null;

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
    void loadResourcesHub();
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
const agentsReadinessRingEl = document.querySelector<HTMLElement>("#agents-readiness-ring")!;
const agentsReadinessValueEl = document.querySelector<HTMLElement>("#agents-readiness-value")!;
const agentsSecurityTitleEl = document.querySelector<HTMLElement>("#agents-security-title")!;
const agentsSecurityDescEl = document.querySelector<HTMLElement>("#agents-security-desc")!;
const agentsSecurityInstalledEl =
  document.querySelector<HTMLElement>("#agents-security-installed")!;
const agentsSecurityIssuesEl = document.querySelector<HTMLElement>("#agents-security-issues")!;
const diagnoseDetailEl = document.querySelector<HTMLElement>("#diagnose-detail")!;
const diagnoseDetailBodyEl = document.querySelector<HTMLElement>("#diagnose-detail-body")!;
const widgetToolbarEl = document.querySelector<HTMLElement>(".widget-toolbar")!;
const windowCloseEl = document.querySelector<HTMLButtonElement>("#window-close")!;
const windowMinimizeEl = document.querySelector<HTMLButtonElement>("#window-minimize")!;
const windowMaximizeEl = document.querySelector<HTMLButtonElement>("#window-maximize")!;
const mainWindow = getCurrentWindow();
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

let lastReport: DoctorReport | null = null;
let lastProfiles: ProfilesDocument | null = null;
let lastWorkspaces: WorkspacesDocument | null = null;
let hermesModel: HermesSettings | null = null;
let activeRuntimeId: string | null = null;

const repairPreviewByRuntime = new Map<string, RepairPreviewResponse>();
const repairFilterByRuntime = new Map<string, RepairStatusFilter>();
const repairConfirmRuntimeIds = new Set<string>();
let selectedPresetName = "";
let selectedWorkspaceName = "";
let presetMenuOpen = false;
let workspaceBusy = false;

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

function updateAgentsSecurityOverview(report: DoctorReport): void {
  const installed = report.runtimes.filter((runtime) => runtime.installed).length;
  const total = report.runtimes.length;
  const issueCount = [...repairPreviewByRuntime.values()].reduce(
    (count, preview) => count + preview.summary.fail + preview.summary.warn,
    0,
  );
  const readiness = total > 0 ? Math.round((installed / total) * 100) : 0;
  agentsReadinessRingEl.style.setProperty("--readiness", String(readiness));
  agentsReadinessValueEl.textContent = total > 0 ? `${installed}/${total}` : "—";
  agentsSecurityTitleEl.textContent =
    installed === total && total > 0 ? t("agents.securityReady") : t("agents.securityTitle");
  agentsSecurityDescEl.textContent = t("agents.securityDesc", {
    installed: String(installed),
    total: String(total),
    issues: String(issueCount),
  });
  agentsSecurityInstalledEl.textContent = t("agents.runtimeAvailable", {
    count: String(installed),
  });
  agentsSecurityIssuesEl.textContent = t("agents.knownIssues", {
    count: String(issueCount),
  });
  agentsSecurityIssuesEl.classList.toggle("has-issues", issueCount > 0);
}

const MAIN_COMPACT_WIDTH = 420;
const MAIN_DETAIL_EXTRA = 380;

let diagnoseDetailOpen = false;
let compactWidthBeforeDetail: number | null = null;
const dismissedDiagnoseRuntimes = new Set<string>();

function runtimeCardEl(runtime: string): HTMLElement | null {
  return runtimesEl.querySelector<HTMLElement>(`[data-runtime="${runtime}"]`);
}

async function setMainWindowWidth(width: number): Promise<WindowSizeReport> {
  return invoke<WindowSizeReport>("resize_main_window_command", {
    width,
    height: null,
  });
}

async function readMainWindowSize(): Promise<WindowSizeReport> {
  return invoke<WindowSizeReport>("resize_main_window_command", {
    width: null,
    height: null,
  });
}

function diagnoseRuntimeLabel(runtimeId: string): string {
  return (
    lastReport?.runtimes.find((item) => item.id === runtimeId)?.display_name ?? runtimeId
  );
}

function showDiagnosePending(
  runtimeId: string,
  message: string,
  step: "diagnose" | "repair" = "diagnose",
): void {
  diagnoseDetailBodyEl.innerHTML = renderDiagnosePendingHtml(
    runtimeId,
    diagnoseRuntimeLabel(runtimeId),
    message,
    step,
  );
  diagnoseDetailEl.dataset.runtime = runtimeId;
  void expandDiagnoseWindowIfNeeded();
}

async function expandDiagnoseWindowIfNeeded(): Promise<void> {
  diagnoseDetailEl.hidden = false;
  document.body.classList.add("is-diagnose-layout");
  if (!diagnoseDetailOpen) {
    try {
      compactWidthBeforeDetail = (await readMainWindowSize()).width;
    } catch {
      compactWidthBeforeDetail = MAIN_COMPACT_WIDTH;
    }
    const compact = compactWidthBeforeDetail ?? MAIN_COMPACT_WIDTH;
    document.body.style.setProperty("--compact-window-width", `${compact}px`);
    diagnoseDetailOpen = true;
    try {
      await setMainWindowWidth(compact + MAIN_DETAIL_EXTRA);
    } catch (error) {
      setStatusBanner("error", t("runtime.openFailed", { error: String(error) }));
    }
  }
  await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  document.body.classList.add("is-diagnose-open");
}

async function openDiagnoseDetail(report: RepairPreviewResponse): Promise<void> {
  const filter = repairFilterByRuntime.get(report.runtime_id) ?? "all";
  diagnoseDetailBodyEl.innerHTML = renderRepairPreview(report, filter, {
    confirmPending: repairConfirmRuntimeIds.has(report.runtime_id),
    isAskRuntime: isAskRuntimeId(report.runtime_id),
    supportsBrowserMcp: supportsBrowserMcp(report.runtime_id),
  });
  diagnoseDetailEl.dataset.runtime = report.runtime_id;
  await expandDiagnoseWindowIfNeeded();
  const card = runtimeCardEl(report.runtime_id);
  if (card) {
    refreshRuntimeCardActions(card, report.runtime_id);
  }
}

async function closeDiagnoseDetail(opts?: {
  keepContent?: boolean;
  skipDismiss?: boolean;
}): Promise<void> {
  const runtime = diagnoseDetailEl.dataset.runtime;
  if (runtime) {
    repairConfirmRuntimeIds.delete(runtime);
  }
  if (!opts?.keepContent && !opts?.skipDismiss && runtime) {
    dismissedDiagnoseRuntimes.add(runtime);
  }
  document.body.classList.remove("is-diagnose-open");
  if (diagnoseDetailOpen) {
    await new Promise((resolve) => window.setTimeout(resolve, 180));
  }
  diagnoseDetailEl.hidden = true;
  if (!opts?.keepContent) {
    diagnoseDetailBodyEl.replaceChildren();
    delete diagnoseDetailEl.dataset.runtime;
  }
  diagnoseDetailOpen = false;
  if (runtime) {
    const card = runtimeCardEl(runtime);
    if (card) {
      refreshRuntimeCardActions(card, runtime);
    }
  }
  const compact = compactWidthBeforeDetail ?? MAIN_COMPACT_WIDTH;
  compactWidthBeforeDetail = null;
  await setMainWindowWidth(compact);
  document.body.classList.remove("is-diagnose-layout");
  document.body.style.removeProperty("--compact-window-width");
}

function mountRepairPreview(report: RepairPreviewResponse, opts?: { resetFilter?: boolean }): void {
  const runtime = report.runtime_id;
  dismissedDiagnoseRuntimes.delete(runtime);
  repairPreviewByRuntime.set(runtime, report);
  if (lastReport) {
    updateAgentsSecurityOverview(lastReport);
  }
  if (opts?.resetFilter || !repairFilterByRuntime.has(runtime)) {
    repairFilterByRuntime.set(runtime, preferredRepairFilter(report));
  }
  const card = runtimeCardEl(runtime);
  const hint = card?.querySelector<HTMLElement>("[data-repair-hint]");
  if (hint && !hint.querySelector("[data-install-progress]")) {
    hint.hidden = true;
    hint.replaceChildren();
  }
  if (card) {
    mountRelatedResources(card, runtime);
    refreshRuntimeCardActions(card, runtime);
  }
  void openDiagnoseDetail(report);
}

function runtimeCardActionContext(runtime: RuntimeDoctorResult): RuntimeCardActionContext {
  return {
    preview: repairPreviewByRuntime.get(runtime.id),
    confirmPending: repairConfirmRuntimeIds.has(runtime.id),
    diagnoseOpenForRuntime:
      !diagnoseDetailEl.hidden &&
      diagnoseDetailEl.dataset.runtime === runtime.id,
    dismissed: dismissedDiagnoseRuntimes.has(runtime.id),
    hasActiveWorkspace: hasActiveWorkspace(),
    isAskRuntime: isAskRuntimeId(runtime.id),
    supportsBrowserMcp: supportsBrowserMcp(runtime.id),
  };
}

function buildRuntimeCardHtml(runtime: RuntimeDoctorResult): string {
  const preview = repairPreviewByRuntime.get(runtime.id);
  const advancedMeta = runtimeAdvancedMeta(runtime, hermesModel);
  const actionsHtml = renderRuntimeCardActions(
    runtime,
    advancedMeta,
    runtimeCardActionContext(runtime),
  );
  const relatedResourcesHtml = renderRelatedResourcesHtml(preview);
  return renderRuntimeCard(runtime, hermesModel, actionsHtml, relatedResourcesHtml);
}

function refreshRuntimeCardActions(card: HTMLElement, runtimeId: string): void {
  const runtime = lastReport?.runtimes.find((item) => item.id === runtimeId);
  if (!runtime) {
    return;
  }
  const actions = card.querySelector(".card-actions");
  if (!actions) {
    return;
  }
  const html = renderRuntimeCardActions(
    runtime,
    runtimeAdvancedMeta(runtime, hermesModel),
    runtimeCardActionContext(runtime),
  );
  if (html) {
    actions.innerHTML = html;
  }
}

function applyRepairFilter(runtime: string, filter: RepairStatusFilter): void {
  const report = repairPreviewByRuntime.get(runtime);
  if (!report) {
    return;
  }
  const current = repairFilterByRuntime.get(runtime) ?? "all";
  const next = current === filter && filter !== "all" ? "all" : filter;
  repairFilterByRuntime.set(runtime, next);
  diagnoseDetailBodyEl.innerHTML = renderRepairPreview(report, next, {
    confirmPending: repairConfirmRuntimeIds.has(report.runtime_id),
    isAskRuntime: isAskRuntimeId(report.runtime_id),
    supportsBrowserMcp: supportsBrowserMcp(report.runtime_id),
  });
}

function hasActiveWorkspace(): boolean {
  return Boolean(lastWorkspaces?.active);
}

function mountRelatedResources(card: HTMLElement, runtime: string): void {
  const el = card.querySelector<HTMLElement>("[data-related-resources]");
  if (!el) {
    return;
  }
  el.outerHTML = renderRelatedResourcesHtml(repairPreviewByRuntime.get(runtime));
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
  updateAgentsSecurityOverview(report);

  setStatusBanner(
    report.profile_env_exists ? "ok" : "warn",
    report.profile_env_exists ? t("doctor.companyOk") : t("doctor.companyMissing"),
  );

  if (report.runtimes.some((runtime) => runtime.id === "hermes" && runtime.installed)) {
    await loadHermesModel();
  } else {
    hermesModel = null;
  }

  if (report.runtimes.length === 0) {
    activeRuntimeId = null;
    runtimeTabsEl.innerHTML = "";
    runtimesEl.innerHTML = `<div class="empty-state">${t("runtimes.empty")}</div>`;
    void closeDiagnoseDetail({ skipDismiss: true });
    return;
  }

  const selectedId = resolveActiveRuntimeId(report.runtimes, activeRuntimeId)!;
  activeRuntimeId = selectedId;
  runtimeTabsEl.innerHTML = renderRuntimeTabs(
    report.runtimes,
    selectedId,
    repairPreviewByRuntime,
  );

  const activeRuntime = report.runtimes.find((runtime) => runtime.id === selectedId);
  runtimesEl.innerHTML = activeRuntime ? buildRuntimeCardHtml(activeRuntime) : "";
  const preview = selectedId ? repairPreviewByRuntime.get(selectedId) : undefined;
  if (preview && !dismissedDiagnoseRuntimes.has(selectedId)) {
    mountRepairPreview(preview);
  } else {
    void closeDiagnoseDetail({ skipDismiss: true });
  }
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
    evotownEngineEl.hidden = false;
    void loadEngineRegisterStatus();
    void loadSkillsInventory();
  } else {
    evotownStatusEl.textContent = t("evotown.notConfigured");
    evotownConnectedMetaEl.textContent = "";
    evotownConnectedMetaEl.title = "";
    evotownResyncEl.hidden = true;
    evotownEngineEl.hidden = true;
    evotownEngineHintEl.textContent = "";
    skillsInventoryEl.hidden = true;
  }
}

const RUNTIME_LABELS: Record<string, string> = {
  hermes: "Hermes",
  openclaw: "OpenClaw",
  "claude-code": "Claude",
  codex: "Codex",
  "deepseek-harness": "DeepSeek Harness",
};

function renderSkillsInventory(report: SkillsInventoryReport) {
  skillsInventoryEl.hidden = false;
  skillsDirEl.textContent = t("skills.dir", { dir: report.skills_dir });
  skillsFootnoteEl.textContent = t("skills.footnote");
  skillsListEl.replaceChildren();
  const needEvotown = report.remote_stats_error === "evotown_not_configured";
  skillsEmptyEl.textContent = needEvotown ? t("skills.emptyNeedEvotown") : t("skills.empty");

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
    updateResourcesHubSummary();
  } catch (error) {
    lastSkillsInventory = null;
    skillsCountEl.textContent = "—";
    skillsInventoryEl.hidden = false;
    skillsListEl.replaceChildren();
    skillsEmptyEl.hidden = false;
    skillsEmptyEl.textContent = t("skills.loadFailed", { error: String(error) });
    skillsDirEl.textContent = "";
    skillsFootnoteEl.textContent = "";
    updateResourcesHubSummary();
  }
}

function updateResourcesHubSummary(): void {
  const uniqueMcpNames = new Set(
    (lastMcpStatus?.inventory.servers ?? []).map((server) => server.name.trim().toLowerCase()),
  );
  const skillCount = lastSkillsInventory?.skills.length ?? 0;
  const mcpCount = uniqueMcpNames.size;
  mcpCountEl.textContent = lastMcpStatus ? String(mcpCount) : "—";
  hubSkillsCountEl.textContent = lastSkillsInventory ? String(skillCount) : "—";
  hubMcpCountEl.textContent = lastMcpStatus ? String(mcpCount) : "—";

  const chrome = lastMcpStatus?.browser;
  const configured = lastMcpStatus?.configured_runtimes ?? [];
  hubConfiguredEl.textContent =
    configured.length > 0 ? configured.join(", ") : t("mcp.configuredNone");

  hubMcpBadgeEl.classList.remove("ok", "warn", "muted", "bad");
  if (!lastMcpStatus || !chrome) {
    hubMcpBadgeEl.textContent = "—";
    hubMcpBadgeEl.classList.add("muted");
    hubBrowserStatusEl.textContent = "—";
    return;
  }
  if (!chrome.chrome_found) {
    hubMcpBadgeEl.textContent = t("mcp.badgeMissing");
    hubMcpBadgeEl.classList.add("bad");
    hubBrowserStatusEl.textContent = t("mcp.chromeMissing");
  } else if (configured.length > 0) {
    hubMcpBadgeEl.textContent = t("mcp.badgeReady");
    hubMcpBadgeEl.classList.add("ok");
    hubBrowserStatusEl.textContent = t("mcp.chromeOk", {
      version: chrome.version || chrome.binary || "OK",
    });
  } else {
    hubMcpBadgeEl.textContent = t("mcp.badgePartial");
    hubMcpBadgeEl.classList.add("warn");
    hubBrowserStatusEl.textContent = t("mcp.chromeOk", {
      version: chrome.version || chrome.binary || "OK",
    });
  }
}

async function loadMcpStatus() {
  try {
    const status = await invoke<McpModuleStatus>("mcp_status_command", {
      port: null,
      probeChrome: false,
    });
    lastMcpStatus = status;
    updateResourcesHubSummary();
  } catch {
    lastMcpStatus = null;
    mcpCountEl.textContent = "—";
    updateResourcesHubSummary();
  }
}

async function loadResourcesHub() {
  await Promise.all([loadMcpStatus(), loadSkillsInventory({ remoteStats: false })]);
}

async function openResourcesWindow(section?: "catalog" | "browser"): Promise<void> {
  await invoke("open_resources_window_command", { section: section ?? null });
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

async function loadEngineRegisterStatus() {
  try {
    const status = await invoke<EngineRegisterStatus>("get_engine_register_status_command");
    if (status.registered && status.engine_id) {
      evotownEngineStatusEl.textContent = t("evotown.engineReady", { id: status.engine_id });
      evotownEngineBadgeEl.hidden = false;
      evotownEngineBadgeEl.className = "badge ok";
      evotownEngineBadgeEl.textContent = t("evotown.engineBadgeOk");
      if (!evotownEngineIdEl.value.trim()) {
        evotownEngineIdEl.value = status.engine_id;
      }
    } else {
      evotownEngineStatusEl.textContent = t("evotown.engineMissing");
      evotownEngineBadgeEl.hidden = false;
      evotownEngineBadgeEl.className = "badge muted";
      evotownEngineBadgeEl.textContent = t("evotown.engineBadgeMissing");
    }
  } catch (error) {
    evotownEngineStatusEl.textContent = t("evotown.engineRegisterFailed", {
      error: String(error),
    });
    evotownEngineBadgeEl.hidden = true;
  }
}

async function runEngineRegister() {
  const bootstrap = evotownBootstrapEl.value.trim();
  if (!bootstrap) {
    evotownEngineHintEl.textContent = t("evotown.engineTokenRequired");
    return;
  }

  evotownEngineRegisterEl.disabled = true;
  evotownEngineHintEl.textContent = t("evotown.engineRegistering");
  try {
    const report = await invoke<RegisterReport>("run_engine_register_command", {
      bootstrapToken: bootstrap,
      engineId: evotownEngineIdEl.value.trim() || null,
      rotate: evotownEngineRotateEl.checked,
    });
    evotownBootstrapEl.value = "";
    evotownEngineRotateEl.checked = false;
    await loadEngineRegisterStatus();
    evotownEngineHintEl.textContent = t("evotown.engineRegisterOk", { id: report.engine_id });
  } catch (error) {
    evotownEngineHintEl.textContent = t("evotown.engineRegisterFailed", {
      error: String(error),
    });
  } finally {
    evotownEngineRegisterEl.disabled = false;
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
    mountRepairPreview(report, { resetFilter: true });
    diagnoseDetailBodyEl.insertAdjacentHTML(
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

function withTimeout<T>(promise: Promise<T>, ms: number, timeoutError: Error): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => reject(timeoutError), ms);
    promise.then(
      (value) => {
        window.clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        window.clearTimeout(timer);
        reject(error);
      },
    );
  });
}

async function openAskWindow(runtime: string): Promise<void> {
  try {
    await withTimeout(
      invoke("open_ask_window_command", { runtime }),
      15_000,
      new Error(t("runtime.openTimeout")),
    );
  } catch (error) {
    try {
      await invoke("close_ask_window_command", { destroy: true });
    } catch {
      /* window may already be gone or the UI thread is stuck */
    }
    setStatusBanner("error", t("runtime.openFailed", { error: String(error) }));
  }
}

const ASK_VERIFY_DRAFT_KEY = "agent-doctor.ask.verifyDraft";

async function openAskWindowForVerify(runtime: string): Promise<void> {
  try {
    localStorage.setItem(
      ASK_VERIFY_DRAFT_KEY,
      JSON.stringify({ prompt: t("ask.verifyPrompt"), autoSend: true }),
    );
    await withTimeout(
      invoke("open_ask_window_command", { runtime }),
      15_000,
      new Error(t("runtime.openTimeout")),
    );
  } catch (error) {
    try {
      await invoke("close_ask_window_command", { destroy: true });
    } catch {
      /* window may already be gone or the UI thread is stuck */
    }
    setStatusBanner("error", t("runtime.openFailed", { error: String(error) }));
  }
}

async function runBrowserSmokeFromCard(root: HTMLElement): Promise<void> {
  const host = diagnoseDetailEl.hidden ? root : diagnoseDetailEl;
  const slot = host.querySelector<HTMLElement>("[data-browser-smoke-slot]");
  const button = host.querySelector<HTMLButtonElement>('[data-action="browser-smoke"]');
  button?.setAttribute("disabled", "true");
  if (slot) {
    slot.className = "repair-smoke-slot";
    slot.textContent = t("repair.browserSmokeRunning");
  }
  try {
    const smoke = await invoke<{ ok: boolean; detail: string }>("run_browser_smoke_command");
    if (slot) {
      slot.className = `repair-smoke-slot ${smoke.ok ? "ok" : "fail"}`;
      slot.textContent = `${smoke.ok ? t("repair.browserSmokeOk") : t("repair.browserSmokeFail")}: ${smoke.detail}`;
    }
  } catch (error) {
    if (slot) {
      slot.className = "repair-smoke-slot fail";
      slot.textContent = `${t("repair.browserSmokeFail")}: ${String(error)}`;
    }
  } finally {
    button?.removeAttribute("disabled");
  }
}

const openingSessionRuntimes = new Set<string>();

async function openSessionFromCard(card: HTMLElement, forceTerminal = false) {
  const runtime = card.dataset.runtime;
  const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
  const openButtons = [
    ...card.querySelectorAll<HTMLButtonElement>('[data-action="open-session"]'),
  ];
  if (!runtime || openingSessionRuntimes.has(runtime)) {
    return;
  }
  openingSessionRuntimes.add(runtime);
  for (const button of openButtons) {
    button.setAttribute("disabled", "true");
  }
  if (hint) {
    hint.hidden = false;
    hint.textContent = t("runtime.opening");
  }
  try {
    const report = await withTimeout(
      invoke<OpenSessionReport>("open_session_command", {
        runtime,
        cwd: null,
        prompt: null,
        terminal: forceTerminal ? true : null,
      }),
      20_000,
      new Error(t("runtime.openTimeout")),
    );
    const method = report.method === "deep-link" ? "deep-link" : "terminal";
    if (hint) {
      hint.textContent = t("runtime.openOk", { method });
    }
    setStatusBanner("ok", t("runtime.openOk", { method }));
  } catch (error) {
    if (hint) {
      hint.hidden = false;
      hint.textContent = t("runtime.openFailed", { error: String(error) });
    }
    setStatusBanner("error", t("runtime.openFailed", { error: String(error) }));
  } finally {
    openingSessionRuntimes.delete(runtime);
    for (const button of openButtons) {
      button.removeAttribute("disabled");
    }
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
    const clamped = Math.min(100, Math.max(0, percent));
    if (statusEl) {
      statusEl.textContent =
        phase === "done"
          ? t("runtime.installOk")
          : phase === "verifying"
            ? t("runtime.installVerifying")
            : message.trim() || t("runtime.installing");
    }
    if (percentEl) {
      percentEl.textContent = `${clamped}%`;
    }
    if (fillEl) {
      fillEl.style.width = `${clamped}%`;
      fillEl.classList.toggle("is-indeterminate", clamped < 2 && phase !== "done");
    }
    if (logEl && message.trim()) {
      const isByteProgress = /Downloading Node\.js .+\/|正在下载/.test(message);
      if (
        isByteProgress &&
        logLines.length > 0 &&
        /Downloading Node\.js .+\/|正在下载/.test(logLines[logLines.length - 1] ?? "")
      ) {
        logLines[logLines.length - 1] = message;
      } else {
        logLines.push(message);
      }
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
  showDiagnosePending(runtime, t("repair.applying"), "repair");
  try {
    const report = await invoke<RepairPreviewResponse>("run_repair_execute_command", { runtime });
    mountRepairPreview(report, { resetFilter: true });
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

/** Diagnose if needed, then apply playbook (gateway + browser MCP) in one click. */
async function oneClickRepairRuntimeCard(card: HTMLElement) {
  const runtime = card.dataset.runtime;
  const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
  if (!runtime || !hint) {
    return;
  }

  let preview = repairPreviewByRuntime.get(runtime);
  if (!preview) {
    hint.hidden = false;
    hint.textContent = t("runtime.diagnosing");
    showDiagnosePending(runtime, t("runtime.diagnosing"));
    try {
      preview = await invoke<RepairPreviewResponse>("run_repair_preview_command", { runtime });
      mountRepairPreview(preview, { resetFilter: true });
    } catch (error) {
      hint.hidden = false;
      hint.textContent = String(error);
      return;
    }
  }

  if (!preview.can_apply_repair) {
    hint.hidden = false;
    mountRepairPreview(preview);
    return;
  }

  await applyRepairRuntimeCard(card);
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
  showDiagnosePending(runtime, t("runtime.diagnosing"));
  try {
    const report = await invoke<RepairPreviewResponse>("run_repair_preview_command", { runtime });
    mountRepairPreview(report, { resetFilter: true });
  } catch (error) {
    hint.textContent = String(error);
  } finally {
    button?.removeAttribute("disabled");
  }
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
  if (lastReport) {
    void renderReport(lastReport);
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
    const forceTerminal =
      target.closest<HTMLElement>("[data-action='open-session']")?.dataset.openTerminal === "1";
    void openSessionFromCard(runtimeCard, forceTerminal);
    return;
  }

  if (action === "ask-session" && runtimeCard) {
    const runtime = runtimeCard.dataset.runtime;
    if (runtime && isAskRuntimeId(runtime)) {
      void openAskWindow(runtime);
    }
    return;
  }

  if (action === "ask-verify" && runtimeCard) {
    const runtime = runtimeCard.dataset.runtime;
    if (runtime && supportsBrowserMcp(runtime)) {
      void openAskWindowForVerify(runtime);
    }
    return;
  }

  if (action === "browser-smoke" && runtimeCard) {
    void runBrowserSmokeFromCard(runtimeCard);
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

  if (action === "activate-workspace") {
    if (!lastWorkspaces || Object.keys(lastWorkspaces.workspaces).length === 0) {
      setMainTab("workspace");
      return;
    }
    toggleAgentsWsPicker();
    return;
  }

  if (action === "wire-runtime" && runtimeCard) {
    const hint = runtimeCard.querySelector<HTMLElement>("[data-repair-hint]");
    void rewireCurrentMode(hint);
    return;
  }

  if (action === "apply-repair" && runtimeCard) {
    const runtime = runtimeCard.dataset.runtime;
    if (runtime) {
      repairConfirmRuntimeIds.add(runtime);
      const report = repairPreviewByRuntime.get(runtime);
      if (report) {
        void openDiagnoseDetail(report);
      }
    }
    return;
  }

  if (action === "rollback-repair" && runtimeCard) {
    void rollbackRepairRuntimeCard(runtimeCard);
    return;
  }

  const guideBtn = target.closest<HTMLButtonElement>('[data-action="open-repair-guide"]');
  if (guideBtn?.dataset.guidePath) {
    void openRepairGuide(decodeURIComponent(guideBtn.dataset.guidePath));
  }
});

diagnoseDetailEl.addEventListener("click", (event) => {
  const target = event.target as HTMLElement;
  const filterBtn = target.closest<HTMLButtonElement>("[data-repair-filter]");
  if (filterBtn && !filterBtn.disabled) {
    const runtime =
      filterBtn.closest<HTMLElement>("[data-runtime]")?.dataset.runtime ||
      diagnoseDetailEl.dataset.runtime;
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

  if (action === "close-diagnose-detail") {
    void closeDiagnoseDetail();
    return;
  }

  const runtime =
    target.closest<HTMLElement>("[data-runtime]")?.dataset.runtime ||
    diagnoseDetailEl.dataset.runtime;
  const card = runtime ? runtimeCardEl(runtime) : null;

  if (action === "ask-session" && runtime) {
    if (isAskRuntimeId(runtime)) {
      void openAskWindow(runtime);
    }
    return;
  }

  if (action === "ask-verify" && runtime) {
    if (supportsBrowserMcp(runtime)) {
      void openAskWindowForVerify(runtime);
    }
    return;
  }

  if (action === "browser-smoke") {
    void runBrowserSmokeFromCard(diagnoseDetailEl);
    return;
  }

  if (action === "go-wiring") {
    void closeDiagnoseDetail();
    setMainTab("provider");
    return;
  }

  if (action === "preview-repair" && runtime) {
    repairConfirmRuntimeIds.add(runtime);
    const report = repairPreviewByRuntime.get(runtime);
    if (report) {
      void openDiagnoseDetail(report);
    } else if (card) {
      void diagnoseRuntimeCard(card);
    } else {
      setStatusBanner("error", t("doctor.empty"));
    }
    return;
  }

  if (action === "cancel-repair-preview" && runtime) {
    repairConfirmRuntimeIds.delete(runtime);
    const report = repairPreviewByRuntime.get(runtime);
    if (report) {
      void openDiagnoseDetail(report);
    }
    return;
  }

  if ((action === "confirm-repair" || action === "apply-repair") && card) {
    if (runtime) {
      repairConfirmRuntimeIds.delete(runtime);
    }
    void oneClickRepairRuntimeCard(card);
    return;
  }

  if (action === "rollback-repair" && card) {
    void rollbackRepairRuntimeCard(card);
    return;
  }

  const guideBtn = target.closest<HTMLButtonElement>('[data-action="open-repair-guide"]');
  if (guideBtn?.dataset.guidePath) {
    void openRepairGuide(decodeURIComponent(guideBtn.dataset.guidePath));
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

openResourcesWindowEl.addEventListener("click", () => {
  void openResourcesWindow("catalog");
});
openResourcesBrowserEl.addEventListener("click", () => {
  void openResourcesWindow("browser");
});
toolbarResourcesEl?.addEventListener("click", () => {
  void openResourcesWindow("catalog");
});

evotownFormEl.addEventListener("submit", (event) => {
  event.preventDefault();
  void runEvotownOnboarding();
});

evotownResyncEl.addEventListener("click", () => {
  void resyncEvotownSkills();
});

evotownEngineFormEl.addEventListener("submit", (event) => {
  event.preventDefault();
  void runEngineRegister();
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

windowCloseEl.addEventListener("click", () => {
  // Hide to tray — destroying main while Ask is alive leaves a tray-only
  // process that cannot reopen the shell without a rebuild.
  void mainWindow.hide();
});

windowMinimizeEl.addEventListener("click", () => {
  // Undecorated WebView2 on Windows often fails to restore from a real
  // minimize via the taskbar; hide to tray and restore via tray / relaunch.
  const isWindows = navigator.userAgent.includes("Windows");
  if (isWindows) {
    void mainWindow.hide();
  } else {
    void mainWindow.minimize();
  }
});

windowMaximizeEl.addEventListener("click", () => {
  void mainWindow.toggleMaximize();
});

widgetToolbarEl.addEventListener("dblclick", (event) => {
  if (!(event.target as HTMLElement).closest("button")) {
    void mainWindow.toggleMaximize();
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

void listen<{ tab?: string }>("main-navigate", (event) => {
  const tab = event.payload?.tab;
  if (tab === "diagnose" || tab === "resources" || tab === "provider" || tab === "workspace") {
    setMainTab(tab);
  }
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

// A webview reload can preserve the native width from an open diagnose panel
// while resetting the frontend's detail state. Always restore compact startup.
void setMainWindowWidth(MAIN_COMPACT_WIDTH);
void loadProfiles();
void loadWorkspaces();
void loadRemoteProjects();
void loadEvotownStatus();
void loadPersonalProviderStatus();
void loadModeStatus();
// Do not call loadMcpStatus() on boot — discover_chrome / CDP probe must not
// wake Chrome until the user opens Resources or clicks Browser smoke.
void refresh();
