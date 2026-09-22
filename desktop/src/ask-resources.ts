import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import type {
  McpInventoryItem,
  McpInventoryReport,
  SkillInventoryItem,
  SkillsInventoryReport,
} from "./types";

export type AskRuntime =
  | "claude-code"
  | "codex"
  | "hermes"
  | "openclaw"
  | "deepseek-harness";

export type MentionKind = "skill" | "mcp";

export type MentionRef = {
  kind: MentionKind;
  id: string;
  label: string;
};

export type WorkspaceDoc = {
  active: string | null;
  workspaces: Record<string, { path: string }>;
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

const MENTION_TOKEN_RE = /@(?:skill|mcp):([^\s@]+)/gi;

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

export function mcpChipLabel(server: McpInventoryItem): string {
  // Name is already "browser" for the Browser MCP entry — don't append "· browser".
  if (server.is_browser && server.name.trim().toLowerCase() === "browser") {
    return "browser";
  }
  if (server.is_browser) return `${server.name} · browser`;
  return server.name;
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
    const kind: MentionKind = raw.toLowerCase().startsWith("@mcp:") ? "mcp" : "skill";
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
  const list = mentions
    .map((m) => {
      if (m.kind === "skill") return `- Skill: ${m.label} (id: ${m.id})`;
      if (m.id.toLowerCase() === "browser" || m.label.toLowerCase().includes("browser")) {
        return `- MCP server: ${m.label} — use tools browser_navigate / browser_click / browser_screenshot (never shell open/curl)`;
      }
      return `- MCP server: ${m.label}`;
    })
    .join("\n");
  return t("chat.mentionHint", { list });
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

export type AskResourcesDom = {
  shellEl: HTMLElement;
  resourcesPanelEl: HTMLElement;
  resourcesToggleEl: HTMLButtonElement;
  resourcesLabelEl: HTMLElement;
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
    private getCwd: () => string,
    private shortCwdLabel: (cwd: string) => string,
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
    const cwd = this.getCwd();
    this.dom.resourcesLabelEl.textContent = t("chat.resourcesSummary", {
      cwd: this.shortCwdLabel(cwd),
      skills: String(this.mountedSkills.length),
      mcp: String(this.enabledMcps.length),
    });
    this.dom.resourcesLabelEl.title = cwd;
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

  renderResourceChips(): void {
    this.dom.skillsListEl.replaceChildren();
    this.dom.mcpListEl.replaceChildren();

    const q = this.resourcesQuery;
    const skills = this.mountedSkills.filter((skill) => {
      if (!q) return true;
      const hay = `${skill.name} ${skill.skill_id} ${skill.description ?? ""}`.toLowerCase();
      return hay.includes(q);
    });
    const mcps = this.enabledMcps.filter((server) => {
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
      this.dom.skillsEmptyEl.textContent = t("chat.resourcesNoMatch");
      this.dom.skillsEmptyEl.hidden = false;
    } else if (showSkills) {
      this.dom.skillsEmptyEl.textContent = t("chat.skillsEmpty");
    }
    if (showMcp && mcps.length === 0 && q) {
      this.dom.mcpEmptyEl.textContent = t("chat.resourcesNoMatch");
      this.dom.mcpEmptyEl.hidden = false;
    } else if (showMcp) {
      this.dom.mcpEmptyEl.textContent = t("chat.mcpEmpty");
    }

    for (const skill of skills) {
      const title = skill.name || skill.skill_id;
      const calls =
        skill.call_count != null ? t("chat.resourcesCalls", { count: String(skill.call_count) }) : "";
      const rate =
        skill.first_success_rate != null
          ? t("chat.resourcesRate", { rate: `${Math.round(skill.first_success_rate * 100)}%` })
          : "";
      const meta = [calls, rate].filter(Boolean).join(" · ") || skill.metrics_source || "local";
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
      const meta = server.healthy
        ? `${server.scope}`
        : server.issue || t("chat.resourcesUnhealthy");
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
  }): HTMLButtonElement {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "chat-res-row";
    btn.setAttribute("role", "listitem");
    btn.dataset.kind = opts.kind;
    btn.dataset.id = opts.id;
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

    const meta = document.createElement("span");
    meta.className = "chat-res-meta";
    meta.textContent = opts.meta;

    btn.title = opts.description;
    btn.append(icon, body, meta);
    btn.addEventListener("click", opts.onClick);
    return btn;
  }

  mentionCandidates(): MentionRef[] {
    const skills = this.mountedSkills.map((s) => ({
      kind: "skill" as const,
      id: s.skill_id,
      label: s.name || s.skill_id,
    }));
    const mcps = this.enabledMcps.map((s) => ({
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
        invoke<SkillsInventoryReport>("list_skills_inventory_command", { remoteStats: false }),
        invoke<McpInventoryReport>("list_mcp_inventory_command"),
      ]);
      workspaceCwd = mcpReport.workspace_path;
      try {
        const doc = await invoke<WorkspaceDoc>("list_workspaces_command");
        workspaceDoc = doc;
        opts.renderWorkspaceSwitcher(doc);
        if (doc.active && doc.workspaces[doc.active]?.path) {
          workspaceCwd = doc.workspaces[doc.active].path;
          opts.cwdEl.dataset.workspace = doc.active;
          opts.setDisplayedCwd(workspaceCwd);
          opts.cwdEl.title = `${t("ask.workspaceActive", { name: doc.active })} · ${workspaceCwd}`;
          opts.workspaceHintEl.textContent = t("ask.workspaceHint");
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
        if (m.kind === "skill") return this.mountedSkills.some((s) => s.skill_id === m.id);
        return this.enabledMcps.some((s) => s.name === m.id);
      });
      this.renderMentions();
      this.renderResourceChips();
    } catch (error) {
      opts.setStatus(t("chat.resourcesLoadFailed", { error: String(error) }), "warn");
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
  ) {}

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

    const match = before.match(/(?:^|\s)(@(?:skill:|mcp:)?([^\s@]*))$/i);
    if (!match || match.index == null) return null;
    const token = match[1];
    const q = match[2] || "";
    const start = match.index + (match[0].startsWith("@") ? 0 : 1);
    const end = caret;
    let kind: MentionKind | "any" = "any";
    const lower = token.toLowerCase();
    if (lower.startsWith("@skill:")) kind = "skill";
    else if (lower.startsWith("@mcp:")) kind = "mcp";
    return { kind, q, start, end, trigger: "at" };
  }

  filteredMentionOptions(): MentionRef[] {
    if (!this.mentionQuery) return [];
    const q = this.mentionQuery.q.toLowerCase();
    const kindFilter =
      this.mentionQuery.trigger === "slash" ? this.slashTab : this.mentionQuery.kind;
    return this.getCandidates().filter((item) => {
      if (kindFilter !== "any" && item.kind !== kindFilter) return false;
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
    const tabs: SlashTab[] = ["skill", "mcp"];
    const idx = tabs.indexOf(this.slashTab);
    const next = tabs[(idx + dir + tabs.length) % tabs.length]!;
    this.setSlashTab(next);
  }

  hideMentionMenu(): void {
    this.mentionQuery = null;
    this.dom.mentionMenuEl.hidden = true;
    this.dom.mentionMenuEl.replaceChildren();
    this.dom.mentionMenuEl.classList.remove("is-slash");
    this.onVisibilityChange?.(false);
  }

  isSlashMenuOpen(): boolean {
    return Boolean(this.mentionQuery?.trigger === "slash" && !this.dom.mentionMenuEl.hidden);
  }

  renderMentionMenu(): void {
    this.mentionQuery = this.detectMentionQuery();
    if (!this.mentionQuery) {
      this.hideMentionMenu();
      return;
    }

    const isSlash = this.mentionQuery.trigger === "slash";
    const options = this.filteredMentionOptions();

    // Keep `/` menu open even when empty so tabs/empty hint stay visible.
    if (!isSlash && options.length === 0) {
      this.hideMentionMenu();
      return;
    }

    this.mentionMenuIndex = Math.max(
      0,
      Math.min(this.mentionMenuIndex, Math.max(options.length - 1, 0)),
    );
    this.dom.mentionMenuEl.replaceChildren();
    this.dom.mentionMenuEl.classList.toggle("is-slash", isSlash);

    if (isSlash) {
      const head = document.createElement("div");
      head.className = "chat-slash-head";

      const tabs = document.createElement("div");
      tabs.className = "chat-slash-tabs";
      tabs.setAttribute("role", "tablist");

      for (const tab of [
        { id: "skill" as const, label: t("chat.slashTabSkills") },
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
    }

    const list = document.createElement("div");
    list.className = "chat-slash-list";
    list.setAttribute("role", "listbox");

    if (options.length === 0) {
      const empty = document.createElement("div");
      empty.className = "chat-slash-empty";
      empty.textContent =
        this.slashTab === "skill" ? t("chat.slashEmptySkills") : t("chat.slashEmptyTools");
      list.appendChild(empty);
    } else {
      options.forEach((option, index) => {
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = `chat-mention-option${index === this.mentionMenuIndex ? " is-active" : ""}`;
        btn.setAttribute("role", "option");
        const title = document.createElement("strong");
        title.textContent = option.label;
        const sub = document.createElement("span");
        sub.textContent =
          option.kind === "skill" ? t("chat.slashKindSkill") : t("chat.slashKindTool");
        btn.append(title, sub);
        btn.addEventListener("mousedown", (event) => {
          event.preventDefault();
          this.applyMentionOption(option);
        });
        list.appendChild(btn);
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
    const trigger = this.mentionQuery.trigger;

    if (trigger === "slash") {
      // Remove `/query` — selection lives in chips (beginner-friendly, no raw tokens).
      this.dom.promptEl.value = `${value.slice(0, this.mentionQuery.start)}${value.slice(this.mentionQuery.end)}`;
      const next = this.mentionQuery.start;
      this.dom.promptEl.setSelectionRange(next, next);
    } else {
      const token = `@${option.kind}:${option.id} `;
      this.dom.promptEl.value = `${value.slice(0, this.mentionQuery.start)}${token}${value.slice(this.mentionQuery.end)}`;
      const next = this.mentionQuery.start + token.length;
      this.dom.promptEl.setSelectionRange(next, next);
    }

    this.upsertMention(option);
    this.hideMentionMenu();
    this.autoResizePrompt();
    this.dom.promptEl.focus();
  }
}
