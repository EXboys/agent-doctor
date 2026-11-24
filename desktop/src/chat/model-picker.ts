import { listen } from "@tauri-apps/api/event";
import type { AskRuntime } from "../ask-resources";
import { appState, applyNativeAccounts } from "../app-state";
import { isPersonalEdition } from "../edition";
import { t } from "../i18n";
import {
  activateClusterModel,
  getClusterStatus,
  getPersonalProviderStatus,
  listPersonalProviders,
  nativeAccountStatus,
  openBrowserUrl,
  refreshSubscriptionModels,
  resignNativeSubscription,
  signOutNativeSubscription,
  teamupsAccountStatus,
  useNativeSubscription,
} from "../ipc";
import {
  builtinQuotaLabel,
  formatOfficialTokens,
  formatResetDate,
  OFFICIAL_GIFT_MODEL,
  OFFICIAL_PROVIDER_ID,
  officialLineUsable,
  officialQuotaState,
} from "../official-quota";
import { modelOptionLabel, modelsForProviderUrl, providerChipForUrl } from "../provider-models";
import { teamupsAccountPageUrl } from "../teamups-site";
import type {
  ClusterStatus,
  NativeAccountRow,
  NativeAccountSwitch,
  PersonalProviderListItem,
  TeamupsAccountStatus,
} from "../types";
import { runtimeDisplayName } from "./runtime";
import type { ChatSession } from "./types";

export const NATIVE_PROVIDER_PREFIX = "native:";
export const NATIVE_ACCOUNT_DEFAULT_MODEL = "auto";
const LOCAL_CLUSTER_PROVIDER_ID = "local-cluster";

/** Running models first, then weights already on disk. */
export function localClusterModelIds(status: ClusterStatus | null): string[] {
  if (!status) return [];
  const out: string[] = [];
  const seen = new Set<string>();
  for (const id of [...(status.active_models ?? []), ...(status.installed_models ?? [])]) {
    const trimmed = id.trim();
    if (!trimmed || seen.has(trimmed)) continue;
    seen.add(trimmed);
    out.push(trimmed);
  }
  return out;
}

/** Local models join the list the chat already had. They never replace it. */
export function providersWithLocalCluster<T extends { id: string }>(
  saved: readonly T[],
  local: T | null,
): T[] {
  if (!local) return [...saved];
  return [...saved.filter((item) => item.id !== local.id), local];
}

function localClusterProvider(
  status: ClusterStatus,
  model: string,
): PersonalProviderListItem {
  const origin = (status.dashboard_url || "http://127.0.0.1:52415").replace(/\/$/, "");
  return {
    id: LOCAL_CLUSTER_PROVIDER_ID,
    name: t("chat.modelLocalGroup"),
    url: `${origin}/v1`,
    model,
    protocol: "openai",
    api_key_hint: "",
    active: Boolean(status.provider_active),
  };
}

export function nativeProviderId(runtimeId: string): string {
  return `${NATIVE_PROVIDER_PREFIX}${runtimeId.trim()}`;
}

export function isNativeProviderId(providerId: string | null | undefined): boolean {
  return Boolean(providerId?.trim().startsWith(NATIVE_PROVIDER_PREFIX));
}

export function accountSupportsModelList(runtimeId: string): boolean {
  const id = runtimeId.trim();
  return (
    id === "codex" ||
    id === "claude-code" ||
    id === "qoder" ||
    id === "cursor" ||
    id === "workbuddy" ||
    id === "zcode"
  );
}

export function nativeAccountLabel(runtimeId: string): string {
  switch (runtimeId.trim()) {
    case "codex":
      return t("native.chatgpt");
    case "claude-code":
      return t("native.claude");
    case "qoder":
      return t("native.qoder");
    case "cursor":
      return t("native.cursor");
    case "workbuddy":
      return t("native.workbuddy");
    case "zcode":
      return t("native.zcode");
    case "antigravity":
      return t("native.antigravity");
    case "deepseek-harness":
      return t("native.deepseek");
    default:
      return runtimeId;
  }
}

/** Why a sign-in did not finish: the tool's own reason, not a generic hint. */
function loginFailureMessage(result: NativeAccountSwitch): string {
  if (result.outcome === "unreachable") return t("native.unreachable");
  if (result.outcome === "missing_tool") return missingToolMessage(result.runtime_id);
  const detail = result.detail?.trim();
  if (detail) return t("native.incompleteDetail", { detail });
  return t("chat.modelSyncDesktopIncomplete");
}

function missingToolMessage(runtimeId: string): string {
  switch (runtimeId.trim()) {
    case "codex":
      return t("native.missingCodex");
    case "claude-code":
      return t("native.missingClaude");
    case "qoder":
      return t("native.missingQoder");
    case "cursor":
      return t("native.missingCursor");
    case "workbuddy":
      return t("native.missingWorkbuddy");
    case "zcode":
      return t("native.missingZcode");
    case "antigravity":
      return t("native.missingAntigravity");
    default:
      return t("native.missingClaude");
  }
}

export type LoggedAccountCatalog = {
  runtimeId: string;
  providerId: string;
  label: string;
  models: string[];
  /** Menu text aligned with `models`. Empty entry keeps the id. */
  modelLabels: string[];
  /** `auto`, `cursor`, or `other`, aligned with `models`. Empty keeps one list. */
  modelSections: string[];
  usingByDefault: boolean;
  /** True when the CLI itself is already authorized and ready to answer. */
  cliReady: boolean;
  /** True when the companion desktop IDE has an active login not yet synced to CLI. */
  desktopNeedsSync: boolean;
  /** Cursor Models used this period, 0–100. Empty when that bar is missing. */
  quotaCursorPercent: number | null;
  /** Other Models used this period, 0–100. Empty when that bar is missing. */
  quotaOtherPercent: number | null;
  /** Short ChatGPT window length, in hours. */
  quotaHours: number | null;
  /** Used percent of that short window, 0–100. */
  quotaHoursPercent: number | null;
  /** Used percent of the weekly window, 0–100. */
  quotaWeekPercent: number | null;
  /** Credits still left. Empty when the account did not report a balance. */
  quotaCreditsLeft: number | null;
};

/**
 * Return the account catalog for `runtime` ONLY when that runtime's CLI account
 * OR companion desktop IDE is currently signed in locally.
 */
export function loggedAccountCatalogFor(
  runtime: string,
  nativeRows: ReadonlyArray<NativeAccountRow>,
): LoggedAccountCatalog | null {
  const row = nativeRows.find((item) => item.runtime_id === runtime);
  const cliReady = Boolean(row?.logged_in);
  const desktopLogged = Boolean(row?.desktop_logged_in);
  if (!cliReady && !desktopLogged) return null;
  return {
    runtimeId: runtime,
    providerId: nativeProviderId(runtime),
    label: nativeAccountLabel(runtime),
    models: cliReady ? row?.models ?? [] : [],
    modelLabels: cliReady ? row?.model_labels ?? [] : [],
    modelSections: cliReady ? row?.model_sections ?? [] : [],
    usingByDefault: cliReady && Boolean(row?.using_subscription),
    cliReady,
    desktopNeedsSync: Boolean(row?.desktop_needs_sync || (!cliReady && desktopLogged)),
    quotaCursorPercent: shownPercent(cliReady ? row?.quota_cursor_percent : undefined),
    quotaOtherPercent: shownPercent(cliReady ? row?.quota_other_percent : undefined),
    quotaHours: shownHours(cliReady ? row?.quota_hours : undefined),
    quotaHoursPercent: shownPercent(cliReady ? row?.quota_hours_percent : undefined),
    quotaWeekPercent: shownPercent(cliReady ? row?.quota_week_percent : undefined),
    quotaCreditsLeft: shownCredits(cliReady ? row?.quota_credits_left : undefined),
  };
}

function shownHours(value: number | undefined): number | null {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) return null;
  return Math.round(value);
}

/** Account title plus how much of the shared allowance is left. */
export function accountAllowanceTitle(catalog: LoggedAccountCatalog): string {
  const parts: string[] = [];
  if (catalog.quotaHours != null && catalog.quotaHoursPercent != null) {
    parts.push(
      t("native.quotaHoursLeft", {
        hours: String(catalog.quotaHours),
        percent: String(100 - catalog.quotaHoursPercent),
      }),
    );
  }
  if (catalog.quotaWeekPercent != null) {
    parts.push(
      t("native.quotaWeekLeft", {
        percent: String(100 - catalog.quotaWeekPercent),
      }),
    );
  }
  if (catalog.quotaCreditsLeft != null) {
    parts.push(
      catalog.quotaCreditsLeft <= 0
        ? t("native.quotaCreditsNone")
        : t("native.quotaCreditsLeft", { left: formatCredits(catalog.quotaCreditsLeft) }),
    );
  }
  return parts.length === 0 ? catalog.label : `${catalog.label} · ${parts.join(" · ")}`;
}

function shownCredits(value: number | undefined): number | null {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) return null;
  return Math.round(value * 100) / 100;
}

function formatCredits(value: number): string {
  const rounded = Math.round(value * 100) / 100;
  if (Number.isInteger(rounded)) return String(rounded);
  return rounded.toFixed(2).replace(/0+$/, "").replace(/\.$/, "");
}

function shownPercent(value: number | undefined): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return Math.max(0, Math.min(100, Math.round(value)));
}

function accountSectionTitle(section: string, catalog: LoggedAccountCatalog): string {
  if (section === "cursor") {
    return catalog.quotaCursorPercent == null
      ? t("chat.modelCursorGroup")
      : t("native.quotaCursor", { percent: String(catalog.quotaCursorPercent) });
  }
  if (section === "other") {
    return catalog.runtimeId === "cursor" && catalog.quotaOtherPercent != null
      ? t("native.quotaOther", { percent: String(catalog.quotaOtherPercent) })
      : t("chat.modelOtherGroup");
  }
  switch (section) {
    case "qoder-power":
      return t("chat.modelQoderPower");
    case "qoder-value":
      return t("chat.modelQoderValue");
    case "qoder-lite":
      return t("chat.modelQoderLite");
    case "qoder-new":
      return t("chat.modelQoderNew");
    case "workbuddy-own":
      return t("chat.modelWorkbuddyOwn");
    case "codex-coding":
      return t("chat.modelCodexCoding");
    case "codex-general":
      return t("chat.modelCodexGeneral");
    default:
      return "";
  }
}

/**
 * Whether `provider` can be selected on `runtime` in the Ask model picker.
 * Un-logged-in official accounts never appear; Cursor and ZCode only accept
 * their own native sign-ins.
 */
export function runtimeAcceptsProvider(
  runtime: string,
  provider: { id: string; url: string },
  opts: { officialSignedIn: boolean; officialLineReady?: boolean },
): boolean {
  if (runtime === "cursor" || runtime === "zcode") return false;
  if (runtime === "antigravity") {
    if (provider.id === OFFICIAL_PROVIDER_ID) return false;
    const url = provider.url.toLowerCase();
    return (
      url.includes("generativelanguage.googleapis.com") ||
      url.includes("aiplatform.googleapis.com")
    );
  }
  if (provider.id === OFFICIAL_PROVIDER_ID) {
    if (!opts.officialSignedIn) return false;
    if (runtime === "deepseek-harness") return opts.officialLineReady !== false;
    return true;
  }
  if (runtime === "deepseek-harness") {
    return provider.url.toLowerCase().includes("api.deepseek.com");
  }
  return true;
}

export type ModelPickerEls = {
  modelBtnEl: HTMLButtonElement;
  modelLabelEl: HTMLElement;
  modelMenuEl: HTMLElement;
  modelWrapEl: HTMLElement | null;
  composerBoxEl: HTMLElement;
  composerEl: HTMLElement;
};

export type ModelPickerDeps = ModelPickerEls & {
  isComposerLocked: () => boolean;
  selectedRuntime: () => AskRuntime;
  getWiredProvider: () => PersonalProviderListItem | null;
  setWiredProvider: (provider: PersonalProviderListItem | null) => void;
  getProviders: () => PersonalProviderListItem[];
  setProviders: (providers: PersonalProviderListItem[]) => void;
  activeSession: () => ChatSession | null;
  pinSessionModel: (provider: PersonalProviderListItem, model: string) => void;
  pinSubscriptionModel: (model: string) => void;
  getModelMenuOpen: () => boolean;
  setModelMenuOpen: (open: boolean) => void;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  updateContextMeter: () => void;
};

export type ModelPickerApi = ReturnType<typeof createModelPickerController>;

/** Account-model alignment for one chat. `replace` means the login may take over. */
export function subscriptionModelForSession(
  session: ChatSession | null | undefined,
  catalogModels: readonly string[],
): { model: string; replace: boolean } {
  if (sessionKeepsSavedService(session)) {
    return { model: session?.model?.trim() || "", replace: false };
  }
  if (catalogModels.length === 0) {
    const raw = session?.model?.trim() ?? "";
    const keep = raw === NATIVE_ACCOUNT_DEFAULT_MODEL ? NATIVE_ACCOUNT_DEFAULT_MODEL : "";
    return { model: keep, replace: false };
  }
  const current = session?.model?.trim() ?? "";
  const model = catalogModels.includes(current) ? current : catalogModels[0];
  return { model, replace: Boolean(session && session.model !== model) };
}

/** This chat named a saved API-key service. Login must not replace it on its own. */
export function sessionKeepsSavedService(session: ChatSession | null | undefined): boolean {
  if (!session) return false;
  if (isNativeProviderId(session.providerId)) return false;
  if (session.providerId?.trim()) return true;
  const model = session.model?.trim().toLowerCase() ?? "";
  return ["glm", "deepseek", "qwen", "kimi", "moonshot", "minimax", "doubao", "gemini", "ernie"].some(
    (prefix) => model.startsWith(prefix),
  );
}

/** Whether `session` is currently routed to `catalog`'s signed-in account. */
export function sessionUsesLoggedAccount(
  session: ChatSession | null | undefined,
  catalog: LoggedAccountCatalog | null,
): boolean {
  if (!catalog || !catalog.cliReady) return false;
  const pinned = session?.providerId?.trim() ?? "";
  if (pinned === catalog.providerId) return true;
  if (pinned) return false;
  if (!catalog.usingByDefault) return false;
  if (catalog.runtimeId === "cursor" || catalog.runtimeId === "zcode") return true;
  const current = session?.model?.trim().toLowerCase() ?? "";
  if (current && catalog.models.some((id) => id.toLowerCase() === current)) return true;
  return !sessionKeepsSavedService(session);
}

/**
 * The main window activated another provider, and the ask window follows it.
 * Chats pinned to a different saved service would keep quietly asking the old
 * one, so release those pins; a pin on the newly active provider or on a
 * signed-in native account stays. Returns whether anything changed.
 */
export function releaseStalePins(
  sessions: Array<{ providerId?: string | null; model?: string | null; providerTag?: string | null }>,
  activeId: string | null | undefined,
): boolean {
  if (!activeId) return false;
  let touched = false;
  for (const session of sessions) {
    if (!session.providerId || session.providerId === activeId) continue;
    if (isNativeProviderId(session.providerId)) continue;
    session.providerId = null;
    session.model = "";
    session.providerTag = null;
    touched = true;
  }
  return touched;
}

export function providerHostTag(provider: { id: string; url: string }): string {
  let host = provider.url;
  try {
    host = new URL(provider.url).host;
  } catch {
    // keep the raw address
  }
  return `${provider.id}|${host}`;
}

/**
 * Stick this chat to the fallback provider that answered it, keeping its prompt
 * cache warm on later turns while leaving the global default and other chats alone.
 */
export function pinSessionAfterFailover(
  session: {
    providerId?: string | null;
    model?: string | null;
    providerTag?: string | null;
    runtimeThreadId?: string | null;
  },
  providers: ReadonlyArray<{ id: string; url: string }>,
  providerId: string,
  model: string,
): void {
  const matched = providers.find((item) => item.id === providerId);
  session.providerId = providerId;
  session.model = model;
  session.runtimeThreadId = null;
  session.providerTag = matched ? providerHostTag(matched) : null;
}

type MenuTab = {
  id: string;
  title: string;
  hasActive: boolean;
  render: (panel: HTMLElement) => void;
};

export function createModelPickerController(deps: ModelPickerDeps) {
  let nativeRows: NativeAccountRow[] = [];
  function rememberNativeRows(rows: NativeAccountRow[]): void {
    nativeRows = rows;
    const previous = appState.nativeAccounts;
    applyNativeAccounts(
      rows.map((row) => {
        const prior = previous.find((item) => item.runtime_id === row.runtime_id);
        if (!prior) return row;
        if (!row.logged_in && prior.logged_in) return row;
        return { ...prior, ...row };
      }),
      false,
    );
  }
  const modelListTriedAt = new Map<string, number>();
  const loadingAccountRuntimes = new Set<string>();
  const probedEmptyRuntimes = new Set<string>();
  const syncFeedbackByRuntime = new Map<string, string>();
  let syncingAccountRuntime: string | null = null;
  let selectedTabId = "";
  let officialStatus: TeamupsAccountStatus | null = null;
  let officialFetchedAt = 0;
  let localCluster: ClusterStatus | null = null;
  let loadingLocalModel = false;

  /** Throttled official-account fetch; drives the built-in agent's quota chip and official sign-in gate. */
  async function refreshOfficialStatus(): Promise<void> {
    if (Date.now() - officialFetchedAt < 15_000) return;
    officialFetchedAt = Date.now();
    try {
      officialStatus = await teamupsAccountStatus();
    } catch {
      officialStatus = null;
    }
  }

  function repaintAfterOfficialRefresh(): void {
    renderModelPickerLabel();
    if (deps.getModelMenuOpen()) renderModelMenu();
  }

  function hasSavedOfficialProvider(): boolean {
    return (
      deps.getProviders().some((item) => item.id === OFFICIAL_PROVIDER_ID) ||
      deps.getWiredProvider()?.id === OFFICIAL_PROVIDER_ID
    );
  }

  function officialSignedIn(): boolean {
    if (officialStatus) return officialQuotaState(officialStatus).signedIn;
    return hasSavedOfficialProvider();
  }

  function officialLineReady(): boolean {
    if (officialStatus?.official) return officialLineUsable(officialStatus);
    return officialSignedIn();
  }

  function loggedAccountCatalog(): LoggedAccountCatalog | null {
    return loggedAccountCatalogFor(deps.selectedRuntime(), nativeRows);
  }

  function compatibleSavedProviders(): PersonalProviderListItem[] {
    const runtime = deps.selectedRuntime();
    const providers = deps.getProviders();
    const wired = deps.getWiredProvider();
    const rows = providers.length > 0 ? providers : wired ? [wired] : [];
    const opts = {
      officialSignedIn: officialSignedIn(),
      officialLineReady: officialLineReady(),
    };
    return rows.filter((provider) => runtimeAcceptsProvider(runtime, provider, opts));
  }

  /** Keep this chat on an account model when it is already using the signed-in account. */
  function alignAccountSession(catalog: LoggedAccountCatalog): string {
    const session = deps.activeSession();
    if (catalog.models.length === 0) {
      return NATIVE_ACCOUNT_DEFAULT_MODEL;
    }
    const current = session?.model?.trim() ?? "";
    if (catalog.models.includes(current)) return current;
    const matchedIgnoreCase = catalog.models.find(
      (id) => id.toLowerCase() === current.toLowerCase(),
    );
    if (matchedIgnoreCase) return matchedIgnoreCase;
    const fallback = catalog.models[0];
    if (session && sessionUsesLoggedAccount(session, catalog) && !session.providerId) {
      deps.pinSubscriptionModel(fallback);
    }
    return fallback;
  }

  function closeModelMenu(): void {
    deps.setModelMenuOpen(false);
    selectedTabId = "";
    deps.modelMenuEl.hidden = true;
    deps.modelBtnEl.classList.remove("is-open");
    deps.modelBtnEl.setAttribute("aria-expanded", "false");
    deps.modelWrapEl?.classList.remove("is-open");
    deps.composerBoxEl.classList.remove("is-model-open");
    deps.composerEl.classList.remove("is-model-open");
    deps.modelMenuEl.style.left = "";
    deps.modelMenuEl.style.right = "";
    deps.modelMenuEl.style.top = "";
    deps.modelMenuEl.style.bottom = "";
    deps.modelMenuEl.style.width = "";
    deps.modelMenuEl.style.minWidth = "";
    deps.modelMenuEl.style.position = "";
    deps.modelMenuEl.style.zIndex = "";
  }

  function positionModelMenu(): void {
    const rect = deps.modelBtnEl.getBoundingClientRect();
    const gap = 8;
    const minWidth = Math.max(rect.width, 280);
    const maxWidth = Math.min(380, window.innerWidth - 24);
    const width = Math.min(Math.max(minWidth, rect.width), maxWidth);
    let left = rect.left;
    if (left + width > window.innerWidth - 12) {
      left = Math.max(12, window.innerWidth - 12 - width);
    }
    deps.modelMenuEl.style.position = "fixed";
    deps.modelMenuEl.style.left = `${Math.round(left)}px`;
    deps.modelMenuEl.style.right = "auto";
    deps.modelMenuEl.style.width = `${Math.round(width)}px`;
    deps.modelMenuEl.style.minWidth = `${Math.round(width)}px`;
    deps.modelMenuEl.style.bottom = `${Math.round(window.innerHeight - rect.top + gap)}px`;
    deps.modelMenuEl.style.top = "auto";
    deps.modelMenuEl.style.zIndex = "120";
  }

  function chosenModel(): { provider: PersonalProviderListItem; model: string } | null {
    const allowed = compatibleSavedProviders();
    const session = deps.activeSession();
    const pinnedId = session?.providerId?.trim() ?? "";
    const pinned =
      pinnedId && !isNativeProviderId(pinnedId)
        ? allowed.find((item) => item.id === pinnedId)
        : undefined;
    const wiredRaw = deps.getWiredProvider();
    const wired = wiredRaw ? allowed.find((item) => item.id === wiredRaw.id) ?? null : null;
    const provider = pinned || wired || allowed[0] || null;
    if (!provider) return null;
    const model = (pinned ? session?.model || provider.model : provider.model).trim();
    if (
      provider.id === LOCAL_CLUSTER_PROVIDER_ID &&
      !pinned &&
      !(localCluster?.active_models ?? []).some((id) => id.trim() === model)
    ) {
      return null;
    }
    return { provider, model: model || "—" };
  }

  function renderModelPickerLabel(): void {
    const runtime = deps.selectedRuntime();
    const catalog = loggedAccountCatalog();
    const session = deps.activeSession();
    if (catalog && sessionUsesLoggedAccount(session, catalog)) {
      const model = alignAccountSession(catalog);
      const onDefault =
        !model || model.toLowerCase() === NATIVE_ACCOUNT_DEFAULT_MODEL;
      const label = onDefault ? catalog.label : `${catalog.label} · ${model}`;
      deps.modelLabelEl.textContent = label;
      deps.modelBtnEl.disabled = deps.isComposerLocked();
      deps.modelBtnEl.title = t("chat.modelSubscriptionHint");
      deps.modelBtnEl.setAttribute("aria-label", label);
      return;
    }

    const quota = officialQuotaState(officialStatus);
    if (runtime === "doctor" && quota.signedIn && !doctorPinnedAwayFromOfficial()) {
      const pinnedModel = session?.model?.trim() ?? "";
      const onGift = pinnedModel === "" || pinnedModel === OFFICIAL_GIFT_MODEL;
      const detail = onGift
        ? builtinQuotaLabel(quota)
        : pinnedModel === "deepseek-flash"
          ? t("chat.modelOfficialAutoModel")
          : pinnedModel;
      deps.modelLabelEl.textContent = `${t("chat.modelOfficialGroup")} · ${detail}`;
      deps.modelBtnEl.disabled = deps.isComposerLocked();
      deps.modelBtnEl.title = t("chat.modelOfficialHint");
      deps.modelBtnEl.setAttribute("aria-label", deps.modelLabelEl.textContent);
      return;
    }

    const runtimeName = runtimeDisplayName(runtime);
    const chosen = chosenModel();
    if (chosen) {
      deps.modelLabelEl.textContent = chosen.model;
      deps.modelBtnEl.disabled = deps.isComposerLocked();
      deps.modelBtnEl.title = t("chat.modelPickHint");
      deps.modelBtnEl.setAttribute("aria-label", chosen.model);
      return;
    }

    if (!isPersonalEdition() && localClusterModelIds(localCluster).length > 0) {
      deps.modelLabelEl.textContent = t("chat.modelPickLabel");
      deps.modelBtnEl.disabled = deps.isComposerLocked();
      deps.modelBtnEl.title = t("chat.modelLocalGroup");
      deps.modelBtnEl.setAttribute("aria-label", t("chat.modelPickLabel"));
      return;
    }

    if (isPersonalEdition()) {
      deps.modelLabelEl.textContent = t("chat.modelPickLabel");
      deps.modelBtnEl.disabled = deps.isComposerLocked();
      deps.modelBtnEl.title =
        runtime === "cursor"
          ? t("chat.modelNeedCursorLogin")
          : runtime === "zcode"
            ? t("chat.modelNeedZcodeLogin")
            : runtime === "doctor"
              ? t("personal.officialNeedsLogin")
              : t("chat.modelNeedProvider");
    } else {
      deps.modelLabelEl.textContent = runtimeName;
      deps.modelBtnEl.disabled = true;
      deps.modelBtnEl.title = `${runtimeName} — ${t("chat.runtimeLockedHint")}`;
    }
    deps.modelBtnEl.setAttribute("aria-label", deps.modelLabelEl.textContent);
  }

  function renderDesktopSyncControls(group: HTMLElement, catalog: LoggedAccountCatalog, hintKey: "chat.modelSyncDesktopHint" | "chat.modelSyncDesktopEditionHint"): void {
    const busy = syncingAccountRuntime === catalog.runtimeId;
    const feedback = syncFeedbackByRuntime.get(catalog.runtimeId) ?? "";
    const hint = document.createElement("div");
    hint.className = "chat-model-menu-hint";
    hint.textContent = feedback || t(hintKey);
    group.appendChild(hint);

    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "chat-model-menu-action";
    btn.disabled = busy;
    btn.textContent = busy
      ? t("chat.modelSyncDesktopBusy")
      : t("chat.modelSyncDesktopCta", { account: catalog.label });
    btn.addEventListener("click", (event) => {
      event.stopPropagation();
      void syncDesktopAccount(catalog);
    });
    group.appendChild(btn);
  }

  async function syncDesktopAccount(
    catalog: LoggedAccountCatalog,
    resync = false,
  ): Promise<void> {
    if (syncingAccountRuntime) return;
    syncingAccountRuntime = catalog.runtimeId;
    syncFeedbackByRuntime.delete(catalog.runtimeId);
    selectedTabId = catalog.providerId;
    if (deps.getModelMenuOpen()) {
      renderModelMenu();
      positionModelMenu();
    }
    deps.setStatus(t("chat.modelSyncDesktopStatus", { account: catalog.label }), "muted");
    try {
      const result = resync
        ? await resignNativeSubscription(catalog.runtimeId)
        : await useNativeSubscription(catalog.runtimeId);
      modelListTriedAt.delete(catalog.runtimeId);
      probedEmptyRuntimes.delete(catalog.runtimeId);
      const again = await nativeAccountStatus(false);
      rememberNativeRows(again.rows);
      const updated = loggedAccountCatalogFor(catalog.runtimeId, nativeRows);
      if (result.outcome === "using" && updated?.cliReady) {
        syncFeedbackByRuntime.delete(catalog.runtimeId);
        const pickedModel = updated.models[0] || NATIVE_ACCOUNT_DEFAULT_MODEL;
        deps.pinSessionModel(
          {
            id: updated.providerId,
            name: updated.label,
            url: updated.providerId,
            model: pickedModel,
            protocol: "openai",
            api_key_hint: "",
            active: updated.usingByDefault,
          },
          pickedModel,
        );
        deps.updateContextMeter();
        deps.setStatus(t("chat.modelSyncDesktopDone", { account: updated.label }), "ok");
      } else {
        const message = loginFailureMessage(result);
        syncFeedbackByRuntime.set(catalog.runtimeId, message);
        deps.setStatus(message, "warn");
      }
    } catch {
      const message = t("chat.modelSyncDesktopIncomplete");
      syncFeedbackByRuntime.set(catalog.runtimeId, message);
      deps.setStatus(message, "warn");
    } finally {
      syncingAccountRuntime = null;
      renderModelPickerLabel();
      if (deps.getModelMenuOpen()) {
        renderModelMenu();
        positionModelMenu();
      }
    }
  }

  async function signOutAccount(catalog: LoggedAccountCatalog): Promise<void> {
    if (syncingAccountRuntime) return;
    syncingAccountRuntime = catalog.runtimeId;
    syncFeedbackByRuntime.delete(catalog.runtimeId);
    if (deps.getModelMenuOpen()) renderModelMenu();
    try {
      await signOutNativeSubscription(catalog.runtimeId);
      modelListTriedAt.delete(catalog.runtimeId);
      probedEmptyRuntimes.delete(catalog.runtimeId);
      const session = deps.activeSession();
      if (session?.providerId === catalog.providerId) {
        deps.pinSubscriptionModel("");
      }
      const again = await nativeAccountStatus(false);
      rememberNativeRows(again.rows);
      deps.updateContextMeter();
      deps.setStatus(t("chat.modelAccountSignedOut", { account: catalog.label }), "ok");
    } catch (error) {
      deps.setStatus(String(error), "error");
    } finally {
      syncingAccountRuntime = null;
      renderModelPickerLabel();
      if (deps.getModelMenuOpen()) {
        renderModelMenu();
        positionModelMenu();
      }
    }
  }

  function appendAccountModelOptions(
    group: HTMLElement,
    catalog: LoggedAccountCatalog,
    options: string[],
    onAccount: boolean,
    activeModel: string,
  ): void {
    const sections = catalog.modelSections;
    const distinct = sections.filter(
      (section, index) => section && section !== "auto" && sections.indexOf(section) === index,
    );
    const grouped = sections.length === options.length && distinct.length >= 2;
    const order = grouped
      ? [
          ...(sections.includes("auto") ? ["auto"] : []),
          ...sections.filter(
            (section, index) => section !== "auto" && sections.indexOf(section) === index,
          ),
        ]
      : ["flat"];
    for (const section of order) {
      const rows = options
        .map((id, index) => ({ id, index }))
        .filter((row) => !grouped || (sections[row.index] || "other") === section);
      if (rows.length === 0) continue;
      if (grouped && section !== "auto") {
        const title = accountSectionTitle(section, catalog);
        if (title) {
          const heading = document.createElement("div");
          heading.className = "chat-model-subgroup-label";
          heading.textContent = title;
          group.appendChild(heading);
        }
      }
      for (const row of rows) {
        const btn = document.createElement("button");
        btn.type = "button";
        const active = onAccount && activeModel.toLowerCase() === row.id.toLowerCase();
        btn.className = `chat-model-option${active ? " is-active" : ""}`;
        btn.role = "option";
        btn.setAttribute("aria-selected", active ? "true" : "false");
        const named = catalog.modelLabels[row.index]?.trim();
        btn.textContent =
          row.id === NATIVE_ACCOUNT_DEFAULT_MODEL
            ? t("chat.modelAccountDefault")
            : named || modelOptionLabel(row.id);
        btn.addEventListener("click", () => {
          selectAccountModel(catalog, row.id);
        });
        group.appendChild(btn);
      }
    }
  }

  function renderAccountPanel(panel: HTMLElement, catalog: LoggedAccountCatalog): void {
    const session = deps.activeSession();
    const onAccount = sessionUsesLoggedAccount(session, catalog);
    const activeModel = onAccount ? alignAccountSession(catalog) : "";
    const group = document.createElement("div");
    group.className = "chat-model-group";
    const label = document.createElement("div");
    label.className = "chat-model-group-label";
    label.textContent = accountAllowanceTitle(catalog);
    if (catalog.usingByDefault) {
      const mark = document.createElement("span");
      mark.className = "chat-model-default";
      mark.textContent = t("chat.modelDefault");
      label.appendChild(mark);
    }
    group.appendChild(label);

    if (!catalog.cliReady) {
      renderDesktopSyncControls(group, catalog, "chat.modelSyncDesktopHint");
      panel.appendChild(group);
      return;
    }

    const options =
      catalog.models.length > 0 ? catalog.models : [NATIVE_ACCOUNT_DEFAULT_MODEL];
    appendAccountModelOptions(group, catalog, options, onAccount, activeModel);

    if (catalog.models.length === 0) {
      if (loadingAccountRuntimes.has(catalog.runtimeId)) {
        const note = document.createElement("div");
        note.className = "chat-model-menu-hint";
        note.textContent = t("chat.modelListLoading");
        group.appendChild(note);
      } else if (catalog.desktopNeedsSync) {
        renderDesktopSyncControls(group, catalog, "chat.modelSyncDesktopEditionHint");
      } else if (
        accountSupportsModelList(catalog.runtimeId) &&
        probedEmptyRuntimes.has(catalog.runtimeId)
      ) {
        const note = document.createElement("div");
        note.className = "chat-model-menu-hint";
        note.textContent = t("chat.modelAccountNoModels");
        group.appendChild(note);
      }
    }

    if (accountSupportsModelList(catalog.runtimeId)) {
      const busy = syncingAccountRuntime === catalog.runtimeId;
      const footer = document.createElement("div");
      footer.className = "chat-model-account-footer";

      const resyncBtn = document.createElement("button");
      resyncBtn.type = "button";
      resyncBtn.className = "chat-model-account-link";
      resyncBtn.disabled = busy;
      resyncBtn.textContent = busy
        ? t("chat.modelSyncDesktopBusy")
        : t("chat.modelAccountResync");
      resyncBtn.addEventListener("click", (event) => {
        event.stopPropagation();
        void syncDesktopAccount(catalog, true);
      });

      const signOutBtn = document.createElement("button");
      signOutBtn.type = "button";
      signOutBtn.className = "chat-model-account-link";
      signOutBtn.disabled = busy;
      signOutBtn.textContent = t("chat.modelAccountSignOut");
      signOutBtn.addEventListener("click", (event) => {
        event.stopPropagation();
        void signOutAccount(catalog);
      });

      footer.append(resyncBtn, signOutBtn);
      group.appendChild(footer);
    }

    panel.appendChild(group);
  }

  /** Badge for the official (external-CLI) line: it spends paid tokens/trial
   * calls, not the built-in agent's monthly gift. */
  function officialLineBadge(): HTMLElement | null {
    const quota = officialQuotaState(officialStatus);
    if (!quota.signedIn) return null;
    const mark = document.createElement("span");
    mark.className = "chat-model-quota";
    if (quota.paid) {
      mark.textContent = t("official.quotaPaid", {
        tokens: formatOfficialTokens(quota.paid.tokens),
      });
    } else if (quota.trial) {
      mark.textContent = t("official.quotaTrial", { count: String(quota.trial.left) });
    } else {
      return null;
    }
    return mark;
  }

  function startOfficialLogin(): void {
    document.querySelector<HTMLButtonElement>("#chat-account")?.click();
  }

  function openOfficialAccountPage(): void {
    void openBrowserUrl(teamupsAccountPageUrl()).catch(() => {});
  }

  function menuActionButton(text: string, onClick: () => void): HTMLButtonElement {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "chat-model-menu-action";
    btn.textContent = text;
    btn.addEventListener("click", (event) => {
      event.stopPropagation();
      onClick();
      closeModelMenu();
    });
    return btn;
  }

  function doctorPinnedAwayFromOfficial(): boolean {
    const pinnedId = deps.activeSession()?.providerId?.trim();
    return Boolean(pinnedId) && pinnedId !== OFFICIAL_PROVIDER_ID;
  }

  function renderDoctorOfficialPanel(panel: HTMLElement): void {
    if (!officialSignedIn()) return;
    const quota = officialQuotaState(officialStatus);
    const quotaLoaded = Boolean(officialStatus?.official || officialStatus?.agent);

    const onOfficial = !doctorPinnedAwayFromOfficial();
    const session = deps.activeSession();
    const pinnedModel = session?.model?.trim() ?? "";
    const giftLeft = quota.free?.left ?? 0;
    const paidLeft = quota.paid?.tokens ?? 0;
    const effectivePaid =
      pinnedModel !== "" && pinnedModel !== OFFICIAL_GIFT_MODEL
        ? onOfficial
        : onOfficial && pinnedModel === "" && giftLeft <= 0 && (paidLeft > 0 || !quotaLoaded);
    const effectiveGift =
      onOfficial &&
      quotaLoaded &&
      (pinnedModel === OFFICIAL_GIFT_MODEL ||
        (pinnedModel === "" && (giftLeft > 0 || paidLeft <= 0)));

    const hint = document.createElement("div");
    hint.className = "chat-model-menu-hint";
    hint.textContent = t("chat.modelOfficialHint");
    panel.appendChild(hint);

    const group = document.createElement("div");
    group.className = "chat-model-group";
    const label = document.createElement("div");
    label.className = "chat-model-group-label";
    label.textContent = t("chat.modelOfficialGroup");
    group.appendChild(label);
    panel.appendChild(group);

    const officialRow =
      deps.getProviders().find((item) => item.id === OFFICIAL_PROVIDER_ID) ??
      deps.getWiredProvider();
    const hasRow = Boolean(officialRow && officialRow.id === OFFICIAL_PROVIDER_ID);

    if (hasRow && giftLeft > 0) {
      group.appendChild(
        officialOption(
          t("chat.officialGiftOption"),
          effectiveGift,
          officialRow!,
          OFFICIAL_GIFT_MODEL,
        ),
      );
    }
    if (hasRow && (paidLeft > 0 || !quotaLoaded)) {
      group.appendChild(
        officialOption(
          t("chat.modelOfficialAutoModel"),
          effectivePaid,
          officialRow!,
          officialRow!.model || "deepseek-flash",
        ),
      );
    }
    if (!hasRow || (quotaLoaded && giftLeft <= 0 && paidLeft <= 0)) {
      const empty = document.createElement("div");
      empty.className = "chat-model-menu-hint";
      const reset = quota.free?.resetDate;
      empty.textContent = reset
        ? t("official.quotaFreeReset", { date: formatResetDate(reset) })
        : t("official.quotaEmpty");
      panel.appendChild(empty);
      group.appendChild(menuActionButton(t("chat.modelOfficialBuyCta"), openOfficialAccountPage));
    }
  }

  function officialOption(
    text: string,
    active: boolean,
    officialRow: PersonalProviderListItem,
    model: string,
  ): HTMLButtonElement {
    const option = document.createElement("button");
    option.type = "button";
    option.className = `chat-model-option${active ? " is-active" : ""}`;
    option.setAttribute("aria-selected", active ? "true" : "false");
    option.textContent = text;
    option.addEventListener("click", () => {
      selectSessionModel(officialRow, model);
    });
    return option;
  }

  function renderProviderPanel(
    panel: HTMLElement,
    provider: PersonalProviderListItem,
    models: string[],
    chosen: { provider: PersonalProviderListItem; model: string } | null,
    catalogUsingDefault: boolean,
  ): void {
    const isOfficial = provider.id === OFFICIAL_PROVIDER_ID;
    const group = document.createElement("div");
    group.className = "chat-model-group";
    const label = document.createElement("div");
    label.className = "chat-model-group-label";
    label.textContent = isOfficial
      ? t("chat.modelOfficialGroup")
      : provider.name.trim() || providerChipForUrl(provider.url, provider.name);
    if (provider.active && !catalogUsingDefault) {
      const mark = document.createElement("span");
      mark.className = "chat-model-default";
      mark.textContent = t("chat.modelDefault");
      label.appendChild(mark);
    }
    if (isOfficial) {
      const badge = officialLineBadge();
      if (badge) label.appendChild(badge);
    }
    group.appendChild(label);
    for (const model of models) {
      const btn = document.createElement("button");
      btn.type = "button";
      const active = chosen?.provider.id === provider.id && chosen.model === model;
      btn.className = `chat-model-option${active ? " is-active" : ""}`;
      btn.role = "option";
      btn.setAttribute("aria-selected", active ? "true" : "false");
      btn.textContent = isOfficial ? t("chat.modelOfficialAutoModel") : modelOptionLabel(model);
      btn.addEventListener("click", () => {
        chooseProviderModel(provider, model);
      });
      group.appendChild(btn);
    }
    panel.appendChild(group);
  }

  function renderEmptyMenuHint(runtime: AskRuntime): void {
    const hint = document.createElement("div");
    hint.className = "chat-model-menu-hint";
    if (runtime === "cursor" || runtime === "zcode" || runtime === "antigravity") {
      const busy = syncingAccountRuntime === runtime;
      const feedback = syncFeedbackByRuntime.get(runtime) ?? "";
      hint.textContent =
        feedback ||
        (runtime === "cursor"
          ? t("chat.modelNeedCursorLogin")
          : runtime === "zcode"
            ? t("chat.modelNeedZcodeLogin")
            : t("chat.modelNeedAntigravityLogin"));
      deps.modelMenuEl.appendChild(hint);

      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "chat-model-menu-action";
      btn.disabled = busy;
      btn.textContent = busy
        ? t("chat.modelSyncDesktopBusy")
        : runtime === "cursor"
          ? t("native.loginCursor")
          : runtime === "zcode"
            ? t("native.loginZcode")
            : t("native.loginAntigravity");
      btn.addEventListener("click", (event) => {
        event.stopPropagation();
        void syncDesktopAccount({
          runtimeId: runtime,
          providerId: nativeProviderId(runtime),
          label: nativeAccountLabel(runtime),
          models: [],
          modelLabels: [],
          modelSections: [],
          usingByDefault: false,
          cliReady: false,
          desktopNeedsSync: false,
          quotaCursorPercent: null,
          quotaOtherPercent: null,
          quotaHours: null,
          quotaHoursPercent: null,
          quotaWeekPercent: null,
          quotaCreditsLeft: null,
        });
      });
      deps.modelMenuEl.appendChild(btn);
      return;
    }
    if (runtime === "doctor") {
      hint.textContent = t("personal.officialNeedsLogin");
      deps.modelMenuEl.appendChild(hint);
      deps.modelMenuEl.appendChild(
        menuActionButton(t("chat.modelOfficialLoginCta"), startOfficialLogin),
      );
      return;
    }
    hint.textContent = t("chat.modelNeedProvider");
    deps.modelMenuEl.appendChild(hint);
  }

  function buildMenuTabs(): MenuTab[] {
    const runtime = deps.selectedRuntime();
    const isDoctor = runtime === "doctor";
    const catalog = loggedAccountCatalog();
    const session = deps.activeSession();
    const onAccount = sessionUsesLoggedAccount(session, catalog);
    const onDoctorOfficial =
      isDoctor && officialSignedIn() && !doctorPinnedAwayFromOfficial();
    const chosen = onAccount || onDoctorOfficial ? null : chosenModel();

    const rows = compatibleSavedProviders();
    const officialProvider = rows.find((p) => p.id === OFFICIAL_PROVIDER_ID) ?? null;
    const customEntries: Array<{ provider: PersonalProviderListItem; models: string[] }> = [];
    for (const provider of rows) {
      if (provider.id === OFFICIAL_PROVIDER_ID) continue;
      const models =
        provider.id === LOCAL_CLUSTER_PROVIDER_ID
          ? localClusterModelIds(localCluster)
          : modelsForProviderUrl(provider.url, provider.model);
      if (models.length > 0) {
        customEntries.push({ provider, models });
      }
    }

    const tabs: MenuTab[] = [];
    if (isDoctor && officialSignedIn()) {
      tabs.push({
        id: "official",
        title: t("chat.modelTabOfficial"),
        hasActive: onDoctorOfficial,
        render: (panel) => renderDoctorOfficialPanel(panel),
      });
    } else if (officialProvider) {
      let officialModels = modelsForProviderUrl(officialProvider.url, officialProvider.model);
      if (officialModels.length === 0) officialModels = ["auto"];
      tabs.push({
        id: "official",
        title: t("chat.modelTabOfficial"),
        hasActive: Boolean(chosen && chosen.provider.id === OFFICIAL_PROVIDER_ID),
        render: (panel) =>
          renderProviderPanel(
            panel,
            officialProvider,
            officialModels,
            chosen,
            Boolean(catalog?.usingByDefault),
          ),
      });
    }

    if (catalog) {
      tabs.push({
        id: "subscription",
        title: t("chat.modelTabSubscription"),
        hasActive: onAccount,
        render: (panel) => renderAccountPanel(panel, catalog),
      });
    }

    if (customEntries.length > 0) {
      const customActive = Boolean(
        chosen && customEntries.some((entry) => entry.provider.id === chosen.provider.id),
      );
      tabs.push({
        id: "custom",
        title:
          customEntries.length === 1 &&
          customEntries[0]?.provider.id === LOCAL_CLUSTER_PROVIDER_ID
            ? t("chat.modelLocalGroup")
            : t("chat.modelTabCustom"),
        hasActive: customActive,
        render: (panel) => {
          for (const entry of customEntries) {
            renderProviderPanel(
              panel,
              entry.provider,
              entry.models,
              chosen,
              Boolean(catalog?.usingByDefault),
            );
          }
        },
      });
    }
    return tabs;
  }

  function renderModelMenu(): void {
    deps.modelMenuEl.replaceChildren();
    const runtime = deps.selectedRuntime();
    const tabs = buildMenuTabs();
    if (tabs.length === 0) {
      renderEmptyMenuHint(runtime);
      return;
    }

    const activeTab =
      tabs.find((tab) => tab.id === selectedTabId) ??
      tabs.find((tab) => tab.hasActive) ??
      tabs[0];
    selectedTabId = activeTab.id;

    if (tabs.length > 1) {
      const tabBar = document.createElement("div");
      tabBar.className = "chat-model-tabs";
      tabBar.setAttribute("role", "tablist");
      for (const tab of tabs) {
        const selected = tab.id === activeTab.id;
        const btn = document.createElement("button");
        btn.type = "button";
        btn.role = "tab";
        btn.className = `chat-model-tab${selected ? " is-selected" : ""}`;
        btn.setAttribute("aria-selected", selected ? "true" : "false");
        btn.textContent = tab.title;
        if (tab.hasActive && !selected) {
          const dot = document.createElement("span");
          dot.className = "chat-model-tab-dot";
          dot.setAttribute("aria-hidden", "true");
          btn.appendChild(dot);
        }
        btn.addEventListener("click", (event) => {
          event.stopPropagation();
          if (selectedTabId === tab.id) return;
          selectedTabId = tab.id;
          renderModelMenu();
          positionModelMenu();
        });
        tabBar.appendChild(btn);
      }
      deps.modelMenuEl.appendChild(tabBar);
    }

    const panel = document.createElement("div");
    panel.className = "chat-model-panel";
    panel.setAttribute("role", "tabpanel");
    activeTab.render(panel);
    deps.modelMenuEl.appendChild(panel);
  }

  function maybeRefreshAccountModels(runtime: string): void {
    if (!accountSupportsModelList(runtime)) return;
    if (loadingAccountRuntimes.has(runtime)) return;
    const lastTried = modelListTriedAt.get(runtime) ?? 0;
    if (Date.now() - lastTried <= 15_000) return;
    modelListTriedAt.set(runtime, Date.now());
    loadingAccountRuntimes.add(runtime);
    if (deps.getModelMenuOpen()) renderModelMenu();
    void refreshSubscriptionModels(runtime)
      .then(async (fetched) => {
        if (fetched.length === 0) probedEmptyRuntimes.add(runtime);
        else probedEmptyRuntimes.delete(runtime);
        const again = await nativeAccountStatus(false);
        rememberNativeRows(again.rows);
      })
      .catch(() => {
        if ((loggedAccountCatalog()?.models.length ?? 0) === 0) {
          probedEmptyRuntimes.add(runtime);
        }
      })
      .finally(() => {
        loadingAccountRuntimes.delete(runtime);
        renderModelPickerLabel();
        if (deps.getModelMenuOpen()) {
          renderModelMenu();
          positionModelMenu();
        }
      });
  }

  function openModelMenu(): void {
    if (deps.modelBtnEl.disabled || deps.isComposerLocked()) return;
    selectedTabId = "";
    void refreshOfficialStatus().then(() => {
      repaintAfterOfficialRefresh();
      if (deps.getModelMenuOpen()) positionModelMenu();
    });
    void nativeAccountStatus(false)
      .then((accounts) => {
        rememberNativeRows(accounts.rows);
        const runtime = deps.selectedRuntime();
        const catalog = loggedAccountCatalog();
        if (catalog?.cliReady) {
          maybeRefreshAccountModels(runtime);
        }
        renderModelPickerLabel();
        if (deps.getModelMenuOpen()) {
          renderModelMenu();
          positionModelMenu();
        }
      })
      .catch(() => {});
    renderModelMenu();
    deps.setModelMenuOpen(true);
    deps.modelMenuEl.hidden = false;
    deps.modelBtnEl.classList.add("is-open");
    deps.modelBtnEl.setAttribute("aria-expanded", "true");
    deps.modelWrapEl?.classList.add("is-open");
    deps.composerBoxEl.classList.add("is-model-open");
    deps.composerEl.classList.add("is-model-open");
    positionModelMenu();
  }

  async function refreshWiredProvider(): Promise<void> {
    await loadSavedAskProviders();
    if (!isPersonalEdition()) await attachLocalModels();
  }

  async function loadSavedAskProviders(): Promise<void> {
    try {
      const [status, doc, accounts, official] = await Promise.all([
        getPersonalProviderStatus(),
        listPersonalProviders(),
        nativeAccountStatus(false).catch(() => null),
        teamupsAccountStatus().catch(() => null),
      ]);
      if (official) {
        officialStatus = official;
        officialFetchedAt = Date.now();
      }
      if (accounts) rememberNativeRows(accounts.rows);
      const missing = nativeRows.filter(
        (row) =>
          row.logged_in &&
          accountSupportsModelList(row.runtime_id) &&
          (row.models ?? []).length === 0 &&
          Date.now() - (modelListTriedAt.get(row.runtime_id) ?? 0) > 15_000,
      );
      if (missing.length > 0) {
        for (const row of missing) {
          modelListTriedAt.set(row.runtime_id, Date.now());
        }
        await Promise.all(
          missing.map((row) =>
            refreshSubscriptionModels(row.runtime_id)
              .then((list) => {
                if (list.length === 0) probedEmptyRuntimes.add(row.runtime_id);
                else probedEmptyRuntimes.delete(row.runtime_id);
              })
              .catch(() => {
                probedEmptyRuntimes.add(row.runtime_id);
              }),
          ),
        );
        const again = await nativeAccountStatus(false).catch(() => null);
        if (again) rememberNativeRows(again.rows);
      }
      deps.setProviders(doc.providers);
      const active =
        doc.providers.find((p) => p.active) ||
        (status.active_id
          ? doc.providers.find((p) => p.id === status.active_id)
          : undefined) ||
        null;
      deps.setWiredProvider(
        active
          ? {
              ...active,
              model: active.model || status.model || "",
              name: active.name || status.active_name || active.name,
            }
          : status.configured &&
              status.active_id &&
              (status.active_id !== OFFICIAL_PROVIDER_ID || officialSignedIn())
            ? {
                id: status.active_id,
                name: status.active_name || "Provider",
                url: status.gateway_url || "",
                model: status.model || "",
                protocol: status.protocol || "openai",
                api_key_hint: status.api_key_hint || "",
                active: true,
              }
            : null,
      );
    } catch (error) {
      console.warn("Ask: failed to load personal provider", error);
      deps.setProviders([]);
      deps.setWiredProvider(null);
    }
    renderModelPickerLabel();
    if (deps.getModelMenuOpen()) renderModelMenu();
  }

  async function attachLocalModels(): Promise<void> {
    try {
      localCluster = await getClusterStatus();
    } catch {
      localCluster = null;
    }
    const models = localClusterModelIds(localCluster);
    if (!localCluster || models.length === 0) {
      renderModelPickerLabel();
      if (deps.getModelMenuOpen()) renderModelMenu();
      return;
    }
    const running = localCluster.active_models?.find((id) => id.trim())?.trim() || "";
    const provider = localClusterProvider(localCluster, running || models[0] || "");
    deps.setProviders(providersWithLocalCluster(deps.getProviders(), provider));
    if (running && deps.getWiredProvider()?.id === LOCAL_CLUSTER_PROVIDER_ID) {
      deps.setWiredProvider(provider);
      adoptRunningModel(provider, running);
    }
    renderModelPickerLabel();
    if (deps.getModelMenuOpen()) renderModelMenu();
  }

  function adoptRunningModel(provider: PersonalProviderListItem, running: string): void {
    if (loadingLocalModel) return;
    const session = deps.activeSession();
    const pinned = session?.providerId?.trim() ?? "";
    // A model that just started on this computer must not take a chat off 官方 or a subscription.
    if (pinned !== LOCAL_CLUSTER_PROVIDER_ID) return;
    const current = session?.model?.trim() ?? "";
    if (!current || current === running) return;
    if ((localCluster?.active_models ?? []).some((id) => id.trim() === current)) return;
    deps.pinSessionModel(provider, running);
  }

  function chooseProviderModel(provider: PersonalProviderListItem, model: string): void {
    if (
      provider.id === LOCAL_CLUSTER_PROVIDER_ID &&
      !(localCluster?.active_models ?? []).some((id) => id.trim() === model)
    ) {
      void loadLocalModel(model);
      return;
    }
    selectSessionModel(provider, model);
  }

  async function loadLocalModel(model: string): Promise<void> {
    if (deps.isComposerLocked() || loadingLocalModel) return;
    loadingLocalModel = true;
    closeModelMenu();
    deps.setStatus(t("chat.modelLocalLoading"), "muted");
    try {
      const status = await activateClusterModel(model);
      localCluster = status;
      const provider = localClusterProvider(status, model);
      deps.setProviders(providersWithLocalCluster(deps.getProviders(), provider));
      deps.setWiredProvider(provider);
      selectSessionModel(provider, model);
      deps.setStatus(t("chat.modelLocalReady"), "ok");
    } catch {
      deps.setStatus(t("chat.modelLocalFailed"), "error");
    } finally {
      loadingLocalModel = false;
      renderModelPickerLabel();
    }
  }

  function selectAccountModel(catalog: LoggedAccountCatalog, model: string): void {
    if (deps.isComposerLocked()) return;
    const next = model.trim() || NATIVE_ACCOUNT_DEFAULT_MODEL;
    if (
      next !== NATIVE_ACCOUNT_DEFAULT_MODEL &&
      catalog.models.length > 0 &&
      !catalog.models.some((id) => id.toLowerCase() === next.toLowerCase())
    ) {
      closeModelMenu();
      return;
    }
    const session = deps.activeSession();
    const alreadyOnAccount = sessionUsesLoggedAccount(session, catalog);
    const currentModel = alreadyOnAccount ? alignAccountSession(catalog) : "";
    closeModelMenu();
    if (
      alreadyOnAccount &&
      session?.providerId === catalog.providerId &&
      currentModel.toLowerCase() === next.toLowerCase()
    ) {
      return;
    }
    deps.pinSessionModel(
      {
        id: catalog.providerId,
        name: catalog.label,
        url: catalog.providerId,
        model: next,
        protocol: "openai",
        api_key_hint: "",
        active: catalog.usingByDefault,
      },
      next,
    );
    renderModelPickerLabel();
    deps.updateContextMeter();
    const switchedText =
      next.toLowerCase() === NATIVE_ACCOUNT_DEFAULT_MODEL
        ? catalog.label
        : `${catalog.label} · ${next}`;
    deps.setStatus(t("chat.modelSwitched", { model: switchedText }), "ok");
    deps.modelBtnEl.disabled = deps.isComposerLocked();
  }

  function selectSessionModel(provider: PersonalProviderListItem, model: string): void {
    if (deps.isComposerLocked()) return;
    const next = model.trim();
    if (!next) {
      closeModelMenu();
      return;
    }
    const session = deps.activeSession();
    const catalog = loggedAccountCatalog();
    const onAccount = sessionUsesLoggedAccount(session, catalog);
    const current = onAccount ? null : chosenModel();
    closeModelMenu();
    if (
      !onAccount &&
      session?.providerId === provider.id &&
      current?.provider.id === provider.id &&
      current.model === next
    ) {
      return;
    }
    deps.pinSessionModel(provider, next);
    renderModelPickerLabel();
    deps.updateContextMeter();
    const name =
      provider.id === OFFICIAL_PROVIDER_ID
        ? t("chat.modelOfficialGroup")
        : providerChipForUrl(provider.url, provider.name);
    let modelText = next;
    if (provider.id === OFFICIAL_PROVIDER_ID) {
      modelText =
        next === OFFICIAL_GIFT_MODEL ? t("chat.officialGiftOption") : t("chat.modelOfficialAutoModel");
    }
    deps.setStatus(t("chat.modelSwitched", { model: `${name} · ${modelText}` }), "ok");
    deps.modelBtnEl.disabled = deps.isComposerLocked();
  }

  function updateRuntimeLabel(): void {
    renderModelPickerLabel();
    deps.updateContextMeter();
  }

  if (isPersonalEdition()) {
    void listen("teamups-account-changed", () => {
      officialFetchedAt = 0;
      void refreshWiredProvider();
    }).catch(() => {
      // No Tauri bridge (tests, plain web preview).
    });
  } else {
    void listen("cluster-status-changed", () => {
      void refreshWiredProvider();
    }).catch(() => {
      // No Tauri bridge (tests, plain web preview).
    });
  }

  return {
    closeModelMenu,
    positionModelMenu,
    renderModelPickerLabel,
    renderModelMenu,
    openModelMenu,
    refreshWiredProvider,
    switchWiredModel: async (model: string) => {
      const wired = deps.getWiredProvider();
      if (!wired) return;
      selectSessionModel(wired, model);
    },
    updateRuntimeLabel,
    doctorOfficialFallback: () => {
      if (deps.selectedRuntime() !== "doctor") return null;
      if (deps.activeSession()?.providerId?.trim()) return null;
      const quota = officialQuotaState(officialStatus);
      if (!quota.signedIn) {
        const fallback = chosenModel();
        if (!fallback || fallback.provider.id === OFFICIAL_PROVIDER_ID) return null;
        return {
          providerId: fallback.provider.id,
          model: fallback.model === "—" ? fallback.provider.model : fallback.model,
        };
      }
      const giftLeft = quota.free?.left ?? 0;
      if (giftLeft > 0) return null;
      const paidLeft = quota.paid?.tokens ?? 0;
      if (paidLeft <= 0) return null;
      const officialRow =
        deps.getProviders().find((item) => item.id === OFFICIAL_PROVIDER_ID) ??
        deps.getWiredProvider();
      if (!officialRow || officialRow.id !== OFFICIAL_PROVIDER_ID) return null;
      return {
        providerId: OFFICIAL_PROVIDER_ID,
        model: officialRow.model || "deepseek-flash",
      };
    },
    refreshOfficialQuota: () => {
      officialFetchedAt = 0;
      void refreshOfficialStatus().then(repaintAfterOfficialRefresh);
    },
    hasChoices: () =>
      loggedAccountCatalog() !== null ||
      deps.selectedRuntime() === "doctor" ||
      compatibleSavedProviders().length > 0,
  };
}
