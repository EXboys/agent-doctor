import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { withErrorDetail } from "./friendly-error";
import { formatTime } from "./format";
import {
  isAskRuntimeId,
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
  repairConfirmRuntimeIds,
  repairPreviewByRuntime,
} from "./app-state";
import { createAgentsDiagnose, MAIN_COMPACT_WIDTH } from "./agents-diagnose";
import { createAgentsInstall } from "./agents-install";
import { createAgentsPresets } from "./agents-presets";
import { createAgentsSessions } from "./agents-sessions";
import { createAgentsWorkspaceChip } from "./agents-workspace-chip";
import type {
  DoctorReport,
  HermesSettings,
  MainTabId,
  ProfilesDocument,
  RepairPreviewResponse,
  RepairStatusFilter,
  RuntimeDoctorResult,
  RuntimeVersionStatus,
  WindowSizeReport,
  WorkspacesDocument,
} from "./types";

export interface AgentsPanelDeps {
  setMainTab: (tab: MainTabId) => void;
  loadWorkspaces: () => Promise<void>;
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
const HOME_AGENT_VISIBLE = 6;
let runtimeTabsExpanded = false;
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

function updateAgentsSecurityOverview(report: DoctorReport): void {
  const installed = report.runtimes.filter((runtime) => runtime.installed).length;
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

function paintRuntimeTabs(installedRuntimes: RuntimeDoctorResult[], selectedId: string): void {
  const solo = installedRuntimes.length < 2;
  runtimeSectionEl.classList.toggle("is-solo", solo);
  runtimeTabsEl.hidden = solo;
  if (solo) {
    runtimeTabsEl.innerHTML = "";
    runtimeTabsMoreEl.hidden = true;
    return;
  }
  const overflow = installedRuntimes.length > HOME_AGENT_VISIBLE;
  if (
    overflow &&
    installedRuntimes.slice(HOME_AGENT_VISIBLE).some((runtime) => runtime.id === selectedId)
  ) {
    runtimeTabsExpanded = true;
  }
  const visible =
    overflow && !runtimeTabsExpanded
      ? installedRuntimes.slice(0, HOME_AGENT_VISIBLE)
      : installedRuntimes;
  runtimeTabsEl.innerHTML = renderRuntimeTabs(visible, selectedId, repairPreviewByRuntime);
  if (!overflow) {
    runtimeTabsMoreEl.hidden = true;
    return;
  }
  runtimeTabsMoreEl.hidden = false;
  const hidden = installedRuntimes.length - HOME_AGENT_VISIBLE;
  runtimeTabsMoreEl.textContent = runtimeTabsExpanded
    ? t("runtimes.showLess")
    : t("runtimes.showMore", { count: String(hidden) });
}

function hasActiveWorkspace(): boolean {
  return Boolean(appState.lastWorkspaces?.active);
}

async function loadHermesModel(): Promise<void> {
  try {
    appState.hermesModel = await invoke<HermesSettings>("get_hermes_model_command");
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

export function initAgentsPanel(d: AgentsPanelDeps): AgentsPanelApi {
  deps = d;

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

  async function refresh() {
    setLoading(true);
    try {
      const report = await invoke<DoctorReport>("run_doctor_command");
      await renderReport(report);
      await deps.onDoctorReport(report);
    } catch (error) {
      setStatusBanner("error", withErrorDetail(t("doctor.failed"), error));
      updateHealthStrip(0, 0);
      runtimesEl.innerHTML = `<div class="empty-state">${t("doctor.empty")}</div>`;
      runtimeTabsEl.innerHTML = "";
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
    const solo =
      (appState.lastReport?.runtimes.filter((item) => item.installed).length ?? 0) === 1;
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
    const installed = report.runtimes.filter((runtime) => runtime.installed).length;
    const total = report.runtimes.length;

    installedCountEl.textContent = String(installed);
    profileStatusEl.textContent = report.active_preset ?? t("status.none");
    if (!opts?.relocalize) {
      lastScanEl.textContent = formatTime(new Date());
    }
    runtimeCountEl.textContent = String(installed);
    updateHealthStrip(installed, total);
    updateAgentsSecurityOverview(report);

    setStatusBanner(
      report.profile_env_exists ? "ok" : "warn",
      report.profile_env_exists ? t("doctor.companyOk") : t("doctor.companyMissing"),
    );

    const hermesInstalled = report.runtimes.some(
      (runtime) => runtime.id === "hermes" && runtime.installed,
    );
    if (hermesInstalled && !(opts?.relocalize && appState.hermesModel)) {
      await loadHermesModel();
    } else if (!hermesInstalled) {
      appState.hermesModel = null;
    }

    const installedRuntimes = report.runtimes.filter((runtime) => runtime.installed);
    if (installedRuntimes.length === 0) {
      appState.activeRuntimeId = null;
      runtimeTabsEl.innerHTML = "";
      runtimeTabsEl.hidden = true;
      runtimeTabsMoreEl.hidden = true;
      runtimeSectionEl.classList.remove("is-solo");
      runtimesEl.innerHTML = `
        <div class="runtime-empty">
          <p>${t("runtimes.emptyInstalled")}</p>
          <p class="runtime-empty-hint">${t("runtimes.emptyInstalledHint")}</p>
          <button type="button" class="btn-primary" data-action="open-agent-catalog">${t("runtimes.addAgent")}</button>
        </div>
      `;
      if (!opts?.relocalize) {
        void diagnose.closeDiagnoseDetail({ skipDismiss: true });
      }
      return;
    }

    const selectedId = resolveActiveRuntimeId(installedRuntimes, appState.activeRuntimeId)!;
    appState.activeRuntimeId = selectedId;
    paintRuntimeTabs(installedRuntimes, selectedId);

    const activeRuntime = installedRuntimes.find((runtime) => runtime.id === selectedId);
    runtimesEl.innerHTML = activeRuntime ? buildRuntimeCardHtml(activeRuntime) : "";
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
      const rows = await invoke<RuntimeVersionStatus[]>("check_runtime_versions_command", {
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
      const selectedId = resolveActiveRuntimeId(installedRuntimes, appState.activeRuntimeId);
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
        runtimesEl.innerHTML = buildRuntimeCardHtml(activeRuntime);
        const nextHint = runtimesEl.querySelector<HTMLElement>("[data-repair-hint]");
        if (nextHint && (hintHtml || hintText)) {
          nextHint.hidden = false;
          if (hintHtml) {
            nextHint.innerHTML = hintHtml;
          } else if (hintText) {
            nextHint.textContent = hintText;
          }
        }
        if (preview && !diagnose.hasDismissed(selectedId)) {
          try {
            const next = await invoke<RepairPreviewResponse>("run_repair_preview_command", {
              runtime: selectedId,
            });
            diagnose.mountRepairPreview(next);
          } catch {
            diagnose.mountRepairPreview(preview);
          }
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
    const selectedId = resolveActiveRuntimeId(installedRuntimes, appState.activeRuntimeId);
    if (selectedId) paintRuntimeTabs(installedRuntimes, selectedId);
  });

  runtimeTabsEl.addEventListener("click", (event) => {
    const tab = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-runtime-tab]");
    const runtimeId = tab?.dataset.runtimeTab;
    if (!runtimeId || runtimeId === appState.activeRuntimeId) {
      return;
    }
    appState.activeRuntimeId = runtimeId;
    if (appState.lastReport) {
      void renderReport(appState.lastReport);
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
        diagnose.applyRepairFilter(runtime, filter);
      }
      return;
    }

    const action = target.closest<HTMLElement>("[data-action]")?.dataset.action;
    if (!action) {
      return;
    }

    if (action === "open-agent-catalog") {
      void invoke("open_resources_window_command", { section: "agents" });
      return;
    }
    if (action === "open-resources-skills") {
      void invoke("open_resources_window_command", { section: "skills" });
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

    if (action === "install-runtime" && runtimeCard) {
      void install.installRuntimeFromCard(runtimeCard);
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
        deps.setMainTab("workspace");
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
