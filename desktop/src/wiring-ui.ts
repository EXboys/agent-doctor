import { invoke } from "@tauri-apps/api/core";
import { t, type MessageKey } from "./i18n";
import { escapeHtml } from "./format";
import { isPersonalEdition, isTeamEdition, productEdition } from "./edition";
import { modelsForPresetId } from "./provider-models";
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
const evotownBadgeEl = document.querySelector<HTMLElement>("#evotown-badge");
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
const personalConnectedUrlEl = document.querySelector<HTMLElement>("#personal-connected-url");
const personalConnectedMetaEl = document.querySelector<HTMLElement>("#personal-connected-meta");
const personalAgentsFootnoteEl = document.querySelector<HTMLElement>("#personal-agents-footnote");
const personalListEl = document.querySelector<HTMLUListElement>("#personal-list")!;
const personalListHintEl = document.querySelector<HTMLElement>("#personal-list-hint")!;
const personalFormEl = document.querySelector<HTMLFormElement>("#personal-form")!;
const personalFormTitleEl = document.querySelector<HTMLElement>("#personal-form-title")!;
const personalIdEl = document.querySelector<HTMLInputElement>("#personal-id")!;
const personalPresetEl = document.querySelector<HTMLSelectElement>("#personal-preset")!;
const personalPresetPickerEl = document.querySelector<HTMLElement>("#personal-preset-picker");
const personalProtocolEl = document.querySelector<HTMLSelectElement>("#personal-protocol")!;
const personalNameRowEl = document.querySelector<HTMLElement>("#personal-name-row")!;
const personalNameEl = document.querySelector<HTMLInputElement>("#personal-name")!;
const personalUrlEl = document.querySelector<HTMLInputElement>("#personal-url")!;
const personalKeyEl = document.querySelector<HTMLInputElement>("#personal-key")!;
const personalModelEl = document.querySelector<HTMLInputElement>("#personal-model")!;
const personalModelSelectEl = document.querySelector<HTMLSelectElement>("#personal-model-select");
const personalModelSuggestionsEl = document.querySelector<HTMLDataListElement>(
  "#personal-model-suggestions",
)!;
const personalPresetUrlEl = document.querySelector<HTMLElement>("#personal-preset-url");
const personalAdvancedEl = document.querySelector<HTMLDetailsElement>("#personal-advanced");
const personalAddEl = document.querySelector<HTMLButtonElement>("#personal-add")!;
const personalBackEl = document.querySelector<HTMLButtonElement>("#personal-back")!;
const personalVerifyEl = document.querySelector<HTMLButtonElement>("#personal-verify")!;
const personalSaveEl = document.querySelector<HTMLButtonElement>("#personal-save")!;
const personalApplyEl = document.querySelector<HTMLButtonElement>("#personal-apply")!;
const personalHintEl = document.querySelector<HTMLElement>("#personal-hint")!;

/** Always write Browser MCP when applying provider — no separate toggle. */
function wantsBrowserMcp(): boolean {
  return true;
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
  // Pathway toggle removed — Browser MCP always writes with provider apply.
  if (!wiringModeFootnoteEl.classList.contains("is-busy")) {
    wiringModeFootnoteEl.textContent = "";
    wiringModeFootnoteEl.removeAttribute("title");
  }
}
const PROVIDER_PRESETS: Record<
  string,
  { name: string; url: string; protocol: ProviderProtocol; models: string[]; chip?: string }
> = {
  deepseek: {
    name: "DeepSeek",
    url: "https://api.deepseek.com/v1",
    protocol: "openai",
    models: modelsForPresetId("deepseek"),
    chip: "DeepSeek",
  },
  qwen: {
    name: "Qwen",
    url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    protocol: "openai",
    models: modelsForPresetId("qwen"),
    chip: "Qwen",
  },
  glm: {
    name: "GLM",
    url: "https://open.bigmodel.cn/api/paas/v4",
    protocol: "openai",
    models: modelsForPresetId("glm"),
    chip: "GLM",
  },
  minimax: {
    name: "MiniMax",
    url: "https://api.minimaxi.com/v1",
    protocol: "openai",
    models: modelsForPresetId("minimax"),
    chip: "MiniMax",
  },
  moonshot: {
    name: "Moonshot / Kimi",
    url: "https://api.moonshot.cn/v1",
    protocol: "openai",
    models: modelsForPresetId("moonshot"),
    chip: "Kimi",
  },
  openai: {
    name: "ChatGPT / OpenAI",
    url: "https://api.openai.com/v1",
    protocol: "openai",
    models: modelsForPresetId("openai"),
    chip: "ChatGPT",
  },
  anthropic: {
    name: "Claude",
    url: "https://api.anthropic.com",
    protocol: "anthropic",
    models: modelsForPresetId("anthropic"),
    chip: "Claude",
  },
  gemini: {
    name: "Gemini",
    url: "https://generativelanguage.googleapis.com/v1beta/openai/",
    protocol: "openai",
    models: modelsForPresetId("gemini"),
    chip: "Gemini",
  },
  siliconflow: {
    name: "SiliconFlow",
    url: "https://api.siliconflow.cn/v1",
    protocol: "openai",
    models: modelsForPresetId("siliconflow"),
    chip: "SiliconFlow",
  },
  openrouter: {
    name: "OpenRouter",
    url: "https://openrouter.ai/api/v1",
    protocol: "openai",
    models: modelsForPresetId("openrouter"),
    chip: "OpenRouter",
  },
  groq: {
    name: "Groq",
    url: "https://api.groq.com/openai/v1",
    protocol: "openai",
    models: modelsForPresetId("groq"),
    chip: "Groq",
  },
};

const PRESET_PICKER_GROUPS: Array<{ labelKey: MessageKey; ids: string[] }> = [
  {
    labelKey: "personal.groupPopular",
    ids: ["deepseek", "qwen", "glm", "minimax", "moonshot"],
  },
  {
    labelKey: "personal.groupGlobal",
    ids: ["openai", "anthropic", "gemini"],
  },
  {
    labelKey: "personal.groupHub",
    ids: ["siliconflow", "openrouter", "groq"],
  },
];

function chipLabel(presetId: string): string {
  if (presetId === "custom") {
    return t("personal.presetCustom");
  }
  return PROVIDER_PRESETS[presetId]?.chip ?? PROVIDER_PRESETS[presetId]?.name ?? presetId;
}

function syncPresetPicker(activeId: string) {
  if (!personalPresetPickerEl) return;
  personalPresetPickerEl.querySelectorAll<HTMLButtonElement>(".provider-chip").forEach((chip) => {
    const selected = chip.dataset.presetId === activeId;
    chip.classList.toggle("is-active", selected);
    chip.setAttribute("aria-selected", selected ? "true" : "false");
  });
}

function renderPresetPicker() {
  if (!personalPresetPickerEl) return;
  personalPresetPickerEl.innerHTML = "";

  for (const group of PRESET_PICKER_GROUPS) {
    const groupEl = document.createElement("div");
    groupEl.className = "provider-picker-group";

    const labelEl = document.createElement("div");
    labelEl.className = "provider-picker-label";
    labelEl.dataset.i18nLabel = group.labelKey;
    labelEl.textContent = t(group.labelKey);
    groupEl.appendChild(labelEl);

    const chipsEl = document.createElement("div");
    chipsEl.className = "provider-picker-chips";
    for (const id of group.ids) {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "provider-chip";
      chip.dataset.presetId = id;
      chip.setAttribute("role", "option");
      chip.textContent = chipLabel(id);
      chipsEl.appendChild(chip);
    }
    groupEl.appendChild(chipsEl);
    personalPresetPickerEl.appendChild(groupEl);
  }

  const customGroup = document.createElement("div");
  customGroup.className = "provider-picker-group";
  const customChips = document.createElement("div");
  customChips.className = "provider-picker-chips";
  const customChip = document.createElement("button");
  customChip.type = "button";
  customChip.className = "provider-chip is-custom";
  customChip.dataset.presetId = "custom";
  customChip.setAttribute("role", "option");
  customChip.textContent = chipLabel("custom");
  customChips.appendChild(customChip);
  customGroup.appendChild(customChips);
  personalPresetPickerEl.appendChild(customGroup);

  syncPresetPicker(personalPresetEl.value || "deepseek");
}

function refreshPresetGroupLabels() {
  if (personalPresetPickerEl) {
    personalPresetPickerEl.querySelectorAll<HTMLElement>("[data-i18n-label]").forEach((el) => {
      const key = el.dataset.i18nLabel;
      if (
        key === "personal.groupPopular" ||
        key === "personal.groupGlobal" ||
        key === "personal.groupHub"
      ) {
        el.textContent = t(key);
      }
    });
    const custom = personalPresetPickerEl.querySelector<HTMLElement>(
      '.provider-chip[data-preset-id="custom"]',
    );
    if (custom) {
      custom.textContent = t("personal.presetCustom");
    }
  }
  personalPresetEl.querySelectorAll("optgroup").forEach((group) => {
    const key = group.getAttribute("data-i18n-label");
    if (key === "personal.groupOpenAI" || key === "personal.groupClaude") {
      group.label = t(key);
    }
  });
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

function setModelSuggestions(models: string[]) {
  personalModelSuggestionsEl.innerHTML = "";
  for (const model of models) {
    const option = document.createElement("option");
    option.value = model;
    personalModelSuggestionsEl.appendChild(option);
  }
  if (personalModelSelectEl) {
    const current = personalModelEl.value.trim();
    personalModelSelectEl.innerHTML = "";
    for (const model of models) {
      const option = document.createElement("option");
      option.value = model;
      option.textContent = model;
      if (model === current) {
        option.selected = true;
      }
      personalModelSelectEl.appendChild(option);
    }
    if (models.length && !models.includes(current)) {
      personalModelSelectEl.value = models[0]!;
      personalModelEl.value = models[0]!;
    } else if (current && models.includes(current)) {
      personalModelSelectEl.value = current;
    }
  }
}

function syncModelFromSelect() {
  if (personalModelSelectEl && !personalModelSelectEl.hidden) {
    personalModelEl.value = personalModelSelectEl.value;
  }
}

function setPresetFormMode(isCustom: boolean, presetUrl?: string) {
  if (personalAdvancedEl) {
    // Presets: hide entirely — URL/protocol auto-routed in background.
    // Custom: keep for power users, but collapse by default.
    personalAdvancedEl.hidden = !isCustom;
    personalAdvancedEl.open = false;
  }
  if (personalModelSelectEl && personalModelEl) {
    personalModelSelectEl.hidden = isCustom;
    personalModelEl.hidden = !isCustom;
  }
  if (personalPresetUrlEl) {
    // Don't surface raw endpoint to beginners; presets already wire it.
    personalPresetUrlEl.hidden = true;
    personalPresetUrlEl.textContent = "";
    void presetUrl;
  }
  if (personalSaveEl) {
    personalSaveEl.hidden = !isCustom;
  }
}

function applyProviderPreset(presetId: string, { forceModel = true } = {}) {
  if (presetId === "custom" || !PROVIDER_PRESETS[presetId]) {
    personalPresetEl.value = "custom";
    syncPresetPicker("custom");
    personalNameRowEl.classList.remove("is-preset-locked");
    personalNameEl.readOnly = false;
    setModelSuggestions(
      personalProtocolEl.value === "anthropic"
        ? ["claude-sonnet-4-5", "claude-opus-4-5", "deepseek-v4-flash"]
        : ["deepseek-v4-flash", "deepseek-v4-pro", "gpt-4.1-mini"],
    );
    setPresetFormMode(true);
    return;
  }
  const preset = PROVIDER_PRESETS[presetId];
  personalPresetEl.value = presetId;
  syncPresetPicker(presetId);
  personalProtocolEl.value = preset.protocol;
  personalNameEl.value = preset.name;
  personalUrlEl.value = preset.url;
  setModelSuggestions(preset.models);
  if (forceModel || !personalModelEl.value.trim()) {
    personalModelEl.value = preset.models[0] ?? "";
    if (personalModelSelectEl) {
      personalModelSelectEl.value = preset.models[0] ?? "";
    }
  }
  personalNameRowEl.classList.add("is-preset-locked");
  personalNameEl.readOnly = true;
  setPresetFormMode(false, preset.url);
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
let lastEvotownStatus: EvotownStatus | null = null;

async function loadEvotownStatus() {
  try {
    const status = await invoke<EvotownStatus>("get_evotown_status_command");
    renderEvotownStatus(status);
  } catch (error) {
    evotownStatusEl.textContent = t("evotown.connectFailed", { error: String(error) });
  }
}

function renderEvotownStatus(status: EvotownStatus, opts?: { refreshSkills?: boolean }) {
  lastEvotownStatus = status;
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
    if (opts?.refreshSkills !== false) {
      void loadEngineRegisterStatus();
      void deps.loadSkillsInventory();
    }
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
  // Status is shown on the active list row — keep this block hidden.
  personalConnectedEl.hidden = true;
  personalStatusEl.textContent = status.configured
    ? t("personal.configured")
    : t("personal.notConfigured");
  if (personalConnectedUrlEl) personalConnectedUrlEl.textContent = "";
  if (personalConnectedMetaEl) personalConnectedMetaEl.textContent = "";
}

const RUNTIME_FOOTNOTE_LABELS: Record<string, string> = {
  hermes: "Hermes",
  openclaw: "OpenClaw",
  "claude-code": "Claude",
  codex: "Codex",
  "deepseek-harness": "DeepSeek",
};

function updatePersonalAgentsFootnote(): void {
  if (!personalAgentsFootnoteEl) return;
  const report = appState.lastReport;
  if (!report) {
    personalAgentsFootnoteEl.textContent = "";
    return;
  }
  const installed = report.runtimes
    .filter((runtime) => runtime.installed)
    .map((runtime) => RUNTIME_FOOTNOTE_LABELS[runtime.id] ?? runtime.display_name)
    .filter(Boolean);
  personalAgentsFootnoteEl.textContent =
    installed.length > 0
      ? t("personal.agentsOk", { list: installed.join("、") })
      : t("personal.agentsNone");
}

function renderPersonalProviderList(doc: PersonalProvidersDocument) {
  personalListEl.innerHTML = "";
  updatePersonalAgentsFootnote();
  if (doc.providers.length === 0) {
    const empty = document.createElement("li");
    empty.className = "provider-item provider-item-empty";
    empty.innerHTML = `
      <div class="provider-item-main">
        <p class="provider-item-kicker">${escapeHtml(t("personal.preset"))}</p>
        <p class="provider-item-title">${escapeHtml(t("personal.emptyTitle"))}</p>
        <p class="provider-item-desc">${escapeHtml(t("personal.emptyList"))}</p>
      </div>
    `;
    personalListEl.appendChild(empty);
    return;
  }

  const personalModeActive =
    isPersonalEdition() || appState.lastModeStatus?.mode === "personal";
  for (const item of doc.providers) {
    const routingActive = item.active && personalModeActive;
    const presetId = matchPresetId(item.name, item.url, item.protocol);
    const brand =
      presetId !== "custom"
        ? PROVIDER_PRESETS[presetId]?.chip ?? PROVIDER_PRESETS[presetId]?.name ?? item.name
        : item.name.trim() || t("personal.presetCustom");
    const titleText = item.name.trim() || brand;

    const li = document.createElement("li");
    li.className = `provider-item${routingActive ? " is-active" : ""}`;
    li.dataset.providerId = item.id;

    const main = document.createElement("div");
    main.className = "provider-item-main";

    const kicker = document.createElement("p");
    kicker.className = "provider-item-kicker";
    kicker.textContent = brand;

    const title = document.createElement("p");
    title.className = "provider-item-title";
    title.textContent = titleText;
    if (routingActive) {
      const badge = document.createElement("span");
      badge.className = "provider-badge";
      badge.textContent = t("personal.activeBadge");
      title.appendChild(badge);
    }

    const meta = document.createElement("p");
    meta.className = "provider-item-meta";
    meta.textContent = item.model;

    const showUse = !item.active;
    const desc = document.createElement("p");
    desc.className = "provider-item-desc";
    desc.textContent = showUse ? t("personal.itemIdleDesc") : t("personal.itemActiveDesc");

    main.append(kicker, title, meta, desc);

    const actions = document.createElement("div");
    actions.className = "provider-item-actions";

    if (!item.active) {
      const activateBtn = document.createElement("button");
      activateBtn.type = "button";
      activateBtn.className = "btn-primary btn-compact";
      activateBtn.dataset.action = "activate-provider";
      activateBtn.dataset.providerId = item.id;
      activateBtn.textContent = t("personal.activate");
      actions.appendChild(activateBtn);
    }

    const editBtn = document.createElement("button");
    editBtn.type = "button";
    editBtn.className = "btn-secondary btn-compact";
    editBtn.dataset.action = "edit-provider";
    editBtn.dataset.providerId = item.id;
    editBtn.textContent = t("personal.edit");
    actions.appendChild(editBtn);

    const deleteBtn = document.createElement("button");
    deleteBtn.type = "button";
    deleteBtn.className = "btn-ghost btn-compact";
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
  applyProviderPreset("deepseek");
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
    personalModelEl.value = item.model;
  } else {
    applyProviderPreset(presetId, { forceModel: false });
    personalNameEl.value = item.name;
    personalUrlEl.value = item.url;
    personalModelEl.value = item.model;
    if (personalModelSelectEl) {
      if (![...personalModelSelectEl.options].some((o) => o.value === item.model)) {
        const option = document.createElement("option");
        option.value = item.model;
        option.textContent = item.model;
        personalModelSelectEl.appendChild(option);
      }
      personalModelSelectEl.value = item.model;
    }
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
  syncModelFromSelect();
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
      personalListHintEl.textContent = t("personal.applyOk", {
        name: report.provider_name ?? values.name,
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
    personalListHintEl.textContent = t("personal.applyOk", {
      name: report.provider_name ?? id,
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

  renderPresetPicker();

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

  personalModelSelectEl?.addEventListener("change", () => {
    syncModelFromSelect();
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

  personalPresetPickerEl?.addEventListener("click", (event) => {
    const target = event.target as HTMLElement | null;
    const chip = target?.closest<HTMLButtonElement>(".provider-chip");
    if (!chip?.dataset.presetId) return;
    const presetId = chip.dataset.presetId;
    applyProviderPreset(presetId, { forceModel: true });
    if (presetId === "custom") {
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
      updateFooterCopy(isTeamEdition() ? "team" : "personal");
      updateWiringModeFootnote(appState.lastModeStatus?.mode);
      if (appState.lastModeStatus) {
        renderModeStatus(appState.lastModeStatus);
      }
      if (lastEvotownStatus) {
        renderEvotownStatus(lastEvotownStatus, { refreshSkills: false });
      }
      if (appState.personalProvidersDoc) {
        renderPersonalProviderList(appState.personalProvidersDoc);
      }
    },
  };
}
