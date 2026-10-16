
import { builtinHandRow, builtinHands } from "./chat/builtin-hands";
import { readBrowserToolsEnabled } from "./chat/hands-pref";
import { t, type MessageKey } from "./i18n";
import { withErrorDetail } from "./friendly-error";
import { attachSkillReorder } from "./chat/skill-drag";
import { loadPinnedSkillIds, loadSkillOrder, orderBySaved } from "./chat/skill-order";
import {
  classifySkillCategory,
  skillCategoryLabelKey,
  SKILL_CATEGORY_ORDER,
  type SkillCategoryId,
} from "./skill-categories";
import {
  loadSkillCategoryOverrides,
  type SkillCategoryOverrides,
} from "./chat/skill-category-order";
import type {
  McpInventoryItem,
  SkillInventoryItem,
} from "./types";
import { knowledgePages, listSkillsInventory, listMcpInventory, listWorkspaces } from "./ipc";

export type ResourceEmptyAction = "skill" | "tool";

/** Empty skill/tool line plus a button. Search-with-no-match stays text only. */
export function paintResourceEmpty(
  el: HTMLElement,
  text: string,
  action: ResourceEmptyAction | null,
): void {
  if (typeof el.querySelector !== "function") {
    el.textContent = text;
    return;
  }
  let line = el.querySelector<HTMLElement>(":scope > .chat-empty-line");
  if (!line) {
    el.replaceChildren();
    line = document.createElement("span");
    line.className = "chat-empty-line";
    el.append(line);
  }
  line.textContent = text;
  const existing = el.querySelector<HTMLButtonElement>(":scope > .chat-empty-next");
  if (!action) {
    existing?.remove();
    return;
  }
  const button = existing ?? document.createElement("button");
  button.type = "button";
  button.className = "chat-empty-next";
  button.dataset.emptyAction = action;
  button.textContent = action === "skill" ? t("chat.skillInstall") : t("chat.addTool");
  if (!existing) el.append(button);
}

export type AskRuntime =
  | "claude-code"
  | "codex"
  | "hermes"
  | "openclaw"
  | "deepseek-harness";

export type MentionKind = "skill" | "mcp" | "knowledge";

export type MentionRef = {
  kind: MentionKind;
  id: string;
  label: string;
  category?: SkillCategoryId;
  pinned?: boolean;
  /** Short scope line, such as 全局 or 这个项目. */
  hint?: string;
};

export type WorkspaceDoc = {
  active: string | null;
  workspaces: Record<string, { path: string; title?: string }>;
};

export type MentionQuery = {
  kind: MentionKind | "any";
  q: string;
  start: number;
  end: number;
  /** `@skill:` style token vs Cursor-like `/` slash picker. */
  trigger: "at" | "slash";
};

export type SlashTab = MentionKind;

const MENTION_TOKEN_RE = /@(?:skill|mcp|knowledge):([^\s@]+)/gi;

const MENTION_TABS: MentionKind[] = ["skill", "knowledge", "mcp"];

export function mcpMatchesRuntime(server: McpInventoryItem, runtime: AskRuntime): boolean {
  const hint = server.runtime_hint.trim();
  return hint === runtime || hint === "shared" || hint === "";
}

/** Prefer workspace/runtime bindings over globals/legacy when collapsing inventory rows. */
function mcpBindingRank(server: McpInventoryItem): number {
  switch (server.scope) {
    case "project":
    case "openclaw-workspace":
    case "codex-home":
    case "hermes-home":
      return 4;
    case "openclaw-global":
    case "claude-user":
      return 3;
    case "claude-settings-ignored":
      return 1;
    default:
      return 2;
  }
}

/**
 * Inventory returns one row per config file. Ask chips should show one entry per
 * server name (same grouping as the Resources tab), preferring the best binding.
 */
export function dedupeMcpsByName(servers: McpInventoryItem[]): McpInventoryItem[] {
  const best = new Map<string, McpInventoryItem>();
  for (const server of servers) {
    const key = server.name.trim().toLowerCase() || server.name;
    const prev = best.get(key);
    if (!prev) {
      best.set(key, server);
      continue;
    }
    const score = (s: McpInventoryItem) =>
      (s.healthy ? 10 : 0) + mcpBindingRank(s);
    if (score(server) > score(prev)) best.set(key, server);
  }
  return [...best.values()].sort((a, b) => a.name.localeCompare(b.name));
}

/** The client already shows these as 操作网页 / 操作这台电脑. */
export function isBuiltinHandMcp(server: McpInventoryItem): boolean {
  const name = server.name.trim().toLowerCase();
  return server.is_browser || name === "browser" || name === "computer";
}

export function mcpChipLabel(server: McpInventoryItem): string {
  // Name is already "browser" for the Browser MCP entry — don't append "· browser".
  if (server.is_browser && server.name.trim().toLowerCase() === "browser") {
    return "browser";
  }
  if (server.is_browser) return `${server.name} · browser`;
  return server.name;
}

function skillUsageLine(skill: SkillInventoryItem): string {
  const calls =
    skill.call_count != null ? t("chat.skillUsageCalls", { count: String(skill.call_count) }) : "";
  const rate =
    skill.first_success_rate != null
      ? t("chat.skillUsageRate", { rate: `${Math.round(skill.first_success_rate * 100)}%` })
      : "";
  return [calls, rate].filter(Boolean).join(" · ");
}

function skillCategory(
  skill: SkillInventoryItem,
  overrides: SkillCategoryOverrides = {},
): SkillCategoryId {
  return overrides[skill.skill_id] ?? classifySkillCategory({
    id: skill.skill_id,
    name: skill.name,
    description: skill.description,
  });
}

type MentionGroup = {
  id: "pinned" | SkillCategoryId;
  label: string;
  options: MentionRef[];
};

function groupSkillMentions(options: MentionRef[]): MentionGroup[] {
  const groups: MentionGroup[] = [];
  const pinned = options.filter((option) => option.pinned);
  if (pinned.length) {
    groups.push({ id: "pinned", label: t("chat.skillPinned"), options: pinned });
  }
  for (const id of SKILL_CATEGORY_ORDER) {
    const items = options.filter(
      (option) => !option.pinned && (option.category ?? "other") === id,
    );
    if (!items.length) continue;
    groups.push({
      id,
      label: t(skillCategoryLabelKey(id) as MessageKey),
      options: items,
    });
  }
  return groups;
}

export function skillMountedForRuntime(skill: SkillInventoryItem, runtime: AskRuntime): boolean {
  return skill.agents.some((agent) => agent.runtime === runtime && agent.mounted);
}

export function mentionKey(m: MentionRef): string {
  return `${m.kind}:${m.id}`;
}

export function parseMentionsFromText(
  text: string,
  mountedSkills: SkillInventoryItem[],
  enabledMcps: McpInventoryItem[],
): MentionRef[] {
  const found: MentionRef[] = [];
  const seen = new Set<string>();
  for (const match of text.matchAll(MENTION_TOKEN_RE)) {
    const raw = match[0];
    const id = (match[1] || "").trim();
    if (!id) continue;
    const token = raw.toLowerCase();
    const kind: MentionKind = token.startsWith("@mcp:")
      ? "mcp"
      : token.startsWith("@knowledge:")
        ? "knowledge"
        : "skill";
    const key = `${kind}:${id}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const label =
      kind === "skill"
        ? mountedSkills.find((s) => s.skill_id === id || s.name === id)?.name || id
        : enabledMcps.find((s) => s.name === id)?.name || id;
    found.push({ kind, id, label });
  }
  return found;
}

export function stripMentionTokens(text: string): string {
  return text
    .replace(MENTION_TOKEN_RE, " ")
    .replace(/[ \t]{2,}/g, " ")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

export function mergeMentionsForSend(
  userText: string,
  selected: MentionRef[],
  mountedSkills: SkillInventoryItem[],
  enabledMcps: McpInventoryItem[],
): MentionRef[] {
  const fromText = parseMentionsFromText(userText, mountedSkills, enabledMcps);
  const map = new Map<string, MentionRef>();
  for (const m of [...selected, ...fromText]) {
    map.set(mentionKey(m), m);
  }
  return [...map.values()];
}

export function buildMentionConstraint(mentions: MentionRef[]): string {
  if (mentions.length === 0) return "";
  const tools = mentions.filter((m) => m.kind !== "knowledge");
  const pages = mentions.filter((m) => m.kind === "knowledge");
  const parts: string[] = [];
  if (tools.length) {
    const list = tools
      .map((m) => {
        if (m.kind === "skill") return `- Skill: ${m.label} (id: ${m.id})`;
        if (m.id.toLowerCase() === "browser" || m.label.toLowerCase().includes("browser")) {
          return `- MCP server: ${m.label} — use tools browser_navigate / browser_click / browser_screenshot (never shell open/curl)`;
        }
        return `- MCP server: ${m.label}`;
      })
      .join("\n");
    parts.push(t("chat.mentionHint", { list }));
  }
  if (pages.length) {
    parts.push(
      pages
        .map((m) => t("chat.mentionKnowledgeUse", { name: m.label, path: m.id }))
        .join("\n"),
    );
  }
  return parts.join("\n\n");
}

/** Wiki pages the composer can attach. `index.md` and `log.md` are not choices. */
export function knowledgeMentionsFromPages(
  scopes: { hint: string; root: string; pages: { path: string; title: string }[] }[],
): MentionRef[] {
  const rows: MentionRef[] = [];
  const seen = new Set<string>();
  for (const scope of scopes) {
    const root = scope.root.replace(/\\/g, "/").replace(/\/+$/, "");
    if (!root) continue;
    for (const page of scope.pages) {
      const rel = page.path.replace(/\\/g, "/");
      if (!rel || rel === "index.md" || rel === "log.md") continue;
      const id = `${root}/wiki/${rel}`;
      if (seen.has(id)) continue;
      seen.add(id);
      rows.push({
        kind: "knowledge",
        id,
        label: page.title.trim() || rel,
        hint: scope.hint,
      });
    }
  }
  return rows;
}

export async function loadKnowledgeMentions(projectPath: string | null): Promise<MentionRef[]> {
  const scopes: { hint: string; projectPath: string | null }[] = [
    { hint: t("chat.knowledgeScopeMenuGlobal"), projectPath: null },
  ];
  const project = projectPath?.trim();
  if (project) scopes.push({ hint: t("chat.knowledgeScopeMenuProject"), projectPath: project });
  const loaded = await Promise.all(
    scopes.map(async (scope) => {
      try {
        const report = await knowledgePages({ projectPath: scope.projectPath });
        return { hint: scope.hint, root: report.root, pages: report.pages };
      } catch {
        return { hint: scope.hint, root: "", pages: [] as { path: string; title: string }[] };
      }
    }),
  );
  return knowledgeMentionsFromPages(loaded);
}

export function promptRequestsBrowserMcp(text: string): boolean {
  const lower = text.toLowerCase();
  if (
    text.includes("浏览器") ||
    lower.includes("browser mcp") ||
    lower.includes("@mcp:browser") ||
    lower.includes("browser_navigate") ||
    lower.includes("open browser") ||
    lower.includes("launch browser") ||
    lower.includes("navigate to")
  ) {
    return true;
  }
  if (
    (lower.includes("open ") || lower.includes("visit ") || lower.includes("go to ")) &&
    (lower.includes("http://") ||
      lower.includes("https://") ||
      lower.includes(".com") ||
      lower.includes(".cn") ||
      lower.includes("baidu") ||
      lower.includes("google"))
  ) {
    return true;
  }
  return false;
}

export function ensureBrowserMention(
  mentions: MentionRef[],
  userText: string,
  enabledMcps: McpInventoryItem[],
): MentionRef[] {
  if (!readBrowserToolsEnabled()) {
    return mentions.filter(
      (item) => !(item.kind === "mcp" && item.id.toLowerCase().includes("browser")),
    );
  }
  if (!promptRequestsBrowserMcp(userText)) return mentions;
  if (mentions.some((m) => m.kind === "mcp" && m.id.toLowerCase().includes("browser"))) {
    return mentions;
  }
  const browser = enabledMcps.find(
    (s) => s.is_browser || s.name.toLowerCase() === "browser" || s.name.toLowerCase().includes("browser"),
  );
  if (!browser) return mentions;
  return [...mentions, { kind: "mcp", id: browser.name, label: browser.name }];
}

export type ResourcesTab = "all" | "skills" | "mcp";
export type ResourcesScopeTab = "global" | "project" | "agent";
export type SkillListCategory = "all" | "pinned" | SkillCategoryId;

const GLOBAL_MCP_SCOPES = new Set(["claude-user", "openclaw-global", "claude-settings-ignored"]);
const PROJECT_MCP_SCOPES = new Set(["project", "openclaw-workspace"]);
const AGENT_MCP_SCOPES = new Set(["codex-home", "hermes-home", "dsh-home"]);

export function mcpMatchesScopeTab(
  server: McpInventoryItem,
  tab: ResourcesScopeTab,
  runtime: AskRuntime,
): boolean {
  switch (tab) {
    case "global":
      return GLOBAL_MCP_SCOPES.has(server.scope);
    case "project":
      return PROJECT_MCP_SCOPES.has(server.scope);
    case "agent":
      return (
        mcpMatchesRuntime(server, runtime) &&
        (AGENT_MCP_SCOPES.has(server.scope) || PROJECT_MCP_SCOPES.has(server.scope))
      );
  }
}

export function skillMatchesScopeTab(
  skill: SkillInventoryItem,
  tab: ResourcesScopeTab,
  projectName: string | null,
  runtime: AskRuntime,
): boolean {
  if (tab === "agent") {
    return skillMountedForRuntime(skill, runtime);
  }
  if (tab === "global") {
    return skill.agents.some(
      (agent) =>
        agent.mounted &&
        (agent.scope === "user" ||
          agent.scope.startsWith("user") ||
          agent.scope.includes("global") ||
          agent.scope.includes("bundled")),
    );
  }
  if (tab === "project") {
    const name = projectName?.trim();
    if (!name) return false;
    const tag = `ws:${name}`;
    return skill.agents.some((agent) => agent.mounted && agent.scope.includes(tag));
  }
  return false;
}

export type AskResourcesDom = {
  shellEl: HTMLElement;
  resourcesPanelEl: HTMLElement;
  resourcesToggleEl: HTMLButtonElement;
  resourcesCountEl?: HTMLElement | null;
  resourcesTabsEl?: HTMLElement | null;
  resourcesSearchEl?: HTMLInputElement | null;
  skillsListEl: HTMLElement;
  mcpListEl: HTMLElement;
  skillsEmptyEl: HTMLElement;
  mcpEmptyEl: HTMLElement;
  mentionsEl: HTMLElement;
};

export type LoadAskResourcesOpts = {
  setStatus: (message: string, tone: "ok" | "warn" | "error" | "muted") => void;
  renderWorkspaceSwitcher: (doc: WorkspaceDoc) => void;
  cwdEl: HTMLElement;
  workspaceHintEl: HTMLElement;
  setDisplayedCwd: (cwd: string) => void;
};

export type LoadAskResourcesResult = {
  workspaceCwd: string | null;
  workspaceDoc: WorkspaceDoc | null;
};

export class AskResourcesController {
  mountedSkills: SkillInventoryItem[] = [];
  enabledMcps: McpInventoryItem[] = [];
  selectedMentions: MentionRef[] = [];
  resourcesTab: ResourcesTab = "all";
  resourcesQuery = "";

  constructor(
    private dom: AskResourcesDom,
    private getRuntime: () => AskRuntime,
  ) {}

  setResourcesOpen(open: boolean): void {
    this.dom.shellEl.classList.toggle("is-resources-open", open);
    this.dom.resourcesToggleEl.classList.toggle("is-open", open);
    this.dom.resourcesToggleEl.setAttribute("aria-expanded", open ? "true" : "false");
    this.dom.resourcesPanelEl.setAttribute("aria-hidden", open ? "false" : "true");
  }

  toggleResourcesPanel(): void {
    this.setResourcesOpen(!this.dom.shellEl.classList.contains("is-resources-open"));
  }

  setResourcesTab(tab: ResourcesTab): void {
    this.resourcesTab = tab;
    this.dom.resourcesTabsEl?.querySelectorAll<HTMLButtonElement>("[data-res-tab]").forEach((btn) => {
      const active = btn.dataset.resTab === tab;
      btn.classList.toggle("is-active", active);
      btn.setAttribute("aria-selected", active ? "true" : "false");
    });
    this.renderResourceChips();
  }

  setResourcesQuery(query: string): void {
    this.resourcesQuery = query.trim().toLowerCase();
    this.renderResourceChips();
  }

  updateResourcesSummary(): void {
    if (this.dom.resourcesCountEl) {
      this.dom.resourcesCountEl.textContent = t("chat.resourcesCount", {
        skills: String(this.mountedSkills.length),
        mcp: String(this.enabledMcps.length),
      });
    }
  }

  hasMention(kind: MentionKind, id: string): boolean {
    return this.selectedMentions.some((m) => m.kind === kind && m.id === id);
  }

  upsertMention(mention: MentionRef): void {
    if (this.hasMention(mention.kind, mention.id)) return;
    this.selectedMentions.push(mention);
    this.renderMentions();
    this.renderResourceChips();
  }

  removeMention(kind: MentionKind, id: string): void {
    this.selectedMentions = this.selectedMentions.filter((m) => !(m.kind === kind && m.id === id));
    this.renderMentions();
    this.renderResourceChips();
  }

  toggleMention(mention: MentionRef): void {
    if (this.hasMention(mention.kind, mention.id)) this.removeMention(mention.kind, mention.id);
    else this.upsertMention(mention);
  }

  clearMentions(): void {
    this.selectedMentions = [];
    this.renderMentions();
    this.renderResourceChips();
  }

  renderMentions(): void {
    this.dom.mentionsEl.replaceChildren();
    this.dom.mentionsEl.hidden = this.selectedMentions.length === 0;
    for (const mention of this.selectedMentions) {
      const chip = document.createElement("span");
      chip.className = "chat-mention-chip";
      const label = document.createElement("span");
      label.textContent =
        mention.kind === "skill"
          ? t("chat.mentionSkill", { name: mention.label })
          : mention.kind === "knowledge"
            ? t("chat.mentionKnowledge", { name: mention.label })
            : t("chat.mentionMcp", { name: mention.label });
      const remove = document.createElement("button");
      remove.type = "button";
      remove.setAttribute("aria-label", t("chat.mentionRemove"));
      remove.textContent = "×";
      remove.addEventListener("click", () => this.removeMention(mention.kind, mention.id));
      chip.append(label, remove);
      this.dom.mentionsEl.appendChild(chip);
    }
  }

  renderScopedResourceLists(
    tab: ResourcesScopeTab,
    projectName: string | null,
    runtime: AskRuntime,
    targets: {
      skillsListEl: HTMLElement;
      mcpListEl: HTMLElement;
      skillsEmptyEl: HTMLElement;
      mcpEmptyEl: HTMLElement;
      query?: string;
      onDeleteSkill?: (skillId: string) => void;
      pendingDeleteId?: string;
      onPinSkill?: (skillId: string) => void;
      onReorderSkill?: (fromId: string, toId: string, position: "before" | "after") => void;
      skillCategory?: SkillListCategory;
      skillCategoriesEl?: HTMLElement | null;
      onSkillCategory?: (category: SkillListCategory) => void;
    },
  ): void {
    const q = (targets.query ?? this.resourcesQuery).trim().toLowerCase();
    const scopedSkills = orderBySaved(
      this.mountedSkills
      .filter((skill) => skillMatchesScopeTab(skill, tab, projectName, runtime))
      .filter((skill) => {
        if (!q) return true;
        const hay = `${skill.name} ${skill.skill_id} ${skill.description ?? ""}`.toLowerCase();
        return hay.includes(q);
      }),
      loadSkillOrder(),
    );
    const categoryOverrides = loadSkillCategoryOverrides();
    const pinnedSkills = new Set(loadPinnedSkillIds());
    const pinnedCount = scopedSkills.filter((skill) => pinnedSkills.has(skill.skill_id)).length;
    const requestedCategory = targets.skillCategory ?? "all";
    const category =
      requestedCategory === "all" ||
      scopedSkills.some((skill) =>
        requestedCategory === "pinned"
          ? pinnedSkills.has(skill.skill_id)
          : !pinnedSkills.has(skill.skill_id) &&
            skillCategory(skill, categoryOverrides) === requestedCategory,
      )
        ? requestedCategory
        : "all";
    const skills = scopedSkills.filter(
      (skill) => {
        if (category === "all") return true;
        if (category === "pinned") return pinnedSkills.has(skill.skill_id);
        return (
          !pinnedSkills.has(skill.skill_id) &&
          skillCategory(skill, categoryOverrides) === category
        );
      },
    );
    const mcps = dedupeMcpsByName(
      this.enabledMcps
        .filter((server) => !isBuiltinHandMcp(server))
        .filter((server) => mcpMatchesScopeTab(server, tab, runtime))
        .filter((server) => {
          if (!q) return true;
          const hay = `${server.name} ${server.scope} ${server.runtime_hint} ${server.issue ?? ""}`.toLowerCase();
          return hay.includes(q);
        }),
    );

    const hands = tab === "global" ? builtinHands(q) : [];
    targets.skillsListEl.replaceChildren();
    targets.mcpListEl.replaceChildren();
    targets.skillsEmptyEl.hidden = skills.length > 0;
    targets.mcpEmptyEl.hidden = mcps.length > 0 || hands.length > 0;
    if (skills.length === 0) {
      paintResourceEmpty(
        targets.skillsEmptyEl,
        q ? t("chat.resourcesNoMatch") : t("chat.skillsEmpty"),
        q ? null : "skill",
      );
    }
    if (mcps.length === 0 && hands.length === 0) {
      paintResourceEmpty(
        targets.mcpEmptyEl,
        q ? t("chat.resourcesNoMatch") : t("chat.mcpEmpty"),
        q ? null : "tool",
      );
    }

    if (targets.skillCategoriesEl) {
      targets.skillCategoriesEl.replaceChildren();
      targets.skillCategoriesEl.setAttribute("aria-label", t("chat.skillCategories"));
      const counts = new Map<SkillCategoryId, number>();
      for (const skill of scopedSkills) {
        if (pinnedSkills.has(skill.skill_id)) continue;
        const id = skillCategory(skill, categoryOverrides);
        counts.set(id, (counts.get(id) ?? 0) + 1);
      }
      const categories: SkillListCategory[] = [
        "all",
        ...(pinnedCount > 0 ? ["pinned" as const] : []),
        ...SKILL_CATEGORY_ORDER.filter((id) => (counts.get(id) ?? 0) > 0),
      ];
      for (const id of categories) {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "chat-resource-category";
        button.classList.toggle("is-on", id === category);
        button.setAttribute("aria-pressed", id === category ? "true" : "false");
        const label =
          id === "all"
            ? t("resources.filterAll")
            : id === "pinned"
              ? t("chat.skillPinned")
              : t(skillCategoryLabelKey(id) as MessageKey);
        const count =
          id === "all"
            ? scopedSkills.length
            : id === "pinned"
              ? pinnedCount
              : counts.get(id) ?? 0;
        button.textContent = `${label} ${count}`;
        button.addEventListener("click", () => targets.onSkillCategory?.(id));
        targets.skillCategoriesEl.append(button);
      }
      targets.skillCategoriesEl.hidden = scopedSkills.length === 0;
    }

    const groups = new Map<Exclude<SkillListCategory, "all">, SkillInventoryItem[]>();
    for (const skill of skills) {
      const id = pinnedSkills.has(skill.skill_id)
        ? "pinned"
        : skillCategory(skill, categoryOverrides);
      groups.set(id, [...(groups.get(id) ?? []), skill]);
    }
    const shownGroups =
      category === "all"
        ? (["pinned", ...SKILL_CATEGORY_ORDER] as const).filter((id) => groups.has(id))
        : [category];
    for (const groupId of shownGroups) {
      const group = groups.get(groupId) ?? [];
      if (category === "all" && group.length) {
        const heading = document.createElement("h4");
        heading.className = "chat-res-group";
        const label =
          groupId === "pinned"
            ? t("chat.skillPinned")
            : t(skillCategoryLabelKey(groupId) as MessageKey);
        heading.textContent = `${label} · ${group.length}`;
        targets.skillsListEl.appendChild(heading);
      }
      const groupList = document.createElement("div");
      groupList.className = "chat-res-group-list";
      groupList.setAttribute("role", "list");
      targets.skillsListEl.appendChild(groupList);
      group.forEach((skill) => {
      const title = skill.name || skill.skill_id;
      const meta = skillUsageLine(skill);
      const removing = targets.pendingDeleteId === skill.skill_id;
      const canSort =
        Boolean(targets.onReorderSkill) &&
        !q &&
        (category === "all" ? skills.length > 1 : group.length > 1);
      const row = this.buildResourceRow({
        kind: "skill",
        id: skill.skill_id,
        title,
        description: skill.description?.trim() || skill.skill_id,
        badge: "Skill",
        meta,
        tone: "skill",
        active: this.hasMention("skill", skill.skill_id),
        onClick: () =>
          this.toggleMention({
            kind: "skill",
            id: skill.skill_id,
            label: title,
          }),
        onDelete: targets.onDeleteSkill ? () => targets.onDeleteSkill?.(skill.skill_id) : undefined,
        deleteLabel: removing ? t("chat.skillDeleteConfirm") : t("chat.skillDelete"),
        deleteArmed: removing,
        onPin: targets.onPinSkill ? () => targets.onPinSkill?.(skill.skill_id) : undefined,
        pinned: pinnedSkills.has(skill.skill_id),
        sortable: canSort,
      });
      row.dataset.category = groupId;
      groupList.appendChild(row);
      if (!canSort || !targets.onReorderSkill) return;
      const grip = row.querySelector<HTMLElement>(".chat-res-grip");
      if (grip) {
        attachSkillReorder(
          category === "all" ? targets.skillsListEl : groupList,
          grip,
          skill.skill_id,
          targets.onReorderSkill,
        );
      }
      });
    }

    for (const hand of hands) {
      targets.mcpListEl.appendChild(builtinHandRow(hand, "page"));
    }

    for (const server of mcps) {
      const title = mcpChipLabel(server);
      const badge = server.is_browser ? "Browser" : "MCP";
      const meta = server.healthy ? "" : server.issue || t("chat.resourcesUnhealthy");
      targets.mcpListEl.appendChild(
        this.buildResourceRow({
          kind: "mcp",
          id: server.name,
          title,
          description: server.command
            ? `${server.command} ${(server.args ?? []).join(" ")}`.trim()
            : `${server.runtime_hint} · ${server.config_path}`,
          badge,
          meta,
          tone: server.is_browser ? "browser" : "mcp",
          active: this.hasMention("mcp", server.name),
          warn: !server.healthy,
          onClick: () =>
            this.toggleMention({
              kind: "mcp",
              id: server.name,
              label: server.name,
            }),
        }),
      );
    }
    this.updateResourcesSummary();
  }

  renderResourceChips(): void {
    this.dom.skillsListEl.replaceChildren();
    this.dom.mcpListEl.replaceChildren();

    const q = this.resourcesQuery;
    const skills = orderBySaved(
      this.mountedSkills.filter((skill) => {
        if (!q) return true;
        const hay = `${skill.name} ${skill.skill_id} ${skill.description ?? ""}`.toLowerCase();
        return hay.includes(q);
      }),
      loadSkillOrder(),
    );
    const mcps = this.enabledMcps.filter((server) => !isBuiltinHandMcp(server)).filter((server) => {
      if (!q) return true;
      const hay = `${server.name} ${server.scope} ${server.runtime_hint} ${server.issue ?? ""}`.toLowerCase();
      return hay.includes(q);
    });

    const showSkills = this.resourcesTab === "all" || this.resourcesTab === "skills";
    const showMcp = this.resourcesTab === "all" || this.resourcesTab === "mcp";

    const skillsSection =
      typeof this.dom.skillsListEl.closest === "function"
        ? this.dom.skillsListEl.closest<HTMLElement>("[data-res-section]")
        : null;
    const mcpSection =
      typeof this.dom.mcpListEl.closest === "function"
        ? this.dom.mcpListEl.closest<HTMLElement>("[data-res-section]")
        : null;
    if (skillsSection) skillsSection.hidden = !showSkills;
    if (mcpSection) mcpSection.hidden = !showMcp;

    this.dom.skillsEmptyEl.hidden = !showSkills || skills.length > 0;
    this.dom.mcpEmptyEl.hidden = !showMcp || mcps.length > 0;
    if (showSkills && skills.length === 0 && q) {
      paintResourceEmpty(this.dom.skillsEmptyEl, t("chat.resourcesNoMatch"), null);
      this.dom.skillsEmptyEl.hidden = false;
    } else if (showSkills && skills.length === 0) {
      paintResourceEmpty(this.dom.skillsEmptyEl, t("chat.skillsEmpty"), "skill");
    }
    if (showMcp && mcps.length === 0 && q) {
      paintResourceEmpty(this.dom.mcpEmptyEl, t("chat.resourcesNoMatch"), null);
      this.dom.mcpEmptyEl.hidden = false;
    } else if (showMcp && mcps.length === 0) {
      paintResourceEmpty(this.dom.mcpEmptyEl, t("chat.mcpEmpty"), "tool");
    }

    for (const skill of skills) {
      const title = skill.name || skill.skill_id;
      const meta = skillUsageLine(skill);
      this.dom.skillsListEl.appendChild(
        this.buildResourceRow({
          kind: "skill",
          id: skill.skill_id,
          title,
          description: skill.description?.trim() || skill.skill_id,
          badge: "Skill",
          meta,
          tone: "skill",
          active: this.hasMention("skill", skill.skill_id),
          onClick: () =>
            this.toggleMention({
              kind: "skill",
              id: skill.skill_id,
              label: title,
            }),
        }),
      );
    }

    for (const server of mcps) {
      const title = mcpChipLabel(server);
      const badge = server.is_browser ? "Browser" : "MCP";
      const meta = server.healthy ? "" : server.issue || t("chat.resourcesUnhealthy");
      this.dom.mcpListEl.appendChild(
        this.buildResourceRow({
          kind: "mcp",
          id: server.name,
          title,
          description: server.command
            ? `${server.command} ${(server.args ?? []).join(" ")}`.trim()
            : `${server.runtime_hint} · ${server.config_path}`,
          badge,
          meta,
          tone: server.is_browser ? "browser" : "mcp",
          active: this.hasMention("mcp", server.name),
          warn: !server.healthy,
          onClick: () =>
            this.toggleMention({
              kind: "mcp",
              id: server.name,
              label: server.name,
            }),
        }),
      );
    }

    this.updateResourcesSummary();
  }

  private buildResourceRow(opts: {
    kind: MentionKind;
    id: string;
    title: string;
    description: string;
    badge: string;
    meta: string;
    tone: "skill" | "mcp" | "browser";
    active: boolean;
    warn?: boolean;
    onClick: () => void;
    onDelete?: () => void;
    deleteLabel?: string;
    deleteArmed?: boolean;
    onPin?: () => void;
    pinned?: boolean;
    sortable?: boolean;
  }): HTMLElement {
    const btn = document.createElement(opts.onDelete ? "div" : "button");
    if (!opts.onDelete) (btn as HTMLButtonElement).type = "button";
    btn.className = opts.onDelete ? "chat-res-row has-delete" : "chat-res-row";
    if (opts.sortable) btn.classList.add("has-order");
    else if (opts.onPin) btn.classList.add("has-pin");
    btn.setAttribute("role", "listitem");
    btn.dataset.kind = opts.kind;
    btn.dataset.id = opts.id;
    btn.setAttribute("aria-pressed", opts.active ? "true" : "false");
    if (opts.active) btn.classList.add("is-active");
    if (opts.warn) btn.classList.add("is-warn");

    const icon = document.createElement("span");
    icon.className = "chat-res-icon";
    icon.dataset.tone = opts.tone;
    icon.textContent = (opts.title.trim().charAt(0) || "?").toUpperCase();
    icon.setAttribute("aria-hidden", "true");

    const body = document.createElement("span");
    body.className = "chat-res-body";

    const titleRow = document.createElement("span");
    titleRow.className = "chat-res-title-row";
    const title = document.createElement("span");
    title.className = "chat-res-title";
    title.textContent = opts.title;
    const badge = document.createElement("span");
    badge.className = "chat-res-badge";
    badge.dataset.tone = opts.tone;
    badge.textContent = opts.badge;
    titleRow.append(title, badge);

    const desc = document.createElement("span");
    desc.className = "chat-res-desc";
    desc.textContent = opts.description;

    body.append(titleRow, desc);
    if (opts.meta) {
      const usage = document.createElement("span");
      usage.className = "chat-res-usage";
      usage.textContent = opts.meta;
      body.append(usage);
    }

    btn.title = opts.description;
    if (!opts.onDelete) {
      btn.append(icon, body);
      btn.addEventListener("click", opts.onClick);
      return btn;
    }
    const open = document.createElement("button");
    open.type = "button";
    open.className = "chat-res-open";
    open.append(icon, body);
    open.addEventListener("click", opts.onClick);
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "chat-res-delete";
    remove.classList.toggle("is-armed", opts.deleteArmed === true);
    remove.textContent = opts.deleteLabel ?? "";
    remove.addEventListener("click", (event) => {
      event.stopPropagation();
      opts.onDelete?.();
    });
    if (!opts.sortable && !opts.onPin) {
      btn.append(open, remove);
      return btn;
    }
    const pin = document.createElement("button");
    pin.type = "button";
    pin.className = "chat-res-pin";
    pin.classList.toggle("is-pinned", opts.pinned === true);
    pin.textContent = t(opts.pinned ? "chat.skillUnpin" : "chat.skillPin");
    pin.title = t(opts.pinned ? "chat.skillUnpinHint" : "chat.skillPinHint");
    pin.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      opts.onPin?.();
    });
    if (!opts.sortable) {
      btn.append(open, pin, remove);
      return btn;
    }
    const grip = document.createElement("button");
    grip.type = "button";
    grip.className = "chat-res-grip";
    grip.title = t("chat.skillReorder");
    grip.setAttribute("aria-label", t("chat.skillReorder"));
    grip.innerHTML = `<svg width="8" height="12" viewBox="0 0 8 12" fill="currentColor" aria-hidden="true"><circle cx="2" cy="2" r="1"/><circle cx="6" cy="2" r="1"/><circle cx="2" cy="6" r="1"/><circle cx="6" cy="6" r="1"/><circle cx="2" cy="10" r="1"/><circle cx="6" cy="10" r="1"/></svg>`;
    btn.append(grip, open, pin, remove);
    return btn;
  }

  mentionCandidates(): MentionRef[] {
    const overrides = loadSkillCategoryOverrides();
    const pinned = new Set(loadPinnedSkillIds());
    const skills = orderBySaved(this.mountedSkills, loadSkillOrder()).map((s) => ({
      kind: "skill" as const,
      id: s.skill_id,
      label: s.name || s.skill_id,
      category: skillCategory(s, overrides),
      pinned: pinned.has(s.skill_id),
    }));
    const mcps = this.enabledMcps.filter((server) => !isBuiltinHandMcp(server)).map((s) => ({
      kind: "mcp" as const,
      id: s.name,
      label: s.name,
    }));
    return [...skills, ...mcps];
  }

  async loadAskResources(opts: LoadAskResourcesOpts): Promise<LoadAskResourcesResult> {
    const runtime = this.getRuntime();
    let workspaceCwd: string | null = null;
    let workspaceDoc: WorkspaceDoc | null = null;
    try {
      const [skillsReport, mcpReport] = await Promise.all([
        listSkillsInventory({ remoteStats: false }),
        listMcpInventory(),
      ]);
      workspaceCwd = mcpReport.workspace_path;
      try {
        const doc = await listWorkspaces();
        workspaceDoc = doc;
        opts.renderWorkspaceSwitcher(doc);
        const projectNames = Object.keys(doc.workspaces);
        if (doc.active && doc.workspaces[doc.active]?.path) {
          workspaceCwd = doc.workspaces[doc.active].path;
          opts.cwdEl.dataset.workspace = doc.active;
          opts.setDisplayedCwd(workspaceCwd);
          opts.cwdEl.title = `${t("ask.workspaceActive", { name: doc.active })} · ${workspaceCwd}`;
          opts.workspaceHintEl.textContent = t("ask.workspaceHint");
        } else if (projectNames.length === 0) {
          opts.cwdEl.dataset.workspace = "";
          opts.setDisplayedCwd("—");
          opts.cwdEl.title = t("workspaces.none");
          opts.workspaceHintEl.textContent = t("workspaces.noneHint");
        } else {
          opts.cwdEl.dataset.workspace = "";
          if (mcpReport.workspace_path) {
            opts.setDisplayedCwd(mcpReport.workspace_path);
          } else {
            opts.setDisplayedCwd("—");
          }
          opts.cwdEl.title = t("ask.workspaceNone");
          opts.workspaceHintEl.textContent = t("ask.workspaceNone");
        }
      } catch {
        workspaceDoc = null;
        if (mcpReport.workspace_path && (!opts.cwdEl.dataset.cwd || opts.cwdEl.dataset.cwd === "—")) {
          opts.setDisplayedCwd(mcpReport.workspace_path);
        }
      }
      this.mountedSkills = (skillsReport.skills ?? []).filter((skill) =>
        skillMountedForRuntime(skill, runtime),
      );
      this.enabledMcps = dedupeMcpsByName(
        (mcpReport.servers ?? []).filter((server) => mcpMatchesRuntime(server, runtime)),
      );
      this.selectedMentions = this.selectedMentions.filter((m) => {
        if (m.kind === "knowledge") return true;
        if (m.kind === "skill") return this.mountedSkills.some((s) => s.skill_id === m.id);
        return this.enabledMcps.some((s) => s.name === m.id);
      });
      this.renderMentions();
      this.renderResourceChips();
    } catch (error) {
      opts.setStatus(withErrorDetail(t("chat.resourcesLoadFailed"), error), "warn");
    }
    return { workspaceCwd, workspaceDoc };
  }
}

export type AskMentionMenuDom = {
  promptEl: HTMLTextAreaElement;
  mentionMenuEl: HTMLElement;
};

export class AskMentionMenuController {
  mentionMenuIndex = 0;
  mentionQuery: MentionQuery | null = null;
  /** Active tab while the `/` slash picker is open. */
  slashTab: SlashTab = "skill";

  constructor(
    private dom: AskMentionMenuDom,
    private getCandidates: () => MentionRef[],
    private upsertMention: (mention: MentionRef) => void,
    private autoResizePrompt: () => void,
    private onVisibilityChange?: (open: boolean) => void,
    private loadKnowledge?: () => Promise<MentionRef[]>,
  ) {}

  private knowledgeRefs: MentionRef[] = [];
  private knowledgeGen = 0;

  detectMentionQuery(): MentionQuery | null {
    const value = this.dom.promptEl.value;
    const caret = this.dom.promptEl.selectionStart ?? value.length;
    const before = value.slice(0, caret);

    const slash = before.match(/(?:^|\s)(\/([^\s/@]*))$/);
    if (slash && slash.index != null) {
      const q = slash[2] || "";
      const start = slash.index + (slash[0].startsWith("/") ? 0 : 1);
      return { kind: this.slashTab, q, start, end: caret, trigger: "slash" };
    }

    const match = before.match(/(?:^|\s)(@(?:skill:|mcp:|knowledge:)?([^\s@]*))$/i);
    if (!match || match.index == null) return null;
    const token = match[1];
    const q = match[2] || "";
    const start = match.index + (match[0].startsWith("@") ? 0 : 1);
    const end = caret;
    let kind: MentionKind | "any" = "any";
    const lower = token.toLowerCase();
    if (lower.startsWith("@skill:")) kind = "skill";
    else if (lower.startsWith("@mcp:")) kind = "mcp";
    else if (lower.startsWith("@knowledge:")) kind = "knowledge";
    return { kind, q, start, end, trigger: "at" };
  }

  filteredMentionOptions(): MentionRef[] {
    if (!this.mentionQuery) return [];
    const q = this.mentionQuery.q.toLowerCase();
    const kindFilter = this.mentionQuery.kind === "any" ? this.slashTab : this.mentionQuery.kind;
    return [...this.getCandidates(), ...this.knowledgeRefs].filter((item) => {
      if (item.kind !== kindFilter) return false;
      if (!q) return true;
      return item.id.toLowerCase().includes(q) || item.label.toLowerCase().includes(q);
    });
  }

  setSlashTab(tab: SlashTab): void {
    if (this.slashTab === tab) return;
    this.slashTab = tab;
    this.mentionMenuIndex = 0;
    this.renderMentionMenu();
  }

  cycleSlashTab(dir: 1 | -1): void {
    const tabs = MENTION_TABS;
    const idx = tabs.indexOf(this.slashTab);
    const next = tabs[(idx + dir + tabs.length) % tabs.length]!;
    this.setSlashTab(next);
  }

  /** Open the same list as typing `/`: skills, knowledge, then tools. */
  openSkillPicker(): void {
    const el = this.dom.promptEl;
    const value = el.value;
    const caret = el.selectionStart ?? value.length;
    const before = value.slice(0, caret);
    const after = value.slice(el.selectionEnd ?? caret);
    this.slashTab = "skill";
    this.mentionMenuIndex = 0;
    if (!/(?:^|\s)\/[^\s/@]*$/.test(before)) {
      const prefix = before.length > 0 && !/\s$/.test(before) ? " /" : "/";
      const next = before + prefix + after;
      const pos = before.length + prefix.length;
      el.value = next;
      el.setSelectionRange(pos, pos);
    }
    el.focus();
    this.autoResizePrompt();
    this.renderMentionMenu();
  }

  hideMentionMenu(): void {
    const menu = this.dom.mentionMenuEl;
    const alreadyClosed = menu.hidden && this.mentionQuery === null && menu.childElementCount === 0;
    this.mentionQuery = null;
    if (alreadyClosed) return;
    menu.hidden = true;
    menu.replaceChildren();
    menu.classList.remove("is-slash");
    this.onVisibilityChange?.(false);
  }

  isSlashMenuOpen(): boolean {
    return Boolean(this.mentionQuery && !this.dom.mentionMenuEl.hidden);
  }

  private queueKnowledge(): void {
    if (!this.loadKnowledge) return;
    const gen = ++this.knowledgeGen;
    void this.loadKnowledge()
      .then((rows) => {
        if (gen !== this.knowledgeGen) return;
        const prev = this.knowledgeRefs.map((row) => row.id).join("\n");
        const next = rows.map((row) => row.id).join("\n");
        this.knowledgeRefs = rows;
        if (prev !== next && this.mentionQuery) this.renderMentionMenu();
      })
      .catch(() => undefined);
  }

  renderMentionMenu(): void {
    const previous = this.mentionQuery;
    this.mentionQuery = this.detectMentionQuery();
    if (!this.mentionQuery) {
      this.hideMentionMenu();
      return;
    }
    if (!previous) this.queueKnowledge();
    if (this.mentionQuery.kind !== "any") this.slashTab = this.mentionQuery.kind;

    const filtered = this.filteredMentionOptions();
    const skillGroups =
      this.slashTab === "skill"
        ? groupSkillMentions(filtered.filter((option) => option.kind === "skill"))
        : [];
    const options = skillGroups.length
      ? skillGroups.flatMap((group) => group.options)
      : filtered;

    this.mentionMenuIndex = Math.max(
      0,
      Math.min(this.mentionMenuIndex, Math.max(options.length - 1, 0)),
    );
    this.dom.mentionMenuEl.replaceChildren();
    this.dom.mentionMenuEl.classList.add("is-slash");

    {
      const head = document.createElement("div");
      head.className = "chat-slash-head";

      const tabs = document.createElement("div");
      tabs.className = "chat-slash-tabs";
      tabs.setAttribute("role", "tablist");

      for (const tab of [
        { id: "skill" as const, label: t("chat.slashTabSkills") },
        { id: "knowledge" as const, label: t("chat.slashTabKnowledge") },
        { id: "mcp" as const, label: t("chat.slashTabTools") },
      ]) {
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = `chat-slash-tab${this.slashTab === tab.id ? " is-active" : ""}`;
        btn.setAttribute("role", "tab");
        btn.setAttribute("aria-selected", this.slashTab === tab.id ? "true" : "false");
        btn.textContent = tab.label;
        btn.addEventListener("mousedown", (event) => {
          event.preventDefault();
          this.setSlashTab(tab.id);
        });
        tabs.appendChild(btn);
      }

      const hint = document.createElement("span");
      hint.className = "chat-slash-hint";
      hint.textContent = t("chat.slashHint");

      head.append(tabs, hint);
      this.dom.mentionMenuEl.appendChild(head);

      if (skillGroups.length) {
        const categories = document.createElement("div");
        categories.className = "chat-slash-categories";
        categories.setAttribute("aria-label", t("chat.skillCategories"));
        skillGroups.forEach((group, index) => {
          const category = document.createElement("button");
          category.type = "button";
          category.className = "chat-slash-category";
          category.classList.toggle("is-pinned", group.id === "pinned");
          category.classList.toggle("is-active", index === 0);
          category.dataset.slashCategory = group.id;
          category.textContent = `${group.label} ${group.options.length}`;
          category.addEventListener("mousedown", (event) => {
            event.preventDefault();
            const target = this.dom.mentionMenuEl.querySelector<HTMLElement>(
              `[data-slash-group="${group.id}"]`,
            );
            target?.scrollIntoView({ block: "start" });
            categories.querySelectorAll<HTMLElement>(".chat-slash-category").forEach((item) => {
              item.classList.toggle("is-active", item === category);
            });
          });
          categories.append(category);
        });
        this.dom.mentionMenuEl.appendChild(categories);
      }
    }

    const list = document.createElement("div");
    list.className = "chat-slash-list";
    list.setAttribute("role", "listbox");

    const hands = this.slashTab === "mcp" ? builtinHands(this.mentionQuery?.q ?? "") : [];
    if (hands.length) {
      const group = document.createElement("div");
      group.className = options.length ? "chat-slash-hands has-more" : "chat-slash-hands";
      for (const hand of hands) group.appendChild(builtinHandRow(hand, "menu"));
      list.appendChild(group);
    }

    if (options.length === 0 && hands.length === 0) {
      const empty = document.createElement("div");
      empty.className = "chat-slash-empty";
      empty.textContent = t(
        this.slashTab === "skill"
          ? "chat.slashEmptySkills"
          : this.slashTab === "knowledge"
            ? "chat.slashEmptyKnowledge"
            : "chat.slashEmptyTools",
      );
      list.appendChild(empty);
    } else {
      const appendOption = (option: MentionRef, index: number) => {
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = `chat-mention-option${index === this.mentionMenuIndex ? " is-active" : ""}`;
        btn.setAttribute("role", "option");
        const title = document.createElement("strong");
        title.textContent = option.label;
        const sub = document.createElement("span");
        sub.textContent =
          option.kind === "knowledge"
            ? option.hint || t("chat.slashKindKnowledge")
            : option.kind === "skill" && option.category
              ? t(skillCategoryLabelKey(option.category) as MessageKey)
              : option.kind === "skill"
                ? t("chat.slashKindSkill")
                : t("chat.slashKindTool");
        btn.append(title, sub);
        btn.addEventListener("mousedown", (event) => {
          event.preventDefault();
          this.applyMentionOption(option);
        });
        list.appendChild(btn);
      };
      if (skillGroups.length) {
        let index = 0;
        for (const group of skillGroups) {
          const heading = document.createElement("div");
          heading.className = "chat-slash-group-title";
          heading.dataset.slashGroup = group.id;
          heading.textContent = `${group.label} · ${group.options.length}`;
          list.appendChild(heading);
          for (const option of group.options) {
            appendOption(option, index);
            index += 1;
          }
        }
      } else {
        options.forEach(appendOption);
      }
    }

    if (skillGroups.length) {
      list.addEventListener("scroll", () => {
        const top = list.getBoundingClientRect().top + 8;
        const headings = [...list.querySelectorAll<HTMLElement>("[data-slash-group]")];
        let active = headings[0]?.dataset.slashGroup;
        for (const heading of headings) {
          if (heading.getBoundingClientRect().top <= top) active = heading.dataset.slashGroup;
        }
        this.dom.mentionMenuEl
          .querySelectorAll<HTMLElement>("[data-slash-category]")
          .forEach((category) => {
            category.classList.toggle("is-active", category.dataset.slashCategory === active);
          });
      });
    }

    this.dom.mentionMenuEl.appendChild(list);
    this.dom.mentionMenuEl.hidden = false;
    this.onVisibilityChange?.(true);

    const active = list.querySelector<HTMLElement>(".chat-mention-option.is-active");
    active?.scrollIntoView({ block: "nearest" });
  }

  applyMentionOption(option: MentionRef): void {
    if (!this.mentionQuery) return;
    const value = this.dom.promptEl.value;
    this.dom.promptEl.value = `${value.slice(0, this.mentionQuery.start)}${value.slice(this.mentionQuery.end)}`;
    const next = this.mentionQuery.start;
    this.dom.promptEl.setSelectionRange(next, next);

    this.upsertMention(option);
    this.hideMentionMenu();
    this.autoResizePrompt();
    this.dom.promptEl.focus();
  }
}
