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
  RepairStatusFilter,
  RuntimeDoctorResult,
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

function updateAgentsSecurityOverview(report: DoctorReport): void {
  const installed = report.runtimes.filter((runtime) => runtime.installed).length;
  const total = report.runtimes.length;
  const issueCount = [...repairPreviewByRuntime.values()].reduce(
    (count, preview) => count + preview.summary.fail + preview.summary.warn,
    0,
  );
  const readiness = total > 0 ? Math.round((installed / total) * 100) : 0;
  agentsReadinessRingEl.style.setProperty("--readiness", String(readiness));
  agentsReadinessValueEl.textContent = total > 0 ? `${installed}/${total}` : "—";
  agentsSecurityTitleEl.textContent =
    installed === total && total > 0 ? t("agents.securityReady") : t("agents.securityTitle");
  agentsSecurityDescEl.textContent = t("agents.securityDesc", {
    installed: String(installed),
    total: String(total),
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
  overview?.classList.toggle("is-ok", installed === total && total > 0 && issueCount === 0);
  agentsReadinessRingEl.classList.toggle("is-warn", issueCount > 0 && installed > 0);
  agentsReadinessRingEl.classList.toggle(
    "is-fail",
    installed === 0 || (issueCount > 0 && installed < total),
  );
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

  const install = createAgentsInstall({ refresh });

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
    return renderRuntimeCard(runtime, appState.hermesModel, actionsHtml, relatedResourcesHtml);
  }

  async function renderReport(
    report: DoctorReport,
    opts?: { relocalize?: boolean },
  ) {
    appState.lastReport = report;
    const installed = report.runtimes.filter((runtime) => runtime.installed).length;
    const total = report.runtimes.length;

    installedCountEl.textContent = `${installed}/${total}`;
    profileStatusEl.textContent = report.active_preset ?? t("status.none");
    if (!opts?.relocalize) {
      lastScanEl.textContent = formatTime(new Date());
    }
    runtimeCountEl.textContent = `${installed}/${total}`;
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

    if (report.runtimes.length === 0) {
      appState.activeRuntimeId = null;
      runtimeTabsEl.innerHTML = "";
      runtimesEl.innerHTML = `<div class="empty-state">${t("runtimes.empty")}</div>`;
      if (!opts?.relocalize) {
        void diagnose.closeDiagnoseDetail({ skipDismiss: true });
      }
      return;
    }

    const selectedId = resolveActiveRuntimeId(report.runtimes, appState.activeRuntimeId)!;
    appState.activeRuntimeId = selectedId;
    runtimeTabsEl.innerHTML = renderRuntimeTabs(
      report.runtimes,
      selectedId,
      repairPreviewByRuntime,
    );

    const activeRuntime = report.runtimes.find((runtime) => runtime.id === selectedId);
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
  }

  diagnose.bindEvents();

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
