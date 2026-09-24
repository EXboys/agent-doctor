import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getLocale, t, type MessageKey } from "./i18n";
import {
  teamupsLoginFailure,
  teamupsMallInstallFailure,
  withErrorDetail,
} from "./friendly-error";
import { isPersonalEdition } from "./edition";
import {
  classifySkillCategory,
  skillCategoryLabelKey,
  SKILL_CATEGORY_ORDER,
  type SkillCategoryId,
} from "./skill-categories";
import type {
  BrowserMcpDiagnoseWireReport,
  BrowserMcpTargetAction,
  BrowserMcpTargetStatus,
  McpInventoryItem,
  McpModuleStatus,
  ResourceRow,
  SkillAgentUsage,
  SkillInventoryItem,
  SkillMountReport,
  SkillsInventoryReport,
  SyncReport,
  TeamupsAccountStatus,
  TeamupsCatalogItem,
  TeamupsLoginPoll,
  TeamupsLoginStart,
  TeamupsMallCatalog,
} from "./types";

type ResourcesSection = "skills" | "tools" | "browser";
type SkillFilter = "all" | "issue" | "store" | SkillCategoryId;
type ToolFilter = "all" | "issue";
type MallFilter = "all" | "free" | "paid" | "pack" | "skill";

const MCP_SHOW_UI_KEY = "agent-doctor.mcp.showUi";
const MCP_USER_DATA_DIR_KEY = "agent-doctor.mcp.userDataDir";
const MCP_PROFILE_DIRECTORY_KEY = "agent-doctor.mcp.profileDirectory";

const subtitleEl = document.querySelector<HTMLElement>("#resources-subtitle")!;
const refreshAllEl = document.querySelector<HTMLButtonElement>("#resources-refresh-all")!;
const closeEl = document.querySelector<HTMLButtonElement>("#resources-close")!;
const sectionTabsEl = document.querySelector<HTMLElement>("#resources-section-tabs")!;
const skillsPanelEl = document.querySelector<HTMLElement>("#panel-skills")!;
const skillFiltersEl = document.querySelector<HTMLElement>("#resources-skill-filters")!;
const agentBarEl = document.querySelector<HTMLElement>("#resources-agent-bar")!;
const agentFiltersEl = document.querySelector<HTMLElement>("#resources-agent-filters")!;
const agentHintEl = document.querySelector<HTMLElement>("#resources-agent-hint")!;
const toolFiltersEl = document.querySelector<HTMLElement>("#resources-tool-filters")!;
const searchEl = document.querySelector<HTMLInputElement>("#resources-search")!;
const listEl = document.querySelector<HTMLElement>("#resources-list")!;
const emptyEl = document.querySelector<HTMLElement>("#resources-empty")!;
const footnoteEl = document.querySelector<HTMLElement>("#resources-footnote")!;
const toolsListEl = document.querySelector<HTMLUListElement>("#resources-tools-list")!;
const toolsEmptyEl = document.querySelector<HTMLElement>("#resources-tools-empty")!;
const mallAccountEl = document.querySelector<HTMLElement>("#resources-mall-account")!;
const mallAccountStatusEl = document.querySelector<HTMLElement>("#resources-mall-account-status")!;
const mallLoginEl = document.querySelector<HTMLButtonElement>("#resources-mall-login")!;
const mallLogoutEl = document.querySelector<HTMLButtonElement>("#resources-mall-logout")!;

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
let lastMallCatalog: TeamupsMallCatalog | null = null;
let lastTeamupsAccount: TeamupsAccountStatus | null = null;
let lastWireActions: BrowserMcpTargetAction[] | null = null;
let skillFilter: SkillFilter = "all";
let toolFilter: ToolFilter = "all";
let mallFilter: MallFilter = "all";
let resourceQuery = "";
let activeSection: ResourcesSection = "skills";
let mcpConfigureInFlight = false;
let mallActionInFlight = false;
let mallLoginInFlight = false;
let mallLoginTimer: number | null = null;
let skillsStatusMessage: string | null = null;
let highlightSkillKey: string | null = null;
let skillNavActive: SkillFilter = "all";
let agentFilter: string = "all";
let skillScrollLock = false;
let skillScrollUnlockTimer: number | null = null;
let skillGroupObserver: IntersectionObserver | null = null;
const personalEdition = isPersonalEdition();

const RUNTIME_LABELS: Record<string, string> = {
  hermes: "Hermes",
  openclaw: "OpenClaw",
  "claude-code": "Claude",
  codex: "Codex",
  "deepseek-harness": "DeepSeek Harness",
};

const SKILL_MOUNT_RUNTIME_ORDER = [
  "hermes",
  "openclaw",
  "claude-code",
  "codex",
  "deepseek-harness",
] as const;

function agentChipLabel(runtime: string): string {
  if (runtime === "deepseek-harness") return "DeepSeek";
  return RUNTIME_LABELS[runtime] ?? runtime;
}

function entryMountedOn(entry: UnifiedSkillEntry, runtime: string): boolean {
  return Boolean(entry.agents?.some((agent) => agent.runtime === runtime && agent.mounted));
}

function mergeSkillAgents(skill: SkillInventoryItem): SkillAgentUsage[] {
  const fromApi = new Map(skill.agents.map((agent) => [agent.runtime, agent]));
  return SKILL_MOUNT_RUNTIME_ORDER.map(
    (runtime) =>
      fromApi.get(runtime) ?? {
        runtime,
        scope: "not mounted",
        path: "",
        mounted: false,
      },
  );
}

function applyI18n(): void {
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
}

function setSection(section: ResourcesSection): void {
  activeSection = section;
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

function setSkillsFootnote(message: string | null): void {
  skillsStatusMessage = message;
  syncSkillsFootnote();
}

function defaultSkillsFootnote(): string {
  if (isStoreScope()) {
    if (lastMallCatalog && lastMallCatalog.items.length > 0) {
      const rows = filteredMallItems();
      return t("resources.mallListHint", {
        count: String(rows.length),
        total: String(lastMallCatalog.items.length),
      });
    }
    return t("resources.mallFootnoteShort");
  }
  const inv = lastMcpStatus?.inventory;
  if (inv?.workspace_name) {
    return `${inv.workspace_name}${inv.workspace_path ? ` · ${inv.workspace_path}` : ""}`;
  }
  return "";
}

function syncSkillsFootnote(): void {
  footnoteEl.textContent = skillsStatusMessage ?? defaultSkillsFootnote();
}

function rowMatchesQuery(row: ResourceRow, extra = ""): boolean {
  if (!resourceQuery) return true;
  return `${row.name} ${row.sub} ${row.meta} ${extra}`.toLowerCase().includes(resourceQuery);
}

function countSkillMatches(): number {
  return buildUnifiedSkillEntries()
    .filter((entry) => entryMatchesQuery(entry))
    .length;
}

function countToolMatches(): number {
  return buildMcpRows().filter((row) => rowMatchesQuery(row)).length;
}

function applyAccountOwnership(): void {
  if (!lastMallCatalog || !lastTeamupsAccount?.signed_in) return;
  const account = lastTeamupsAccount as TeamupsAccountStatus & {
    packCount?: number;
  };
  const owned = new Set(
    (account.packs ?? []).map((slug) => slug.trim()).filter(Boolean),
  );
  if (owned.size === 0) return;
  for (const item of lastMallCatalog.items) {
    if (owned.has(item.id) || (item.pack_slug && owned.has(item.pack_slug))) {
      item.owned = true;
    }
  }
  const base = lastMallCatalog.base_url.replace(/\/$/, "");
  for (const slug of owned) {
    if (lastMallCatalog.items.some((item) => item.id === slug)) continue;
    lastMallCatalog.items.push({
      id: slug,
      kind: "pack",
      name: slug,
      description: "",
      free: false,
      price_label: null,
      owned: true,
      installed: false,
      skill_count: null,
      pack_slug: slug,
      purchase_url: base ? `${base}/packs/${slug}` : null,
      version: null,
    });
  }
}

function filteredMallItems(): TeamupsCatalogItem[] {
  const items = lastMallCatalog?.items ?? [];
  return items
    .filter((item) => {
      if (mallFilter === "free" && !item.free) return false;
      if (mallFilter === "paid" && item.free) return false;
      if (mallFilter === "pack" && item.kind !== "pack") return false;
      if (mallFilter === "skill" && item.kind !== "skill") return false;
      if (!resourceQuery) return true;
      const blob = `${item.name} ${item.description} ${item.id} ${item.kind}`.toLowerCase();
      return blob.includes(resourceQuery);
    })
    .sort((a, b) => Number(b.owned) - Number(a.owned));
}

/** When searching, leave Browser (no list) and jump to the tab that has hits. */
function maybeJumpToSearchHits(): void {
  if (!resourceQuery) return;
  const skillHits = countSkillMatches();
  const toolHits = countToolMatches();
  if (activeSection === "browser") {
    if (skillHits > 0) setSection("skills");
    else if (toolHits > 0) setSection("tools");
    else setSection("skills");
    return;
  }
  if (activeSection === "skills" && skillHits === 0 && toolHits > 0) {
    setSection("tools");
  } else if (activeSection === "tools" && toolHits === 0 && skillHits > 0) {
    setSection("skills");
  }
  if (activeSection === "skills" && resourceQuery && skillHits > 0 && skillFilter === "store") {
    skillFilter = "all";
  }
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
    ? t("mcp.cdpConnected")
    : t("mcp.cdpIdle");
  mcpCdpEl.title = chrome.port ? `:${chrome.port}` : "";
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

function buildMcpRows(): ResourceRow[] {
  const rows: ResourceRow[] = [];
  const mcpGroups = new Map<string, McpInventoryItem[]>();
  for (const server of lastMcpStatus?.inventory.servers ?? []) {
    if (server.is_browser) continue; // Browser has its own tab.
    const key = server.name.trim().toLowerCase();
    const group = mcpGroups.get(key) ?? [];
    group.push(server);
    mcpGroups.set(key, group);
  }

  for (const servers of mcpGroups.values()) {
    const primary = servers[0]!;
    const runtimes = [...new Set(servers.map((server) => server.runtime_hint))].map((runtime) => {
      if (runtime === "claude-code") return "Claude";
      if (runtime === "codex") return "Codex";
      if (runtime === "openclaw") return "OpenClaw";
      if (runtime === "hermes") return "Hermes";
      if (runtime === "deepseek-harness") return "DeepSeek";
      if (runtime === "shared") return t("resources.shared");
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
          : `${t("resources.mcpHealthy")} · ${bindingLabel}`,
      tone: issues.length > 0 ? "bad" : "ok",
      issue: issues.length > 0,
    });
  }
  return rows.sort((a, b) => a.name.localeCompare(b.name));
}

type UnifiedSkillEntry = {
  key: string;
  name: string;
  description: string;
  category: SkillCategoryId;
  badgeLabel: string;
  sub: string;
  meta: string;
  tone: ResourceRow["tone"];
  issue: boolean;
  skillId?: string;
  needsMount?: boolean;
  canUnmount?: boolean;
  mallItem?: TeamupsCatalogItem;
  storeOnly: boolean;
  iconKind: "skill" | "pack";
  agents?: SkillAgentUsage[];
};

function localSkillById(skillId: string) {
  return lastSkillsInventory?.skills.find((s) => s.skill_id === skillId);
}

function entryFromLocalSkill(skill: NonNullable<SkillsInventoryReport["skills"][number]>): UnifiedSkillEntry {
  const agents = mergeSkillAgents(skill);
  const mounted = agents.filter((a) => a.mounted).length;
  const totalAgents = agents.length;
  const needsMount = totalAgents > 0 && mounted === 0;
  const anyUnmounted = totalAgents > 0 && mounted < totalAgents;
  const category = classifySkillCategory({
    id: skill.skill_id,
    name: skill.name,
    description: skill.description,
  });
  const mallItem = lastMallCatalog?.items.find(
    (item) => item.kind === "skill" && item.id === skill.skill_id,
  );
  return {
    key: skill.skill_id,
    name: skill.name || skill.skill_id,
    description: skill.description?.trim() || "",
    category,
    badgeLabel: t(skillCategoryLabelKey(category) as MessageKey),
    sub: t("resources.skillMountStatus", {
      mounted: String(mounted),
      total: String(totalAgents),
    }),
    meta:
      totalAgents === 0
        ? t("resources.noAgentsOnDevice")
        : mounted === totalAgents
          ? t("resources.allAgentsMounted")
          : t("resources.skillMountStatus", {
              mounted: String(mounted),
              total: String(totalAgents),
            }),
    tone: needsMount ? "warn" : anyUnmounted ? "warn" : "ok",
    issue: needsMount,
    skillId: skill.skill_id,
    needsMount: anyUnmounted,
    canUnmount: mounted > 0,
    mallItem,
    storeOnly: false,
    iconKind: "skill",
    agents,
  };
}

function storeIssue(item: TeamupsCatalogItem): boolean {
  return !item.installed && item.owned;
}

function isStoreScope(): boolean {
  return personalEdition && skillFilter === "store";
}

function mallItemPriceMeta(item: TeamupsCatalogItem): string {
  const bits: string[] = [];
  if (item.installed) {
    bits.push(t("resources.mallInstalled"));
  } else if (item.free) {
    bits.push(t("resources.mallFree"));
  } else if (item.price_label) {
    bits.push(item.price_label);
  }
  if (item.skill_count != null) {
    bits.push(t("resources.mallSkillCount", { count: String(item.skill_count) }));
  }
  if (item.owned && !item.free && !item.installed) {
    bits.push(t("resources.mallOwned"));
  }
  return bits.join(" · ");
}

function entryFromMallItem(item: TeamupsCatalogItem): UnifiedSkillEntry | null {
  if (item.kind === "skill") {
    const local = localSkillById(item.id);
    if (local) return entryFromLocalSkill(local);
  }
  const category: SkillCategoryId = "other";
  const badgeLabel =
    item.kind === "pack" ? t("resources.mallPackBadge") : t("resources.mallSkillBadge");
  const issue = storeIssue(item);
  return {
    key: `store:${item.kind}:${item.id}`,
    name: item.name || item.id,
    description: item.description.trim() || item.id,
    category,
    badgeLabel,
    sub: badgeLabel,
    meta: mallItemPriceMeta(item),
    tone: item.installed ? "ok" : issue ? "warn" : "muted",
    issue,
    mallItem: item,
    storeOnly: true,
    iconKind: item.kind === "pack" ? "pack" : "skill",
  };
}

function buildLocalSkillEntries(): UnifiedSkillEntry[] {
  const rows: UnifiedSkillEntry[] = [];
  for (const skill of lastSkillsInventory?.skills ?? []) {
    rows.push(entryFromLocalSkill(skill));
  }
  return rows.sort((a, b) => a.name.localeCompare(b.name));
}

function buildStoreSkillEntries(): UnifiedSkillEntry[] {
  const rows: UnifiedSkillEntry[] = [];
  for (const item of filteredMallItems()) {
    const entry = entryFromMallItem(item);
    if (entry) rows.push(entry);
  }
  return rows;
}

function buildUnifiedSkillEntries(): UnifiedSkillEntry[] {
  if (skillFilter === "store" && personalEdition) {
    return buildStoreSkillEntries();
  }
  if (skillFilter === "issue" && personalEdition) {
    const localIssues = buildLocalSkillEntries().filter((entry) => entry.issue);
    const storeIssues: UnifiedSkillEntry[] = [];
    for (const item of lastMallCatalog?.items ?? []) {
      if (!storeIssue(item)) continue;
      const entry = entryFromMallItem(item);
      if (entry?.storeOnly) storeIssues.push(entry);
    }
    return [...localIssues, ...storeIssues];
  }
  const local = buildLocalSkillEntries();
  if (!personalEdition || skillFilter !== "all" || resourceQuery) {
    return local;
  }
  const localIds = new Set(local.map((row) => row.skillId).filter(Boolean));
  const storeOnly: UnifiedSkillEntry[] = [];
  for (const item of lastMallCatalog?.items ?? []) {
    if (item.kind === "skill" && (item.installed || localIds.has(item.id))) continue;
    if (item.kind === "pack" && item.installed) continue;
    const entry = entryFromMallItem(item);
    if (entry?.storeOnly) storeOnly.push(entry);
  }
  return [...local, ...storeOnly];
}

function entryMatchesQuery(entry: UnifiedSkillEntry): boolean {
  if (!resourceQuery) return true;
  const blob = `${entry.name} ${entry.description} ${entry.sub} ${entry.meta} ${entry.skillId ?? ""} ${
    entry.mallItem?.id ?? ""
  }`.toLowerCase();
  return blob.includes(resourceQuery);
}

function skillDescription(skillId: string | undefined): string {
  if (!skillId) return "";
  return lastSkillsInventory?.skills.find((s) => s.skill_id === skillId)?.description?.trim() || "";
}

function appendResourceRow(parent: HTMLElement, row: ResourceRow, tagLabel: string): void {
  const li = document.createElement("li");
  li.className = "res-catalog-item";

  const icon = document.createElement("span");
  icon.className = "res-catalog-icon";
  icon.classList.add(row.kind === "skill" ? "is-skill" : "is-mcp");
  icon.textContent = (row.name.trim().charAt(0) || "?").toUpperCase();

  const body = document.createElement("div");
  body.className = "res-catalog-body";
  const titleRow = document.createElement("div");
  titleRow.className = "res-catalog-title-row";
  const strong = document.createElement("strong");
  strong.textContent = row.name;
  const badge = document.createElement("span");
  badge.className = "res-catalog-badge";
  badge.textContent = tagLabel;
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
  parent.appendChild(li);
}

function appendFilterChip(
  label: string,
  active: boolean,
  onClick: () => void,
  dataset: Record<string, string>,
  parent: HTMLElement = skillFiltersEl,
  count?: number,
): void {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = `filter-chip${active ? " is-active" : ""}`;
  for (const [key, value] of Object.entries(dataset)) {
    btn.dataset[key] = value;
  }
  if (count == null) {
    btn.textContent = label;
  } else {
    const name = document.createElement("span");
    name.textContent = label;
    const num = document.createElement("span");
    num.className = "chip-count";
    num.textContent = String(count);
    btn.append(name, num);
  }
  btn.addEventListener("click", onClick);
  parent.appendChild(btn);
}

function appendScopeChips(localCount: number, storeCount: number): void {
  if (!personalEdition) return;
  appendFilterChip(`${t("resources.scopeLocal")} ${localCount}`, !isStoreScope(), () => {
    if (!isStoreScope()) return;
    skillFilter = "all";
    skillNavActive = "all";
    skillsStatusMessage = null;
    renderResourcesList();
  }, { scope: "local" });
  appendFilterChip(`${t("resources.scopeStore")} ${storeCount}`, isStoreScope(), () => {
    if (skillFilter === "store") return;
    skillFilter = "store";
    agentFilter = "all";
    skillsStatusMessage = null;
    if (!lastMallCatalog) {
      void loadMall().then(() => renderResourcesList());
      return;
    }
    renderResourcesList();
  }, { scope: "store" });
  const rule = document.createElement("span");
  rule.className = "resources-nav-rule";
  rule.setAttribute("aria-hidden", "true");
  skillFiltersEl.appendChild(rule);
}

function renderSkillFilters(): void {
  const local = buildLocalSkillEntries();
  const storeItems = lastMallCatalog?.items ?? [];
  const issueCount =
    local.filter((row) => row.issue).length +
    (personalEdition ? storeItems.filter((item) => storeIssue(item)).length : 0);

  if (
    !isStoreScope() &&
    skillFilter !== "all" &&
    skillFilter !== "issue" &&
    !SKILL_CATEGORY_ORDER.includes(skillFilter as SkillCategoryId)
  ) {
    skillFilter = "all";
  }

  const showStoreChrome = isStoreScope();
  mallAccountEl.hidden = !showStoreChrome;
  skillFiltersEl.replaceChildren();
  appendScopeChips(local.length, storeItems.length);

  if (showStoreChrome) {
    agentBarEl.hidden = true;
    renderMallAccount();
    const mallChips: Array<{ id: MallFilter; label: string }> = [
      { id: "all", label: t("resources.filterAll") },
      { id: "free", label: t("resources.mallFilterFree") },
      { id: "paid", label: t("resources.mallFilterPaid") },
      { id: "pack", label: t("resources.mallFilterPack") },
      { id: "skill", label: t("resources.mallFilterSkill") },
    ];
    for (const chip of mallChips) {
      appendFilterChip(chip.label, mallFilter === chip.id, () => {
        mallFilter = chip.id;
        renderSkillsList();
      }, { mallFilter: chip.id });
    }
    return;
  }

  renderAgentBar(local);
  const scoped =
    agentFilter === "all" ? local : local.filter((row) => entryMountedOn(row, agentFilter));
  const scopedIssue =
    agentFilter === "all" ? issueCount : scoped.filter((row) => row.issue).length;

  const counts = new Map<SkillFilter, number>();
  counts.set("all", scoped.length);
  counts.set("issue", scopedIssue);
  for (const id of SKILL_CATEGORY_ORDER) {
    counts.set(id, scoped.filter((row) => row.category === id).length);
  }

  const chips: Array<{ id: SkillFilter; label: string; count: number }> = [
    { id: "all", label: t("resources.filterAll"), count: counts.get("all") ?? 0 },
    { id: "issue", label: t("resources.filterIssue"), count: counts.get("issue") ?? 0 },
  ];
  for (const id of SKILL_CATEGORY_ORDER) {
    const count = counts.get(id) ?? 0;
    if (id !== "other" && count === 0) continue;
    chips.push({
      id,
      label: t(skillCategoryLabelKey(id) as MessageKey),
      count,
    });
  }

  if (!chips.some((chip) => chip.id === skillFilter)) {
    skillFilter = "all";
  }

  const navActive = skillFilter === "issue" ? "issue" : skillNavActive;
  for (const chip of chips) {
    if (chip.id === "issue" && chip.count === 0) continue;
    const active =
      chip.id === "issue" ? navActive === "issue" : navActive === chip.id && skillFilter !== "issue";
    appendFilterChip(`${chip.label} ${chip.count}`, active, () => {
      void onSkillNavClick(chip.id);
    }, { skillFilter: chip.id });
  }
}

function renderAgentBar(local: UnifiedSkillEntry[]): void {
  agentBarEl.hidden = false;
  agentFiltersEl.replaceChildren();
  appendFilterChip(
    t("resources.agentFilterAll"),
    agentFilter === "all",
    () => {
      if (agentFilter === "all") return;
      setAgentFilter("all");
    },
    { agentFilter: "all" },
    agentFiltersEl,
  );
  for (const runtime of SKILL_MOUNT_RUNTIME_ORDER) {
    const count = local.filter((entry) => entryMountedOn(entry, runtime)).length;
    appendFilterChip(
      agentChipLabel(runtime),
      agentFilter === runtime,
      () => {
        setAgentFilter(agentFilter === runtime ? "all" : runtime);
      },
      { agentFilter: runtime },
      agentFiltersEl,
      count,
    );
  }

  if (agentFilter === "all") {
    agentHintEl.hidden = true;
    agentHintEl.replaceChildren();
    return;
  }
  const count = local.filter((entry) => entryMountedOn(entry, agentFilter)).length;
  agentHintEl.hidden = false;
  agentHintEl.replaceChildren();
  const text = document.createElement("span");
  text.textContent = t("resources.agentFilterHint", {
    agent: agentChipLabel(agentFilter),
    count: String(count),
  });
  const clear = document.createElement("button");
  clear.type = "button";
  clear.className = "resources-agent-clear";
  clear.textContent = t("resources.agentFilterClear");
  clear.addEventListener("click", () => setAgentFilter("all"));
  agentHintEl.append(text, clear);
}

function paintSkillNav(active: SkillFilter): void {
  skillNavActive = active;
  skillFiltersEl.querySelectorAll<HTMLButtonElement>("[data-skill-filter]").forEach((chip) => {
    chip.classList.toggle("is-active", chip.dataset.skillFilter === active);
  });
  const current = skillFiltersEl.querySelector<HTMLButtonElement>(`[data-skill-filter="${active}"]`);
  current?.scrollIntoView({ block: "nearest", inline: "center", behavior: "smooth" });
}

function setAgentFilter(runtime: string): void {
  if (agentFilter === runtime) return;
  agentFilter = runtime;
  skillsStatusMessage = null;
  renderResourcesList();
  scrollSkillsPanelTo(0);
}

function lockSkillScrollSpy(): void {
  skillScrollLock = true;
  if (skillScrollUnlockTimer != null) window.clearTimeout(skillScrollUnlockTimer);
  skillScrollUnlockTimer = window.setTimeout(() => {
    skillScrollLock = false;
    skillScrollUnlockTimer = null;
  }, 500);
}

function scrollSkillsPanelTo(top: number): void {
  lockSkillScrollSpy();
  skillsPanelEl.scrollTo({ top: Math.max(0, top), behavior: "smooth" });
}

function scrollToSkillGroup(id: SkillFilter): void {
  if (id === "all") {
    scrollSkillsPanelTo(0);
    paintSkillNav("all");
    return;
  }
  const heading = listEl.querySelector<HTMLElement>(`[data-skill-group="${id}"]`);
  if (!heading) return;
  const sticky = document.querySelector<HTMLElement>("#resources-skills-sticky");
  const offset = (sticky?.offsetHeight ?? 0) + 8;
  const top = heading.getBoundingClientRect().top - skillsPanelEl.getBoundingClientRect().top
    + skillsPanelEl.scrollTop
    - offset;
  paintSkillNav(id);
  scrollSkillsPanelTo(top);
}

async function onSkillNavClick(id: SkillFilter): Promise<void> {
  skillsStatusMessage = null;
  if (id === "issue") {
    skillFilter = "issue";
    renderResourcesList();
    scrollSkillsPanelTo(0);
    return;
  }
  const needRebuild = skillFilter !== "all" || Boolean(resourceQuery);
  if (resourceQuery) {
    resourceQuery = "";
    searchEl.value = "";
  }
  skillFilter = "all";
  if (needRebuild) renderResourcesList();
  requestAnimationFrame(() => {
    scrollToSkillGroup(id);
  });
}

function bindSkillGroupSpy(): void {
  skillGroupObserver?.disconnect();
  skillGroupObserver = null;
  if (skillFilter !== "all" || resourceQuery || isStoreScope()) return;
  const headings = [...listEl.querySelectorAll<HTMLElement>("[data-skill-group]")];
  if (headings.length === 0) return;

  skillGroupObserver = new IntersectionObserver(
    (entries) => {
      if (skillScrollLock) return;
      const visible = entries
        .filter((entry) => entry.isIntersecting)
        .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
      const topMost = visible[0]?.target;
      const group = topMost instanceof HTMLElement ? topMost.dataset.skillGroup : undefined;
      if (!group) {
        if (skillsPanelEl.scrollTop < 24) paintSkillNav("all");
        return;
      }
      paintSkillNav(group as SkillFilter);
    },
    {
      root: skillsPanelEl,
      rootMargin: "-20% 0px -65% 0px",
      threshold: 0,
    },
  );
  for (const heading of headings) skillGroupObserver.observe(heading);
}

function onSkillsPanelScroll(): void {
  if (skillScrollLock) return;
  if (skillsPanelEl.scrollTop < 20) paintSkillNav("all");
}

function appendSkillAgentChips(body: HTMLElement, entry: UnifiedSkillEntry): void {
  if (entry.storeOnly || !entry.skillId) return;
  const agents = entry.agents ?? [];
  const row = document.createElement("div");
  row.className = "res-skill-agents";
  const label = document.createElement("span");
  label.className = "res-skill-agents-label";
  label.textContent = t("resources.skillAgentsLabel");
  row.appendChild(label);

  const chips = document.createElement("div");
  chips.className = "res-skill-agents-chips";
  if (agents.length === 0) {
    const none = document.createElement("span");
    none.className = "res-skill-agents-empty";
    none.textContent = t("resources.noAgentsOnDevice");
    chips.appendChild(none);
  } else {
    for (const agent of agents) {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = agent.mounted ? "skills-runtime is-on" : "skills-runtime";
      const runtimeLabel = RUNTIME_LABELS[agent.runtime] ?? agent.runtime;
      chip.title = agent.mounted
        ? t("skills.unmountRuntime", { runtime: runtimeLabel })
        : t("skills.mountRuntime", { runtime: runtimeLabel });
      chip.setAttribute("aria-pressed", agent.mounted ? "true" : "false");
      const dot = document.createElement("i");
      dot.className = "skills-runtime-dot";
      dot.setAttribute("aria-hidden", "true");
      const name = document.createElement("span");
      name.textContent = runtimeLabel;
      chip.append(dot, name);
      const skillId = entry.skillId;
      const runtime = agent.runtime;
      chip.addEventListener("click", () => {
        void toggleSkillRuntimeMount(chip, skillId, runtime, chip.classList.contains("is-on"));
      });
      chips.appendChild(chip);
    }
  }
  row.append(chips);
  body.appendChild(row);
}

async function toggleSkillRuntimeMount(
  chip: HTMLButtonElement,
  skillId: string,
  runtime: string,
  wasMounted: boolean,
): Promise<void> {
  if (chip.classList.contains("is-busy")) return;
  chip.classList.add("is-busy");
  setSkillsFootnote(wasMounted ? t("skills.unmounting") : t("skills.mounting"));
  try {
    const report = wasMounted
      ? await invoke<SkillMountReport>("unmount_synced_skills_command", {
          skillIds: [skillId],
          runtimes: [runtime],
        })
      : await invoke<SkillMountReport>("mount_synced_skills_command", {
          skillIds: [skillId],
          runtimes: [runtime],
        });
    setSkillsFootnote(
      wasMounted
        ? t("skills.unmountOk", {
            unmounted: String(report.unmounted),
            skipped: String(report.skipped),
            failed: String(report.failed),
          })
        : t("skills.mountOk", {
            mounted: String(report.mounted),
            skipped: String(report.skipped),
            failed: String(report.failed),
          }),
    );
    await loadSkills();
  } catch (error) {
    setSkillsFootnote(withErrorDetail(t("skills.mountFailed"), error));
  } finally {
    chip.classList.remove("is-busy");
  }
}

function appendUnifiedSkillActions(metaWrap: HTMLElement, entry: UnifiedSkillEntry): void {
  if (entry.skillId && entry.agents && entry.agents.length > 0) {
    const skillId = entry.skillId;
    if (entry.needsMount) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "btn-primary btn-compact";
      btn.textContent = t("resources.mountAllAgents");
      btn.title = t("resources.mountAllAgentsHint");
      btn.addEventListener("click", () => {
        void mountSkill(skillId);
      });
      metaWrap.appendChild(btn);
    }
    if (entry.canUnmount) {
      const removeBtn = document.createElement("button");
      removeBtn.type = "button";
      removeBtn.className = "btn-ghost btn-compact";
      removeBtn.textContent = t("resources.removeFromAgents");
      removeBtn.title = t("resources.removeFromAgentsHint");
      removeBtn.addEventListener("click", () => {
        void unmountSkill(skillId, entry.name);
      });
      metaWrap.appendChild(removeBtn);
    }
    return;
  }

  const item = entry.mallItem;
  if (!item || !entry.storeOnly) return;

  if (item.installed) {
    const done = document.createElement("span");
    done.className = "tone-ok";
    done.textContent = t("resources.mallInstalled");
    metaWrap.appendChild(done);
    return;
  }
  if (canInstallMallItem(item)) {
    const installBtn = document.createElement("button");
    installBtn.type = "button";
    installBtn.className = "btn-primary btn-compact";
    installBtn.textContent = t("resources.mallInstall");
    installBtn.addEventListener("click", () => {
      void installMallItem(item, installBtn);
    });
    metaWrap.appendChild(installBtn);
    return;
  }
  const buyBtn = document.createElement("button");
  buyBtn.type = "button";
  buyBtn.className = "btn-secondary btn-compact";
  buyBtn.textContent = lastTeamupsAccount?.signed_in
    ? t("resources.mallBuy")
    : t("resources.mallLogin");
  buyBtn.addEventListener("click", () => {
    if (lastTeamupsAccount?.signed_in) {
      void openMallPurchase(item);
    } else {
      void startMallLogin();
    }
  });
  metaWrap.appendChild(buyBtn);
}

function appendUnifiedSkillRow(parent: HTMLElement, entry: UnifiedSkillEntry): void {
  const li = document.createElement("li");
  li.className = "res-catalog-item";
  if (entry.storeOnly) li.classList.add("is-store-item");
  li.dataset.skillKey = entry.key;
  if (highlightSkillKey && highlightSkillKey === entry.key) {
    li.classList.add("is-highlight");
  }

  const icon = document.createElement("span");
  icon.className = "res-catalog-icon";
  icon.classList.add(entry.iconKind === "pack" ? "is-pack" : "is-skill");
  icon.textContent = (entry.name.trim().charAt(0) || "?").toUpperCase();

  const body = document.createElement("div");
  body.className = "res-catalog-body";
  const titleRow = document.createElement("div");
  titleRow.className = "res-catalog-title-row";
  const strong = document.createElement("strong");
  strong.textContent = entry.name;
  const badge = document.createElement("span");
  badge.className = "res-catalog-badge";
  badge.textContent = entry.badgeLabel;
  titleRow.append(strong, badge);
  const desc = document.createElement("div");
  desc.className = "res-catalog-desc";
  desc.textContent = entry.description || entry.sub;
  body.append(titleRow, desc);
  appendSkillAgentChips(body, entry);

  const metaWrap = document.createElement("div");
  metaWrap.className = "res-catalog-meta mall-actions";
  if (entry.meta) {
    const meta = document.createElement("span");
    meta.className = entry.storeOnly ? "res-catalog-store-price" : `tone-${entry.tone}`;
    meta.textContent = entry.meta;
    metaWrap.appendChild(meta);
  }
  appendUnifiedSkillActions(metaWrap, entry);

  li.append(icon, body, metaWrap);
  parent.appendChild(li);
}

function renderSkillsList(): void {
  renderSkillFilters();

  let entries = buildUnifiedSkillEntries().filter((entry) => entryMatchesQuery(entry));

  if (skillFilter === "issue") {
    entries = entries.filter((entry) => entry.issue);
  } else if (skillFilter !== "all" && skillFilter !== "store") {
    entries = entries.filter((entry) => !entry.storeOnly && entry.category === skillFilter);
  }
  if (agentFilter !== "all") {
    entries = entries.filter((entry) => entryMountedOn(entry, agentFilter));
  }

  listEl.replaceChildren();
  const emptyCopy =
    skillFilter === "store"
      ? resourceQuery
        ? t("chat.resourcesNoMatch")
        : t("resources.emptyMall")
      : resourceQuery
        ? t("chat.resourcesNoMatch")
        : agentFilter !== "all"
          ? t("resources.emptyAgentSkills", { agent: agentChipLabel(agentFilter) })
          : t("resources.emptySkills");
  emptyEl.hidden = entries.length > 0;
  emptyEl.textContent = emptyCopy;

  if (skillFilter === "all" && !resourceQuery) {
    const localEntries = entries.filter((entry) => !entry.storeOnly);
    const storeEntries = entries.filter((entry) => entry.storeOnly);
    for (const category of SKILL_CATEGORY_ORDER) {
      const group = localEntries.filter((entry) => entry.category === category);
      if (group.length === 0) continue;
      const heading = document.createElement("h3");
      heading.className = "res-catalog-group";
      heading.dataset.skillGroup = category;
      heading.id = `skill-group-${category}`;
      heading.textContent = `${t(skillCategoryLabelKey(category) as MessageKey)} · ${group.length}`;
      listEl.appendChild(heading);
      const ul = document.createElement("ul");
      ul.className = "res-catalog-list";
      for (const entry of group) {
        appendUnifiedSkillRow(ul, entry);
      }
      listEl.appendChild(ul);
    }
    if (storeEntries.length > 0 && personalEdition) {
      const heading = document.createElement("h3");
      heading.className = "res-catalog-group";
      heading.dataset.skillGroup = "store";
      heading.id = "skill-group-store";
      heading.textContent = `${t("resources.groupStore")} · ${storeEntries.length}`;
      listEl.appendChild(heading);
      const ul = document.createElement("ul");
      ul.className = "res-catalog-list";
      for (const entry of storeEntries) {
        appendUnifiedSkillRow(ul, entry);
      }
      listEl.appendChild(ul);
    }
  } else {
    const ul = document.createElement("ul");
    ul.className = "res-catalog-list";
    for (const entry of entries) {
      appendUnifiedSkillRow(ul, entry);
    }
    listEl.appendChild(ul);
  }

  if (highlightSkillKey) {
    const target = listEl.querySelector<HTMLElement>(`[data-skill-key="${highlightSkillKey}"]`);
    target?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    highlightSkillKey = null;
  }

  bindSkillGroupSpy();
  syncSkillsFootnote();
}

function renderToolsList(): void {
  const allRows = buildMcpRows();
  const rows = allRows.filter((row) => {
    if (toolFilter === "issue" && !row.issue) return false;
    return rowMatchesQuery(row);
  });

  toolFiltersEl.querySelectorAll<HTMLButtonElement>("[data-tool-filter]").forEach((chip) => {
    chip.classList.toggle("is-active", chip.dataset.toolFilter === toolFilter);
  });

  toolsListEl.replaceChildren();
  toolsEmptyEl.hidden = rows.length > 0;
  toolsEmptyEl.textContent = resourceQuery ? t("chat.resourcesNoMatch") : t("resources.emptyTools");
  for (const row of rows) {
    appendResourceRow(toolsListEl, row, t("resources.toolBadge"));
  }
}

function canInstallMallItem(item: TeamupsCatalogItem): boolean {
  return item.free || item.owned;
}

function stopMallLoginPoll(): void {
  if (mallLoginTimer != null) {
    window.clearTimeout(mallLoginTimer);
    mallLoginTimer = null;
  }
}

function renderMallAccount(): void {
  const signedIn = Boolean(lastTeamupsAccount?.signed_in);
  mallAccountEl.classList.toggle("is-signed-in", signedIn);
  mallLoginEl.hidden = signedIn;
  mallLogoutEl.hidden = !signedIn;
  mallLoginEl.disabled = mallLoginInFlight;
  mallLogoutEl.disabled = mallLoginInFlight;
  if (mallLoginInFlight) {
    mallAccountStatusEl.textContent = t("resources.mallLoggingIn");
  } else if (signedIn) {
    mallAccountStatusEl.textContent = t("resources.mallAccountSignedIn", {
      count: String(lastTeamupsAccount?.pack_count ?? 0),
    });
  } else {
    mallAccountStatusEl.textContent = t("resources.mallAccountSignedOut");
  }
}

function renderResourcesList(): void {
  const uniqueMcp = new Set(
    (lastMcpStatus?.inventory.servers ?? [])
      .filter((s) => !s.is_browser)
      .map((s) => s.name.trim().toLowerCase()),
  );
  subtitleEl.textContent = t("resources.windowSubtitle", {
    skills: String(lastSkillsInventory?.skills.length ?? 0),
    mcp: String(uniqueMcp.size),
  });
  renderSkillsList();
  renderToolsList();
  if (!skillsStatusMessage) {
    syncSkillsFootnote();
  }
}

async function mountSkill(skillId: string): Promise<void> {
  setSkillsFootnote(t("resources.installingToAgents"));
  try {
    const report = await invoke<SkillMountReport>("mount_synced_skills_command", {
      skillIds: [skillId],
      runtimes: null,
    });
    setSkillsFootnote(t("resources.installToAgentsOk", {
      mounted: String(report.mounted),
      skipped: String(report.skipped),
      failed: String(report.failed),
    }));
    await loadSkills();
  } catch (error) {
    setSkillsFootnote(withErrorDetail(t("resources.installToAgentsFailed"), error));
  }
}

async function unmountSkill(skillId: string, name: string): Promise<void> {
  let ok = false;
  try {
    ok = await ask(t("resources.removeFromAgentsConfirm", { name }), {
      title: t("resources.removeFromAgents"),
      kind: "warning",
      okLabel: t("resources.removeFromAgents"),
      cancelLabel: t("resources.cancel"),
    });
  } catch {
    ok = window.confirm(t("resources.removeFromAgentsConfirm", { name }));
  }
  if (!ok) return;
  setSkillsFootnote(t("resources.removingFromAgents"));
  try {
    const report = await invoke<SkillMountReport>("unmount_synced_skills_command", {
      skillIds: [skillId],
      runtimes: null,
    });
    setSkillsFootnote(t("resources.removeFromAgentsOk", {
      unmounted: String(report.unmounted),
      skipped: String(report.skipped),
      failed: String(report.failed),
    }));
    await loadSkills();
  } catch (error) {
    setSkillsFootnote(withErrorDetail(t("resources.removeFromAgentsFailed"), error));
  }
}

async function loadSkills(): Promise<void> {
  try {
    lastSkillsInventory = await invoke<SkillsInventoryReport>("list_skills_inventory_command", {
      remoteStats: false,
    });
    if (!lastSkillsInventory.available_mount_runtimes?.length) {
      try {
        lastSkillsInventory.available_mount_runtimes = await invoke<string[]>(
          "skill_mount_runtime_ids_command",
        );
      } catch {
        // Older desktop build — fall back to per-skill agents from the API.
      }
    }
  } catch {
    lastSkillsInventory = null;
  }
  renderResourcesList();
}

async function loadMall(): Promise<void> {
  try {
    const [catalog, account] = await Promise.all([
      invoke<TeamupsMallCatalog>("list_teamups_mall_catalog_command"),
      invoke<TeamupsAccountStatus>("teamups_account_status_command").catch(() => null),
    ]);
    lastMallCatalog = catalog;
    lastTeamupsAccount = account;
    applyAccountOwnership();
    renderResourcesList();
  } catch (error) {
    lastMallCatalog = null;
    setSkillsFootnote(withErrorDetail(t("resources.mallLoadFailed"), error));
    renderMallAccount();
    renderSkillsList();
  }
}

async function scheduleMallLoginPoll(
  deviceCode: string,
  intervalSec: number,
  expiresAt: number,
): Promise<void> {
  stopMallLoginPoll();
  const tick = async () => {
    if (Date.now() >= expiresAt) {
      mallLoginInFlight = false;
      renderMallAccount();
      setSkillsFootnote(t("resources.mallLoginExpired"));
      return;
    }
    try {
      const poll = await invoke<TeamupsLoginPoll>("poll_teamups_login_command", {
        deviceCode,
      });
      if (poll.status === "pending") {
        mallLoginTimer = window.setTimeout(() => {
          void tick();
        }, Math.max(1, intervalSec) * 1000);
        return;
      }
      mallLoginInFlight = false;
      if (poll.status === "approved") {
        setSkillsFootnote(t("resources.mallLoginOk"));
        await loadMall();
        return;
      }
      if (poll.status === "expired" || poll.status === "denied") {
        setSkillsFootnote(t("resources.mallLoginExpired"));
        renderMallAccount();
        return;
      }
    } catch (error) {
      mallLoginInFlight = false;
      setSkillsFootnote(teamupsLoginFailure(error));
      renderMallAccount();
    }
  };
  await tick();
}

async function startMallLogin(): Promise<void> {
  if (mallLoginInFlight) return;
  mallLoginInFlight = true;
  renderMallAccount();
  setSkillsFootnote(t("resources.mallLoggingIn"));
  try {
    const started = await invoke<TeamupsLoginStart>("start_teamups_login_command");
    await openUrl(started.verification_url);
    const expiresAt = Date.now() + Math.max(30, started.expires_in_sec) * 1000;
    await scheduleMallLoginPoll(started.device_code, started.interval_sec, expiresAt);
  } catch (error) {
    mallLoginInFlight = false;
    setSkillsFootnote(teamupsLoginFailure(error));
    renderMallAccount();
  }
}

async function signOutMallAccount(): Promise<void> {
  if (mallLoginInFlight) return;
  stopMallLoginPoll();
  try {
    lastTeamupsAccount = await invoke<TeamupsAccountStatus>("sign_out_teamups_command");
    setSkillsFootnote(t("resources.mallLogoutOk"));
    await loadMall();
  } catch (error) {
    setSkillsFootnote(withErrorDetail(t("resources.mallLoginFailed"), error));
  }
}

async function openMallPurchase(item: TeamupsCatalogItem): Promise<void> {
  const url = item.purchase_url?.trim();
  if (!url) {
    setSkillsFootnote(t("resources.mallInstallFailed"));
    return;
  }
  try {
    await openUrl(url);
    skillsStatusMessage = null;
    syncSkillsFootnote();
  } catch (error) {
    setSkillsFootnote(withErrorDetail(t("resources.mallInstallFailed"), error));
  }
}

async function installMallItem(
  item: TeamupsCatalogItem,
  button: HTMLButtonElement,
): Promise<void> {
  if (mallActionInFlight) return;
  mallActionInFlight = true;
  button.disabled = true;
  const previous = button.textContent;
  button.textContent = t("resources.mallInstalling");
  setSkillsFootnote(t("resources.mallInstalling"));
  try {
    const report = await invoke<SyncReport>("install_teamups_mall_item_command", {
      kind: item.kind,
      id: item.id,
      packSlug:
        item.kind === "skill"
          ? (item.pack_slug?.trim() || item.id)
          : item.pack_slug,
    });
    setSkillsFootnote(t("resources.mallInstallOk", {
      installed: String(report.installed),
    }));
    skillFilter = "issue";
    highlightSkillKey = item.kind === "skill" ? item.id : null;
    await Promise.all([loadMall(), loadSkills()]);
  } catch (error) {
    setSkillsFootnote(teamupsMallInstallFailure(error));
    button.disabled = false;
    button.textContent = previous;
  } finally {
    mallActionInFlight = false;
  }
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
    mcpFootnoteEl.textContent = withErrorDetail(t("mcp.loadFailed"), error);
    mcpTargetsEl.replaceChildren();
    renderResourcesList();
  }
}

async function refreshAll(): Promise<void> {
  const loads: Array<Promise<void>> = [loadMcpStatus(), loadSkills()];
  if (personalEdition) loads.push(loadMall());
  await Promise.all(loads);
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
    mcpFootnoteEl.textContent = withErrorDetail(t("mcp.configureFailed"), error);
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
  if (
    btn.dataset.section === "skills" ||
    btn.dataset.section === "tools" ||
    btn.dataset.section === "browser"
  ) {
    setSection(btn.dataset.section as ResourcesSection);
  }
});

toolFiltersEl.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-tool-filter]");
  const filter = button?.dataset.toolFilter as ToolFilter | undefined;
  if (!filter) return;
  toolFilter = filter;
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
  resourceQuery = searchEl.value.trim().toLowerCase();
  maybeJumpToSearchHits();
  renderSkillsList();
  renderToolsList();
});

refreshAllEl.addEventListener("click", () => {
  skillsStatusMessage = null;
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
  if (section === "browser") setSection("browser");
  else if (section === "tools" || section === "mcp") setSection("tools");
  else if (section === "mall" || section === "store") {
    setSection("skills");
    skillFilter = "store";
    if (!lastMallCatalog) void loadMall();
    else renderResourcesList();
  }
  else if (section === "skills" || section === "catalog") setSection("skills");
  void refreshAll();
});
