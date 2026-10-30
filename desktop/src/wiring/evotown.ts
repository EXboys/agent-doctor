
import { setAgentBrandIcon } from "../agent-brand";
import { isTeamEdition } from "../edition";
import { withErrorDetail } from "../friendly-error";
import { t, type MessageKey } from "../i18n";
import type {
  EvotownStatus,
  SubscriptionPoolRow,
} from "../types";
import { getEngineRegisterStatus, getEvotownStatus, listSubscriptionPool, reclaimSubscription, runEvotownOnboarding as runEvotownOnboardingCommand, runEngineRegister as runEngineRegisterCommand, runSync, setSubscriptionCap, setSubscriptionDisabled, setSubscriptionProxy, startSubscriptionLogin } from "../ipc";

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
const subscriptionPoolEl = document.querySelector<HTMLElement>("#subscription-pool")!;
const subscriptionPoolEmptyEl = document.querySelector<HTMLElement>("#subscription-pool-empty")!;
const subscriptionPoolListEl = document.querySelector<HTMLElement>("#subscription-pool-list")!;
const subscriptionPoolResultEl = document.querySelector<HTMLElement>("#subscription-pool-result")!;
const subscriptionAddEl = document.querySelector<HTMLButtonElement>("#subscription-add")!;
const subscriptionEmptyClaudeEl = document.querySelector<HTMLButtonElement>("#subscription-empty-claude")!;
const subscriptionEmptyCodexEl = document.querySelector<HTMLButtonElement>("#subscription-empty-codex")!;
const subscriptionLoginEl = document.querySelector<HTMLElement>("#subscription-login")!;
const subscriptionLoginBackEl = document.querySelector<HTMLButtonElement>("#subscription-login-back")!;
const subscriptionLoginClaudeEl = document.querySelector<HTMLButtonElement>("#subscription-login-claude")!;
const subscriptionLoginCodexEl = document.querySelector<HTMLButtonElement>("#subscription-login-codex")!;
const subscriptionLoginNameEl = document.querySelector<HTMLInputElement>("#subscription-login-name")!;
const subscriptionLoginStartEl = document.querySelector<HTMLButtonElement>("#subscription-login-start")!;
const subscriptionLoginResultEl = document.querySelector<HTMLElement>("#subscription-login-result")!;
const personalListViewEl = document.querySelector<HTMLElement>("#personal-list-view")!;
const subscriptionDetailEl = document.querySelector<HTMLElement>("#subscription-detail")!;
const subscriptionDetailBackEl = document.querySelector<HTMLButtonElement>("#subscription-detail-back")!;
const subscriptionDetailNameEl = document.querySelector<HTMLElement>("#subscription-detail-name")!;
const subscriptionDetailStatusEl = document.querySelector<HTMLElement>("#subscription-detail-status")!;
const subscriptionDetailMetaEl = document.querySelector<HTMLElement>("#subscription-detail-meta")!;
const subscriptionDetailHolderEl = document.querySelector<HTMLElement>("#subscription-detail-holder")!;
const subscriptionProxyEl = document.querySelector<HTMLInputElement>("#subscription-proxy")!;
const subscriptionCapEl = document.querySelector<HTMLInputElement>("#subscription-cap")!;
const subscriptionProxySaveEl = document.querySelector<HTMLButtonElement>("#subscription-proxy-save")!;
const subscriptionCapSaveEl = document.querySelector<HTMLButtonElement>("#subscription-cap-save")!;
const subscriptionDisableEl = document.querySelector<HTMLButtonElement>("#subscription-disable")!;
const subscriptionReclaimEl = document.querySelector<HTMLButtonElement>("#subscription-reclaim")!;
const subscriptionDetailResultEl = document.querySelector<HTMLElement>("#subscription-detail-result")!;

export type EvotownDeps = {
  refresh: () => Promise<void>;
  loadSkillsInventory: () => Promise<void>;
  hideSkillsInventory: () => void;
  loadModeStatus: () => Promise<void>;
};

export type EvotownApi = ReturnType<typeof createEvotownController>;

export function createEvotownController(deps: EvotownDeps) {
  let lastEvotownStatus: EvotownStatus | null = null;

  async function loadEngineRegisterStatus() {
    try {
      const status = await getEngineRegisterStatus();
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

  function renderFixedCompanyHost() {
    const host = "https://agentdoctor.cn";
    evotownFormEl.hidden = true;
    evotownEngineEl.hidden = true;
    evotownResyncEl.hidden = true;
    evotownHintEl.textContent = "";
    evotownStatusEl.hidden = true;
    evotownConnectedEl.hidden = false;
    evotownSectionEl.classList.add("is-connected");
    evotownConnectedUrlEl.textContent = host;
    evotownConnectedMetaEl.textContent = t("evotown.fixedHost");
    if (evotownBadgeEl) {
      evotownBadgeEl.hidden = false;
      evotownBadgeEl.className = "badge ok";
      evotownBadgeEl.textContent = t("evotown.connectedBadge");
    }
  }

  function renderEvotownStatus(status: EvotownStatus, opts?: { refreshSkills?: boolean }) {
    lastEvotownStatus = status;
    if (isTeamEdition()) {
      renderFixedCompanyHost();
      return;
    }
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

  async function loadEvotownStatus() {
    try {
      const status = await getEvotownStatus();
      renderEvotownStatus(status);
    } catch (error) {
      if (isTeamEdition()) {
        renderFixedCompanyHost();
      } else {
        evotownStatusEl.textContent = withErrorDetail(t("evotown.connectFailed"), error);
      }
    }
    if (isTeamEdition()) {
      subscriptionPoolEl.hidden = false;
      await loadSubscriptionPool();
    } else {
      subscriptionPoolEl.hidden = true;
    }
  }

  let poolRows: SubscriptionPoolRow[] = [];
  let selectedPoolName: string | null = null;
  let loginRuntime = "";

  function poolStatusKey(row: SubscriptionPoolRow): MessageKey {
    if (row.disabled) return "pool.status.disabled";
    if (!row.quota_known) return "pool.status.updating";
    const expires = Date.parse(row.expires_at);
    if (!Number.isNaN(expires) && expires <= Date.now()) return "pool.status.expired";
    if (row.remaining <= 0) return "pool.status.exhausted";
    if (row.holder.trim()) return "pool.status.inUse";
    return "pool.status.idle";
  }

  function renderSubscriptionPool() {
    if (!isTeamEdition()) {
      subscriptionPoolEl.hidden = true;
      closePoolDetail();
      closeSubscriptionLogin();
      return;
    }
    paintEmptyKindIcons();
    subscriptionAddEl.hidden = poolRows.length === 0;
    subscriptionPoolEmptyEl.hidden = poolRows.length > 0;
    subscriptionPoolListEl.replaceChildren();
    for (const row of poolRows) {
      const item = document.createElement("li");
      item.className = `provider-item is-pickable subscription-pool-item${poolTone(row)}`;
      item.tabIndex = 0;
      item.setAttribute("role", "button");
      const open = () => openPoolDetail(row.name);
      item.addEventListener("click", open);
      item.addEventListener("keydown", (event) => {
        if (event.key !== "Enter" && event.key !== " ") return;
        event.preventDefault();
        open();
      });

      const avatar = document.createElement("span");
      avatar.className = "provider-avatar is-brand";
      avatar.setAttribute("aria-hidden", "true");
      setAgentBrandIcon(avatar, row.runtime === "codex" ? "codex" : "claude-code");

      const main = document.createElement("div");
      main.className = "provider-item-main";
      const title = document.createElement("p");
      title.className = "provider-item-title";
      const name = document.createElement("span");
      name.className = "provider-item-name";
      name.textContent = row.name;
      const status = document.createElement("span");
      status.className = `subscription-pool-status${poolTone(row)}`;
      status.textContent = t(poolStatusKey(row));
      title.append(name, status);

      const sub = document.createElement("p");
      sub.className = "provider-item-sub";
      const kind = poolKind(row.runtime);
      if (kind) {
        const chip = document.createElement("span");
        chip.className = "provider-item-meta";
        chip.textContent = kind;
        sub.append(chip);
      }
      const note = document.createElement("span");
      note.className = "provider-item-note";
      const holder = row.holder.trim() ? t("pool.holder", { name: row.holder }) : t("pool.holderNone");
      const exit = row.proxy.trim() ? t("pool.proxyExit", { proxy: row.proxy }) : t("pool.proxyLocal");
      note.textContent = `${t("pool.remaining", { remaining: String(row.remaining) })} · ${holder} · ${exit}`;
      sub.append(note);
      main.append(title, sub);

      const chevron = document.createElement("span");
      chevron.className = "subscription-pool-chevron";
      chevron.setAttribute("aria-hidden", "true");
      chevron.textContent = "›";

      item.append(avatar, main, chevron);
      subscriptionPoolListEl.append(item);
    }
    if (!selectedPoolName) return;
    const selected = poolRows.find((row) => row.name === selectedPoolName);
    if (!selected) {
      closePoolDetail();
      return;
    }
    paintPoolDetail(selected);
  }

  function paintEmptyKindIcons() {
    const claude = document.querySelector<HTMLElement>("#subscription-empty-claude-icon");
    const codex = document.querySelector<HTMLElement>("#subscription-empty-codex-icon");
    if (claude) setAgentBrandIcon(claude, "claude-code");
    if (codex) setAgentBrandIcon(codex, "codex");
  }

  function poolKind(runtime: string): string {
    if (runtime === "claude-code") return t("pool.kindClaude");
    if (runtime === "codex") return t("pool.kindCodex");
    return "";
  }

  function poolTone(row: SubscriptionPoolRow): string {
    const key = poolStatusKey(row);
    if (key === "pool.status.inUse") return " is-in-use";
    if (key === "pool.status.exhausted" || key === "pool.status.expired" || key === "pool.status.disabled") {
      return " is-bad";
    }
    return "";
  }

  function openPoolDetail(name: string) {
    const row = poolRows.find((item) => item.name === name);
    if (!row) return;
    selectedPoolName = name;
    personalListViewEl.classList.add("is-pool-detail");
    subscriptionDetailEl.hidden = false;
    subscriptionDetailResultEl.textContent = "";
    paintPoolDetail(row);
  }

  function closePoolDetail() {
    selectedPoolName = null;
    subscriptionDetailEl.hidden = true;
    subscriptionDetailResultEl.textContent = "";
    if (subscriptionLoginEl.hidden) {
      personalListViewEl.classList.remove("is-pool-detail");
    }
  }

  function openSubscriptionLogin(runtime: string) {
    selectedPoolName = null;
    subscriptionDetailEl.hidden = true;
    subscriptionDetailResultEl.textContent = "";
    loginRuntime = runtime;
    personalListViewEl.classList.add("is-pool-detail");
    subscriptionLoginEl.hidden = false;
    subscriptionLoginNameEl.placeholder = t("pool.namePlaceholder");
    subscriptionLoginResultEl.textContent = "";
    paintLoginKind();
  }

  function closeSubscriptionLogin() {
    subscriptionLoginEl.hidden = true;
    subscriptionLoginResultEl.textContent = "";
    if (subscriptionDetailEl.hidden) {
      personalListViewEl.classList.remove("is-pool-detail");
    }
  }

  function paintLoginKind() {
    subscriptionLoginClaudeEl.setAttribute("aria-pressed", loginRuntime === "claude-code" ? "true" : "false");
    subscriptionLoginCodexEl.setAttribute("aria-pressed", loginRuntime === "codex" ? "true" : "false");
  }

  async function beginSubscriptionLogin() {
    const name = subscriptionLoginNameEl.value.trim();
    if (!loginRuntime) {
      subscriptionLoginResultEl.textContent = t("pool.pickKind");
      return;
    }
    if (!name) {
      subscriptionLoginResultEl.textContent = t("pool.nameRequired");
      return;
    }
    subscriptionLoginStartEl.disabled = true;
    subscriptionLoginResultEl.textContent = t("pool.loginWorking");
    try {
      await startSubscriptionLogin(loginRuntime, name);
      subscriptionLoginNameEl.value = "";
      await loadSubscriptionPool();
      closeSubscriptionLogin();
      openPoolDetail(name);
      subscriptionDetailResultEl.textContent = t("pool.loginOk", { name });
    } catch (error) {
      subscriptionLoginResultEl.textContent = withErrorDetail(t("pool.failed"), error);
    } finally {
      subscriptionLoginStartEl.disabled = false;
    }
  }

  function paintPoolDetail(row: SubscriptionPoolRow) {
    subscriptionDetailNameEl.textContent = row.name;
    subscriptionDetailStatusEl.textContent = t(poolStatusKey(row));
    subscriptionDetailStatusEl.className = `subscription-pool-status${poolTone(row)}`;
    subscriptionDetailMetaEl.textContent = `${t("pool.remaining", { remaining: String(row.remaining) })} · ${t("pool.expires", { date: row.expires_at })}`;
    subscriptionDetailHolderEl.textContent = [poolKind(row.runtime), row.holder.trim()
      ? t("pool.holder", { name: row.holder })
      : t("pool.holderNone")].filter(Boolean).join(" · ");
    subscriptionProxyEl.placeholder = t("pool.proxyPlaceholder");
    subscriptionProxyEl.value = row.proxy;
    subscriptionCapEl.placeholder = t("pool.capPlaceholder");
    subscriptionCapEl.value = row.cap == null ? "" : String(row.cap);
    subscriptionDisableEl.textContent = row.disabled ? t("pool.enable") : t("pool.disable");
  }

  async function loadSubscriptionPool() {
    try {
      poolRows = await listSubscriptionPool();
      renderSubscriptionPool();
    } catch (error) {
      subscriptionPoolResultEl.textContent = withErrorDetail(t("pool.failed"), error);
    }
  }

  async function changePool(name: string, action: "disable" | "enable" | "reclaim" | "cap" | "proxy", extra?: string) {
    subscriptionDetailResultEl.textContent = "";
    try {
      if (action === "disable") {
        await setSubscriptionDisabled(name, true);
        subscriptionDetailResultEl.textContent = t("pool.disabledOk");
      } else if (action === "enable") {
        await setSubscriptionDisabled(name, false);
        subscriptionDetailResultEl.textContent = t("pool.enabledOk");
      } else if (action === "reclaim") {
        await reclaimSubscription(name);
        subscriptionDetailResultEl.textContent = extra === "idle" ? t("pool.reclaimIdle") : t("pool.reclaimOk");
      } else if (action === "cap") {
        const raw = (extra ?? "").trim();
        const cap = raw === "" ? null : Number(raw);
        if (cap != null && (!Number.isInteger(cap) || cap <= 0)) {
          subscriptionDetailResultEl.textContent = t("pool.failed");
          return;
        }
        await setSubscriptionCap(name, cap);
        subscriptionDetailResultEl.textContent = cap == null ? t("pool.capCleared") : t("pool.capOk", { cap: String(cap) });
      } else {
        const raw = (extra ?? "").trim();
        if (raw !== "" && !/^(https?:\/\/)?(\[[0-9a-fA-F:]+\]|[^/\s@:]+):\d{1,5}$/.test(raw)) {
          subscriptionDetailResultEl.textContent = t("pool.proxyInvalid");
          return;
        }
        await setSubscriptionProxy(name, raw === "" ? null : raw);
        subscriptionDetailResultEl.textContent = raw === ""
          ? t("pool.proxyCleared")
          : t("pool.proxyOk", { proxy: raw.replace(/^https?:\/\//, "") });
      }
      await loadSubscriptionPool();
    } catch (error) {
      subscriptionDetailResultEl.textContent = withErrorDetail(t("pool.failed"), error);
    }
  }

  async function runEvotownOnboarding() {
    const url = evotownUrlEl.value.trim();
    const key = evotownKeyEl.value.trim();
    if (!url || !key) {
      evotownHintEl.textContent = t("evotown.connectFailed", {
        error: "URL and API key are required",
      });
      return;
    }

    evotownConnectEl.disabled = true;
    evotownResyncEl.disabled = true;
    evotownHintEl.textContent = t("evotown.connecting");
    try {
      const report = await runEvotownOnboardingCommand({
        url,
        key,
        syncSkills: true,
        pullPolicies: true,
      });
      evotownKeyEl.value = "";
      await loadEvotownStatus();
      await deps.loadModeStatus();
      await deps.refresh();
      evotownHintEl.textContent = t("evotown.connectOk", {
        installed: String(report.sync?.installed ?? 0),
        policies: String(report.policy?.policy_count ?? 0),
      });
    } catch (error) {
      evotownHintEl.textContent = withErrorDetail(t("evotown.connectFailed"), error);
    } finally {
      evotownConnectEl.disabled = false;
      evotownResyncEl.disabled = false;
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
      const report = await runEngineRegisterCommand({
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
      const report = await runSync();
      evotownHintEl.textContent = t("evotown.resyncOk", {
        installed: String(report.installed),
        skipped: String(report.skipped),
        failed: String(report.failed),
      });
      await deps.loadSkillsInventory();
    } catch (error) {
      evotownHintEl.textContent = withErrorDetail(t("evotown.resyncFailed"), error);
    } finally {
      evotownResyncEl.disabled = false;
    }
  }

  function bindEvents() {
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

    subscriptionDetailBackEl.addEventListener("click", () => closePoolDetail());
    subscriptionProxySaveEl.addEventListener("click", () => {
      if (!selectedPoolName) return;
      void changePool(selectedPoolName, "proxy", subscriptionProxyEl.value);
    });
    subscriptionCapSaveEl.addEventListener("click", () => {
      if (!selectedPoolName) return;
      void changePool(selectedPoolName, "cap", subscriptionCapEl.value);
    });
    subscriptionDisableEl.addEventListener("click", () => {
      const row = poolRows.find((item) => item.name === selectedPoolName);
      if (!row) return;
      void changePool(row.name, row.disabled ? "enable" : "disable");
    });
    subscriptionReclaimEl.addEventListener("click", () => {
      const row = poolRows.find((item) => item.name === selectedPoolName);
      if (!row) return;
      void changePool(row.name, "reclaim", row.holder.trim() ? "busy" : "idle");
    });
    subscriptionAddEl.addEventListener("click", () => openSubscriptionLogin(""));
    subscriptionEmptyClaudeEl.addEventListener("click", () => openSubscriptionLogin("claude-code"));
    subscriptionEmptyCodexEl.addEventListener("click", () => openSubscriptionLogin("codex"));
    subscriptionLoginBackEl.addEventListener("click", () => closeSubscriptionLogin());
    subscriptionLoginClaudeEl.addEventListener("click", () => {
      loginRuntime = "claude-code";
      paintLoginKind();
    });
    subscriptionLoginCodexEl.addEventListener("click", () => {
      loginRuntime = "codex";
      paintLoginKind();
    });
    subscriptionLoginStartEl.addEventListener("click", () => {
      void beginSubscriptionLogin();
    });
  }

  return {
    loadEvotownStatus,
    renderEvotownStatus,
    renderSubscriptionPool,
    getLastStatus: () => lastEvotownStatus,
    bindEvents,
  };
}
