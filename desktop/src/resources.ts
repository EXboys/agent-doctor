import { listen } from "@tauri-apps/api/event";
import { getLocale, t } from "./i18n";
import type { MessageKey } from "./i18n";
import type { ResourceRow } from "./types";
import { resourcesState, subtitleEl, sectionTabsEl, mainHeadEl, skillsPanelEl, skillScopeEl, skillScopeHintEl, panelLeadEl, skillsStickyEl, toolFiltersEl, searchEl, footnoteEl, toolsFootnoteEl, mallAccountEl, mallLoginEl, mallLogoutEl, mcpShowUiEl, mcpUserDataDirEl, mcpProfileDirectoryEl, mcpProfileSystemEl, mcpProfileIsolatedEl, mcpRefreshEl, mcpDiagnoseWireEl, personalEdition } from "./resources-state";
import type { ResourcesSection, SkillScope, ToolFilter } from "./resources-state";
import { catalogAgents, countAgentMatches, loadDoctor, renderAgentCatalog } from "./resources-agents";
import { filteredMallItems, filteredMallToolItems, loadMall, mallToolPool, renderMallAccount, renderToolsList, signOutMallAccount, startMallLogin } from "./resources-mall";
import { browserStatusIsFresh, buildMcpRows, diagnoseAndWireBrowserMcp, loadMcpStatus, persistProfileDirectory, persistShowBrowserUi, persistUserDataDir, refreshMcpSnippet, renderMcpBrowserStatus, selectedProfileDirectory, selectedUserDataDir, syncProfileModeButtons } from "./resources-browser";
import { buildLocalSkillEntries, entryMatchesQuery, isStoreScope, loadSkills, onSkillsPanelScroll, renderSkillsList } from "./resources-skills";
import { loadRuntimeCatalog } from "./runtime-catalog";

export function applyI18n(): void {
  document.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    const key = el.dataset.i18n as MessageKey | undefined;
    if (key) el.textContent = t(key);
  });
  document.querySelectorAll<HTMLElement>("[data-i18n-title]").forEach((el) => {
    const key = el.dataset.i18nTitle as MessageKey | undefined;
    if (key) el.title = t(key);
  });
  searchEl.placeholder = t("resources.searchGlobal");
  mcpDiagnoseWireEl.title = t("mcp.diagnoseWireHint");
  document.documentElement.lang = getLocale() === "zh" ? "zh-CN" : "en";
  skillScopeEl.setAttribute("aria-label", t("resources.skillScopeAria"));
  syncSkillScopeChrome();
}

export function skillsPanelActive(): boolean {
  return resourcesState.activeSection === "skills";
}

export function currentSkillScope(): SkillScope {
  return personalEdition && resourcesState.skillFilter === "store" ? "store" : "local";
}

export function showScopeChrome(): boolean {
  return personalEdition && (resourcesState.activeSection === "skills" || resourcesState.activeSection === "tools");
}

export function syncSkillScopeChrome(): void {
  const showScope = showScopeChrome();
  const scope = currentSkillScope();
  mainHeadEl.classList.toggle("is-skills", showScope);
  skillScopeEl.hidden = !showScope;
  skillScopeHintEl.hidden = !showScope;
  mallAccountEl.hidden = !(showScope && scope === "store");
  if (!mallAccountEl.hidden) renderMallAccount();
  skillScopeEl.querySelectorAll<HTMLButtonElement>("[data-skill-scope]").forEach((btn) => {
    const active = btn.dataset.skillScope === scope;
    btn.classList.toggle("is-active", active);
    btn.setAttribute("aria-selected", active ? "true" : "false");
  });
  if (resourcesState.activeSection === "tools") {
    skillScopeHintEl.textContent =
      scope === "store" ? t("resources.toolScopeStoreHint") : t("resources.toolScopeLocalHint");
  } else {
    skillScopeHintEl.textContent =
      scope === "store" ? t("resources.skillScopeStoreHint") : t("resources.skillScopeLocalHint");
  }
  skillsStickyEl.hidden = resourcesState.activeSection !== "skills";
  syncPanelLead();
}

export function syncPanelLead(): void {
  if (resourcesState.activeSection === "agents") {
    panelLeadEl.hidden = false;
    const missing = catalogAgents().some((runtime) => !runtime.installed);
    panelLeadEl.textContent = missing
      ? t("resources.agentCatalogHint")
      : t("resources.agentCatalogHintAllOn");
    return;
  }
  if (resourcesState.activeSection === "browser") {
    panelLeadEl.hidden = false;
    panelLeadEl.textContent = t("resources.hubBrowserCardDesc");
    return;
  }
  panelLeadEl.hidden = true;
}

export function paintSection(section: ResourcesSection): void {
  resourcesState.activeSection = section;
  sectionTabsEl.querySelectorAll<HTMLButtonElement>("[data-section]").forEach((btn) => {
    const active = btn.dataset.section === section;
    btn.classList.toggle("is-active", active);
    btn.setAttribute("aria-selected", active ? "true" : "false");
  });
  document.querySelectorAll<HTMLElement>("[data-section-panel]").forEach((panel) => {
    const active = panel.dataset.sectionPanel === section;
    panel.classList.toggle("is-active", active);
    panel.hidden = !active;
  });
  syncSkillScopeChrome();
}

export function setSkillScope(scope: SkillScope): void {
  if (scope === "store" && !personalEdition) scope = "local";
  if (scope === "store") {
    resourcesState.skillFilter = "store";
    resourcesState.agentFilter = "all";
  } else if (resourcesState.skillFilter === "store") {
    resourcesState.skillFilter = "all";
  }
  syncSkillScopeChrome();
  if (resourcesState.activeSection === "tools") {
    if (scope === "store") {
      if (resourcesState.lastMallCatalog) renderToolsList();
      else void loadMall();
    } else if (resourcesState.lastMcpStatus) {
      renderToolsList();
    } else {
      void loadMcpStatus();
    }
    return;
  }
  if (scope === "store") {
    if (resourcesState.lastMallCatalog) renderResourcesList();
    else void loadMall();
    return;
  }
  if (resourcesState.lastSkillsInventory || resourcesState.lastMallCatalog) renderResourcesList();
  else void loadSkills();
}

export function setSection(section: ResourcesSection | "mall"): void {
  if (section === "mall") {
    paintSection("skills");
    if ((resourcesState.activeSection === "skills" || section === "mall") && !resourcesState.lastDoctorReport) {
      void loadDoctor();
    }
    setSkillScope(personalEdition ? "store" : "local");
    return;
  }
  paintSection(section);
  if (section === "browser") {
    if (resourcesState.lastMcpStatus && !resourcesState.lastMcpStatus.browser_deferred) {
      renderMcpBrowserStatus(resourcesState.lastMcpStatus);
    }
    if (!browserStatusIsFresh()) void loadMcpStatus({ discoverChrome: true });
    return;
  }
  if ((section === "agents" || section === "skills") && !resourcesState.lastDoctorReport) {
    void loadDoctor();
  }
  if (section === "skills" || section === "tools") {
    setSkillScope(currentSkillScope());
  }
}

export function setSkillsFootnote(message: string | null): void {
  resourcesState.skillsStatusMessage = message;
  syncSkillsFootnote();
  if (resourcesState.activeSection === "tools") {
    toolsFootnoteEl.textContent = message ?? defaultToolsFootnote();
  }
}

export function defaultToolsFootnote(): string {
  if (isStoreScope()) {
    if (resourcesState.lastMallCatalog && resourcesState.lastMallCatalog.items.length > 0) {
      const rows = filteredMallToolItems();
      return t("resources.mallListHint", {
        count: String(rows.length),
        total: String(mallToolPool().length),
      });
    }
    return t("resources.mallFootnoteShort");
  }
  return "";
}

export function defaultSkillsFootnote(): string {
  if (isStoreScope()) {
    if (resourcesState.lastMallCatalog && resourcesState.lastMallCatalog.items.length > 0) {
      const rows = filteredMallItems();
      return t("resources.mallListHint", {
        count: String(rows.length),
        total: String(resourcesState.lastMallCatalog.items.length),
      });
    }
    return t("resources.mallFootnoteShort");
  }
  const inv = resourcesState.lastMcpStatus?.inventory;
  if (inv?.workspace_name) {
    return `${inv.workspace_name}${inv.workspace_path ? ` · ${inv.workspace_path}` : ""}`;
  }
  return "";
}

export function syncSkillsFootnote(): void {
  footnoteEl.textContent = resourcesState.skillsStatusMessage ?? defaultSkillsFootnote();
}

export function rowMatchesQuery(row: ResourceRow, extra = ""): boolean {
  if (!resourcesState.resourceQuery) return true;
  return `${row.name} ${row.sub} ${row.meta} ${extra}`.toLowerCase().includes(resourcesState.resourceQuery);
}

export function countLocalSkillMatches(): number {
  return buildLocalSkillEntries().filter((entry) => entryMatchesQuery(entry)).length;
}

export function countLocalToolMatches(): number {
  return buildMcpRows().filter((row) => rowMatchesQuery(row)).length;
}

export function maybeJumpToSearchHits(): void {
  if (!resourcesState.resourceQuery) return;
  const skillHits = countLocalSkillMatches();
  const toolHits = countLocalToolMatches();
  const mallToolHits = personalEdition ? filteredMallToolItems().length : 0;
  const agentHits = countAgentMatches();
  const mallHits = personalEdition ? filteredMallItems().length : 0;
  if (resourcesState.activeSection === "browser") {
    if (agentHits > 0) setSection("agents");
    else if (skillHits > 0) setSection("skills");
    else if (mallHits > 0) setSection("mall");
    else if (toolHits > 0 || mallToolHits > 0) setSection("tools");
    else setSection("skills");
    return;
  }
  if (resourcesState.activeSection === "skills" && currentSkillScope() === "store" && mallHits === 0) {
    if (skillHits > 0) setSkillScope("local");
    else if (agentHits > 0) setSection("agents");
    else if (toolHits > 0 || mallToolHits > 0) setSection("tools");
    return;
  }
  if (resourcesState.activeSection === "skills" && currentSkillScope() === "local" && skillHits === 0) {
    if (mallHits > 0) setSkillScope("store");
    else if (agentHits > 0) setSection("agents");
    else if (toolHits > 0 || mallToolHits > 0) setSection("tools");
  } else if (resourcesState.activeSection === "tools" && currentSkillScope() === "store" && mallToolHits === 0) {
    if (toolHits > 0) setSkillScope("local");
    else if (skillHits > 0) setSection("skills");
    else if (mallHits > 0) setSection("mall");
    else if (agentHits > 0) setSection("agents");
    return;
  } else if (resourcesState.activeSection === "tools" && currentSkillScope() === "local" && toolHits === 0) {
    if (mallToolHits > 0) setSkillScope("store");
    else if (skillHits > 0) setSection("skills");
    else if (mallHits > 0) setSection("mall");
    else if (agentHits > 0) setSection("agents");
  } else if (resourcesState.activeSection === "agents" && agentHits === 0) {
    if (skillHits > 0) setSection("skills");
    else if (mallHits > 0) setSection("mall");
    else if (toolHits > 0) setSection("tools");
  }
}

export function renderResourcesList(): void {
  const uniqueMcp = new Set(
    (resourcesState.lastMcpStatus?.inventory.servers ?? [])
      .filter((s) => !s.is_browser)
      .map((s) => s.name.trim().toLowerCase()),
  );
  if (subtitleEl) {
    subtitleEl.textContent = t("resources.windowSubtitle", {
      skills: String(resourcesState.lastSkillsInventory?.skills.length ?? 0),
      mcp: String(uniqueMcp.size),
    });
  }
  renderSkillsList();
  renderToolsList();
  if (!resourcesState.skillsStatusMessage) {
    syncSkillsFootnote();
  }
}

export function paintVisibleSection(): void {
  if (resourcesState.activeSection === "agents") {
    renderAgentCatalog();
    return;
  }
  if (resourcesState.activeSection === "tools") {
    renderToolsList();
    return;
  }
  if (resourcesState.activeSection === "browser") {
    if (resourcesState.lastMcpStatus && !resourcesState.lastMcpStatus.browser_deferred) {
      renderMcpBrowserStatus(resourcesState.lastMcpStatus);
    }
    if (!browserStatusIsFresh()) void loadMcpStatus({ discoverChrome: true });
    return;
  }
  renderResourcesList();
}

export async function refreshVisible(): Promise<void> {
  if (resourcesState.activeSection === "agents") {
    await loadDoctor();
    void loadMcpStatus();
    void loadSkills();
    return;
  }
  if (resourcesState.activeSection === "tools" || resourcesState.activeSection === "browser") {
    await loadMcpStatus();
    if (personalEdition && isStoreScope()) void loadMall();
    void loadDoctor();
    void loadSkills();
    return;
  }
  await loadSkills();
  if (personalEdition) void loadMall();
  void loadDoctor();
  void loadMcpStatus();
}

function browserStatusReady(): boolean {
  return Boolean(resourcesState.lastMcpStatus && !resourcesState.lastMcpStatus.browser_deferred);
}

export async function refreshAll(force = false): Promise<void> {
  const browserOpen = resourcesState.activeSection === "browser";
  if (resourcesState.refreshInFlight) {
    if (browserOpen && !browserStatusReady()) void loadMcpStatus({ discoverChrome: true });
    return resourcesState.refreshInFlight;
  }
  if (!force && resourcesState.lastRefreshAt > 0 && Date.now() - resourcesState.lastRefreshAt < 12_000) {
    if (browserOpen && !browserStatusReady()) {
      void loadMcpStatus({ discoverChrome: true });
      return;
    }
    paintVisibleSection();
    return;
  }
  resourcesState.refreshInFlight = refreshVisible()
    .then(() => {
      resourcesState.lastRefreshAt = Date.now();
    })
    .finally(() => {
      resourcesState.refreshInFlight = null;
    });
  return resourcesState.refreshInFlight;
}

sectionTabsEl.addEventListener("click", (event) => {
  const btn = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-section]");
  if (!btn?.dataset.section) return;
  if (
    btn.dataset.section === "agents" ||
    btn.dataset.section === "skills" ||
    btn.dataset.section === "tools" ||
    btn.dataset.section === "browser"
  ) {
    setSection(btn.dataset.section as ResourcesSection);
  }
});

skillScopeEl.addEventListener("click", (event) => {
  const btn = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-skill-scope]");
  const scope = btn?.dataset.skillScope;
  if (scope === "local" || scope === "store") setSkillScope(scope);
});

toolFiltersEl.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-tool-filter]");
  const filter = button?.dataset.toolFilter as ToolFilter | undefined;
  if (!filter) return;
  resourcesState.toolFilter = filter;
  renderToolsList();
});

skillsPanelEl.addEventListener("scroll", onSkillsPanelScroll, { passive: true });

mallLoginEl.addEventListener("click", () => {
  void startMallLogin();
});

mallLogoutEl.addEventListener("click", () => {
  void signOutMallAccount();
});

searchEl.addEventListener("input", () => {
  resourcesState.resourceQuery = searchEl.value.trim().toLowerCase();
  maybeJumpToSearchHits();
  renderSkillsList();
  renderToolsList();
  renderAgentCatalog();
});

mcpRefreshEl.addEventListener("click", () => {
  resourcesState.lastWireActions = null;
  void loadMcpStatus();
});

mcpShowUiEl.addEventListener("change", () => {
  persistShowBrowserUi(mcpShowUiEl.checked);
  refreshMcpSnippet();
});
mcpUserDataDirEl.addEventListener("change", () => {
  persistUserDataDir(selectedUserDataDir());
  refreshMcpSnippet();
  syncProfileModeButtons();
});
mcpUserDataDirEl.addEventListener("input", () => {
  refreshMcpSnippet();
  syncProfileModeButtons();
});
mcpProfileDirectoryEl.addEventListener("change", () => {
  persistProfileDirectory(selectedProfileDirectory());
  refreshMcpSnippet();
});
mcpProfileDirectoryEl.addEventListener("input", () => {
  refreshMcpSnippet();
});
mcpProfileSystemEl.addEventListener("click", () => {
  const path =
    resourcesState.lastMcpStatus?.browser.system_user_data_dir || resourcesState.lastMcpStatus?.browser.user_data_dir || "";
  mcpUserDataDirEl.value = path;
  mcpProfileDirectoryEl.value = "Default";
  persistUserDataDir(path);
  persistProfileDirectory("Default");
  refreshMcpSnippet();
  syncProfileModeButtons();
});
mcpProfileIsolatedEl.addEventListener("click", () => {
  const path = resourcesState.lastMcpStatus?.browser.isolated_user_data_dir || "";
  mcpUserDataDirEl.value = path;
  mcpProfileDirectoryEl.value = "Default";
  persistUserDataDir(path);
  persistProfileDirectory("Default");
  refreshMcpSnippet();
  syncProfileModeButtons();
});
mcpDiagnoseWireEl.addEventListener("click", () => {
  void diagnoseAndWireBrowserMcp();
});

function resourcesLaunchSection(): string | null {
  const injected = (window as Window & { __AD_RESOURCES_SECTION__?: string }).__AD_RESOURCES_SECTION__;
  if (typeof injected === "string" && injected.trim()) return injected.trim();
  try {
    const pending = sessionStorage.getItem("ad.resources.pendingSection");
    if (!pending) return null;
    sessionStorage.removeItem("ad.resources.pendingSection");
    return pending;
  } catch {
    return null;
  }
}

function openFocusedSection(section?: string | null): void {
  if (section === "agents") setSection("agents");
  else if (section === "browser") setSection("browser");
  else if (section === "tools" || section === "mcp") setSection("tools");
  else if (section === "mall" || section === "store") setSection("mall");
  else if (section === "skills" || section === "catalog") setSection("skills");
}

applyI18n();
(window as Window & { __AD_RESOURCES_APPLY_SECTION__?: (section: string) => void }).__AD_RESOURCES_APPLY_SECTION__ =
  (section) => {
    openFocusedSection(section);
  };
openFocusedSection(resourcesLaunchSection());
void listen<{ section?: string }>("resources-window-focus", (event) => {
  openFocusedSection(event.payload?.section);
  void refreshAll();
});
void (async () => {
  await loadRuntimeCatalog();
  if (resourcesState.lastRefreshAt === 0 && !resourcesState.refreshInFlight) void refreshAll(true);
})();
