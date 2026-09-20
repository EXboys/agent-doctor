import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  applyStaticI18n,
  getLocale,
  setLocale,
  type Locale,
} from "./i18n";
import { initUpdaterUi } from "./updater";
import { isTeamEdition } from "./edition";
import {
  appState,
  repairFilterByRuntime,
  repairPreviewByRuntime,
} from "./app-state";
import { initAgentsPanel } from "./agents-panel";
import { initWiringUi } from "./wiring-ui";
import { initWorkspaceUi } from "./workspace-ui";
import { initResourcesHub } from "./resources-hub";
import { initFirstRunUi } from "./first-run-ui";
import type { DoctorReport, MainTabId, WorkspaceDoctorReport } from "./types";

if (navigator.userAgent.includes("Windows")) {
  document.documentElement.classList.add("is-opaque-shell");
}

const mainTabsEl = document.querySelector<HTMLElement>("#main-tabs")!;
const mainPanels = Array.from(document.querySelectorAll<HTMLElement>("[data-main-panel]"));
const langSwitchEl = document.querySelector<HTMLElement>(".lang-switch")!;
const widgetToolbarEl = document.querySelector<HTMLElement>(".widget-toolbar")!;
const windowCloseEl = document.querySelector<HTMLButtonElement>("#window-close")!;
const windowMinimizeEl = document.querySelector<HTMLButtonElement>("#window-minimize")!;
const windowMaximizeEl = document.querySelector<HTMLButtonElement>("#window-maximize")!;
const mainWindow = getCurrentWindow();
const appVersionEl = document.querySelector<HTMLElement>("#app-version");
const checkUpdateEl = document.querySelector<HTMLButtonElement>("#check-update");

type ModuleRefs = {
  agents: ReturnType<typeof initAgentsPanel> | null;
  wiring: ReturnType<typeof initWiringUi> | null;
  workspace: ReturnType<typeof initWorkspaceUi> | null;
  resources: ReturnType<typeof initResourcesHub> | null;
  firstRun: ReturnType<typeof initFirstRunUi> | null;
};

const refs: ModuleRefs = {
  agents: null,
  wiring: null,
  workspace: null,
  resources: null,
  firstRun: null,
};

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
  if (tab === "resources") {
    void refs.resources?.loadResourcesHub();
  }
  if (tab === "diagnose") {
    refs.firstRun?.onDiagnoseTabVisible();
  }
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
  if (refs.firstRun && refs.firstRun.getPhase() !== "hidden" && refs.firstRun.getPhase() !== "awaitingWiring") {
    refs.firstRun.renderFirstRunUi();
  }
  updateLangButtons();
  await refs.agents?.reloadLocale();
  refs.workspace?.reloadLocale();
  refs.resources?.reloadLocale();
  await refs.wiring?.reloadWiringLocale();
}

refs.agents = initAgentsPanel({
  setMainTab,
  loadWorkspaces: () => refs.workspace!.loadWorkspaces(),
  rewireCurrentMode: (hintEl) => refs.wiring!.rewireCurrentMode(hintEl),
  onDoctorReport: async (report) => {
    if (refs.firstRun && refs.firstRun.getPhase() !== "hidden" && !refs.firstRun.isBusy()) {
      await refs.firstRun.evaluateFirstRunFromReport(report);
    }
  },
});

refs.wiring = initWiringUi({
  setMainTab: (tab) => setMainTab(tab),
  refresh: () => refs.agents!.refresh(),
  loadSkillsInventory: () => refs.resources!.loadSkillsInventory(),
  hideSkillsInventory: () => refs.resources!.hideSkillsInventory(),
});

refs.workspace = initWorkspaceUi({
  onWorkspacesChanged: (doc) => {
    refs.agents!.updateAgentsWorkspaceChip(doc);
  },
});

refs.resources = initResourcesHub();

refs.firstRun = initFirstRunUi({
  panelDiagnoseEl: refs.agents.panelDiagnoseEl,
  repairPreviewByRuntime,
  repairFilterByRuntime,
  getLastReport: () => appState.lastReport,
  renderReport: (report) => refs.agents!.renderReport(report),
  setLoading: (loading) => refs.agents!.setLoading(loading),
  setStatusBanner: (kind, message) => refs.agents!.setStatusBanner(kind, message),
  updateAgentsSecurityOverview: (report) => refs.agents!.updateAgentsSecurityOverview(report),
  setMainTab: (tab) => setMainTab(tab),
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
  if (tab === "diagnose" || tab === "resources" || tab === "provider" || tab === "workspace") {
    setMainTab(tab);
  }
});

windowCloseEl.addEventListener("click", () => {
  // Hide to tray — destroying main while Ask is alive leaves a tray-only
  // process that cannot reopen the shell without a rebuild.
  void mainWindow.hide();
});

windowMinimizeEl.addEventListener("click", () => {
  // Undecorated WebView2 on Windows often fails to restore from a real
  // minimize via the taskbar; hide to tray and restore via tray / relaunch.
  const isWindows = navigator.userAgent.includes("Windows");
  if (isWindows) {
    void mainWindow.hide();
  } else {
    void mainWindow.minimize();
  }
});

windowMaximizeEl.addEventListener("click", () => {
  void mainWindow.toggleMaximize();
});

widgetToolbarEl.addEventListener("dblclick", (event) => {
  if (!(event.target as HTMLElement).closest("button")) {
    void mainWindow.toggleMaximize();
  }
});

void listen<DoctorReport>("doctor-report", (event) => {
  void (async () => {
    await refs.agents!.renderReport(event.payload);
    if (refs.firstRun && refs.firstRun.getPhase() !== "hidden" && !refs.firstRun.isBusy()) {
      await refs.firstRun.evaluateFirstRunFromReport(event.payload);
    }
  })();
});

void listen<{ tab?: string }>("main-navigate", (event) => {
  const tab = event.payload?.tab;
  if (tab === "diagnose" || tab === "resources" || tab === "provider" || tab === "workspace") {
    setMainTab(tab);
  }
});

void listen("workspace-changed", () => {
  void refs.workspace!.loadWorkspaces();
  void refs.resources!.loadMcpStatus();
});

void listen<WorkspaceDoctorReport>("workspace-doctor-report", (event) => {
  refs.workspace!.renderWorkspaceChecks(event.payload);
});

setLocale(getLocale());
applyStaticI18n();
refs.wiring.syncProviderPanelToEdition();
refs.wiring.updateFooterCopy(isTeamEdition() ? "team" : "personal");
refs.wiring.updateWiringModeFootnote();
updateLangButtons();
refs.wiring.refreshPresetGroupLabels();
refs.wiring.applyProviderPreset("custom");
refs.wiring.showPersonalListView();

// A webview reload can preserve the native width from an open diagnose panel
// while resetting the frontend's detail state. Always restore compact startup.
void refs.agents.setMainWindowWidth(refs.agents.MAIN_COMPACT_WIDTH);
void refs.agents.loadProfiles();
void refs.workspace.loadWorkspaces();
void refs.workspace.loadRemoteProjects();
if (isTeamEdition()) {
  void refs.wiring.loadEvotownStatus();
} else {
  void refs.wiring.loadPersonalProviderStatus();
}
void refs.wiring.loadModeStatus();
// Do not call loadMcpStatus() on boot — discover_chrome / CDP probe must not
// wake Chrome until the user opens Resources or clicks Browser smoke.
refs.firstRun.initPersonalFirstRun();
if (refs.firstRun.getPhase() === "hidden") {
  void refs.agents.refresh();
}
void initUpdaterUi({ versionEl: appVersionEl, checkBtn: checkUpdateEl });
