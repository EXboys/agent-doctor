import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { formatCount, formatRate } from "./format";
import { getLocale, t, type MessageKey } from "./i18n";
import {
  teamupsLoginFailure,
  teamupsMallInstallFailure,
  withErrorDetail,
} from "./friendly-error";
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
  SkillMountReport,
  SkillsInventoryReport,
  SyncReport,
  TeamupsAccountStatus,
  TeamupsCatalogItem,
  TeamupsLoginPoll,
  TeamupsLoginStart,
  TeamupsMallCatalog,
} from "./types";

type ResourcesSection = "skills" | "mall" | "tools" | "browser";
type SkillFilter = "all" | "issue" | SkillCategoryId;
type ToolFilter = "all" | "issue";
type MallFilter = "all" | "free" | "paid" | "pack" | "skill";

const MCP_SHOW_UI_KEY = "agent-doctor.mcp.showUi";
const MCP_USER_DATA_DIR_KEY = "agent-doctor.mcp.userDataDir";
const MCP_PROFILE_DIRECTORY_KEY = "agent-doctor.mcp.profileDirectory";

const subtitleEl = document.querySelector<HTMLElement>("#resources-subtitle")!;
const refreshAllEl = document.querySelector<HTMLButtonElement>("#resources-refresh-all")!;
const closeEl = document.querySelector<HTMLButtonElement>("#resources-close")!;
const sectionTabsEl = document.querySelector<HTMLElement>("#resources-section-tabs")!;
const skillFiltersEl = document.querySelector<HTMLElement>("#resources-skill-filters")!;
const toolFiltersEl = document.querySelector<HTMLElement>("#resources-tool-filters")!;
const searchEl = document.querySelector<HTMLInputElement>("#resources-search")!;
const listEl = document.querySelector<HTMLElement>("#resources-list")!;
const emptyEl = document.querySelector<HTMLElement>("#resources-empty")!;
const footnoteEl = document.querySelector<HTMLElement>("#resources-footnote")!;
const toolsListEl = document.querySelector<HTMLUListElement>("#resources-tools-list")!;
const toolsEmptyEl = document.querySelector<HTMLElement>("#resources-tools-empty")!;
const mallFiltersEl = document.querySelector<HTMLElement>("#resources-mall-filters")!;
const mallListEl = document.querySelector<HTMLUListElement>("#resources-mall-list")!;
const mallEmptyEl = document.querySelector<HTMLElement>("#resources-mall-empty")!;
const mallFootnoteEl = document.querySelector<HTMLElement>("#resources-mall-footnote")!;
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
  if (section === "mall" && !lastMallCatalog) {
    void loadMall();
  }
}

function rowMatchesQuery(row: ResourceRow, extra = ""): boolean {
  if (!resourceQuery) return true;
  return `${row.name} ${row.sub} ${row.meta} ${extra}`.toLowerCase().includes(resourceQuery);
}

function countSkillMatches(): number {
  return buildSkillRows().filter((row) => rowMatchesQuery(row, skillDescription(row.skillId))).length;
}

function countToolMatches(): number {
  return buildMcpRows().filter((row) => rowMatchesQuery(row)).length;
}

function countMallMatches(): number {
  return filteredMallItems().length;
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
  const mallHits = countMallMatches();
  if (activeSection === "browser") {
    if (skillHits > 0) setSection("skills");
    else if (mallHits > 0) setSection("mall");
    else if (toolHits > 0) setSection("tools");
    else setSection("skills");
    return;
  }
  if (activeSection === "skills" && skillHits === 0) {
    if (mallHits > 0) setSection("mall");
    else if (toolHits > 0) setSection("tools");
  } else if (activeSection === "mall" && mallHits === 0) {
    if (skillHits > 0) setSection("skills");
    else if (toolHits > 0) setSection("tools");
  } else if (activeSection === "tools" && toolHits === 0) {
    if (skillHits > 0) setSection("skills");
    else if (mallHits > 0) setSection("mall");
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

type SkillRow = ResourceRow & { category: SkillCategoryId };

function buildSkillRows(): SkillRow[] {
  const rows: SkillRow[] = [];
  for (const skill of lastSkillsInventory?.skills ?? []) {
    const mounted = skill.agents.filter((a) => a.mounted).length;
    // Only "needs attention" when no Agent has it — missing on some Agents is normal.
    const needsMount = skill.agents.length > 0 && mounted === 0;
    const category = classifySkillCategory({
      id: skill.skill_id,
      name: skill.name,
      description: skill.description,
    });
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
      category,
    });
  }
  return rows.sort((a, b) => a.name.localeCompare(b.name));
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

function renderSkillFilters(rows: SkillRow[]): void {
  const counts = new Map<SkillFilter, number>();
  counts.set("all", rows.length);
  counts.set("issue", rows.filter((row) => row.issue).length);
  for (const id of SKILL_CATEGORY_ORDER) {
    counts.set(id, rows.filter((row) => row.category === id).length);
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

  skillFiltersEl.replaceChildren();
  for (const chip of chips) {
    if (chip.id === "issue" && chip.count === 0) continue;
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `filter-chip${skillFilter === chip.id ? " is-active" : ""}`;
    btn.dataset.skillFilter = chip.id;
    btn.textContent = `${chip.label} ${chip.count}`;
    btn.addEventListener("click", () => {
      skillFilter = chip.id;
      renderResourcesList();
    });
    skillFiltersEl.appendChild(btn);
  }
}

function renderSkillsList(): void {
  const allRows = buildSkillRows();
  renderSkillFilters(allRows);

  const rows = allRows.filter((row) => {
    if (skillFilter === "issue") {
      if (!row.issue) return false;
    } else if (skillFilter !== "all" && row.category !== skillFilter) {
      return false;
    }
    return rowMatchesQuery(row, skillDescription(row.skillId));
  });

  listEl.replaceChildren();
  emptyEl.hidden = rows.length > 0;
  emptyEl.textContent = resourceQuery ? t("chat.resourcesNoMatch") : t("resources.emptySkills");

  if (skillFilter === "all" && !resourceQuery) {
    for (const category of SKILL_CATEGORY_ORDER) {
      const group = rows.filter((row) => row.category === category);
      if (group.length === 0) continue;
      const heading = document.createElement("h3");
      heading.className = "res-catalog-group";
      heading.textContent = `${t(skillCategoryLabelKey(category) as MessageKey)} · ${group.length}`;
      listEl.appendChild(heading);
      const ul = document.createElement("ul");
      ul.className = "res-catalog-list";
      for (const row of group) {
        appendResourceRow(ul, row, t(skillCategoryLabelKey(row.category) as MessageKey));
      }
      listEl.appendChild(ul);
    }
  } else {
    const ul = document.createElement("ul");
    ul.className = "res-catalog-list";
    for (const row of rows) {
      appendResourceRow(ul, row, t(skillCategoryLabelKey(row.category) as MessageKey));
    }
    listEl.appendChild(ul);
  }
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

function renderMallList(): void {
  renderMallAccount();
  mallFiltersEl.querySelectorAll<HTMLButtonElement>("[data-mall-filter]").forEach((chip) => {
    chip.classList.toggle("is-active", chip.dataset.mallFilter === mallFilter);
  });

  const rows = filteredMallItems();
  mallListEl.replaceChildren();
  mallEmptyEl.hidden = rows.length > 0;
  mallEmptyEl.textContent = resourceQuery
    ? t("chat.resourcesNoMatch")
    : t("resources.emptyMall");
  if (lastMallCatalog && lastMallCatalog.items.length > 0) {
    mallFootnoteEl.textContent = `${t("resources.mallCount", {
      count: String(rows.length),
      total: String(lastMallCatalog.items.length),
    })} · ${t("resources.mallFootnote")}`;
  } else {
    mallFootnoteEl.textContent = t("resources.mallFootnote");
  }

  for (const item of rows) {
    const li = document.createElement("li");
    li.className = "res-catalog-item";

    const icon = document.createElement("div");
    icon.className = `res-catalog-icon ${item.kind === "pack" ? "is-pack" : "is-skill"}`;
    icon.textContent = (item.name.trim()[0] || "?").toUpperCase();

    const body = document.createElement("div");
    body.className = "res-catalog-body";
    const titleRow = document.createElement("div");
    titleRow.className = "res-catalog-title-row";
    const title = document.createElement("strong");
    title.textContent = item.name;
    const badge = document.createElement("span");
    badge.className = "res-catalog-badge";
    badge.textContent =
      item.kind === "pack" ? t("resources.mallPackBadge") : t("resources.mallSkillBadge");
    titleRow.append(title, badge);

    const desc = document.createElement("p");
    desc.className = "res-catalog-desc";
    const bits = [item.description.trim()];
    if (item.skill_count != null) {
      bits.push(t("resources.mallSkillCount", { count: String(item.skill_count) }));
    }
    if (item.price_label) {
      bits.push(item.free ? t("resources.mallFree") : item.price_label);
    }
    if (item.owned && !item.free) {
      bits.push(t("resources.mallOwned"));
    }
    desc.textContent = bits.filter(Boolean).join(" · ") || item.id;

    body.append(titleRow, desc);

    const meta = document.createElement("div");
    meta.className = "res-catalog-meta mall-actions";

    if (item.installed) {
      const done = document.createElement("span");
      done.className = "tone-ok";
      done.textContent = t("resources.mallInstalled");
      meta.appendChild(done);
    } else if (canInstallMallItem(item)) {
      const installBtn = document.createElement("button");
      installBtn.type = "button";
      installBtn.className = "btn-primary btn-compact";
      installBtn.textContent = t("resources.mallInstall");
      installBtn.addEventListener("click", () => {
        void installMallItem(item, installBtn);
      });
      meta.appendChild(installBtn);
    } else {
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
      meta.appendChild(buyBtn);
    }

    li.append(icon, body, meta);
    mallListEl.appendChild(li);
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
  footnoteEl.textContent = lastMcpStatus?.inventory.workspace_name
    ? `${lastMcpStatus.inventory.workspace_name}${
        lastMcpStatus.inventory.workspace_path ? ` · ${lastMcpStatus.inventory.workspace_path}` : ""
      }`
    : "";

  renderSkillsList();
  renderToolsList();
  renderMallList();
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
    footnoteEl.textContent = withErrorDetail(t("skills.mountFailed"), error);
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

async function loadMall(): Promise<void> {
  mallFootnoteEl.textContent = t("resources.mallFootnote");
  try {
    const [catalog, account] = await Promise.all([
      invoke<TeamupsMallCatalog>("list_teamups_mall_catalog_command"),
      invoke<TeamupsAccountStatus>("teamups_account_status_command").catch(() => null),
    ]);
    lastMallCatalog = catalog;
    lastTeamupsAccount = account;
    applyAccountOwnership();
    renderMallList();
  } catch (error) {
    lastMallCatalog = null;
    mallListEl.replaceChildren();
    mallEmptyEl.hidden = false;
    mallEmptyEl.textContent = withErrorDetail(t("resources.mallLoadFailed"), error);
    mallFootnoteEl.textContent = withErrorDetail(t("resources.mallLoadFailed"), error);
    renderMallAccount();
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
      mallFootnoteEl.textContent = t("resources.mallLoginExpired");
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
        mallFootnoteEl.textContent = t("resources.mallLoginOk");
        await loadMall();
        return;
      }
      if (poll.status === "expired" || poll.status === "denied") {
        mallFootnoteEl.textContent = t("resources.mallLoginExpired");
        renderMallAccount();
        return;
      }
    } catch (error) {
      mallLoginInFlight = false;
      mallFootnoteEl.textContent = teamupsLoginFailure(error);
      renderMallAccount();
    }
  };
  await tick();
}

async function startMallLogin(): Promise<void> {
  if (mallLoginInFlight) return;
  mallLoginInFlight = true;
  renderMallAccount();
  mallFootnoteEl.textContent = t("resources.mallLoggingIn");
  try {
    const started = await invoke<TeamupsLoginStart>("start_teamups_login_command");
    await openUrl(started.verification_url);
    const expiresAt = Date.now() + Math.max(30, started.expires_in_sec) * 1000;
    await scheduleMallLoginPoll(started.device_code, started.interval_sec, expiresAt);
  } catch (error) {
    mallLoginInFlight = false;
    mallFootnoteEl.textContent = teamupsLoginFailure(error);
    renderMallAccount();
  }
}

async function signOutMallAccount(): Promise<void> {
  if (mallLoginInFlight) return;
  stopMallLoginPoll();
  try {
    lastTeamupsAccount = await invoke<TeamupsAccountStatus>("sign_out_teamups_command");
    mallFootnoteEl.textContent = t("resources.mallLogoutOk");
    await loadMall();
  } catch (error) {
    mallFootnoteEl.textContent = withErrorDetail(t("resources.mallLoginFailed"), error);
  }
}

async function openMallPurchase(item: TeamupsCatalogItem): Promise<void> {
  const url = item.purchase_url?.trim();
  if (!url) {
    mallFootnoteEl.textContent = t("resources.mallInstallFailed");
    return;
  }
  try {
    await openUrl(url);
    mallFootnoteEl.textContent = t("resources.mallFootnote");
  } catch (error) {
    mallFootnoteEl.textContent = withErrorDetail(t("resources.mallInstallFailed"), error);
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
  mallFootnoteEl.textContent = t("resources.mallInstalling");
  try {
    const report = await invoke<SyncReport>("install_teamups_mall_item_command", {
      kind: item.kind,
      id: item.id,
      packSlug:
        item.kind === "skill"
          ? (item.pack_slug?.trim() || item.id)
          : item.pack_slug,
    });
    mallFootnoteEl.textContent = t("resources.mallInstallOk", {
      installed: String(report.installed),
    });
    await Promise.all([loadMall(), loadSkills()]);
  } catch (error) {
    mallFootnoteEl.textContent = teamupsMallInstallFailure(error);
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
  await Promise.all([loadMcpStatus(), loadSkills(), loadMall()]);
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
    btn.dataset.section === "mall" ||
    btn.dataset.section === "tools" ||
    btn.dataset.section === "browser"
  ) {
    setSection(btn.dataset.section);
  }
});

toolFiltersEl.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-tool-filter]");
  const filter = button?.dataset.toolFilter as ToolFilter | undefined;
  if (!filter) return;
  toolFilter = filter;
  renderToolsList();
});

mallFiltersEl.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-mall-filter]");
  const filter = button?.dataset.mallFilter as MallFilter | undefined;
  if (!filter) return;
  mallFilter = filter;
  renderMallList();
});

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
  renderMallList();
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
  if (section === "browser") setSection("browser");
  else if (section === "tools" || section === "mcp") setSection("tools");
  else if (section === "mall" || section === "store") setSection("mall");
  else if (section === "skills" || section === "catalog") setSection("skills");
  void refreshAll();
});
