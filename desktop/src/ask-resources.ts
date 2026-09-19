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
};

const MENTION_TOKEN_RE = /@(?:skill|mcp):([^\s@]+)/gi;

export function mcpMatchesRuntime(server: McpInventoryItem, runtime: AskRuntime): boolean {
  const hint = server.runtime_hint.trim();
  return hint === runtime || hint === "shared" || hint === "";
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

export type AskResourcesDom = {
  shellEl: HTMLElement;
  resourcesPanelEl: HTMLElement;
  resourcesToggleEl: HTMLButtonElement;
  resourcesLabelEl: HTMLElement;
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

  updateResourcesSummary(): void {
    const cwd = this.getCwd();
    this.dom.resourcesLabelEl.textContent = t("chat.resourcesSummary", {
      cwd: this.shortCwdLabel(cwd),
      skills: String(this.mountedSkills.length),
      mcp: String(this.enabledMcps.length),
    });
    this.dom.resourcesLabelEl.title = cwd;
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
    this.dom.skillsEmptyEl.hidden = this.mountedSkills.length > 0;
    this.dom.mcpEmptyEl.hidden = this.enabledMcps.length > 0;

    for (const skill of this.mountedSkills) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "chat-res-chip";
      if (this.hasMention("skill", skill.skill_id)) btn.classList.add("is-active");
      btn.textContent = skill.name || skill.skill_id;
      btn.title = skill.description || skill.skill_id;
      btn.addEventListener("click", () =>
        this.toggleMention({
          kind: "skill",
          id: skill.skill_id,
          label: skill.name || skill.skill_id,
        }),
      );
      this.dom.skillsListEl.appendChild(btn);
    }

    for (const server of this.enabledMcps) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "chat-res-chip";
      if (!server.healthy) btn.classList.add("is-warn");
      if (this.hasMention("mcp", server.name)) btn.classList.add("is-active");
      btn.textContent = server.is_browser ? `${server.name} · browser` : server.name;
      btn.title = server.issue || `${server.scope} · ${server.runtime_hint}`;
      btn.addEventListener("click", () =>
        this.toggleMention({
          kind: "mcp",
          id: server.name,
          label: server.name,
        }),
      );
      this.dom.mcpListEl.appendChild(btn);
    }

    this.updateResourcesSummary();
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
      this.enabledMcps = (mcpReport.servers ?? []).filter((server) => mcpMatchesRuntime(server, runtime));
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

  constructor(
    private dom: AskMentionMenuDom,
    private getCandidates: () => MentionRef[],
    private upsertMention: (mention: MentionRef) => void,
    private autoResizePrompt: () => void,
  ) {}

  detectMentionQuery(): MentionQuery | null {
    const value = this.dom.promptEl.value;
    const caret = this.dom.promptEl.selectionStart ?? value.length;
    const before = value.slice(0, caret);
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
    return { kind, q, start, end };
  }

  filteredMentionOptions(): MentionRef[] {
    if (!this.mentionQuery) return [];
    const q = this.mentionQuery.q.toLowerCase();
    return this.getCandidates().filter((item) => {
      if (this.mentionQuery!.kind !== "any" && item.kind !== this.mentionQuery!.kind) return false;
      if (!q) return true;
      return item.id.toLowerCase().includes(q) || item.label.toLowerCase().includes(q);
    });
  }

  hideMentionMenu(): void {
    this.mentionQuery = null;
    this.dom.mentionMenuEl.hidden = true;
    this.dom.mentionMenuEl.replaceChildren();
  }

  renderMentionMenu(): void {
    this.mentionQuery = this.detectMentionQuery();
    const options = this.filteredMentionOptions();
    if (!this.mentionQuery || options.length === 0) {
      this.hideMentionMenu();
      return;
    }
    this.mentionMenuIndex = Math.max(0, Math.min(this.mentionMenuIndex, options.length - 1));
    this.dom.mentionMenuEl.replaceChildren();
    options.forEach((option, index) => {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "chat-mention-option";
      if (index === this.mentionMenuIndex) btn.classList.add("is-active");
      btn.setAttribute("role", "option");
      const title = document.createElement("strong");
      title.textContent =
        option.kind === "skill"
          ? t("chat.mentionSkill", { name: option.label })
          : t("chat.mentionMcp", { name: option.label });
      const sub = document.createElement("span");
      sub.textContent = `@${option.kind}:${option.id}`;
      btn.append(title, sub);
      btn.addEventListener("mousedown", (event) => {
        event.preventDefault();
        this.applyMentionOption(option);
      });
      this.dom.mentionMenuEl.appendChild(btn);
    });
    this.dom.mentionMenuEl.hidden = false;
  }

  applyMentionOption(option: MentionRef): void {
    if (!this.mentionQuery) return;
    const value = this.dom.promptEl.value;
    const token = `@${option.kind}:${option.id} `;
    this.dom.promptEl.value = `${value.slice(0, this.mentionQuery.start)}${token}${value.slice(this.mentionQuery.end)}`;
    const next = this.mentionQuery.start + token.length;
    this.dom.promptEl.setSelectionRange(next, next);
    this.upsertMention(option);
    this.hideMentionMenu();
    this.autoResizePrompt();
    this.dom.promptEl.focus();
  }
}
