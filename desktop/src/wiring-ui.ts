import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { isPersonalEdition, isTeamEdition, productEdition } from "./edition";
import { appState } from "./app-state";
import type {
  EngineRegisterStatus,
  EvotownStatus,
  ModeStatus,
  ModeSwitchReport,
  OnboardingReport,
  PersonalProviderListItem,
  PersonalProviderSetupReport,
  PersonalProvidersDocument,
  PersonalProviderStatus,
  PersonalProviderVerifyReport,
  ProviderProtocol,
  RegisterReport,
  SyncReport,
} from "./types";

export interface WiringUiDeps {
  setMainTab: (tab: "provider") => void;
  refresh: () => Promise<void>;
  loadSkillsInventory: () => Promise<void>;
  hideSkillsInventory: () => void;
}

let deps!: WiringUiDeps;

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
const wiringModeFootnoteEl = document.querySelector<HTMLElement>("#wiring-mode-footnote")!;
const footerCopyEl = document.querySelector<HTMLElement>("#footer-copy")!;
function updateFooterCopy(mode?: string): void {
  const activeMode = mode ?? appState.lastModeStatus?.mode ?? productEdition();
  footerCopyEl.textContent = activeMode === "team" ? t("app.footerTeam") : t("app.footer");
}

function updateWiringModeFootnote(_mode?: string): void {
  wiringModeFootnoteEl.textContent = isTeamEdition()
    ? t("wiring.modeTeamFootnote")
    : t("wiring.modePersonalFootnote");
  wiringModeFootnoteEl.title = isTeamEdition()
    ? t("wiring.modeHintTeam")
    : t("wiring.modeHintPersonal");
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
const providerPanels = Array.from(document.querySelectorAll<HTMLElement>("[data-provider-panel]"));
/** Provider panel is locked to the build edition — no Personal/Team switch tabs. */
function syncProviderPanelToEdition() {
  const tab = isTeamEdition() ? "evotown" : "personal";
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
  appState.lastModeStatus = status;
  const meta = status.active_gateway_url
    ? t("mode.meta", {
        label: status.active_label || modeDisplayName(status.mode),
        url: status.active_gateway_url,
        key: status.active_key_hint || "—",
      })
    : t("mode.metaEmpty");
  modeMetaEl.textContent = meta;
  syncProviderPanelToEdition();
  updateWiringModeFootnote(status.mode);
  updateFooterCopy(isTeamEdition() ? "team" : "personal");
}

async function loadModeStatus() {
  try {
    const status = await invoke<ModeStatus>("get_mode_status_command");
    renderModeStatus(status);
  } catch (error) {
    modeMetaEl.textContent = String(error);
    syncProviderPanelToEdition();
  }
}

function showModeHint(text: string, detail?: string) {
  // Keep sr-only #mode-hint for a11y, but also surface on the visible footnote —
  // otherwise wiring actions look "stuck" while busy.
  modeHintEl.hidden = !text;
  modeHintEl.textContent = text;
  if (text) {
    wiringModeFootnoteEl.textContent = text;
    wiringModeFootnoteEl.title = detail || text;
  }
}

function setModeSwitchBusy(busy: boolean) {
  modeSwitchInFlight = busy;
  if (isPersonalEdition()) {
    personalApplyEl.disabled = busy;
    personalVerifyEl.disabled = busy;
    personalSaveEl.disabled = busy;
  } else {
    evotownConnectEl.disabled = busy;
    evotownResyncEl.disabled = busy;
  }
  wiringModeFootnoteEl.classList.toggle("is-busy", busy);
}

async function rewireCurrentMode(hintEl?: HTMLElement | null) {
  if (modeSwitchInFlight) return;
  const locked = productEdition();
  const mode = appState.lastModeStatus?.mode;
  if (mode !== locked) {
    // Edition package only rewires its own path; jump to wiring to configure.
    deps.setMainTab("provider");
    syncProviderPanelToEdition();
    showModeHint(
      isTeamEdition() ? t("mode.teamNotReady") : t("mode.personalNotReady"),
    );
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
    void deps.refresh();
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
const evotownBadgeEl = document.querySelector<HTMLElement>("#evotown-badge");
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
    void deps.loadSkillsInventory();
  } else {
    evotownStatusEl.textContent = t("evotown.notConfigured");
    evotownConnectedMetaEl.textContent = "";
    evotownConnectedMetaEl.title = "";
    evotownResyncEl.hidden = true;
    evotownEngineEl.hidden = true;
    evotownEngineHintEl.textContent = "";
    deps.hideSkillsInventory();
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
    await deps.refresh();
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
    await deps.loadSkillsInventory();
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
    appState.personalProvidersDoc = doc;
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

  const personalModeActive = appState.lastModeStatus?.mode === "personal";
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
      await deps.refresh();
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
    await deps.refresh();
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
    appState.personalProvidersDoc = doc;
    await loadPersonalProviderStatus();
    personalListHintEl.textContent = t("personal.deleteOk");
    showPersonalListView();
  } catch (error) {
    personalListHintEl.textContent = t("personal.applyFailed", { error: String(error) });
  } finally {
    setPersonalBusy(false);
  }
}


export interface WiringUiApi {
  syncProviderPanelToEdition: () => void;
  loadModeStatus: () => Promise<void>;
  renderModeStatus: (status: ModeStatus) => void;
  loadEvotownStatus: () => Promise<void>;
  loadPersonalProviderStatus: () => Promise<void>;
  rewireCurrentMode: (hintEl?: HTMLElement | null) => Promise<void>;
  updateFooterCopy: (mode?: string) => void;
  updateWiringModeFootnote: (mode?: string) => void;
  refreshPresetGroupLabels: () => void;
  applyProviderPreset: (presetId: string, opts?: { forceModel?: boolean }) => void;
  showPersonalListView: () => void;
  reloadWiringLocale: () => Promise<void>;
}

export function initWiringUi(d: WiringUiDeps): WiringUiApi {
  deps = d;

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
      const item = appState.personalProvidersDoc?.providers.find((p) => p.id === id);
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

  return {
    syncProviderPanelToEdition,
    loadModeStatus,
    renderModeStatus,
    loadEvotownStatus,
    loadPersonalProviderStatus,
    rewireCurrentMode,
    updateFooterCopy,
    updateWiringModeFootnote,
    refreshPresetGroupLabels,
    applyProviderPreset,
    showPersonalListView,
    reloadWiringLocale: async () => {
      refreshPresetGroupLabels();
      updateFooterCopy();
      updateWiringModeFootnote();
      await loadEvotownStatus();
      await loadPersonalProviderStatus();
      await loadModeStatus();
    },
  };
}
