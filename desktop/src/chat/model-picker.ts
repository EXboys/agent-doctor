
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { AskRuntime } from "../ask-resources";
import { isPersonalEdition } from "../edition";
import { t } from "../i18n";
import { nativeAccountStatus, refreshSubscriptionModels, teamupsAccountStatus } from "../ipc";
import {
  builtinQuotaLabel,
  formatOfficialTokens,
  formatResetDate,
  OFFICIAL_GIFT_MODEL,
  OFFICIAL_PROVIDER_ID,
  officialQuotaState,
} from "../official-quota";
import { modelsForProviderUrl, providerChipForUrl } from "../provider-models";
import { teamupsAccountPageUrl } from "../teamups-site";
import type {
  NativeAccountRow,
  PersonalProviderListItem,
  TeamupsAccountStatus,
} from "../types";
import { runtimeDisplayName } from "./runtime";
import type { ChatSession } from "./types";
import { getPersonalProviderStatus, listPersonalProviders } from "../ipc";

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
  if (catalogModels.length === 0) return { model: "", replace: false };
  const current = session?.model?.trim() ?? "";
  const model = catalogModels.includes(current) ? current : catalogModels[0];
  return { model, replace: Boolean(session && session.model !== model) };
}

/** This chat named a saved service. Login must not replace it on its own. */
export function sessionKeepsSavedService(session: ChatSession | null | undefined): boolean {
  if (!session) return false;
  if (session.providerId?.trim()) return true;
  const model = session.model?.trim().toLowerCase() ?? "";
  return ["glm", "deepseek", "qwen", "kimi", "moonshot", "minimax", "doubao", "gemini", "ernie"].some(
    (prefix) => model.startsWith(prefix),
  );
}

/**
 * The main window activated another provider, and the ask window follows it.
 * Chats pinned to a different saved service would keep quietly asking the old
 * one, so release those pins; a pin on the newly active provider already
 * agrees with it and stays. Returns whether anything changed.
 */
export function releaseStalePins(
  sessions: Array<{ providerId?: string | null; model?: string | null; providerTag?: string | null }>,
  activeId: string | null | undefined,
): boolean {
  if (!activeId) return false;
  let touched = false;
  for (const session of sessions) {
    if (!session.providerId || session.providerId === activeId) continue;
    session.providerId = null;
    session.model = "";
    session.providerTag = null;
    touched = true;
  }
  return touched;
}

type SubscriptionCatalog = {
  models: string[];
  label: string;
};

export function createModelPickerController(deps: ModelPickerDeps) {
  let nativeRows: NativeAccountRow[] = [];
  let modelListTriedAt = 0;
  let officialStatus: TeamupsAccountStatus | null = null;
  let officialFetchedAt = 0;

  /** Throttled official-account fetch; drives the built-in agent's quota chip. */
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

  function subscriptionCatalog(): SubscriptionCatalog | null {
    const runtime = deps.selectedRuntime();
    if (runtime !== "codex" && runtime !== "claude-code") return null;
    const row = nativeRows.find((item) => item.runtime_id === runtime);
    if (!row?.using_subscription) return null;
    return {
      models: row.models ?? [],
      label: runtime === "codex" ? t("native.chatgpt") : t("native.claude"),
    };
  }

  /** Keep this chat on an account model when it is already using the login.
   * A saved service stays selected. Replacing it here makes the next message
   * delete that service's key. */
  function alignSubscriptionSession(catalog: SubscriptionCatalog): string {
    const decision = subscriptionModelForSession(deps.activeSession(), catalog.models);
    if (decision.replace) deps.pinSubscriptionModel(decision.model);
    return decision.model;
  }

  function closeModelMenu(): void {
    deps.setModelMenuOpen(false);
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
    const minWidth = Math.max(rect.width, 220);
    const maxWidth = Math.min(340, window.innerWidth - 24);
    const width = Math.min(Math.max(minWidth, rect.width), maxWidth);
    let left = rect.left;
    if (left + width > window.innerWidth - 12) {
      left = Math.max(12, window.innerWidth - 12 - width);
    }
    // Anchor just above the model button (original interaction).
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
    const wired = deps.getWiredProvider();
    const session = deps.activeSession();
    const pinned = session?.providerId
      ? deps.getProviders().find((item) => item.id === session.providerId)
      : undefined;
    const provider = pinned || wired;
    if (!provider) return null;
    const model = (pinned ? session?.model || provider.model : provider.model).trim();
    return { provider, model: model || "—" };
  }

  function renderModelPickerLabel(): void {
    const catalog = subscriptionCatalog();
    if (catalog) {
      const model = alignSubscriptionSession(catalog);
      const saved = sessionKeepsSavedService(deps.activeSession());
      deps.modelLabelEl.textContent = model || t("chat.modelListMissing");
      deps.modelBtnEl.disabled = deps.isComposerLocked();
      deps.modelBtnEl.title = saved ? t("chat.modelPickHint") : t("chat.modelSubscriptionHint");
      deps.modelBtnEl.setAttribute("aria-label", model);
      return;
    }
    // The built-in agent's chip: the official quota while the chat rides the
    // gift (its default), the pinned provider's model once the user picks one.
    if (deps.selectedRuntime() === "doctor" && !doctorPinnedAwayFromOfficial()) {
      const quota = officialQuotaState(officialStatus);
      const pinnedModel = deps.activeSession()?.model?.trim() ?? "";
      const onGift = pinnedModel === "" || pinnedModel === OFFICIAL_GIFT_MODEL;
      const detail = onGift
        ? builtinQuotaLabel(quota)
        : pinnedModel === "deepseek-flash"
          ? t("chat.modelOfficialAutoModel")
          : pinnedModel;
      deps.modelLabelEl.textContent = `${t("chat.modelOfficialGroup")} · ${detail}`;
      deps.modelBtnEl.disabled = deps.isComposerLocked();
      deps.modelBtnEl.title = quota.signedIn
        ? t("chat.modelOfficialHint")
        : t("personal.officialNeedsLogin");
      deps.modelBtnEl.setAttribute("aria-label", deps.modelLabelEl.textContent);
      return;
    }
    const runtimeName = runtimeDisplayName(deps.selectedRuntime());
    const chosen = chosenModel();
    if (chosen) {
      deps.modelLabelEl.textContent = chosen.model;
      deps.modelBtnEl.disabled = deps.isComposerLocked();
      deps.modelBtnEl.title = t("chat.modelPickHint");
      deps.modelBtnEl.setAttribute("aria-label", chosen.model);
      return;
    }
    // Personal: keep clickable so the menu can say “go wire a provider”.
    // Team / locked: show runtime name only — model is chosen on the Agents page.
    if (isPersonalEdition()) {
      deps.modelLabelEl.textContent = t("chat.modelPickLabel");
      deps.modelBtnEl.disabled = deps.isComposerLocked();
      deps.modelBtnEl.title = t("chat.modelNeedProvider");
    } else {
      deps.modelLabelEl.textContent = runtimeName;
      deps.modelBtnEl.disabled = true;
      deps.modelBtnEl.title = `${runtimeName} — ${t("chat.runtimeLockedHint")}`;
    }
    deps.modelBtnEl.setAttribute("aria-label", deps.modelLabelEl.textContent);
  }

  function renderSubscriptionMenu(catalog: SubscriptionCatalog): void {
    if (catalog.models.length === 0) {
      const hint = document.createElement("div");
      hint.className = "chat-model-menu-hint";
      hint.textContent = t("chat.modelListMissing");
      deps.modelMenuEl.appendChild(hint);
      return;
    }
    const model = alignSubscriptionSession(catalog);
    const group = document.createElement("div");
    group.className = "chat-model-group";
    const label = document.createElement("div");
    label.className = "chat-model-group-label";
    label.textContent = catalog.label;
    group.appendChild(label);
    for (const id of catalog.models) {
      const btn = document.createElement("button");
      btn.type = "button";
      const active = model === id;
      btn.className = `chat-model-option${active ? " is-active" : ""}`;
      btn.role = "option";
      btn.setAttribute("aria-selected", active ? "true" : "false");
      btn.textContent = id;
      btn.addEventListener("click", () => {
        selectSubscriptionModel(id);
      });
      group.appendChild(btn);
    }
    deps.modelMenuEl.appendChild(group);
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
    // The account bar owns the device-code flow and its pending state.
    document.querySelector<HTMLButtonElement>("#chat-account")?.click();
  }

  function openOfficialAccountPage(): void {
    void openUrl(teamupsAccountPageUrl()).catch(() => {});
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

  /** The chat's effective official usage: no pin (or pinned official) rides
   * the official account's gift; anything else is the user's own provider. */
  function doctorPinnedAwayFromOfficial(): boolean {
    const pinnedId = deps.activeSession()?.providerId?.trim();
    return Boolean(pinnedId) && pinnedId !== OFFICIAL_PROVIDER_ID;
  }

  /** The built-in agent picks like everyone else now: the official group holds
   * two entries — the monthly gift (agent channel, doctor only) and the logged-in
   * official model line (purchased quota, every agent). */
  function renderDoctorMenu(): void {
    const quota = officialQuotaState(officialStatus);
    const onOfficial = !doctorPinnedAwayFromOfficial();
    const session = deps.activeSession();
    const pinnedModel = session?.model?.trim() ?? "";
    const giftLeft = quota.free?.left ?? 0;
    const paidLeft = quota.paid?.tokens ?? 0;
    // Effective default mirrors doctorOfficialFallback: no pin rides the gift
    // while it lasts, then the paid line once the gift drains.
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
    deps.modelMenuEl.appendChild(hint);

    const group = document.createElement("div");
    group.className = "chat-model-group";
    const label = document.createElement("div");
    label.className = "chat-model-group-label";
    label.textContent = t("chat.modelOfficialGroup");
    group.appendChild(label);
    deps.modelMenuEl.appendChild(group);

    if (!quota.signedIn) {
      const line = document.createElement("div");
      line.className = "chat-model-menu-hint";
      line.textContent = t("personal.officialNeedsLogin");
      deps.modelMenuEl.appendChild(line);
      group.appendChild(menuActionButton(t("chat.modelOfficialLoginCta"), startOfficialLogin));
      return;
    }

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
      deps.modelMenuEl.appendChild(empty);
      group.appendChild(menuActionButton(t("chat.modelOfficialBuyCta"), openOfficialAccountPage));
    }
  }

  /** One official entry, name only — quota lives on the chip button. */
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

  function renderModelMenu(): void {
    deps.modelMenuEl.replaceChildren();
    const isDoctor = deps.selectedRuntime() === "doctor";
    if (isDoctor) {
      renderDoctorMenu();
    }
    const catalog = subscriptionCatalog();
    const keepSaved = sessionKeepsSavedService(deps.activeSession());
    if (catalog && !keepSaved) {
      renderSubscriptionMenu(catalog);
      return;
    }
    if (catalog) renderSubscriptionMenu(catalog);
    const providers = deps.getProviders();
    const wired = deps.getWiredProvider();
    // The doctor menu already shows the official group up top, so an empty
    // provider list is not an empty menu for it.
    if (!isDoctor && providers.length === 0 && !wired) {
      const hint = document.createElement("div");
      hint.className = "chat-model-menu-hint";
      hint.textContent = t("chat.modelNeedProvider");
      deps.modelMenuEl.appendChild(hint);
      return;
    }
    const chosen = chosenModel();
    const rows = providers.length > 0 ? providers : wired ? [wired] : [];
    for (const provider of rows) {
      // Already painted by renderDoctorMenu for the built-in agent.
      if (isDoctor && provider.id === OFFICIAL_PROVIDER_ID) continue;
      const isOfficial = provider.id === OFFICIAL_PROVIDER_ID;
      let models = modelsForProviderUrl(provider.url, provider.model);
      // The official gateway picks the model itself; the stored id is a marker.
      if (isOfficial && models.length === 0) models = ["auto"];
      if (models.length === 0) continue;
      const group = document.createElement("div");
      group.className = "chat-model-group";
      const label = document.createElement("div");
      label.className = "chat-model-group-label";
      label.textContent = isOfficial
        ? t("chat.modelOfficialGroup")
        : provider.name.trim() || providerChipForUrl(provider.url, provider.name);
      if (provider.active) {
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
        btn.textContent = isOfficial ? t("chat.modelOfficialAutoModel") : model;
        btn.addEventListener("click", () => {
          selectSessionModel(provider, model);
        });
        group.appendChild(btn);
      }
      deps.modelMenuEl.appendChild(group);
    }
    if (!deps.modelMenuEl.childElementCount) {
      const hint = document.createElement("div");
      hint.className = "chat-model-menu-hint";
      hint.textContent = t("chat.modelNeedProvider");
      deps.modelMenuEl.appendChild(hint);
    }
  }

  function openModelMenu(): void {
    if (deps.modelBtnEl.disabled || deps.isComposerLocked()) return;
    if (deps.selectedRuntime() === "doctor") {
      void refreshOfficialStatus().then(() => {
        repaintAfterOfficialRefresh();
        if (deps.getModelMenuOpen()) positionModelMenu();
      });
    }
    const catalog = subscriptionCatalog();
    if (catalog && Date.now() - modelListTriedAt > 15000) {
      modelListTriedAt = Date.now();
      const runtime = deps.selectedRuntime();
      void refreshSubscriptionModels(runtime)
        .then(async () => {
          const again = await nativeAccountStatus(false);
          nativeRows = again.rows;
          renderModelPickerLabel();
          if (deps.getModelMenuOpen()) {
            renderModelMenu();
            positionModelMenu();
          }
        })
        .catch(() => {
          if ((subscriptionCatalog()?.models.length ?? 0) === 0) {
            deps.setStatus(t("chat.modelListMissing"), "warn");
          }
        });
    }
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
      const [status, doc, accounts] = await Promise.all([
        getPersonalProviderStatus(),
        listPersonalProviders(),
        nativeAccountStatus(false).catch(() => null),
      ]);
      if (accounts) nativeRows = accounts.rows;
      const missing = nativeRows.filter(
        (row) => row.using_subscription && (row.models ?? []).length === 0,
      );
      if (missing.length > 0 && Date.now() - modelListTriedAt > 15000) {
        modelListTriedAt = Date.now();
        await Promise.all(
          missing.map((row) => refreshSubscriptionModels(row.runtime_id).catch(() => [])),
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
          : status.configured && status.active_id
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
    void refreshOfficialStatus().then(repaintAfterOfficialRefresh);
    renderModelPickerLabel();
    if (deps.getModelMenuOpen()) renderModelMenu();
  }

  function selectSubscriptionModel(model: string): void {
    if (deps.isComposerLocked()) return;
    const next = model.trim();
    const catalog = subscriptionCatalog();
    if (!next || !catalog?.models.includes(next)) {
      closeModelMenu();
      return;
    }
    const current = alignSubscriptionSession(catalog);
    closeModelMenu();
    if (current === next) return;
    deps.pinSubscriptionModel(next);
    renderModelPickerLabel();
    deps.updateContextMeter();
    deps.setStatus(t("chat.modelSwitched", { model: `${catalog.label} · ${next}` }), "ok");
    deps.modelBtnEl.disabled = deps.isComposerLocked();
  }

  function selectSessionModel(provider: PersonalProviderListItem, model: string): void {
    if (deps.isComposerLocked()) return;
    const next = model.trim();
    if (!next) {
      closeModelMenu();
      return;
    }
    const current = chosenModel();
    closeModelMenu();
    if (current?.provider.id === provider.id && current.model === next) return;
    deps.pinSessionModel(provider, next);
    renderModelPickerLabel();
    deps.updateContextMeter();
    const name =
      provider.id === OFFICIAL_PROVIDER_ID
        ? t("chat.modelOfficialGroup")
        : providerChipForUrl(provider.url, provider.name);
    let modelText = next;
    if (provider.id === OFFICIAL_PROVIDER_ID) {
      modelText = next === OFFICIAL_GIFT_MODEL ? t("chat.officialGiftOption") : next;
    }
    deps.setStatus(t("chat.modelSwitched", { model: `${name} · ${modelText}` }), "ok");
    deps.modelBtnEl.disabled = deps.isComposerLocked() || !chosenModel();
  }

  function updateRuntimeLabel(): void {
    renderModelPickerLabel();
    deps.updateContextMeter();
  }

  // Login / sign-out elsewhere repaints the quota chip immediately.
  if (isPersonalEdition()) {
    void listen("teamups-account-changed", () => {
      officialFetchedAt = 0;
      void refreshOfficialStatus().then(repaintAfterOfficialRefresh);
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
    /**
     * Where an unpinned built-in-agent chat's next message would go: the gift
     * while it lasts, the paid official model line once the gift drains. The
     * send path injects this so the chip's promise (official quota shown) is
     * what actually gets billed — no pin required, no session state mutated.
     */
    doctorOfficialFallback: () => {
      if (deps.selectedRuntime() !== "doctor") return null;
      if (deps.activeSession()?.providerId?.trim()) return null;
      const quota = officialQuotaState(officialStatus);
      if (!quota.signedIn) return null;
      const giftLeft = quota.free?.left ?? 0;
      if (giftLeft > 0) return null; // 未 pin 默认走赠送，Rust 侧已兜底
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
    /** A finished built-in-agent turn just changed the server-side quota. */
    refreshOfficialQuota: () => {
      officialFetchedAt = 0;
      void refreshOfficialStatus().then(repaintAfterOfficialRefresh);
    },
    hasChoices: () =>
      subscriptionCatalog() !== null ||
      deps.selectedRuntime() === "doctor" ||
      deps.getProviders().length > 0 ||
      Boolean(deps.getWiredProvider()),
  };
}
