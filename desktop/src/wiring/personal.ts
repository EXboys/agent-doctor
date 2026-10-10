
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { appState } from "../app-state";
import { isPersonalEdition } from "../edition";
import { escapeHtml } from "../format";
import { formatProviderFailure, teamupsLoginFailure, withProviderFailure } from "../friendly-error";
import { getLocale, t } from "../i18n";
import { mergeLiveModels } from "../provider-models";
import { PROVIDER_PRESETS } from "../provider-presets";
import { DEFAULT_TEAMUPS_BASE_URL, setTeamupsBaseUrl } from "../teamups-site";
import type {
  PersonalProviderListItem,
  PersonalProvidersDocument,
  PersonalProviderStatus,
  ProviderProtocol,
  TeamupsAccountStatus,
} from "../types";
import type { PresetsApi } from "./presets";
import {
  activatePersonalProvider,
  deletePersonalProvider,
  getPersonalProviderStatus,
  listPersonalProviders,
  pollTeamupsLogin,
  signOutTeamups,
  startTeamupsLogin,
  teamupsAccountStatus,
  upsertPersonalProvider as upsertPersonalProviderCommand,
  verifyPersonalProvider as verifyPersonalProviderCommand,
} from "../ipc";

const personalSectionEl = document.querySelector<HTMLElement>("#personal-section")!;
const personalListViewEl = document.querySelector<HTMLElement>("#personal-list-view")!;
const personalFormViewEl = document.querySelector<HTMLElement>("#personal-form-view")!;
const personalUsageViewEl = document.querySelector<HTMLElement>("#personal-usage-view");
const personalStatusEl = document.querySelector<HTMLElement>("#personal-status")!;
const personalConnectedEl = document.querySelector<HTMLElement>("#personal-connected")!;
const personalConnectedUrlEl = document.querySelector<HTMLElement>("#personal-connected-url");
const personalConnectedMetaEl = document.querySelector<HTMLElement>("#personal-connected-meta");
const personalListEl = document.querySelector<HTMLUListElement>("#personal-list")!;
const personalListHintEl = document.querySelector<HTMLElement>("#personal-list-hint")!;
const personalFormEl = document.querySelector<HTMLFormElement>("#personal-form")!;
const personalFormTitleEl = document.querySelector<HTMLElement>("#personal-form-title")!;
const personalIdEl = document.querySelector<HTMLInputElement>("#personal-id")!;
const personalNameEl = document.querySelector<HTMLInputElement>("#personal-name")!;
const personalUrlEl = document.querySelector<HTMLInputElement>("#personal-url")!;
const personalKeyEl = document.querySelector<HTMLInputElement>("#personal-key")!;
const personalModelEl = document.querySelector<HTMLInputElement>("#personal-model")!;
const personalModelSelectEl = document.querySelector<HTMLSelectElement>("#personal-model-select");
const personalProtocolEl = document.querySelector<HTMLSelectElement>("#personal-protocol")!;
const personalPresetEl = document.querySelector<HTMLSelectElement>("#personal-preset")!;
const personalAddEl = document.querySelector<HTMLButtonElement>("#personal-add")!;
const personalBackEl = document.querySelector<HTMLButtonElement>("#personal-back")!;
const personalVerifyEl = document.querySelector<HTMLButtonElement>("#personal-verify")!;
const personalSaveEl = document.querySelector<HTMLButtonElement>("#personal-save")!;
const personalApplyEl = document.querySelector<HTMLButtonElement>("#personal-apply")!;
const personalHintEl = document.querySelector<HTMLElement>("#personal-hint")!;

const OFFICIAL_PROVIDER_ID = "teamups-official";
let officialAccount: TeamupsAccountStatus | null = null;
let officialLoginTimer: ReturnType<typeof setTimeout> | null = null;
let officialBalanceTimer: ReturnType<typeof setTimeout> | null = null;
let officialBalanceWatch: { until: number; leftApp: boolean; baseline: string } | null = null;

function stopOfficialLoginPoll(): void {
  if (officialLoginTimer != null) {
    clearTimeout(officialLoginTimer);
    officialLoginTimer = null;
  }
}

/** Personal edition always shows 官方, even before sign-in creates the saved row. */
function providersForDisplay(doc: PersonalProvidersDocument): PersonalProviderListItem[] {
  if (!isPersonalEdition() || doc.providers.some((item) => item.id === OFFICIAL_PROVIDER_ID)) {
    return doc.providers;
  }
  const placeholder: PersonalProviderListItem = {
    id: OFFICIAL_PROVIDER_ID,
    name: t("personal.officialName"),
    url: officialAccount?.base_url || DEFAULT_TEAMUPS_BASE_URL,
    model: "",
    protocol: "openai",
    api_key_hint: "",
    active: false,
  };
  return [placeholder, ...doc.providers];
}

function formatOfficialTokens(tokens: number): string {
  if (!Number.isFinite(tokens) || tokens <= 0) return "0";
  return new Intl.NumberFormat(getLocale() === "zh" ? "zh-CN" : "en", {
    notation: tokens >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(Math.floor(tokens));
}

const AVATAR_TONES = 6;

function avatarTone(key: string): number {
  let hash = 0;
  for (const ch of key) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return hash % AVATAR_TONES;
}

/** Signed in and has trial calls or tokens left. Unknown until the account loads. */
function officialUsable(): boolean {
  const official = officialAccount?.official;
  if (!officialAccount) return true;
  if (!officialAccount.signed_in || !official) return false;
  const trials = Math.max(0, official.trial_calls_left ?? 0);
  const tokens = Math.max(0, official.tokens_remaining ?? 0);
  const hasTokenBalance = official.via === "member" || (official.token_cap ?? 0) > 0 || tokens > 0;
  const onTrial = official.via === "trial";
  return official.active && (onTrial ? trials > 0 || tokens > 0 : hasTokenBalance && tokens > 0);
}

function officialDescription(): string {
  const official = officialAccount?.official;
  if (!officialAccount?.signed_in) return t("personal.officialNeedsLogin");
  if (!official) return t("personal.officialNeedsMembership");

  const trials = Math.max(0, official.trial_calls_left ?? 0);
  const tokens = Math.max(0, official.tokens_remaining ?? 0);
  const hasTokenBalance = official.via === "member" || (official.token_cap ?? 0) > 0 || tokens > 0;
  const onTrial = official.via === "trial";
  const usable = officialUsable();
  if (!onTrial && !hasTokenBalance) return t("personal.officialNeedsMembership");

  const parts = [usable ? t("personal.officialUsable") : t("personal.officialUnusable")];
  if (onTrial && (trials > 0 || !hasTokenBalance)) {
    parts.push(t("personal.officialTrialLeft", { count: String(trials) }));
  }
  if (hasTokenBalance) {
    parts.push(t("personal.officialTokensLeft", { remaining: formatOfficialTokens(tokens) }));
  }
  return parts.join(" · ");
}

export type PersonalDeps = {
  presets: PresetsApi;
  refresh: () => Promise<void>;
  loadModeStatus: () => Promise<void>;
  /** This saved service's own tokens, or null when it has none this month. */
  usageFor?: (id: string, name: string) => { today: number; month: number } | null;
};

export type PersonalApi = ReturnType<typeof createPersonalController>;

export function createPersonalController(deps: PersonalDeps) {
  const { presets } = deps;

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

  function renderPersonalProviderList(doc: PersonalProvidersDocument) {
    personalListEl.innerHTML = "";
    const providers = providersForDisplay(doc);
    const officialSaved = doc.providers.some((item) => item.id === OFFICIAL_PROVIDER_ID);
    if (providers.length === 0) {
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
    for (const item of providers) {
      const officialProvider = item.id === OFFICIAL_PROVIDER_ID;
      const officialSignedIn = officialProvider && officialAccount?.signed_in === true;
      const officialNeedsLogin =
        officialProvider && (!officialSaved || officialAccount?.signed_in === false);
      const officialName = officialAccount?.name?.trim() || "";
      const routingActive = item.active && personalModeActive;
      const presetId = presets.matchPresetId(item.name, item.url, item.protocol);
      const brand =
        officialProvider
          ? t("personal.officialName")
          : presetId !== "custom"
          ? PROVIDER_PRESETS[presetId]?.chip ?? PROVIDER_PRESETS[presetId]?.name ?? item.name
          : item.name.trim() || t("personal.presetCustom");
      const titleText = officialProvider ? t("personal.officialName") : item.name.trim() || brand;

      // A click on the card itself switches to it, or signs in for 官方.
      const cardAction = officialNeedsLogin
        ? "official-login"
        : item.active
          ? ""
          : "activate-provider";

      const li = document.createElement("li");
      li.className = `provider-item${routingActive ? " is-active" : ""}${
        cardAction ? " is-pickable" : ""
      }${switchingId === item.id ? " is-switching" : ""}`;
      li.dataset.providerId = item.id;
      if (cardAction) {
        li.dataset.action = cardAction;
        li.tabIndex = 0;
        li.setAttribute("role", "button");
        li.title =
          cardAction === "official-login"
            ? t("personal.cardLoginHint")
            : t("personal.cardSwitchHint", { name: titleText });
      }

      const main = document.createElement("div");
      main.className = "provider-item-main";

      const avatar = document.createElement("span");
      avatar.className = "provider-avatar";
      avatar.dataset.tone = String(avatarTone(officialProvider ? "official" : presetId || brand));
      avatar.textContent = Array.from(brand.trim() || "?")[0].toUpperCase();
      avatar.setAttribute("aria-hidden", "true");

      const title = document.createElement("p");
      title.className = "provider-item-title";
      const titleName = document.createElement("span");
      titleName.className = "provider-item-name";
      titleName.textContent =
        officialSignedIn && officialName ? `${titleText} · ${officialName}` : titleText;
      title.appendChild(titleName);
      if (routingActive) {
        const badge = document.createElement("span");
        badge.className = "provider-badge";
        badge.textContent = t("personal.activeBadge");
        title.appendChild(badge);
      }

      // Second line: model, then what this service has used (or 官方's balance).
      const sub = document.createElement("p");
      sub.className = "provider-item-sub";
      if (item.model.trim()) {
        const model = document.createElement("span");
        model.className = "provider-item-meta";
        model.textContent = item.model;
        sub.appendChild(model);
      }
      const used = deps.usageFor?.(item.id, item.name) ?? null;
      const note =
        switchingId === item.id
          ? t("personal.switching")
          : officialProvider
            ? officialNeedsLogin
              ? t("personal.officialNeedsLogin")
              : officialDescription()
            : used
              ? t("personal.itemUsage", {
                  today: formatOfficialTokens(used.today),
                  month: formatOfficialTokens(used.month),
                })
              : "";
      if (note) {
        const noteEl = document.createElement("span");
        noteEl.className = "provider-item-note";
        noteEl.textContent = note;
        noteEl.title = note;
        sub.appendChild(noteEl);
      }
      if (!sub.childElementCount) sub.hidden = true;

      main.append(title, sub);

      const actions = document.createElement("div");
      actions.className = "provider-item-actions";

      if (officialNeedsLogin) {
        const loginBtn = document.createElement("button");
        loginBtn.type = "button";
        loginBtn.className = "btn-primary btn-compact";
        loginBtn.dataset.action = "official-login";
        loginBtn.dataset.providerId = item.id;
        loginBtn.textContent = t("personal.officialLogin");
        actions.appendChild(loginBtn);
      } else if (officialProvider && !officialUsable()) {
        // Out of balance: the way to fix it stays in sight, not in the menu.
        const membershipBtn = document.createElement("button");
        membershipBtn.type = "button";
        membershipBtn.className = "btn-secondary btn-compact";
        membershipBtn.dataset.action = "official-membership";
        membershipBtn.dataset.providerId = item.id;
        membershipBtn.textContent = t("personal.officialOpen");
        actions.appendChild(membershipBtn);
      } else {
        const pick = document.createElement("span");
        pick.className = "provider-pick";
        pick.setAttribute("aria-hidden", "true");
        actions.appendChild(pick);
      }

      const menuItems: Array<{ action: string; label: string; danger?: boolean }> =
        officialProvider
          ? [
              ...(officialSignedIn
                ? [{ action: "official-switch", label: t("personal.officialSwitch") }]
                : []),
              {
                action: "official-membership",
                label:
                  officialAccount?.official?.via === "member"
                    ? t("personal.officialManage")
                    : t("personal.officialOpen"),
              },
            ]
          : [
              { action: "edit-provider", label: t("personal.edit") },
              { action: "delete-provider", label: t("personal.delete"), danger: true },
            ];

      const menuBtn = document.createElement("button");
      menuBtn.type = "button";
      menuBtn.className = "provider-menu-btn";
      menuBtn.dataset.action = "open-menu";
      menuBtn.dataset.providerId = item.id;
      menuBtn.setAttribute("aria-haspopup", "menu");
      menuBtn.setAttribute("aria-label", t("personal.moreActions"));
      menuBtn.title = t("personal.moreActions");
      menuBtn.textContent = "⋯";

      const menu = document.createElement("div");
      menu.className = "provider-menu";
      menu.setAttribute("role", "menu");
      menu.hidden = true;
      for (const entry of menuItems) {
        const option = document.createElement("button");
        option.type = "button";
        option.setAttribute("role", "menuitem");
        option.className = `provider-menu-item${entry.danger ? " is-danger" : ""}`;
        option.dataset.action = entry.action;
        option.dataset.providerId = item.id;
        option.textContent = entry.label;
        menu.appendChild(option);
      }
      actions.append(menuBtn, menu);

      li.append(avatar, main, actions);
      personalListEl.appendChild(li);
    }
  }

  let switchInFlight = false;
  let switchingId: string | null = null;

  function closeProviderMenus(except?: HTMLElement | null): void {
    personalListEl.querySelectorAll<HTMLElement>(".provider-menu").forEach((menu) => {
      if (menu === except) return;
      menu.hidden = true;
      menu.closest(".provider-item")?.classList.remove("is-menu-open");
      const del = menu.querySelector<HTMLButtonElement>('[data-action="confirm-delete"]');
      if (del) {
        del.dataset.action = "delete-provider";
        del.textContent = t("personal.delete");
      }
    });
  }

  async function loadPersonalProviderList() {
    const [status, doc] = await Promise.all([
      getPersonalProviderStatus(),
      listPersonalProviders(),
    ]);
    appState.personalProvidersDoc = doc;
    renderPersonalProviderStatus(status);
    renderPersonalProviderList(doc);
  }

  function officialBalanceKey(): string {
    const official = officialAccount?.official;
    if (!official) return "";
    return `${official.via}:${official.trial_calls_left}:${official.tokens_remaining}:${official.token_cap}`;
  }

  function stopOfficialBalanceWatch(): void {
    officialBalanceWatch = null;
    if (officialBalanceTimer != null) {
      clearTimeout(officialBalanceTimer);
      officialBalanceTimer = null;
    }
  }

  function noteOfficialBalanceRefresh(): void {
    const watch = officialBalanceWatch;
    if (!watch) return;
    const official = officialAccount?.official;
    const credited =
      official?.via === "member" ||
      (official?.tokens_remaining ?? 0) > 0 ||
      (official?.token_cap ?? 0) > 0;
    if (credited || officialBalanceKey() !== watch.baseline) {
      if (
        personalListHintEl.textContent === t("personal.officialBalancePending") ||
        personalListHintEl.textContent === t("personal.officialBalanceWatch")
      ) {
        personalListHintEl.hidden = true;
        personalListHintEl.textContent = "";
      }
      stopOfficialBalanceWatch();
      return;
    }
    if (Date.now() >= watch.until) {
      stopOfficialBalanceWatch();
    }
    if (watch.leftApp && officialAccount?.signed_in) {
      personalListHintEl.hidden = false;
      personalListHintEl.textContent = t("personal.officialBalancePending");
    }
  }

  function scheduleOfficialBalancePoll(): void {
    if (officialBalanceTimer != null) clearTimeout(officialBalanceTimer);
    const watch = officialBalanceWatch;
    if (!watch || Date.now() >= watch.until) {
      officialBalanceTimer = null;
      return;
    }
    officialBalanceTimer = setTimeout(() => {
      officialBalanceTimer = null;
      void refreshOfficialAccount();
    }, 4000);
  }

  function startOfficialBalanceWatch(): void {
    officialBalanceWatch = {
      until: Date.now() + 3 * 60 * 1000,
      leftApp: document.visibilityState === "hidden",
      baseline: officialBalanceKey(),
    };
    personalListHintEl.hidden = false;
    personalListHintEl.textContent = t("personal.officialBalanceWatch");
    scheduleOfficialBalancePoll();
  }

  let officialRefreshInFlight = false;

  async function refreshOfficialAccount() {
    if (officialRefreshInFlight) return;
    officialRefreshInFlight = true;
    try {
      const account = await teamupsAccountStatus().catch(() => null);
      if (!account) return;
      officialAccount = account;
      setTeamupsBaseUrl(account.base_url);
      const doc = appState.personalProvidersDoc;
      if (doc) renderPersonalProviderList(doc);
      noteOfficialBalanceRefresh();
      if (officialBalanceWatch) scheduleOfficialBalancePoll();
    } finally {
      officialRefreshInFlight = false;
    }
  }

  async function loadPersonalProviderStatus() {
    try {
      await loadPersonalProviderList();
      await refreshOfficialAccount();
    } catch (error) {
      personalStatusEl.textContent = withProviderFailure("personal.applyFailed", error);
    }
  }

  async function pollOfficialLogin(
    deviceCode: string,
    intervalSec: number,
    expiresAt: number,
  ): Promise<void> {
    if (Date.now() >= expiresAt) {
      personalListHintEl.hidden = false;
      personalListHintEl.textContent = t("resources.mallLoginExpired");
      return;
    }
    try {
      const poll = await pollTeamupsLogin({ deviceCode });
      if (poll.status === "pending") {
        officialLoginTimer = setTimeout(() => {
          void pollOfficialLogin(deviceCode, intervalSec, expiresAt);
        }, Math.max(1, intervalSec) * 1000);
        return;
      }
      if (poll.status === "approved") {
        personalListHintEl.hidden = false;
        personalListHintEl.textContent = t("resources.mallLoginOk");
        await loadPersonalProviderStatus();
        return;
      }
      personalListHintEl.hidden = false;
      personalListHintEl.textContent = t("resources.mallLoginExpired");
    } catch (error) {
      personalListHintEl.hidden = false;
      personalListHintEl.textContent = teamupsLoginFailure(error);
    }
  }

  async function switchOfficialAccount(): Promise<void> {
    stopOfficialLoginPoll();
    stopOfficialBalanceWatch();
    personalListHintEl.hidden = false;
    personalListHintEl.textContent = t("personal.officialSwitching");
    try {
      officialAccount = await signOutTeamups();
      setTeamupsBaseUrl(officialAccount?.base_url);
      const doc = appState.personalProvidersDoc;
      if (doc) renderPersonalProviderList(doc);
      await startOfficialLogin();
    } catch (error) {
      personalListHintEl.hidden = false;
      personalListHintEl.textContent = teamupsLoginFailure(error);
    }
  }

  async function startOfficialLogin(): Promise<void> {
    stopOfficialLoginPoll();
    personalListHintEl.hidden = false;
    personalListHintEl.textContent = t("resources.mallLoggingIn");
    try {
      const started = await startTeamupsLogin();
      await openUrl(started.verification_url);
      await pollOfficialLogin(
        started.device_code,
        started.interval_sec,
        Date.now() + Math.max(30, started.expires_in_sec) * 1000,
      );
    } catch (error) {
      personalListHintEl.hidden = false;
      personalListHintEl.textContent = teamupsLoginFailure(error);
    }
  }

  function settleAfterProviderSwitch() {
    void refreshOfficialAccount().finally(() => {
      void deps.loadModeStatus();
      void deps.refresh();
    });
  }

  function showPersonalListView() {
    personalListViewEl.hidden = false;
    personalFormViewEl.hidden = true;
    if (personalUsageViewEl) personalUsageViewEl.hidden = true;
  }

  function showPersonalFormView(mode: "add" | "edit") {
    personalListViewEl.hidden = true;
    personalFormViewEl.hidden = false;
    if (personalUsageViewEl) personalUsageViewEl.hidden = true;
    personalFormTitleEl.textContent =
      mode === "edit" ? t("personal.formEdit") : t("personal.formAdd");
    setPersonalHint("hide");
  }

  function setPersonalHint(
    tone: "ok" | "error" | "busy" | "info" | "hide",
    message = "",
  ): void {
    personalHintEl.classList.remove("is-ok", "is-error", "is-busy", "is-info");
    if (tone === "hide" || !message) {
      personalHintEl.hidden = true;
      personalHintEl.textContent = "";
      return;
    }
    personalHintEl.hidden = false;
    personalHintEl.textContent = message;
    personalHintEl.classList.add(`is-${tone}`);
  }

  function resetPersonalForm() {
    personalIdEl.value = "";
    personalNameEl.value = "";
    personalUrlEl.value = "";
    personalKeyEl.value = "";
    personalModelEl.value = "";
    personalProtocolEl.value = "openai";
    personalKeyEl.placeholder = "sk-…";
    presets.applyProviderPreset("deepseek");
  }

  function fillPersonalForm(item: PersonalProviderListItem) {
    personalIdEl.value = item.id;
    personalNameEl.value = item.name;
    personalUrlEl.value = item.url;
    personalModelEl.value = item.model;
    personalProtocolEl.value = item.protocol === "anthropic" ? "anthropic" : "openai";
    personalKeyEl.value = "";
    personalKeyEl.placeholder = t("personal.keyKeepHint");
    const presetId = presets.matchPresetId(item.name, item.url, item.protocol);
    if (presetId === "custom") {
      presets.applyProviderPreset("custom");
      personalNameEl.value = item.name;
      personalUrlEl.value = item.url;
      personalProtocolEl.value = item.protocol === "anthropic" ? "anthropic" : "openai";
      personalModelEl.value = item.model;
    } else {
      presets.applyProviderPreset(presetId, { forceModel: false });
      personalNameEl.value = item.name;
      personalUrlEl.value = item.url;
      personalModelEl.value = item.model;
      presets.syncGlmPlan();
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
    presets.syncModelFromSelect();
    const id = personalIdEl.value.trim() || null;
    const name = personalNameEl.value.trim();
    const url = personalUrlEl.value.trim();
    const key = personalKeyEl.value.trim();
    const model = personalModelEl.value.trim();
    const protocol: ProviderProtocol =
      personalProtocolEl.value === "anthropic" ? "anthropic" : "openai";
    if (!name || !url || !model || (requireKey && !key && !id)) {
      setPersonalHint("error", t("personal.missingFields"));
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
    personalListEl.classList.toggle("is-busy", busy);
  }

  async function verifyPersonalProvider() {
    const values = personalFormValues(true);
    if (!values) {
      setPersonalHint("error", t("personal.missingFields"));
      return;
    }
    if (!values.key) {
      setPersonalHint("error", t("personal.missingFields"));
      return;
    }
    setPersonalBusy(true);
    setPersonalHint("busy", t("personal.verifying"));
    try {
      const report = await verifyPersonalProviderCommand({
        url: values.url,
        key: values.key,
        protocol: values.protocol,
      });
      if (report.ok) {
        if (report.resolved_url) {
          personalUrlEl.value = report.resolved_url;
          presets.syncGlmPlan();
          setPersonalHint("ok", t("personal.glmPlanSwitched"));
          return;
        }
        const presetId = personalPresetEl.value;
        const base =
          presetId !== "custom" && PROVIDER_PRESETS[presetId]
            ? presets.modelsForPresetId(presetId)
            : presets.modelsForCustomProtocol(personalProtocolEl.value);
        if (report.models_sample.length > 0) {
          presets.setModelSuggestions(
            mergeLiveModels(base, report.models_sample, personalModelEl.value),
          );
        }
        setPersonalHint("ok", t("personal.verifyOk"));
      } else {
        setPersonalHint(
          "error",
          formatProviderFailure(report.message, {
            statusCode: report.status_code,
          }),
        );
      }
    } catch (error) {
      setPersonalHint("error", withProviderFailure("personal.verifyFailed", error));
    } finally {
      setPersonalBusy(false);
    }
  }

  async function upsertPersonalProvider(activate: boolean) {
    const editing = Boolean(personalIdEl.value.trim());
    const values = personalFormValues(!editing);
    if (!values) {
      setPersonalHint("error", t("personal.missingFields"));
      return;
    }
    setPersonalBusy(true);
    setPersonalHint("busy", activate ? t("personal.applying") : t("personal.saving"));
    try {
      if (activate) {
        // Save first without activate, then activate for a proper setup report.
        const doc = await upsertPersonalProviderCommand({
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
        const report = await activatePersonalProvider({ id: targetId });
        personalKeyEl.value = "";
        await loadPersonalProviderList();
        personalListHintEl.hidden = false;
        personalListHintEl.textContent = t("personal.applyOk", {
          name: report.provider_name ?? values.name,
        });
        resetPersonalForm();
        setPersonalHint("hide");
        showPersonalListView();
        settleAfterProviderSwitch();
      } else {
        await upsertPersonalProviderCommand({
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
        await deps.loadModeStatus();
        personalListHintEl.hidden = false;
        personalListHintEl.textContent = t("personal.saveOk", { name: values.name });
        resetPersonalForm();
        setPersonalHint("hide");
        showPersonalListView();
      }
    } catch (error) {
      setPersonalHint("error", withProviderFailure("personal.applyFailed", error));
    } finally {
      setPersonalBusy(false);
    }
  }

  async function activateProviderById(id: string) {
    if (switchInFlight) return;
    switchInFlight = true;
    switchingId = id;
    if (appState.personalProvidersDoc) renderPersonalProviderList(appState.personalProvidersDoc);
    setPersonalBusy(true);
    personalListHintEl.hidden = false;
    personalListHintEl.textContent = t("personal.applying");
    try {
      const report = await activatePersonalProvider({
        id,
      });
      switchingId = null;
      await loadPersonalProviderList();
      personalListHintEl.textContent = t("personal.applyOk", {
        name: report.provider_name ?? id,
      });
      settleAfterProviderSwitch();
    } catch (error) {
      personalListHintEl.textContent = withProviderFailure("personal.applyFailed", error);
    } finally {
      switchInFlight = false;
      if (switchingId) {
        switchingId = null;
        if (appState.personalProvidersDoc) {
          renderPersonalProviderList(appState.personalProvidersDoc);
        }
      }
      setPersonalBusy(false);
    }
  }

  async function deleteProviderById(id: string) {
    setPersonalBusy(true);
    try {
      const doc = await deletePersonalProvider({
        id,
      });
      appState.personalProvidersDoc = doc;
      await loadPersonalProviderStatus();
      personalListHintEl.textContent = t("personal.deleteOk");
      showPersonalListView();
    } catch (error) {
      personalListHintEl.textContent = withProviderFailure("personal.applyFailed", error);
    } finally {
      setPersonalBusy(false);
    }
  }

  function bindEvents() {
    personalAddEl.addEventListener("click", () => {
      resetPersonalForm();
      showPersonalFormView("add");
    });

    personalBackEl.addEventListener("click", () => {
      resetPersonalForm();
      showPersonalListView();
    });

    personalModelSelectEl?.addEventListener("change", () => {
      presets.syncModelFromSelect();
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
      presets.applyProviderPreset(personalPresetEl.value, { forceModel: true });
      presets.focusAfterPreset(personalPresetEl.value);
    });

    presets.els.personalPresetPickerEl?.addEventListener("click", (event) => {
      const target = event.target as HTMLElement | null;
      const chip = target?.closest<HTMLButtonElement>(".provider-chip");
      if (!chip?.dataset.presetId) return;
      const presetId = chip.dataset.presetId;
      presets.applyProviderPreset(presetId, { forceModel: true });
      presets.focusAfterPreset(presetId);
    });

    personalProtocolEl.addEventListener("change", () => {
      presets.maybePromoteToCustomFromProtocol();
    });

    personalUrlEl.addEventListener("change", () => {
      presets.maybePromoteToCustomFromUrl();
    });

    void listen("teamups-account-changed", () => {
      void refreshOfficialAccount();
    });

    document.addEventListener("visibilitychange", () => {
      if (officialBalanceWatch && document.visibilityState === "hidden") {
        officialBalanceWatch.leftApp = true;
        return;
      }
      if (document.visibilityState === "visible") void refreshOfficialAccount();
    });
    window.addEventListener("focus", () => {
      void refreshOfficialAccount();
    });

    document.addEventListener("click", (event) => {
      if (!(event.target as HTMLElement).closest(".provider-menu, .provider-menu-btn")) {
        closeProviderMenus();
      }
    });
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape") closeProviderMenus();
    });

    personalListEl.addEventListener("keydown", (event) => {
      const card = event.target as HTMLElement;
      if ((event.key === "Enter" || event.key === " ") && card.matches("li.is-pickable")) {
        event.preventDefault();
        card.click();
      }
    });

    personalListEl.addEventListener("click", (event) => {
      const button = (event.target as HTMLElement).closest<HTMLElement>("[data-action]");
      const action = button?.dataset.action;
      const id = button?.dataset.providerId;
      if (!action || !id) {
        return;
      }
      if (action === "open-menu") {
        const menu = button.parentElement?.querySelector<HTMLElement>(".provider-menu") ?? null;
        const opening = Boolean(menu?.hidden);
        closeProviderMenus();
        if (menu && opening) {
          menu.hidden = false;
          menu.closest(".provider-item")?.classList.add("is-menu-open");
        }
        return;
      }
      if (button.classList.contains("provider-menu-item") && action !== "delete-provider") {
        closeProviderMenus();
      }
      if (action === "delete-provider") {
        // First press asks; the same spot confirms, so a stray click cannot delete.
        button.dataset.action = "confirm-delete";
        button.textContent = t("personal.deleteConfirm");
        return;
      }
      if (action === "confirm-delete") {
        closeProviderMenus();
        void deleteProviderById(id);
        return;
      }
      if (action === "activate-provider") {
        void activateProviderById(id);
        return;
      }
      if (action === "official-login") {
        void startOfficialLogin();
        return;
      }
      if (action === "official-switch") {
        void switchOfficialAccount();
        return;
      }
      if (action === "official-membership") {
        const base = officialAccount?.base_url?.replace(/\/+$/, "") || DEFAULT_TEAMUPS_BASE_URL;
        startOfficialBalanceWatch();
        void openUrl(`${base}/membership`);
        return;
      }
      if (action === "edit-provider") {
        const item = appState.personalProvidersDoc?.providers.find((p) => p.id === id);
        if (item) {
          fillPersonalForm(item);
          showPersonalFormView("edit");
          setPersonalHint("info", t("personal.keyKeepHint"));
        }
        return;
      }
    });
  }

  return {
    loadPersonalProviderStatus,
    renderPersonalProviderList,
    showPersonalListView,
    bindEvents,
  };
}
