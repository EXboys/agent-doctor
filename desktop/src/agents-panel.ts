
import { emit, listen } from "@tauri-apps/api/event";
import { t } from "./i18n";
import { agentBrandIconHtml } from "./agent-brand";
import { isPersonalEdition } from "./edition";
import { listPersonalProviders, nativeAccountStatus, teamupsAccountStatus } from "./ipc";
import { withErrorDetail } from "./friendly-error";
import { formatTime } from "./format";
import { officialQuotaLines, officialQuotaState } from "./official-quota";
import {
  isAskRuntimeId,
  metaRow,
  renderRuntimeCard,
  renderRuntimeCardActions,
  renderRuntimeTabs,
  resolveActiveRuntimeId,
  runtimeAdvancedMeta,
  supportsBrowserMcp,
} from "./agents-ui";
import { renderRelatedResourcesHtml } from "./repair-ui";
import {
  appState,
  applyNativeAccounts,
  repairConfirmRuntimeIds,
  repairPreviewByRuntime,
} from "./app-state";
import { createAgentsDiagnose, MAIN_COMPACT_WIDTH } from "./agents-diagnose";
import { createAgentsInstall } from "./agents-install";
import { createAgentsPresets } from "./agents-presets";
import { createAgentsSessions } from "./agents-sessions";
import { createAgentsWorkspaceChip } from "./agents-workspace-chip";
import { DOCTOR_RUNTIME_ID, planAgentTiles } from "./agent-tiles";
import { resetHomeHero, syncHomeHero } from "./home-hero";
import type {
  DoctorReport,
  MainTabId,
  ProfilesDocument,
  RepairStatusFilter,
  RuntimeDoctorResult,
  WindowSizeReport,
  WorkspacesDocument,
} from "./types";
import { getHermesModel, runDoctor, checkRuntimeVersions, openResourcesWindow } from "./ipc";
import { readActiveSessionRuntime } from "./chat/store";
import {
  ASK_ACTIVE_RUNTIME_EVENT,
  ASK_RUNTIME_QUERY_EVENT,
  ASK_SELECT_RUNTIME_EVENT,
} from "./chat/runtime";

export interface AgentsPanelDeps {
  setMainTab: (tab: MainTabId) => void;
  loadWorkspaces: () => Promise<void>;
  addProject: () => void;
  rewireCurrentMode: (hintEl?: HTMLElement | null) => Promise<void>;
  onDoctorReport: (report: DoctorReport) => Promise<void>;
}

export interface AgentsPanelApi {
  panelDiagnoseEl: HTMLElement;
  renderReport: (report: DoctorReport) => Promise<void>;
  refresh: () => Promise<void>;
  setLoading: (loading: boolean) => void;
  setStatusBanner: (kind: "ok" | "warn" | "error" | "neutral", message: string) => void;
  updateAgentsSecurityOverview: (report: DoctorReport) => void;
  openAskWindowForVerify: (runtime: string) => Promise<void>;
  loadProfiles: () => Promise<void>;
  renderProfiles: (doc: ProfilesDocument) => void;
  updateAgentsWorkspaceChip: (doc: WorkspacesDocument) => void;
  closeAgentsWsPicker: () => void;
  closePresetMenu: () => void;
  setMainWindowWidth: (width: number) => Promise<WindowSizeReport>;
  reloadLocale: () => Promise<void>;
  MAIN_COMPACT_WIDTH: number;
}

let deps!: AgentsPanelDeps;

const statusEl = document.querySelector<HTMLElement>("#status")!;
const runtimesEl = document.querySelector<HTMLElement>("#runtimes")!;
const runtimeTabsEl = document.querySelector<HTMLElement>("#runtime-tabs")!;
const runtimeTabsMoreEl = document.querySelector<HTMLButtonElement>("#runtime-tabs-more")!;
const runtimeSectionEl = document.querySelector<HTMLElement>(".runtime-section")!;
let runtimeTabsExpanded = false;
/** Tiles currently in the DOM, with the active state stripped out. */
let lastTabSkeleton = "";
/** Markup currently in the card slot. */
let lastCardHtml = "";
const agentsReadinessRingEl = document.querySelector<HTMLElement>("#agents-readiness-ring")!;
const agentsReadinessValueEl = document.querySelector<HTMLElement>("#agents-readiness-value")!;
const agentsSecurityTitleEl = document.querySelector<HTMLElement>("#agents-security-title")!;
const agentsSecurityDescEl = document.querySelector<HTMLElement>("#agents-security-desc")!;
const agentsSecurityInstalledEl =
  document.querySelector<HTMLElement>("#agents-security-installed")!;
const agentsSecurityIssuesEl = document.querySelector<HTMLElement>("#agents-security-issues")!;
const refreshBtn = document.querySelector<HTMLButtonElement>("#refresh")!;
const spinnerEl = refreshBtn.querySelector<HTMLElement>(".spinner")!;
const installedCountEl = document.querySelector<HTMLElement>("#installed-count")!;
const profileStatusEl = document.querySelector<HTMLElement>("#profile-status")!;
const lastScanEl = document.querySelector<HTMLElement>("#last-scan")!;
const runtimeCountEl = document.querySelector<HTMLElement>("#runtime-count")!;
const healthPillEl = document.querySelector<HTMLElement>("#health-pill")!;
const healthLabelEl = document.querySelector<HTMLElement>("#health-label")!;
const panelDiagnoseEl = document.querySelector<HTMLElement>("#panel-diagnose")!;

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
    healthPillEl.classList.add("is-partial");
    healthLabelEl.textContent = t("health.empty");
    return;
  }
  healthPillEl.classList.add("is-good");
  healthLabelEl.textContent = t("health.good");
}

/**
 * Count of local agents shown to the user: managed runtimes plus the
 * always-present built-in doctor tile, so the ring, the counters and the
 * tile grid all say the same number. A machine with no managed runtime
 * stays at zero — the doctor alone is not a healthy environment.
 */
function countLocalAgents(managed: number): number {
  return managed > 0 ? managed + 1 : 0;
}

/**
 * The solo layout is for a home with one agent *in total* — the built-in doctor
 * included. Judging by managed CLIs alone painted the big solo card next to two
 * tiles, so every solo decision goes through here.
 */
function isSoloHome(managedInstalled: number): boolean {
  return countLocalAgents(managedInstalled) < 2;
}

function updateAgentsSecurityOverview(report: DoctorReport): void {
  const installed = countLocalAgents(
    report.runtimes.filter((runtime) => runtime.installed).length,
  );
  const issueCount = [...repairPreviewByRuntime.values()].reduce(
    (count, preview) => count + preview.summary.fail + preview.summary.warn,
    0,
  );
  const environmentOk = installed > 0 && issueCount === 0;
  const readiness = environmentOk ? 100 : installed === 0 ? 0 : 70;
  agentsReadinessRingEl.style.setProperty("--readiness", String(readiness));
  agentsReadinessValueEl.textContent = installed > 0 ? String(installed) : "0";
  agentsSecurityTitleEl.textContent =
    installed === 0
      ? t("agents.securityEmpty")
      : environmentOk
        ? t("agents.securityReady")
        : t("agents.securityTitle");
  agentsSecurityDescEl.textContent =
    installed === 0
      ? t("agents.securityDescEmpty")
      : t("agents.securityDesc", {
          installed: String(installed),
          issues: String(issueCount),
        });
  agentsSecurityInstalledEl.textContent = t("agents.runtimeAvailable", {
    count: String(installed),
  });
  agentsSecurityIssuesEl.textContent = t("agents.knownIssues", {
    count: String(issueCount),
  });
  agentsSecurityIssuesEl.classList.toggle("has-issues", issueCount > 0);
  const overview = document.querySelector<HTMLElement>("#agents-security-overview");
  overview?.classList.toggle("is-ok", environmentOk);
  agentsReadinessRingEl.classList.toggle("is-warn", issueCount > 0 && installed > 0);
  agentsReadinessRingEl.classList.toggle("is-fail", false);
  agentsReadinessRingEl.classList.toggle("is-empty", installed === 0);
}

/**
 * The doctor tile is virtual — "doctor" is never in the installed list — so a
 * plain resolve would drop an active doctor selection onto the first CLI.
 */
function resolvePaintedRuntimeId(installedRuntimes: RuntimeDoctorResult[]): string | null {
  return appState.activeRuntimeId === DOCTOR_RUNTIME_ID
    ? DOCTOR_RUNTIME_ID
    : resolveActiveRuntimeId(installedRuntimes, appState.activeRuntimeId);
}

/** Tile markup with the active state normalised away, so a selection-only
 * change compares equal to what is already on screen. */
function tabSkeleton(html: string): string {
  return html.replace(/ is-active/g, "").replace(/aria-selected="true"/g, 'aria-selected="false"');
}

/** The first click on an agent pays for parsing its brand mark and laying out a
 * card that has never been on screen; every later click is cheap. Pay that once
 * while the window is idle instead, so the first switch feels like the rest. */
let warmed = false;
function warmAgentCards(runtimes: RuntimeDoctorResult[]): void {
  if (warmed || runtimes.length < 2) return;
  warmed = true;
  const warm = () => {
    const host = document.createElement("div");
    host.setAttribute("aria-hidden", "true");
    host.style.cssText =
      "position:fixed;top:0;left:-9999px;width:420px;visibility:hidden;pointer-events:none;";
    host.innerHTML = runtimes
      .map((runtime) => `<span class="runtime-solo-logo">${agentBrandIconHtml(runtime.id)}</span>`)
      .join("");
    // Reading a box forces the parse + paint the first switch would otherwise do.
    void host.offsetHeight;
    host.remove();
  };
  const idle = (window as unknown as {
    requestIdleCallback?: (cb: () => void, opts?: { timeout: number }) => void;
  }).requestIdleCallback;
  if (idle) idle(warm, { timeout: 3000 });
  else window.setTimeout(warm, 1500);
}

/** Card swap + hero measurement both read layout, so running them inside the
 * click handler forces a synchronous reflow before the tile can repaint — on a
 * cold window that is the hitch users feel. Coalesce them into one frame.
 * `afterPaint` runs once the swapped card is in the DOM; the doctor card's
 * quota fill queries `[data-doctor-service]`, which only exists after the
 * swap. */
let cardFrame = 0;
function scheduleCardPaint(build: () => string, afterPaint?: () => void): void {
  if (cardFrame) cancelAnimationFrame(cardFrame);
  cardFrame = requestAnimationFrame(() => {
    cardFrame = 0;
    paintRuntimeCard(build());
    syncHomeHero();
    afterPaint?.();
  });
}

/** Replace the card only when its markup really changed: a repaint re-decodes
 * the brand mark and re-measures the hero, which is visible as a flicker. */
function paintRuntimeCard(html: string): boolean {
  if (html === lastCardHtml && runtimesEl.childElementCount > 0) {
    return false;
  }
  lastCardHtml = html;
  runtimesEl.innerHTML = html;
  return true;
}

function paintRuntimeTabs(installedRuntimes: RuntimeDoctorResult[], selectedId: string): void {
  const plan = planAgentTiles(
    installedRuntimes.map((runtime) => runtime.id),
    selectedId,
    runtimeTabsExpanded,
  );
  if (plan.expandForSelection) {
    runtimeTabsExpanded = true;
  }
  runtimeSectionEl.classList.toggle("is-solo", isSoloHome(installedRuntimes.length));
  const visible = installedRuntimes.slice(0, plan.visibleCount);
  const html = renderRuntimeTabs(visible, selectedId, repairPreviewByRuntime);
  // Switching agents only flips which tile is active. Re-parsing the tiles
  // would re-decode every brand mark and re-lay out the row on each click,
  // which reads as a hitch, so keep the nodes and toggle the state instead.
  if (tabSkeleton(html) === lastTabSkeleton && runtimeTabsEl.childElementCount > 0) {
    for (const tile of runtimeTabsEl.querySelectorAll<HTMLElement>("[data-runtime-tab], [data-doctor-tab]")) {
      const id = tile.dataset.runtimeTab ?? DOCTOR_RUNTIME_ID;
      const active = id === selectedId;
      tile.classList.toggle("is-active", active);
      tile.setAttribute("aria-selected", active ? "true" : "false");
    }
  } else {
    lastTabSkeleton = tabSkeleton(html);
    runtimeTabsEl.innerHTML = html;
    runtimeTabsEl.prepend(doctorTabTile(selectedId === DOCTOR_RUNTIME_ID));
  }
  runtimeTabsEl.hidden = false;
  if (!plan.overflow) {
    runtimeTabsMoreEl.hidden = true;
    return;
  }
  runtimeTabsMoreEl.hidden = false;
  runtimeTabsMoreEl.textContent = runtimeTabsExpanded
    ? t("runtimes.showLess")
    : t("runtimes.showMore", { count: String(plan.hiddenCount) });
}

function hasActiveWorkspace(): boolean {
  return Boolean(appState.lastWorkspaces?.active);
}

/** Home entry for the built-in doctor agent: always installed, opens a chat. */
function doctorTabTile(active: boolean): HTMLElement {
  const tile = document.createElement("button");
  tile.type = "button";
  tile.className = `runtime-tab is-doctor${active ? " is-active" : ""}`;
  tile.dataset.doctorTab = DOCTOR_RUNTIME_ID;
  tile.setAttribute("role", "tab");
  tile.setAttribute("aria-selected", active ? "true" : "false");
  tile.title = t("chat.emptyDoctorHint");
  tile.innerHTML = `
    <span class="runtime-tab-dot is-ok" aria-hidden="true"></span>
    <span class="runtime-tab-logo" aria-hidden="true">${agentBrandIconHtml("doctor")}</span>
    <span class="runtime-tab-label">Agent Doctor</span>
    <span class="runtime-tab-meta">${t("runtime.doctorTileMeta")}</span>`;
  return tile;
}

/** Detail card for the built-in doctor agent, styled like a runtime card. */
function doctorCardHtml(solo = false): string {
  // The built-in agent rides the official account only — never the active
  // provider. The row starts at the sign-in line and `paintDoctorServiceRow`
  // fills in the live quota once the account status arrives.
  const service = metaRow("meta.service", t("runtime.doctorNoService"));
  const serviceHtml = service.replace(
    'class="meta-value"',
    'class="meta-value" data-doctor-service',
  );
  const actions = `
    <button type="button" class="btn-primary" data-action="ask-doctor">${t("chat.emptyDoctorAction")}</button>
    <button type="button" class="btn-secondary" data-action="go-provider">${t("runtime.doctorWire")}</button>`;
  // The only agent on the machine gets the large card, the same treatment a
  // lone CLI runtime gets — the built-in agent is not a lesser one. The block
  // is always painted and `home-hero.ts` shows it, so a multi-agent home can
  // borrow it when the panel has room.
  const hero = `
      <div class="runtime-solo-hero">
        <span class="runtime-solo-logo" aria-hidden="true">${agentBrandIconHtml(DOCTOR_RUNTIME_ID)}</span>
        <div class="runtime-solo-copy">
          <p class="runtime-solo-kicker">${t("runtimes.soloKicker")}</p>
          <h2 class="runtime-tab-title">Agent Doctor</h2>
          <p class="runtime-solo-blurb">${t("runtime.doctorTileMeta")}</p>
        </div>
      </div>`;
  return `
    <article class="runtime${solo ? " has-hero" : ""}" data-runtime="${DOCTOR_RUNTIME_ID}">
      <div class="section-label runtime-card-label">
        <h2 class="runtime-tab-title">Agent Doctor</h2>
        <span class="runtime-badges">
          <span class="badge ok">${t("runtime.builtinBadge")}</span>
        </span>
      </div>
      ${hero}
      <div class="meta-grid">
        ${serviceHtml}
        <div class="meta-row">
          <span class="meta-label">${t("meta.status")}</span>
          <p class="meta-value">${t("runtime.doctorAlwaysReady")}</p>
        </div>
      </div>
      <div class="card-actions">${actions}</div>
      <p class="footnote runtime-install-footnote">${t("chat.emptyDoctorHint")}</p>
    </article>
  `;
}

/** Fill the doctor card's service row with the official account's live quota. */
async function paintDoctorServiceRow(): Promise<void> {
  const row = runtimesEl.querySelector<HTMLElement>("[data-doctor-service]");
  if (!row) return;
  const status = await teamupsAccountStatus().catch(() => null);
  if (!row.isConnected) return;
  const quota = officialQuotaState(status);
  row.textContent = quota.signedIn
    ? `${t("chat.modelOfficialGroup")} · ${officialQuotaLines(quota).join(" · ")}`
    : t("personal.officialNeedsLogin");
}



async function loadHermesModel(): Promise<void> {
  try {
    appState.hermesModel = await getHermesModel();
  } catch {
    appState.hermesModel = null;
  }
}

function setLoading(loading: boolean) {
  refreshBtn.disabled = loading;
  refreshBtn.classList.toggle("is-loading", loading);
  spinnerEl.hidden = !loading;
  runtimesEl.classList.toggle("is-loading", loading);
  runtimeTabsEl.classList.toggle("is-loading", loading);

  if (loading) {
    const installed = appState.lastReport?.runtimes.filter((runtime) => runtime.installed).length ?? 0;
    const total = appState.lastReport?.runtimes.length ?? 0;
    updateHealthStrip(installed, total, true);
    setStatusBanner("neutral", t("doctor.running"));
  }
}

/**
 * Follow the room the agent panel has. Dragging the window edge, docking Ask
 * beside it, or a finished scan all change the height the big card may take.
 */
function watchHomeHeroRoom(): void {
  if (typeof ResizeObserver === "undefined") return;
  let queued = false;
  const observer = new ResizeObserver(() => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      syncHomeHero();
    });
  });
  observer.observe(runtimeSectionEl);
}

export function initAgentsPanel(d: AgentsPanelDeps): AgentsPanelApi {
  deps = d;

  watchHomeHeroRoom();
  resetHomeHero();

  const sessions = createAgentsSessions({ setStatusBanner });

  const install = createAgentsInstall({ refresh, setStatusBanner });

  const diagnose = createAgentsDiagnose({
    setStatusBanner,
    updateAgentsSecurityOverview,
    loadHermesModel,
    getRuntimesEl: () => runtimesEl,
    hasActiveWorkspace,
    openAskWindow: sessions.openAskWindow,
    openAskWindowForVerify: sessions.openAskWindowForVerify,
    setMainTab: (tab) => deps.setMainTab(tab),
    uninstallRuntime: (runtime, name) => install.uninstallRuntime(runtime, name),
  });

  /** Selecting the doctor tile: highlight it and show its detail card. */
  function selectDoctorRuntime(): void {
    appState.activeRuntimeId = DOCTOR_RUNTIME_ID;
    const report = appState.lastReport;
    const installedRuntimes = report ? report.runtimes.filter((runtime) => runtime.installed) : [];
    paintRuntimeTabs(installedRuntimes, DOCTOR_RUNTIME_ID);
    scheduleCardPaint(doctorCardHtml, () => void paintDoctorServiceRow());
    void diagnose.closeDiagnoseDetail({ skipDismiss: true });
  }

  async function refresh() {
    setLoading(true);
    try {
      const report = await runDoctor();
      await renderReport(report);
      await deps.onDoctorReport(report);
    } catch (error) {
      setStatusBanner("error", withErrorDetail(t("doctor.failed"), error));
      updateHealthStrip(0, 0);
      paintRuntimeCard(`<div class="empty-state">${t("doctor.empty")}</div>`);
      runtimeTabsEl.innerHTML = "";
      lastTabSkeleton = "";
      appState.activeRuntimeId = null;
      installedCountEl.textContent = "—";
      profileStatusEl.textContent = t("status.error");
      runtimeCountEl.textContent = "—";
    } finally {
      setLoading(false);
    }
  }

  const presets = createAgentsPresets({ refresh });
  const workspaceChip = createAgentsWorkspaceChip({
    setMainTab: (tab) => deps.setMainTab(tab),
    loadWorkspaces: () => deps.loadWorkspaces(),
    addProject: () => deps.addProject(),
    refreshRuntimeCardActions: diagnose.refreshRuntimeCardActions,
    getActiveRuntimeId: () => appState.activeRuntimeId,
    getRuntimesEl: () => runtimesEl,
  });

  function buildRuntimeCardHtml(runtime: RuntimeDoctorResult): string {
    const preview = repairPreviewByRuntime.get(runtime.id);
    const advancedMeta = runtimeAdvancedMeta(runtime, appState.hermesModel);
    const actionsHtml = renderRuntimeCardActions(
      runtime,
      advancedMeta,
      diagnose.runtimeCardActionContext(runtime),
    );
    const relatedResourcesHtml = renderRelatedResourcesHtml(preview);
    const solo = isSoloHome(
      appState.lastReport?.runtimes.filter((item) => item.installed).length ?? 0,
    );
    return renderRuntimeCard(
      runtime,
      appState.hermesModel,
      actionsHtml,
      relatedResourcesHtml,
      solo,
    );
  }

  async function renderReport(
    report: DoctorReport,
    opts?: { relocalize?: boolean },
  ) {
    appState.lastReport = report;
    const installed = countLocalAgents(
      report.runtimes.filter((runtime) => runtime.installed).length,
    );
    const total = report.runtimes.length;

    installedCountEl.textContent = String(installed);
    profileStatusEl.textContent = report.active_preset ?? t("status.none");
    if (!opts?.relocalize) {
      lastScanEl.textContent = formatTime(new Date());
    }
    runtimeCountEl.textContent = String(installed);
    updateHealthStrip(installed, total);
    updateAgentsSecurityOverview(report);

    if (isPersonalEdition()) {
      setStatusBanner("ok", t("doctor.scanOk"));
    } else {
      setStatusBanner(
        report.profile_env_exists ? "ok" : "warn",
        report.profile_env_exists ? t("doctor.companyOk") : t("doctor.companyMissing"),
      );
    }

    const hermesInstalled = report.runtimes.some(
      (runtime) => runtime.id === "hermes" && runtime.installed,
    );
    if (hermesInstalled && !(opts?.relocalize && appState.hermesModel)) {
      await loadHermesModel();
    } else if (!hermesInstalled) {
      appState.hermesModel = null;
    }

    if (isPersonalEdition()) {
      try {
        const [doc, accounts] = await Promise.all([
          listPersonalProviders(),
          nativeAccountStatus(false),
        ]);
        appState.personalProvidersDoc = doc;
        applyNativeAccounts(accounts.rows, false);
      } catch {
        // The card can still name a known service from the address alone.
      }
    }

    const installedRuntimes = report.runtimes.filter((runtime) => runtime.installed);
    if (installedRuntimes.length === 0) {
      // The doctor ships with the app, so a home with no CLI still has one
      // agent to show. Hiding the tiles here made the built-in agent vanish
      // exactly when it was the only one left; the install prompt now rides
      // under its card instead of replacing it.
      appState.activeRuntimeId = DOCTOR_RUNTIME_ID;
      paintRuntimeTabs(installedRuntimes, DOCTOR_RUNTIME_ID);
      // One agent in total: the large card is the whole story, so the tile row
      // would only repeat the name it already shows.
      runtimeTabsEl.hidden = true;
      paintRuntimeCard(`${doctorCardHtml(true)}
        <div class="runtime-empty is-compact">
          <p>${t("runtimes.emptyInstalled")}</p>
          <p class="runtime-empty-hint">${t("runtimes.emptyInstalledHint")}</p>
          <button type="button" class="btn-primary" data-action="open-agent-catalog">${t("runtimes.addAgent")}</button>
        </div>
      `);
      void paintDoctorServiceRow();
      syncHomeHero();
      if (!opts?.relocalize) {
        void diagnose.closeDiagnoseDetail({ skipDismiss: true });
      }
      return;
    }

    if (!appState.activeRuntimeId) {
      const fromChat = readActiveSessionRuntime();
      if (fromChat && installedRuntimes.some((runtime) => runtime.id === fromChat)) {
        appState.activeRuntimeId = fromChat;
      } else if (fromChat === DOCTOR_RUNTIME_ID) {
        appState.activeRuntimeId = DOCTOR_RUNTIME_ID;
      }
    }

    const selectedId = resolvePaintedRuntimeId(installedRuntimes)!;
    appState.activeRuntimeId = selectedId;
    paintRuntimeTabs(installedRuntimes, selectedId);

    const activeRuntime = installedRuntimes.find((runtime) => runtime.id === selectedId);
    paintRuntimeCard(
      selectedId === DOCTOR_RUNTIME_ID
        ? doctorCardHtml()
        : activeRuntime
          ? buildRuntimeCardHtml(activeRuntime)
          : "",
    );
    if (selectedId === DOCTOR_RUNTIME_ID) void paintDoctorServiceRow();
    syncHomeHero();
    warmAgentCards(installedRuntimes);
    if (opts?.relocalize) {
      diagnose.refreshDiagnoseLocale();
      return;
    }
    const preview = selectedId ? repairPreviewByRuntime.get(selectedId) : undefined;
    if (preview && !diagnose.hasDismissed(selectedId)) {
      diagnose.mountRepairPreview(preview);
    } else {
      void diagnose.closeDiagnoseDetail({ skipDismiss: true });
    }
    install.reapplyStickyInstallHints(report);
    if (!opts?.relocalize) {
      void refreshRuntimeVersions(report);
    }
  }

  /** Swap the visible agent from the last scan. No probe, no version fetch. */
  function showInstalledRuntime(runtimeId: string): void {
    // The doctor tile is virtual — it never appears in the installed scan, so
    // events like ask-active-runtime route here with "doctor" (same gap as
    // resolvePaintedRuntimeId).
    if (runtimeId === DOCTOR_RUNTIME_ID) {
      selectDoctorRuntime();
      return;
    }
    const report = appState.lastReport;
    if (!report) {
      return;
    }
    const installedRuntimes = report.runtimes.filter((runtime) => runtime.installed);
    const selected = installedRuntimes.find((runtime) => runtime.id === runtimeId);
    if (!selected) {
      return;
    }
    appState.activeRuntimeId = runtimeId;
    paintRuntimeTabs(installedRuntimes, runtimeId);
    // The tile state is already on screen; the card can land next frame.
    scheduleCardPaint(() => buildRuntimeCardHtml(selected));
  }

  async function refreshRuntimeVersions(report: DoctorReport): Promise<void> {
    const installed = report.runtimes
      .filter((runtime) => runtime.installed)
      .map((runtime) => ({
        runtimeId: runtime.id,
        version: runtime.version,
      }));
    if (installed.length === 0) {
      appState.runtimeVersions.clear();
      return;
    }
    try {
      const rows = await checkRuntimeVersions({
        installed,
      });
      appState.runtimeVersions.clear();
      for (const row of rows) {
        appState.runtimeVersions.set(row.runtime_id, row);
      }
      if (appState.lastReport !== report) {
        return;
      }
      const installedRuntimes = report.runtimes.filter((runtime) => runtime.installed);
      const selectedId = resolvePaintedRuntimeId(installedRuntimes);
      if (!selectedId) {
        return;
      }
      appState.activeRuntimeId = selectedId;
      paintRuntimeTabs(installedRuntimes, selectedId);
      const activeRuntime = installedRuntimes.find((runtime) => runtime.id === selectedId);
      if (activeRuntime) {
        const preview = repairPreviewByRuntime.get(selectedId);
        const hint = runtimesEl.querySelector<HTMLElement>("[data-repair-hint]");
        const hintHtml = hint && !hint.hidden ? hint.innerHTML : "";
        const hintText = hint && !hint.hidden ? hint.textContent : "";
        // Same markup means the version fetch found nothing new — leave the
        // card alone instead of re-rendering it under the user's cursor.
        if (paintRuntimeCard(buildRuntimeCardHtml(activeRuntime))) {
          const nextHint = runtimesEl.querySelector<HTMLElement>("[data-repair-hint]");
          if (nextHint && (hintHtml || hintText)) {
            nextHint.hidden = false;
            if (hintHtml) {
              nextHint.innerHTML = hintHtml;
            } else if (hintText) {
              nextHint.textContent = hintText;
            }
          }
        }
        if (preview && !diagnose.hasDismissed(selectedId)) {
          diagnose.mountRepairPreview(preview);
        }
      }
    } catch {
      // Network/cache failures stay silent — local version still shows.
    }
  }

  diagnose.bindEvents();

  runtimeTabsMoreEl.addEventListener("click", () => {
    runtimeTabsExpanded = !runtimeTabsExpanded;
    const report = appState.lastReport;
    if (!report) return;
    const installedRuntimes = report.runtimes.filter((runtime) => runtime.installed);
    const selectedId = resolvePaintedRuntimeId(installedRuntimes);
    if (selectedId) paintRuntimeTabs(installedRuntimes, selectedId);
  });

  runtimeTabsEl.addEventListener("click", (event) => {
    const doctorTab = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-doctor-tab]");
    if (doctorTab) {
      selectDoctorRuntime();
      // The ask window listens for this to switch to the runtime's session;
      // "doctor" is ask-capable in the runtime catalog, same as the CLIs.
      void emit(ASK_SELECT_RUNTIME_EVENT, { runtime: DOCTOR_RUNTIME_ID }).catch(() => {});
      return;
    }
    const tab = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-runtime-tab]");
    const runtimeId = tab?.dataset.runtimeTab;
    if (!runtimeId || runtimeId === appState.activeRuntimeId) {
      return;
    }
    showInstalledRuntime(runtimeId);
    if (isAskRuntimeId(runtimeId)) {
      void emit(ASK_SELECT_RUNTIME_EVENT, { runtime: runtimeId }).catch(() => {});
    }
  });

  runtimesEl.addEventListener("click", (event) => {
    const target = event.target as HTMLElement;
    if (target.closest<HTMLButtonElement>("[data-action='ask-doctor']")) {
      void sessions.openAskWindow("doctor").catch(() => {});
      return;
    }
    const filterBtn = target.closest<HTMLButtonElement>("[data-repair-filter]");
    if (filterBtn && !filterBtn.disabled) {
      const card = filterBtn.closest<HTMLElement>("[data-runtime]");
      const runtime = card?.dataset.runtime;
      const filter = filterBtn.dataset.repairFilter as RepairStatusFilter | undefined;
      if (runtime && filter) {
        diagnose.applyRepairFilter(runtime, filter);
      }
      return;
    }

    const action = target.closest<HTMLElement>("[data-action]")?.dataset.action;
    if (!action) {
      return;
    }

    if (action === "open-agent-catalog" || action === "open-resources-skills") {
      const section = action === "open-agent-catalog" ? "agents" : "skills";
      void openResourcesWindow({ section }).catch((error) => {
        setStatusBanner("error", withErrorDetail(t("runtime.openFailed"), error));
      });
      return;
    }

    const runtimeCard = target.closest<HTMLElement>("[data-runtime]");
    if (action === "diagnose-runtime" && runtimeCard) {
      void diagnose.diagnoseRuntimeCard(runtimeCard);
      return;
    }

    if (action === "open-session" && runtimeCard) {
      const forceTerminal =
        target.closest<HTMLElement>("[data-action='open-session']")?.dataset.openTerminal === "1";
      void sessions.openSessionFromCard(runtimeCard, forceTerminal);
      return;
    }

    if (action === "ask-session" && runtimeCard) {
      const runtime = runtimeCard.dataset.runtime;
      if (runtime && isAskRuntimeId(runtime)) {
        void (async () => {
          await diagnose.closeDiagnoseDetail({ skipDismiss: true });
          await sessions.openAskWindow(runtime);
        })();
      }
      return;
    }

    if (action === "ask-verify" && runtimeCard) {
      const runtime = runtimeCard.dataset.runtime;
      if (runtime && supportsBrowserMcp(runtime)) {
        void (async () => {
          await diagnose.closeDiagnoseDetail({ skipDismiss: true });
          await sessions.openAskWindowForVerify(runtime);
        })();
      }
      return;
    }

    if (action === "browser-smoke" && runtimeCard) {
      void diagnose.runBrowserSmokeFromCard(runtimeCard);
      return;
    }

    if (action === "live-probe" && runtimeCard) {
      void diagnose.runLiveProbeFromCard(runtimeCard);
      return;
    }

    if (action === "install-runtime" && runtimeCard) {
      void install.installRuntimeFromCard(runtimeCard);
      return;
    }

    if (action === "update-runtime" && runtimeCard) {
      void install.installRuntimeFromCard(runtimeCard, { update: true });
      return;
    }

    if (action === "force-reinstall-runtime" && runtimeCard) {
      void install.installRuntimeFromCard(runtimeCard, { force: true });
      return;
    }

    if (action === "uninstall-runtime" && runtimeCard) {
      const runtime = runtimeCard.dataset.runtime;
      const name =
        runtimeCard.querySelector(".runtime-tab-title")?.textContent?.trim() || runtime || "";
      if (runtime) {
        void install.uninstallRuntime(runtime, name);
      }
      return;
    }

    if (action === "open-install-log") {
      const path = (event.target as HTMLElement | null)
        ?.closest<HTMLElement>("[data-action='open-install-log']")
        ?.dataset.logPath;
      if (path) {
        void diagnose.openRepairGuide(path);
      }
      return;
    }

    if (action === "go-provider") {
      deps.setMainTab("provider");
      return;
    }

    if (action === "activate-workspace") {
      if (!appState.lastWorkspaces || Object.keys(appState.lastWorkspaces.workspaces).length === 0) {
        deps.addProject();
        return;
      }
      workspaceChip.toggleAgentsWsPicker();
      return;
    }

    if (action === "wire-runtime" && runtimeCard) {
      const hint = runtimeCard.querySelector<HTMLElement>("[data-repair-hint]");
      void deps.rewireCurrentMode(hint);
      return;
    }

    if (action === "apply-repair" && runtimeCard) {
      const runtime = runtimeCard.dataset.runtime;
      if (runtime) {
        repairConfirmRuntimeIds.add(runtime);
        const report = repairPreviewByRuntime.get(runtime);
        if (report) {
          void diagnose.openDiagnoseDetail(report);
        }
      }
      return;
    }

    if (action === "rollback-repair" && runtimeCard) {
      void diagnose.rollbackRepairRuntimeCard(runtimeCard);
      return;
    }

    const guideBtn = target.closest<HTMLButtonElement>('[data-action="open-repair-guide"]');
    if (guideBtn?.dataset.guidePath) {
      void diagnose.openRepairGuide(decodeURIComponent(guideBtn.dataset.guidePath));
    }
  });

  refreshBtn.addEventListener("click", () => {
    void refresh();
  });

  void listen<{ runtime?: string }>(ASK_ACTIVE_RUNTIME_EVENT, (event) => {
    const runtime = event.payload?.runtime;
    if (!runtime) return;
    if (!appState.lastReport) {
      appState.activeRuntimeId = runtime;
      return;
    }
    if (runtime === appState.activeRuntimeId) return;
    showInstalledRuntime(runtime);
  });
  void emit(ASK_RUNTIME_QUERY_EVENT).catch(() => {});

  // Signing in or out anywhere (ask window, provider tab, mall) must re-query
  // the account on the visible doctor card — no other repaint covers it. The
  // fill no-ops when the doctor card is not on screen.
  void listen("teamups-account-changed", () => {
    void paintDoctorServiceRow();
  });

  document.addEventListener("click", (event) => {
    const target = event.target as Node;
    if (appState.presetMenuOpen && !presets.presetPickerEl.contains(target)) {
      presets.closePresetMenu();
    }
    if (
      appState.agentsWsPickerOpen &&
      !workspaceChip.agentsWsChipEl.contains(target) &&
      !workspaceChip.agentsWsPickerEl.contains(target)
    ) {
      workspaceChip.closeAgentsWsPicker();
    }
  });

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      presets.closePresetMenu();
      workspaceChip.closeAgentsWsPicker();
    }
  });

  return {
    panelDiagnoseEl,
    renderReport,
    refresh,
    setLoading,
    setStatusBanner,
    updateAgentsSecurityOverview,
    openAskWindowForVerify: sessions.openAskWindowForVerify,
    loadProfiles: presets.loadProfiles,
    renderProfiles: presets.renderProfiles,
    updateAgentsWorkspaceChip: workspaceChip.updateAgentsWorkspaceChip,
    closeAgentsWsPicker: workspaceChip.closeAgentsWsPicker,
    closePresetMenu: presets.closePresetMenu,
    setMainWindowWidth: diagnose.setMainWindowWidth,
    MAIN_COMPACT_WIDTH,
    reloadLocale: async () => {
      if (appState.lastWorkspaces) {
        workspaceChip.updateAgentsWorkspaceChip(appState.lastWorkspaces);
      }
      if (appState.lastProfiles) {
        presets.renderProfiles(appState.lastProfiles);
      }
      if (appState.lastReport) {
        await renderReport(appState.lastReport, { relocalize: true });
      } else {
        setStatusBanner("neutral", t("doctor.loading"));
        presets.setLoadingStatus(t("presets.loading"));
        healthLabelEl.textContent = t("health.ready");
      }
    },
  };
}
