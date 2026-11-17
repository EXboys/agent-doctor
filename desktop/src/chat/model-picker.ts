import { listen } from "@tauri-apps/api/event";
import type { AskRuntime } from "../ask-resources";
import { isPersonalEdition } from "../edition";
import { t } from "../i18n";
import {
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
  NativeAccountRow,
  PersonalProviderListItem,
  TeamupsAccountStatus,
} from "../types";
import { runtimeDisplayName } from "./runtime";
import type { ChatSession } from "./types";

export const NATIVE_PROVIDER_PREFIX = "native:";
export const NATIVE_ACCOUNT_DEFAULT_MODEL = "auto";

export function nativeProviderId(runtimeId: string): string {
  return `${NATIVE_PROVIDER_PREFIX}${runtimeId.trim()}`;
}

export function isNativeProviderId(providerId: string | null | undefined): boolean {
  return Boolean(providerId?.trim().startsWith(NATIVE_PROVIDER_PREFIX));
}

export function accountSupportsModelList(runtimeId: string): boolean {
  const id = runtimeId.trim();
  return id === "codex" || id === "claude-code" || id === "qoder" || id === "cursor";
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
    case "deepseek-harness":
      return t("native.deepseek");
    default:
      return runtimeId;
  }
}

export type LoggedAccountCatalog = {
  runtimeId: string;
  providerId: string;
  label: string;
  models: string[];
  usingByDefault: boolean;
  /** True when the CLI itself is already authorized and ready to answer. */
  cliReady: boolean;
  /** True when the companion desktop IDE has an active login not yet synced to CLI. */
  desktopNeedsSync: boolean;
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
    usingByDefault: cliReady && Boolean(row?.using_subscription),
    cliReady,
    desktopNeedsSync: Boolean(row?.desktop_needs_sync || (!cliReady && desktopLogged)),
  };
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
  return catalog.usingByDefault && !sessionKeepsSavedService(session);
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
  const modelListTriedAt = new Map<string, number>();
  const loadingAccountRuntimes = new Set<string>();
  const probedEmptyRuntimes = new Set<string>();
  const syncFeedbackByRuntime = new Map<string, string>();
  let syncingAccountRuntime: string | null = null;
  let selectedTabId = "";
  let officialStatus: TeamupsAccountStatus | null = null;
  let officialFetchedAt = 0;

  /** Throttled official-account fetch; drives the built-in agent's quota chip and official sign-in gate. */
  async function refreshOfficialStatus(): Promise<void> {
    if (!isPersonalEdition()) {
      officialStatus = null;
      return;
    }
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

  function officialSignedIn(): boolean {
    return officialQuotaState(officialStatus).signedIn;
  }

  function officialLineReady(): boolean {
    return officialLineUsable(officialStatus);
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
    return { provider, model: model || "—" };
  }

  function renderModelPickerLabel(): void {
    const runtime = deps.selectedRuntime();
    const catalog = loggedAccountCatalog();
    const session = deps.activeSession();
    if (catalog && sessionUsesLoggedAccount(session, catalog)) {
      const model = alignAccountSession(catalog);
      const label =
        !model || model.toLowerCase() === NATIVE_ACCOUNT_DEFAULT_MODEL
          ? catalog.label
          : `${catalog.label} · ${model}`;
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
      nativeRows = again.rows;
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
        const message =
          result.outcome === "unreachable"
            ? t("native.unreachable")
            : t("chat.modelSyncDesktopIncomplete");
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
      nativeRows = again.rows;
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

  function renderAccountPanel(panel: HTMLElement, catalog: LoggedAccountCatalog): void {
    const session = deps.activeSession();
    const onAccount = sessionUsesLoggedAccount(session, catalog);
    const activeModel = onAccount ? alignAccountSession(catalog) : "";
    const group = document.createElement("div");
    group.className = "chat-model-group";
    const label = document.createElement("div");
    label.className = "chat-model-group-label";
    label.textContent = catalog.label;
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
    for (const id of options) {
      const btn = document.createElement("button");
      btn.type = "button";
      const active =
        onAccount && activeModel.toLowerCase() === id.toLowerCase();
      btn.className = `chat-model-option${active ? " is-active" : ""}`;
      btn.role = "option";
      btn.setAttribute("aria-selected", active ? "true" : "false");
      btn.textContent =
        id === NATIVE_ACCOUNT_DEFAULT_MODEL ? t("chat.modelAccountDefault") : modelOptionLabel(id);
      btn.addEventListener("click", () => {
        selectAccountModel(catalog, id);
      });
      group.appendChild(btn);
    }

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
    const quota = officialQuotaState(officialStatus);
    if (!quota.signedIn) return;

    const onOfficial = !doctorPinnedAwayFromOfficial();
    const session = deps.activeSession();
    const pinnedModel = session?.model?.trim() ?? "";
    const giftLeft = quota.free?.left ?? 0;
    const paidLeft = quota.paid?.tokens ?? 0;
    const effectivePaid =
      pinnedModel !== "" && pinnedModel !== OFFICIAL_GIFT_MODEL
        ? onOfficial
        : onOfficial && pinnedModel === "" && giftLeft <= 0 && paidLeft > 0;
    const effectiveGift =
      onOfficial &&
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
    if (hasRow && paidLeft > 0) {
      group.appendChild(
        officialOption(
          t("chat.modelOfficialAutoModel"),
          effectivePaid,
          officialRow!,
          officialRow!.model || "deepseek-flash",
        ),
      );
    }
    if (!hasRow || (giftLeft <= 0 && paidLeft <= 0)) {
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
        selectSessionModel(provider, model);
      });
      group.appendChild(btn);
    }
    panel.appendChild(group);
  }

  function renderEmptyMenuHint(runtime: AskRuntime): void {
    const hint = document.createElement("div");
    hint.className = "chat-model-menu-hint";
    if (runtime === "cursor") {
      hint.textContent = t("chat.modelNeedCursorLogin");
      deps.modelMenuEl.appendChild(hint);
      return;
    }
    if (runtime === "zcode") {
      hint.textContent = t("chat.modelNeedZcodeLogin");
      deps.modelMenuEl.appendChild(hint);
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

    const tabs: MenuTab[] = [];
    if (isDoctor && officialSignedIn()) {
      tabs.push({
        id: OFFICIAL_PROVIDER_ID,
        title: t("chat.modelOfficialGroup"),
        hasActive: onDoctorOfficial,
        render: (panel) => renderDoctorOfficialPanel(panel),
      });
    }
    if (catalog) {
      tabs.push({
        id: catalog.providerId,
        title: catalog.label,
        hasActive: onAccount,
        render: (panel) => renderAccountPanel(panel, catalog),
      });
    }

    const rows = compatibleSavedProviders();
    for (const provider of rows) {
      if (isDoctor && provider.id === OFFICIAL_PROVIDER_ID) continue;
      const isOfficial = provider.id === OFFICIAL_PROVIDER_ID;
      let models = modelsForProviderUrl(provider.url, provider.model);
      if (isOfficial && models.length === 0) models = ["auto"];
      if (models.length === 0) continue;
      const title = isOfficial
        ? t("chat.modelOfficialGroup")
        : providerChipForUrl(provider.url, provider.name);
      tabs.push({
        id: provider.id,
        title,
        hasActive: Boolean(chosen && chosen.provider.id === provider.id),
        render: (panel) =>
          renderProviderPanel(
            panel,
            provider,
            models,
            chosen,
            Boolean(catalog?.usingByDefault),
          ),
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
        nativeRows = again.rows;
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
        nativeRows = accounts.rows;
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
    if (!isPersonalEdition()) {
      deps.setWiredProvider(null);
      renderModelPickerLabel();
      return;
    }
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
      if (accounts) nativeRows = accounts.rows;
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
        if (again) nativeRows = again.rows;
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
