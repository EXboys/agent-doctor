import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
const providerTabsEl = document.querySelector<HTMLElement>("#provider-tabs")!;
const mainPanels = Array.from(document.querySelectorAll<HTMLElement>("[data-main-panel]"));
const providerPanels = Array.from(document.querySelectorAll<HTMLElement>("[data-provider-panel]"));

type MainTabId = "diagnose" | "provider" | "workspace";
type ProviderTabId = "personal" | "evotown";

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
}

function setProviderTab(tab: ProviderTabId) {
  providerTabsEl.querySelectorAll<HTMLButtonElement>("[data-provider-tab]").forEach((button) => {
    const active = button.dataset.providerTab === tab;
    button.classList.toggle("is-active", active);
    button.setAttribute("aria-selected", active ? "true" : "false");
  });
  for (const panel of providerPanels) {
    const active = panel.dataset.providerPanel === tab;
    panel.classList.toggle("is-active", active);
    panel.hidden = !active;
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
const workspaceApplyEl = document.querySelector<HTMLButtonElement>("#workspace-apply")!;
const workspaceDoctorEl = document.querySelector<HTMLButtonElement>("#workspace-doctor")!;
const workspaceFixEl = document.querySelector<HTMLButtonElement>("#workspace-fix")!;
const workspaceChecksEl = document.querySelector<HTMLUListElement>("#workspace-checks")!;
const workspaceHintEl = document.querySelector<HTMLElement>("#workspace-hint")!;
const workspacePickerEl = document.querySelector<HTMLElement>("#workspace-picker")!;
const workspaceTriggerEl = document.querySelector<HTMLButtonElement>("#workspace-trigger")!;
const workspaceTriggerLabelEl = document.querySelector<HTMLElement>("#workspace-trigger-label")!;
const workspaceMenuEl = document.querySelector<HTMLElement>("#workspace-menu")!;
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
let workspaceMenuOpen = false;

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

function runtimeInitials(id: string, displayName: string): string {
  return RUNTIME_SHORT[id] ?? displayName.slice(0, 2).toUpperCase();
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

function renderHermesCard(runtime: RuntimeDoctorResult): string {
  const model = hermesModel ?? {
    provider: "",
    model: "",
    base_url: runtime.profile.gateway_url ?? "",
    api_key_env: null,
    api_key_configured: false,
    api_key_hint: null,
  };

  const editButton = hermesEditing
    ? ""
    : `<button type="button" class="btn-ghost" data-action="edit-hermes">${t("runtime.edit")}</button>`;
  const diagnoseButton = `<button type="button" class="btn-ghost" data-action="diagnose-runtime">${t("runtime.diagnose")}</button>`;
  const openButton =
    runtime.installed && canOpenSession(runtime.id)
      ? `<button type="button" class="btn-ghost" data-action="open-session">${t("runtime.open")}</button>`
      : "";
  const installButton = !runtime.installed
    ? `<button type="button" class="btn-ghost" data-action="install-runtime">${t("runtime.install")}</button>`
    : "";

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
    : "";

  return `
    <article class="runtime hermes ${hermesEditing ? "is-editing" : ""}" data-runtime="hermes">
      <div class="runtime-head runtime-head-compact">
        <p class="runtime-tab-title">${runtime.display_name}</p>
        <div class="runtime-actions">
          ${openButton}
          ${installButton}
          ${diagnoseButton}
          ${editButton}
          <p class="badge ok">${t("runtime.installed")}</p>
        </div>
      </div>
      ${meta ? `<div class="meta-grid">${meta}</div>` : ""}
      <div class="card-hint repair-hint" data-repair-hint hidden></div>
      ${actions}
    </article>
  `;
}

function renderRuntimeCard(runtime: RuntimeDoctorResult): string {
  if (runtime.id === "hermes" && runtime.installed) {
    return renderHermesCard(runtime);
  }

  const state = runtime.installed ? t("runtime.installed") : t("runtime.notInstalled");
  const badgeClass = runtime.installed ? "ok" : "muted";
  const openButton =
    runtime.installed && canOpenSession(runtime.id)
      ? `<button type="button" class="btn-ghost" data-action="open-session">${t("runtime.open")}</button>`
      : "";
  const installButton = !runtime.installed
    ? `<button type="button" class="btn-ghost" data-action="install-runtime">${t("runtime.install")}</button>`
    : "";
  const rows = [
    runtime.version ? metaRow("meta.version", runtime.version) : "",
    runtime.binary_path ? metaRow("meta.binary", runtime.binary_path) : "",
    runtime.config_paths.length ? metaRow("meta.config", runtime.config_paths.join("\n")) : "",
    runtime.profile.gateway_url ? metaRow("meta.gateway", runtime.profile.gateway_url) : "",
  ]
    .filter(Boolean)
    .join("");

  return `
    <article class="runtime ${runtimeClass(runtime.id)}" data-runtime="${runtime.id}">
      <div class="runtime-head runtime-head-compact">
        <p class="runtime-tab-title">${runtime.display_name}</p>
        <div class="runtime-actions">
          ${openButton}
          ${installButton}
          <button type="button" class="btn-ghost" data-action="diagnose-runtime">${t("runtime.diagnose")}</button>
          <p class="badge ${badgeClass}">${state}</p>
        </div>
      </div>
      ${rows ? `<div class="meta-grid">${rows}</div>` : ""}
      <div class="card-hint repair-hint" data-repair-hint hidden></div>
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

function renderRuntimeTabs(runtimes: RuntimeDoctorResult[], selectedId: string): string {
  return runtimes
    .map((runtime) => {
      const active = runtime.id === selectedId;
      const dotClass = runtime.installed ? "ok" : "muted";
      return `
        <button
          type="button"
          class="runtime-tab ${runtimeClass(runtime.id)} ${active ? "is-active" : ""}"
          role="tab"
          aria-selected="${active}"
          data-runtime-tab="${runtime.id}"
        >
          <span class="runtime-tab-icon">${runtimeInitials(runtime.id, runtime.display_name)}</span>
          <span class="runtime-tab-label">${escapeHtml(runtime.display_name)}</span>
          <span class="runtime-tab-dot ${dotClass}" aria-hidden="true"></span>
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

  if (names.length === 0) {
    presetStatusEl.textContent = t("presets.none");
    presetApplyEl.disabled = true;
    presetHintEl.textContent = t("presets.noneHint");
    renderPresetOptions([], null, doc.profiles);
    return;
  }

  presetStatusEl.textContent = doc.active
    ? t("presets.active", { name: doc.active })
    : t("presets.noActive");

  renderPresetOptions(names, doc.active, doc.profiles);
  presetApplyEl.disabled = false;
  presetHintEl.textContent = t("presets.switchHint");
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

  if (connected && status.base_url) {
    evotownStatusEl.textContent = t("evotown.connected");
    evotownConnectedUrlEl.textContent = status.base_url;
    evotownConnectedMetaEl.textContent = t("evotown.meta", {
      runtime: status.runtime_target ?? "openclaw",
      bundle: status.bundle_id ?? "default-agent-skills",
      key: status.api_key_hint ?? "evk_…",
    });
    evotownUrlEl.value = status.base_url;
    evotownResyncEl.hidden = false;
  } else {
    evotownStatusEl.textContent = t("evotown.notConfigured");
    evotownConnectedMetaEl.textContent = "";
    evotownResyncEl.hidden = true;
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

  for (const item of doc.providers) {
    const li = document.createElement("li");
    li.className = `provider-item${item.active ? " is-active" : ""}`;
    li.dataset.providerId = item.id;

    const main = document.createElement("div");
    main.className = "provider-item-main";
    const title = document.createElement("p");
    title.className = "provider-item-title";
    title.textContent = item.name;
    if (item.active) {
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

function setWorkspaceTriggerLabel(name: string | null) {
  workspaceTriggerLabelEl.textContent = name ?? t("workspaces.noActive");
}

function closeWorkspaceMenu() {
  workspaceMenuOpen = false;
  workspaceMenuEl.hidden = true;
  workspaceTriggerEl.setAttribute("aria-expanded", "false");
  workspacePickerEl.classList.remove("is-open");
}

function openWorkspaceMenu() {
  if (workspaceTriggerEl.disabled) {
    return;
  }
  workspaceMenuOpen = true;
  workspaceMenuEl.hidden = false;
  workspaceTriggerEl.setAttribute("aria-expanded", "true");
  workspacePickerEl.classList.add("is-open");
}

function toggleWorkspaceMenu() {
  if (workspaceMenuOpen) {
    closeWorkspaceMenu();
  } else {
    openWorkspaceMenu();
  }
}

function renderWorkspaceOptions(doc: WorkspacesDocument) {
  const names = Object.keys(doc.workspaces).sort();
  if (names.length === 0) {
    workspaceMenuEl.innerHTML = "";
    selectedWorkspaceName = "";
    setWorkspaceTriggerLabel(null);
    workspaceTriggerEl.disabled = true;
    closeWorkspaceMenu();
    return;
  }

  selectedWorkspaceName =
    selectedWorkspaceName && names.includes(selectedWorkspaceName)
      ? selectedWorkspaceName
      : (doc.active ?? names[0]);
  setWorkspaceTriggerLabel(selectedWorkspaceName);
  workspaceTriggerEl.disabled = false;

  workspaceMenuEl.innerHTML = names
    .map((name) => {
      const activeOption = name === selectedWorkspaceName;
      const entry = doc.workspaces[name];
      const meta = entry?.path ?? "";
      return `
        <button
          type="button"
          class="picker-option ${activeOption ? "is-active" : ""}"
          role="option"
          aria-selected="${activeOption}"
          data-workspace="${escapeHtml(name)}"
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

function renderWorkspaces(doc: WorkspacesDocument) {
  lastWorkspaces = doc;
  const names = Object.keys(doc.workspaces);

  if (names.length === 0) {
    workspaceStatusEl.textContent = t("workspaces.none");
    workspaceApplyEl.disabled = true;
    workspaceHintEl.textContent = t("workspaces.noneHint");
    renderWorkspaceOptions({ active: null, workspaces: {} });
    return;
  }

  workspaceStatusEl.textContent = doc.active
    ? t("workspaces.active", { name: doc.active })
    : t("workspaces.noActive");
  renderWorkspaceOptions(doc);
  workspaceApplyEl.disabled = false;
  workspaceHintEl.textContent = t("workspaces.switchHint");
}

async function loadWorkspaces() {
  try {
    const doc = await invoke<WorkspacesDocument>("list_workspaces_command");
    renderWorkspaces(doc);
  } catch (error) {
    workspaceStatusEl.textContent = t("workspaces.failed");
    workspaceHintEl.textContent = String(error);
    workspaceApplyEl.disabled = true;
  }
}

async function applyWorkspace() {
  const name = selectedWorkspaceName;
  if (!name) {
    return;
  }

  closeWorkspaceMenu();
  workspaceApplyEl.disabled = true;
  workspaceHintEl.textContent = t("workspaces.applying", { name });
  try {
    await invoke("use_workspace_command", { name });
    workspaceHintEl.textContent = t("workspaces.updated", { name });
    await loadWorkspaces();
  } catch (error) {
    workspaceHintEl.textContent = String(error);
  } finally {
    workspaceApplyEl.disabled = false;
  }
}

async function doctorWorkspace() {
  workspaceDoctorEl.disabled = true;
  workspaceHintEl.textContent = t("workspaces.doctorRunning");
  try {
    const report = await invoke<WorkspaceDoctorReport>("workspace_doctor_command");
    renderWorkspaceChecks(report);
  } catch (error) {
    workspaceChecksEl.hidden = true;
    workspaceChecksEl.innerHTML = "";
    workspaceHintEl.textContent = String(error);
  } finally {
    workspaceDoctorEl.disabled = false;
  }
}

async function fixWorkspace() {
  workspaceFixEl.disabled = true;
  workspaceDoctorEl.disabled = true;
  workspaceHintEl.textContent = t("workspaces.fixRunning");
  try {
    const report = await invoke<WorkspaceFixReport>("workspace_fix_command", {
      migrateClaudeMcp: false,
    });
    const applied = report.actions.filter((action) => action.applied).length;
    workspaceHintEl.textContent = t("workspaces.fixSummary", { count: String(applied) });
    await doctorWorkspace();
  } catch (error) {
    workspaceHintEl.textContent = String(error);
  } finally {
    workspaceFixEl.disabled = false;
    workspaceDoctorEl.disabled = false;
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
  if (lastReport) {
    await renderReport(lastReport);
  } else {
    setStatusBanner("neutral", t("doctor.loading"));
    presetStatusEl.textContent = t("presets.loading");
    healthLabelEl.textContent = t("health.ready");
  }
  await loadEvotownStatus();
  await loadPersonalProviderStatus();
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
  if (tab === "diagnose" || tab === "provider" || tab === "workspace") {
    setMainTab(tab);
  }
});

providerTabsEl.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-provider-tab]");
  const tab = button?.dataset.providerTab;
  if (tab === "personal" || tab === "evotown") {
    setProviderTab(tab);
  }
});

evotownFormEl.addEventListener("submit", (event) => {
  event.preventDefault();
  void runEvotownOnboarding();
});

evotownResyncEl.addEventListener("click", () => {
  void resyncEvotownSkills();
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
  // Changing protocol manually marks the form as custom unless a matching preset stays valid.
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
  // If user edits URL away from the selected preset, flip to Custom but keep name.
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

workspaceApplyEl.addEventListener("click", () => {
  void applyWorkspace();
});

workspaceDoctorEl.addEventListener("click", () => {
  void doctorWorkspace();
});

workspaceFixEl.addEventListener("click", () => {
  void fixWorkspace();
});

workspaceTriggerEl.addEventListener("click", () => {
  toggleWorkspaceMenu();
});

workspaceMenuEl.addEventListener("click", (event) => {
  const option = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-workspace]");
  const name = option?.dataset.workspace;
  if (!name || !lastWorkspaces) {
    return;
  }
  selectedWorkspaceName = name;
  renderWorkspaceOptions(lastWorkspaces);
  closeWorkspaceMenu();
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
  if (workspaceMenuOpen && !workspacePickerEl.contains(target)) {
    closeWorkspaceMenu();
  }
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    closePresetMenu();
    closeWorkspaceMenu();
  }
});

void listen<DoctorReport>("doctor-report", (event) => {
  void renderReport(event.payload);
});

void listen("workspace-changed", () => {
  void loadWorkspaces();
});

void listen<WorkspaceDoctorReport>("workspace-doctor-report", (event) => {
  renderWorkspaceChecks(event.payload);
});

setLocale(getLocale());
applyStaticI18n();
updateLangButtons();
refreshPresetGroupLabels();
applyProviderPreset("custom");
showPersonalListView();
void loadProfiles();
void loadWorkspaces();
void loadEvotownStatus();
void loadPersonalProviderStatus();
void refresh();
