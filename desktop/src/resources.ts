import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { formatCount, formatRate } from "./format";
import { getLocale, t, type MessageKey } from "./i18n";
import type {
  BrowserMcpDiagnoseWireReport,
  BrowserMcpTargetAction,
  BrowserMcpTargetStatus,
  McpInventoryItem,
  McpModuleStatus,
  ResourceFilter,
  ResourceRow,
  SkillMountReport,
  SkillsInventoryReport,
} from "./types";

const MCP_SHOW_UI_KEY = "agent-doctor.mcp.showUi";
const MCP_USER_DATA_DIR_KEY = "agent-doctor.mcp.userDataDir";
const MCP_PROFILE_DIRECTORY_KEY = "agent-doctor.mcp.profileDirectory";

const subtitleEl = document.querySelector<HTMLElement>("#resources-subtitle")!;
const refreshAllEl = document.querySelector<HTMLButtonElement>("#resources-refresh-all")!;
const closeEl = document.querySelector<HTMLButtonElement>("#resources-close")!;
const sectionTabsEl = document.querySelector<HTMLElement>("#resources-section-tabs")!;
const filtersEl = document.querySelector<HTMLElement>("#resources-filters")!;
const searchEl = document.querySelector<HTMLInputElement>("#resources-search")!;
const listEl = document.querySelector<HTMLUListElement>("#resources-list")!;
const emptyEl = document.querySelector<HTMLElement>("#resources-empty")!;
const footnoteEl = document.querySelector<HTMLElement>("#resources-footnote")!;

const mcpBrowserBadgeEl = document.querySelector<HTMLElement>("#mcp-browser-badge")!;
const mcpChromeEl = document.querySelector<HTMLElement>("#mcp-chrome")!;
const mcpCdpEl = document.querySelector<HTMLElement>("#mcp-cdp")!;
const mcpConfiguredEl = document.querySelector<HTMLElement>("#mcp-configured")!;
const mcpBinaryEl = document.querySelector<HTMLElement>("#mcp-binary")!;
const mcpShowUiEl = document.querySelector<HTMLInputElement>("#mcp-show-ui")!;
const mcpUserDataDirEl = document.querySelector<HTMLInputElement>("#mcp-user-data-dir")!;
const mcpProfileDirectoryEl = document.querySelector<HTMLInputElement>("#mcp-profile-directory")!;
const mcpProfileSystemEl = document.querySelector<HTMLButtonElement>("#mcp-profile-system")!;
const mcpProfileIsolatedEl = document.querySelector<HTMLButtonElement>("#mcp-profile-isolated")!;
const mcpRefreshEl = document.querySelector<HTMLButtonElement>("#mcp-refresh")!;
const mcpDiagnoseWireEl = document.querySelector<HTMLButtonElement>("#mcp-diagnose-wire")!;
const mcpTargetsEl = document.querySelector<HTMLUListElement>("#mcp-targets")!;
const mcpSnippetEl = document.querySelector<HTMLElement>("#mcp-snippet")!;
const mcpFootnoteEl = document.querySelector<HTMLElement>("#mcp-footnote")!;

let lastSkillsInventory: SkillsInventoryReport | null = null;
let lastMcpStatus: McpModuleStatus | null = null;
let lastWireActions: BrowserMcpTargetAction[] | null = null;
let resourceFilter: ResourceFilter = "all";
let resourceQuery = "";
let mcpConfigureInFlight = false;

function applyI18n(): void {
  document.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    const key = el.dataset.i18n as MessageKey | undefined;
    if (key) el.textContent = t(key);
  });
  document.querySelectorAll<HTMLElement>("[data-i18n-title]").forEach((el) => {
    const key = el.dataset.i18nTitle as MessageKey | undefined;
    if (key) el.title = t(key);
  });
  searchEl.placeholder = t("chat.resourcesSearch");
  mcpDiagnoseWireEl.title = t("mcp.diagnoseWireHint");
  document.documentElement.lang = getLocale() === "zh" ? "zh-CN" : "en";
}

function setSection(section: "catalog" | "browser"): void {
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
}

function persistShowBrowserUi(show: boolean): void {
  try {
    localStorage.setItem(MCP_SHOW_UI_KEY, show ? "1" : "0");
  } catch {
    // ignore
  }
}

function persistUserDataDir(path: string): void {
  try {
    localStorage.setItem(MCP_USER_DATA_DIR_KEY, path);
  } catch {
    // ignore
  }
}

function persistProfileDirectory(name: string): void {
  try {
    localStorage.setItem(MCP_PROFILE_DIRECTORY_KEY, name.trim() || "Default");
  } catch {
    // ignore
  }
}

function isShowBrowserUi(): boolean {
  return mcpShowUiEl.checked;
}

function selectedUserDataDir(): string {
  return mcpUserDataDirEl.value.trim();
}

function selectedProfileDirectory(): string {
  return mcpProfileDirectoryEl.value.trim() || "Default";
}

function configuredBrowserArg(status: McpModuleStatus, flag: string): string | null {
  const browser = status.inventory.servers.find((server) => server.is_browser);
  if (!browser) return null;
  const idx = browser.args.findIndex((arg) => arg === flag);
  if (idx >= 0 && browser.args[idx + 1]) return browser.args[idx + 1];
  return null;
}

function syncShowBrowserUiPreference(status: McpModuleStatus): void {
  try {
    const saved = localStorage.getItem(MCP_SHOW_UI_KEY);
    if (saved === "0" || saved === "1") {
      mcpShowUiEl.checked = saved === "1";
      return;
    }
  } catch {
    // fall through
  }
  const browser = status.inventory.servers.find((server) => server.is_browser);
  mcpShowUiEl.checked = browser ? !browser.args.includes("--headless") : true;
}

function syncProfileModeButtons(): void {
  const dir = selectedUserDataDir();
  const isolated = lastMcpStatus?.browser.isolated_user_data_dir || "";
  const system = lastMcpStatus?.browser.system_user_data_dir || "";
  mcpProfileIsolatedEl.classList.toggle("is-active", Boolean(isolated) && dir === isolated);
  mcpProfileSystemEl.classList.toggle("is-active", Boolean(system) && dir === system);
}

function syncUserDataDirPreference(status: McpModuleStatus): void {
  const chrome = status.browser;
  const fromConfig = configuredBrowserArg(status, "--user-data-dir");
  let saved: string | null = null;
  try {
    saved = localStorage.getItem(MCP_USER_DATA_DIR_KEY);
  } catch {
    saved = null;
  }
  mcpUserDataDirEl.value =
    (fromConfig && fromConfig.trim()) ||
    (saved && saved.trim()) ||
    chrome.isolated_user_data_dir ||
    chrome.user_data_dir ||
    "";

  const profileFromConfig = configuredBrowserArg(status, "--profile-directory");
  let savedProfile: string | null = null;
  try {
    savedProfile = localStorage.getItem(MCP_PROFILE_DIRECTORY_KEY);
  } catch {
    savedProfile = null;
  }
  mcpProfileDirectoryEl.value =
    (profileFromConfig && profileFromConfig.trim()) ||
    (savedProfile && savedProfile.trim()) ||
    chrome.profile_directory ||
    "Default";
  syncProfileModeButtons();
}

function refreshMcpSnippet(): void {
  if (!lastMcpStatus) return;
  const port = lastMcpStatus.browser.port;
  const args = ["mcp", "browser", "--port", String(port)];
  if (!isShowBrowserUi()) args.push("--headless");
  const dir = selectedUserDataDir();
  if (dir) args.push("--user-data-dir", dir);
  args.push("--profile-directory", selectedProfileDirectory());
  mcpSnippetEl.textContent = JSON.stringify(
    {
      mcpServers: {
        browser: {
          command: lastMcpStatus.binary,
          args,
        },
      },
    },
    null,
    2,
  );
}

function targetStatusLabel(target: BrowserMcpTargetStatus): string {
  if (!target.installed) return t("mcp.targetNotInstalled");
  return target.configured
    ? t("mcp.targetInstalledConfigured")
    : t("mcp.targetInstalledMissing");
}

function targetActionLabel(action: BrowserMcpTargetAction): string {
  if (action.action === "wrote") return t("mcp.targetWrote");
  if (action.action === "skipped_not_installed") return t("mcp.targetSkipped");
  return t("mcp.targetFailed");
}

function renderMcpTargets(
  targets: BrowserMcpTargetStatus[],
  actions: BrowserMcpTargetAction[] | null = lastWireActions,
): void {
  mcpTargetsEl.replaceChildren();
  for (const target of targets) {
    const action = actions?.find((item) => item.runtime_id === target.runtime_id);
    const li = document.createElement("li");
    li.className = "mcp-target-row";
    const installed = action?.installed ?? target.installed;
    const configured = action?.action === "wrote" ? true : target.configured;
    const tone = action
      ? action.ok
        ? action.action === "wrote"
          ? "ok"
          : "muted"
        : "bad"
      : !installed
        ? "muted"
        : configured
          ? "ok"
          : "warn";
    li.classList.add(`tone-${tone}`);

    const name = document.createElement("strong");
    name.textContent = target.display_name;
    const status = document.createElement("span");
    status.className = "mcp-target-status";
    status.textContent = action ? targetActionLabel(action) : targetStatusLabel(target);
    const detail = document.createElement("span");
    detail.className = "mcp-target-detail";
    detail.textContent =
      action?.message || (installed ? target.runtime_id : t("mcp.targetNotInstalled"));
    li.append(name, status, detail);
    mcpTargetsEl.appendChild(li);
  }
}

function renderMcpBrowserStatus(status: McpModuleStatus): void {
  lastMcpStatus = status;
  const chrome = status.browser;
  mcpChromeEl.textContent = chrome.chrome_found
    ? t("mcp.chromeOk", { version: chrome.version || chrome.binary || "OK" })
    : t("mcp.chromeMissing");
  mcpChromeEl.title = chrome.binary || "";
  mcpCdpEl.textContent = chrome.cdp_connected
    ? t("mcp.cdpConnected", { port: String(chrome.port) })
    : t("mcp.cdpIdle", { port: String(chrome.port) });
  mcpConfiguredEl.textContent =
    status.configured_runtimes.length > 0
      ? t("mcp.configuredList", { list: status.configured_runtimes.join(", ") })
      : t("mcp.configuredNone");
  mcpBinaryEl.textContent = status.binary;
  mcpBinaryEl.title = status.binary;
  syncShowBrowserUiPreference(status);
  syncUserDataDirPreference(status);
  refreshMcpSnippet();
  renderMcpTargets(status.targets ?? []);

  const snippetError =
    status.config_snippet &&
    typeof status.config_snippet === "object" &&
    status.config_snippet !== null &&
    "error" in status.config_snippet
      ? String((status.config_snippet as { error?: unknown }).error ?? "")
      : "";
  const cliBroken =
    !status.binary.trim() ||
    /VCRUNTIME|Visual C\+\+|could not start|Could not find the Agent Doctor CLI/i.test(snippetError);
  if (!mcpConfigureInFlight) {
    mcpFootnoteEl.textContent = cliBroken ? t("mcp.cliUnresolved") : "";
  }

  mcpBrowserBadgeEl.classList.remove("ok", "warn", "muted", "bad");
  if (cliBroken) {
    mcpBrowserBadgeEl.textContent = t("mcp.badgePartial");
    mcpBrowserBadgeEl.classList.add("warn");
  } else if (!chrome.chrome_found) {
    mcpBrowserBadgeEl.textContent = t("mcp.badgeMissing");
    mcpBrowserBadgeEl.classList.add("bad");
  } else if (status.configured_runtimes.length > 0) {
    mcpBrowserBadgeEl.textContent = t("mcp.badgeReady");
    mcpBrowserBadgeEl.classList.add("ok");
  } else {
    mcpBrowserBadgeEl.textContent = t("mcp.badgePartial");
    mcpBrowserBadgeEl.classList.add("warn");
  }

  const canWire = chrome.chrome_found && !cliBroken && !mcpConfigureInFlight;
  mcpDiagnoseWireEl.disabled = !canWire;
}

function buildResourceRows(): ResourceRow[] {
  const rows: ResourceRow[] = [];
  const mcpGroups = new Map<string, McpInventoryItem[]>();
  for (const server of lastMcpStatus?.inventory.servers ?? []) {
    const key = server.name.trim().toLowerCase();
    const group = mcpGroups.get(key) ?? [];
    group.push(server);
    mcpGroups.set(key, group);
  }

  for (const servers of mcpGroups.values()) {
    const primary = servers[0];
    const runtimes = [...new Set(servers.map((server) => server.runtime_hint))].map((runtime) => {
      if (runtime === "claude-code") return "Claude";
      if (runtime === "codex") return "Codex";
      if (runtime === "openclaw") return "OpenClaw";
      if (runtime === "hermes") return "Hermes";
      if (runtime === "deepseek-harness") return "DeepSeek Harness";
      if (runtime === "shared") return "Shared";
      return runtime;
    });
    const issues = servers.filter((server) => !server.healthy);
    const bindingLabel = t("resources.mcpBindings", { count: String(servers.length) });
    rows.push({
      kind: "mcp",
      name: primary.name,
      sub: runtimes.join(" · "),
      meta:
        issues.length > 0
          ? t("resources.mcpBindingIssues", {
              issues: String(issues.length),
              count: String(servers.length),
            })
          : primary.is_browser
            ? `${t("resources.mcpBrowser")} · ${bindingLabel}`
            : `${t("resources.mcpHealthy")} · ${bindingLabel}`,
      tone: issues.length > 0 ? "bad" : "ok",
      issue: issues.length > 0,
    });
  }

  for (const skill of lastSkillsInventory?.skills ?? []) {
    const mounted = skill.agents.filter((a) => a.mounted).length;
    const needsMount = skill.agents.some((a) => !a.mounted);
    rows.push({
      kind: "skill",
      name: skill.name || skill.skill_id,
      sub: t("resources.skillMounted", { count: String(mounted) }),
      meta: t("resources.skillUsage", {
        calls: formatCount(skill.call_count),
        rate: formatRate(skill.first_success_rate),
      }),
      tone: needsMount ? "warn" : "ok",
      issue: needsMount,
      skillId: skill.skill_id,
      needsMount,
    });
  }

  return rows;
}

function skillDescription(skillId: string | undefined): string {
  if (!skillId) return "";
  return lastSkillsInventory?.skills.find((s) => s.skill_id === skillId)?.description?.trim() || "";
}

function mcpPrimary(name: string): McpInventoryItem | undefined {
  return lastMcpStatus?.inventory.servers.find((s) => s.name === name);
}

function renderResourcesList(): void {
  const rows = buildResourceRows().filter((row) => {
    if (resourceFilter === "all") {
      // keep
    } else if (resourceFilter === "issue") {
      if (!row.issue) return false;
    } else if (row.kind !== resourceFilter) {
      return false;
    }
    if (!resourceQuery) return true;
    const desc =
      row.kind === "skill"
        ? skillDescription(row.skillId)
        : mcpPrimary(row.name)?.config_path || "";
    return `${row.name} ${row.sub} ${row.meta} ${desc}`.toLowerCase().includes(resourceQuery);
  });

  listEl.replaceChildren();
  emptyEl.hidden = rows.length > 0;
  emptyEl.textContent = resourceQuery ? t("chat.resourcesNoMatch") : t("resources.empty");
  footnoteEl.textContent = lastMcpStatus?.inventory.workspace_name
    ? `${lastMcpStatus.inventory.workspace_name}${
        lastMcpStatus.inventory.workspace_path ? ` · ${lastMcpStatus.inventory.workspace_path}` : ""
      }`
    : "";

  const uniqueMcp = new Set(
    (lastMcpStatus?.inventory.servers ?? []).map((s) => s.name.trim().toLowerCase()),
  );
  subtitleEl.textContent = t("resources.windowSubtitle", {
    skills: String(lastSkillsInventory?.skills.length ?? 0),
    mcp: String(uniqueMcp.size),
  });

  for (const row of rows) {
    const li = document.createElement("li");
    li.className = "res-catalog-item";

    const icon = document.createElement("span");
    icon.className = "res-catalog-icon";
    const browser = row.kind === "mcp" && Boolean(mcpPrimary(row.name)?.is_browser);
    icon.classList.add(row.kind === "skill" ? "is-skill" : browser ? "is-browser" : "is-mcp");
    icon.textContent = (row.name.trim().charAt(0) || "?").toUpperCase();

    const body = document.createElement("div");
    body.className = "res-catalog-body";
    const titleRow = document.createElement("div");
    titleRow.className = "res-catalog-title-row";
    const strong = document.createElement("strong");
    strong.textContent = row.name;
    const badge = document.createElement("span");
    badge.className = "res-catalog-badge";
    badge.textContent = row.kind === "skill" ? "Skill" : browser ? "Browser" : "MCP";
    titleRow.append(strong, badge);
    const desc = document.createElement("div");
    desc.className = "res-catalog-desc";
    desc.textContent = row.kind === "skill" ? skillDescription(row.skillId) || row.sub : row.sub;
    body.append(titleRow, desc);

    const metaWrap = document.createElement("div");
    metaWrap.className = "res-catalog-meta";
    const meta = document.createElement("span");
    meta.className = `tone-${row.tone}`;
    meta.textContent = row.meta;
    metaWrap.appendChild(meta);

    if (row.kind === "skill" && row.needsMount && row.skillId) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "btn-secondary btn-compact";
      btn.textContent = t("resources.mount");
      const skillId = row.skillId;
      btn.addEventListener("click", () => {
        void mountSkill(skillId);
      });
      metaWrap.appendChild(btn);
    }

    li.append(icon, body, metaWrap);
    listEl.appendChild(li);
  }
}

async function mountSkill(skillId: string): Promise<void> {
  footnoteEl.textContent = t("skills.mounting");
  try {
    const report = await invoke<SkillMountReport>("mount_synced_skills_command", {
      skillIds: [skillId],
      runtimes: null,
    });
    footnoteEl.textContent = t("skills.mountOk", {
      mounted: String(report.mounted),
      skipped: String(report.skipped),
      failed: String(report.failed),
    });
    await loadSkills();
  } catch (error) {
    footnoteEl.textContent = t("skills.mountFailed", { error: String(error) });
  }
}

async function loadSkills(): Promise<void> {
  try {
    lastSkillsInventory = await invoke<SkillsInventoryReport>("list_skills_inventory_command", {
      remoteStats: false,
    });
  } catch {
    lastSkillsInventory = null;
  }
  renderResourcesList();
}

async function loadMcpStatus(): Promise<void> {
  try {
    const status = await invoke<McpModuleStatus>("mcp_status_command", {
      port: null,
      probeChrome: false,
    });
    renderMcpBrowserStatus(status);
    renderResourcesList();
  } catch (error) {
    lastMcpStatus = null;
    mcpBrowserBadgeEl.textContent = "—";
    mcpBrowserBadgeEl.className = "badge muted";
    mcpFootnoteEl.textContent = t("mcp.loadFailed", { error: String(error) });
    mcpTargetsEl.replaceChildren();
    renderResourcesList();
  }
}

async function refreshAll(): Promise<void> {
  await Promise.all([loadMcpStatus(), loadSkills()]);
}

async function diagnoseAndWireBrowserMcp(): Promise<void> {
  if (mcpConfigureInFlight) return;
  mcpConfigureInFlight = true;
  mcpDiagnoseWireEl.disabled = true;
  mcpFootnoteEl.textContent = t("mcp.configuring");
  try {
    const showUi = isShowBrowserUi();
    persistShowBrowserUi(showUi);
    const userDataDir = selectedUserDataDir();
    const profileDirectory = selectedProfileDirectory();
    persistUserDataDir(userDataDir);
    persistProfileDirectory(profileDirectory);
    const report = await invoke<BrowserMcpDiagnoseWireReport>("mcp_diagnose_wire_command", {
      port: null,
      headless: !showUi,
      userDataDir: userDataDir || null,
      profileDirectory,
    });
    lastWireActions = report.targets;
    if (report.wrote > 0) {
      mcpFootnoteEl.textContent = t("mcp.diagnoseWireOk", {
        wrote: String(report.wrote),
        skipped: String(report.skipped),
        failed: String(report.failed),
      });
    } else if (report.issues.length > 0) {
      mcpFootnoteEl.textContent = report.issues[0]?.message || t("mcp.diagnoseWireNone");
    } else {
      mcpFootnoteEl.textContent = t("mcp.diagnoseWireNone");
    }
    await loadMcpStatus();
    if (lastMcpStatus) {
      renderMcpTargets(lastMcpStatus.targets ?? [], lastWireActions);
    }
  } catch (error) {
    mcpFootnoteEl.textContent = t("mcp.configureFailed", { error: String(error) });
  } finally {
    mcpConfigureInFlight = false;
    const chromeOk = lastMcpStatus?.browser.chrome_found ?? false;
    const cliOk = Boolean(lastMcpStatus?.binary?.trim());
    mcpDiagnoseWireEl.disabled = !(chromeOk && cliOk);
  }
}

sectionTabsEl.addEventListener("click", (event) => {
  const btn = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-section]");
  if (!btn?.dataset.section) return;
  if (btn.dataset.section === "catalog" || btn.dataset.section === "browser") {
    setSection(btn.dataset.section);
  }
});

filtersEl.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-res-filter]");
  const filter = button?.dataset.resFilter as ResourceFilter | undefined;
  if (!filter) return;
  resourceFilter = filter;
  filtersEl.querySelectorAll<HTMLButtonElement>("[data-res-filter]").forEach((chip) => {
    chip.classList.toggle("is-active", chip.dataset.resFilter === filter);
  });
  renderResourcesList();
});

searchEl.addEventListener("input", () => {
  resourceQuery = searchEl.value.trim().toLowerCase();
  renderResourcesList();
});

refreshAllEl.addEventListener("click", () => {
  void refreshAll();
});
mcpRefreshEl.addEventListener("click", () => {
  lastWireActions = null;
  void loadMcpStatus();
});
closeEl.addEventListener("click", () => {
  void invoke("close_resources_window_command", { destroy: false });
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
    lastMcpStatus?.browser.system_user_data_dir || lastMcpStatus?.browser.user_data_dir || "";
  mcpUserDataDirEl.value = path;
  mcpProfileDirectoryEl.value = "Default";
  persistUserDataDir(path);
  persistProfileDirectory("Default");
  refreshMcpSnippet();
  syncProfileModeButtons();
});
mcpProfileIsolatedEl.addEventListener("click", () => {
  const path = lastMcpStatus?.browser.isolated_user_data_dir || "";
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

applyI18n();
void refreshAll();
void listen<{ section?: string }>("resources-window-focus", (event) => {
  const section = event.payload?.section;
  if (section === "browser" || section === "catalog") setSection(section);
  void refreshAll();
});
