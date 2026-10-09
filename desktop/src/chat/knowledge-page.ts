import { getLocale, t, tNamed } from "../i18n";
import { listAgentMemory } from "../ipc";
import { migrateLegacyNotes, mountWiki, wikiPromptSection, type WikiScope } from "./knowledge-wiki";
import { closeKnowledgePage, closeResourcesMainPage, isKnowledgePageOpen } from "./overlay-pages";
import { isAskRuntime, runtimeDisplayName } from "./runtime";

const STORAGE_KEY = "agent-doctor-ask-knowledge";

const AGENTS = ["codex", "claude-code", "hermes", "openclaw", "deepseek-harness"] as const;

type Tab = "global" | "project" | "agent";
type AgentKind = "docs" | "memories";
type Note = { title: string; body: string };
type Library = {
  global: Note[];
  projects: Record<string, Note[]>;
};

type Deps = {
  projects: () => string[];
  currentProject: () => string | null;
  currentAgent: () => string;
  projectPath: (projectName: string) => string | null;
  chatTranscript: () => { title: string; text: string } | null;
  chats: () => { title: string; text: string; projectName: string | null }[];
};

function asText(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

/** Notes saved before the wiki existed: `notes` plus the old separate `memories` list. */
function asNotes(value: unknown): Note[] {
  const raw = value && typeof value === "object" ? (value as { notes?: unknown; memories?: unknown }) : {};
  const notes: Note[] = [];
  for (const item of Array.isArray(raw.memories) ? raw.memories : []) {
    const body = asText((item as { text?: unknown })?.text);
    if (body) notes.push({ title: "", body });
  }
  for (const item of Array.isArray(raw.notes) ? raw.notes : []) {
    const row = (item ?? {}) as { title?: unknown; body?: unknown };
    const title = asText(row.title);
    const body = asText(row.body);
    if (title || body) notes.push({ title, body });
  }
  return notes;
}

function loadLibrary(): Library {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { global: [], projects: {} };
    const parsed = JSON.parse(raw) as { global?: unknown; projects?: unknown };
    const projects: Record<string, Note[]> = {};
    if (parsed.projects && typeof parsed.projects === "object") {
      for (const [name, bucket] of Object.entries(parsed.projects)) {
        const notes = asNotes(bucket);
        if (name.trim() && notes.length) projects[name] = notes;
      }
    }
    return { global: asNotes(parsed.global), projects };
  } catch {
    return { global: [], projects: {} };
  }
}

function saveLibrary(library: Library): void {
  try {
    if (!library.global.length && !Object.keys(library.projects).length) {
      localStorage.removeItem(STORAGE_KEY);
      return;
    }
    const projects = Object.fromEntries(
      Object.entries(library.projects).map(([name, notes]) => [name, { notes }]),
    );
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ global: { notes: library.global }, projects }));
  } catch {
    /* Notes stay in memory until the next try. */
  }
}

function agentLabel(runtime: string): string {
  return isAskRuntime(runtime) ? runtimeDisplayName(runtime) : runtime;
}

function legacyLines(notes: Note[] | undefined): string {
  return (notes ?? [])
    .map((note) => `- ${[note.title, note.body].filter(Boolean).join("：")}`)
    .join("\n");
}

async function scopeSection(label: string, projectPath: string | null, legacy: Note[] | undefined): Promise<string | null> {
  const wiki = await wikiPromptSection(label, projectPath);
  const notes = legacyLines(legacy);
  if (!wiki && !notes) return null;
  return [wiki ?? label, notes].filter(Boolean).join("\n");
}

export async function knowledgePromptBlock(project: string | null): Promise<string> {
  const library = loadLibrary();
  const zh = getLocale() !== "en";
  const name = project?.trim() || null;
  const projectPath = name ? deps.projectPath(name) : null;
  const sections = await Promise.all([
    scopeSection(zh ? "全局知识库（所有项目、所有助手都要遵守）：" : "Global knowledge (every project and every agent):", null, library.global),
    name && projectPath
      ? scopeSection(
          zh ? `项目「${name}」的知识库（只在这个项目遵守）：` : `Knowledge for project “${name}” (this project only):`,
          projectPath,
          library.projects[name],
        )
      : null,
  ]);
  const parts = sections.filter((part): part is string => Boolean(part));
  if (!parts.length) return "";
  const lead = zh
    ? "下面是知识库，回答时以它为准。"
    : "Follow this knowledge base when you answer.";
  return `${lead}\n\n${parts.join("\n\n")}`;
}

export async function appendKnowledgeToPrompt(prompt: string, project: string | null): Promise<string> {
  const block = await knowledgePromptBlock(project).catch(() => "");
  return block ? `${block}\n\n${prompt}` : prompt;
}

const shellEl = () => document.querySelector<HTMLElement>("#chat-shell");
const mainEl = () => document.querySelector<HTMLElement>("#chat-main");
const pageEl = () => document.querySelector<HTMLElement>("#chat-knowledge");
const openEl = () => document.querySelector<HTMLButtonElement>("#chat-knowledge-open");
const agentKindTabsEl = () => document.querySelector<HTMLElement>(".chat-knowledge-kind-tabs");

let deps: Deps = {
  projects: () => [],
  currentProject: () => null,
  currentAgent: () => "codex",
  projectPath: () => null,
  chatTranscript: () => null,
  chats: () => [],
};
let agentMemoryFetchId = 0;
let tab: Tab = "global";
let agentKind: AgentKind = "docs";
let projectPick = "";
let agentPick: string = AGENTS[0];
const migrating = new Set<string>();

export function isKnowledgeOpen(): boolean {
  return isKnowledgePageOpen();
}

export function closeKnowledge(): void {
  closeKnowledgePage();
}

function closeSettings(): void {
  shellEl()?.classList.remove("is-settings");
  mainEl()?.classList.remove("is-settings");
  const page = document.querySelector<HTMLElement>("#chat-settings");
  if (page) {
    page.hidden = true;
    page.setAttribute("aria-hidden", "true");
  }
  const button = document.querySelector<HTMLButtonElement>("#chat-settings-open");
  button?.classList.remove("is-on");
  button?.setAttribute("aria-pressed", "false");
}

function closeFiles(): void {
  const panel = document.querySelector<HTMLElement>("#chat-files-panel");
  if (panel && !panel.hidden) {
    document.querySelector<HTMLButtonElement>("#chat-files-close")?.click();
  }
}

function el(tag: string, className: string, text?: string): HTMLElement {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function paintPicks(): void {
  const host = document.querySelector<HTMLElement>("#chat-knowledge-picks");
  if (!host) return;
  host.replaceChildren();
  if (tab === "global") {
    host.hidden = true;
    return;
  }
  const items = tab === "project" ? deps.projects() : AGENTS.map((id) => id);
  if (tab === "project" && items.length === 0) {
    host.hidden = true;
    return;
  }
  host.hidden = false;
  for (const id of items) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "chat-knowledge-pick";
    button.textContent = tab === "agent" ? agentLabel(id) : id;
    const on = id === (tab === "project" ? projectPick : agentPick);
    button.setAttribute("aria-pressed", on ? "true" : "false");
    button.addEventListener("click", () => {
      if (tab === "project") projectPick = id;
      else agentPick = id;
      paint();
    });
    host.append(button);
  }
}

function isAgentMemorySnippet(label: string): boolean {
  const normalized = label.trim().toLowerCase();
  if (!normalized) return false;
  if (normalized.includes("memories/") || normalized.includes("memory/")) return true;
  if (normalized === "memory.md" || normalized.endsWith("/memory.md")) return true;
  if (normalized.includes("dsh memory.md")) return true;
  if (normalized.startsWith("claude memory")) return true;
  if (normalized.startsWith("hermes memories")) return true;
  if (normalized.startsWith("codex memories")) return true;
  if (normalized.startsWith("dsh memories")) return true;
  if (normalized.startsWith("openclaw memory")) return true;
  return false;
}

function fillAgentReadonlyList(
  list: HTMLUListElement,
  snippets: { label: string; body: string }[],
  emptyText: string,
): void {
  list.replaceChildren();
  if (!snippets.length) {
    list.append(el("li", "chat-knowledge-empty", emptyText));
    return;
  }
  for (const snippet of snippets) {
    const row = el("li", "chat-knowledge-note");
    row.append(el("strong", "chat-knowledge-note-title", snippet.label));
    row.append(el("p", "chat-knowledge-note-body", snippet.body));
    list.append(row);
  }
}

function agentReadonlyBlock(title: string, hint: string, kind: AgentKind): HTMLElement {
  const section = el("section", "chat-knowledge-block");
  section.dataset.knowledgeSection = kind;
  const head = el("div", "chat-knowledge-block-head");
  head.append(el("h3", "", title));
  section.append(head);
  section.append(el("p", "chat-knowledge-block-hint", hint));
  const list = document.createElement("ul");
  list.className = "chat-knowledge-list";
  fillAgentReadonlyList(list, [], t("chat.knowledgeAgentSyncLoading"));
  section.append(list);
  return section;
}

async function refreshAgentNativeMemory(
  runtime: string,
  workspaceName: string | null,
  docsBlock: HTMLElement,
  memBlock: HTMLElement,
): Promise<void> {
  const fetchId = ++agentMemoryFetchId;
  const docsList = docsBlock.querySelector<HTMLUListElement>(".chat-knowledge-list");
  const memList = memBlock.querySelector<HTMLUListElement>(".chat-knowledge-list");
  if (!docsList || !memList) return;

  let snippets: { label: string; body: string }[] = [];
  let emptyHint: string | null = null;
  try {
    const report = await listAgentMemory({
      runtime,
      workspaceName: workspaceName?.trim() || null,
    });
    snippets = report.snippets ?? [];
    emptyHint = report.emptyHint ?? null;
  } catch {
    emptyHint = getLocale() !== "en" ? "暂时读不到助手记忆，请确认是在桌面 App 里打开。" : "Could not load agent memory. Open this in the desktop app.";
  }
  if (fetchId !== agentMemoryFetchId || tab !== "agent" || agentPick !== runtime) return;
  fillAgentReadonlyList(
    docsList,
    snippets.filter((snippet) => !isAgentMemorySnippet(snippet.label)),
    t("chat.knowledgeEmptyDocs"),
  );
  fillAgentReadonlyList(
    memList,
    snippets.filter((snippet) => isAgentMemorySnippet(snippet.label)),
    emptyHint?.trim() || t("chat.knowledgeEmptyMem"),
  );
}

function activeWikiScope(): WikiScope | null {
  if (tab === "global") return { key: "global", projectPath: null, projectName: null };
  if (tab !== "project" || !projectPick) return null;
  const projectPath = deps.projectPath(projectPick);
  return projectPath ? { key: `project:${projectPick}`, projectPath, projectName: projectPick } : null;
}

function legacyNotesFor(scope: WikiScope, library: Library): Note[] {
  return scope.projectName ? library.projects[scope.projectName] ?? [] : library.global;
}

async function migrateScope(scope: WikiScope): Promise<void> {
  if (migrating.has(scope.key)) return;
  const notes = legacyNotesFor(scope, loadLibrary());
  if (!notes.length) return;
  migrating.add(scope.key);
  const moved = await migrateLegacyNotes(scope, notes);
  migrating.delete(scope.key);
  if (!moved) return;
  const library = loadLibrary();
  if (scope.projectName) delete library.projects[scope.projectName];
  else library.global = [];
  saveLibrary(library);
  if (isKnowledgeOpen()) paint();
}

function paintContent(): void {
  const host = document.querySelector<HTMLElement>("#chat-knowledge-content");
  const scope = document.querySelector<HTMLElement>("#chat-knowledge-scope");
  if (!host || !scope) return;
  host.replaceChildren();
  if (tab === "project" && deps.projects().length === 0) {
    scope.textContent = t("chat.knowledgeEmptyProjects");
    return;
  }
  scope.textContent =
    tab === "global"
      ? t("chat.knowledgeScopeGlobal")
      : tab === "project"
        ? tNamed("chat.knowledgeScopeProject", projectPick)
        : tNamed("chat.knowledgeScopeAgent", agentLabel(agentPick));
  if (tab === "agent") {
    const docsBlock = agentReadonlyBlock(t("chat.knowledgeDocs"), t("chat.knowledgeAgentDocsHint"), "docs");
    const memBlock = agentReadonlyBlock(t("chat.knowledgeMemories"), t("chat.knowledgeAgentMemHint"), "memories");
    host.append(docsBlock, memBlock);
    const workspaceName = deps.currentProject() ?? projectPick ?? null;
    void refreshAgentNativeMemory(agentPick, workspaceName, docsBlock, memBlock);
    return;
  }
  const wikiScope = activeWikiScope();
  if (!wikiScope) {
    host.append(el("p", "chat-knowledge-empty", t("chat.wikiProjectMissing")));
    return;
  }
  const wikiHost = el("div", "chat-knowledge-wiki");
  host.append(wikiHost);
  mountWiki(wikiHost, wikiScope, {
    currentAgent: () => deps.currentAgent(),
    chatTranscript: () => deps.chatTranscript(),
    chats: () => deps.chats(),
  });
  void migrateScope(wikiScope);
}

function paintAgentKind(): void {
  const tabs = agentKindTabsEl();
  if (tabs) tabs.hidden = tab !== "agent";
  document.querySelectorAll<HTMLButtonElement>("[data-knowledge-kind]").forEach((button) => {
    const on = button.dataset.knowledgeKind === agentKind;
    button.classList.toggle("is-on", on);
    button.setAttribute("aria-selected", on ? "true" : "false");
    button.tabIndex = on ? 0 : -1;
  });
  document.querySelectorAll<HTMLElement>("[data-knowledge-section]").forEach((section) => {
    section.hidden = section.dataset.knowledgeSection !== agentKind;
  });
}

function paint(): void {
  document.querySelectorAll<HTMLButtonElement>("[data-kb-tab]").forEach((button) => {
    const on = button.dataset.kbTab === tab;
    button.setAttribute("aria-selected", on ? "true" : "false");
    button.classList.toggle("is-on", on);
  });
  const projects = deps.projects();
  if (tab === "project" && (!projectPick || !projects.includes(projectPick))) {
    const current = deps.currentProject();
    projectPick = current && projects.includes(current) ? current : projects[0] ?? "";
  }
  if (!AGENTS.includes(agentPick as (typeof AGENTS)[number])) agentPick = deps.currentAgent();
  if (!AGENTS.includes(agentPick as (typeof AGENTS)[number])) agentPick = AGENTS[0];
  paintPicks();
  paintContent();
  paintAgentKind();
}

export function syncKnowledgeLabels(): void {
  const button = openEl();
  if (button) {
    button.title = t("chat.knowledge");
    button.setAttribute("aria-label", t("chat.knowledge"));
  }
  if (isKnowledgeOpen()) paint();
}

function openKnowledge(): void {
  const current = deps.currentAgent();
  if (AGENTS.includes(current as (typeof AGENTS)[number])) agentPick = current;
  const project = deps.currentProject();
  if (project) projectPick = project;
  closeSettings();
  closeFiles();
  closeResourcesMainPage();
  shellEl()?.classList.add("is-knowledge");
  mainEl()?.classList.add("is-knowledge");
  const page = pageEl();
  if (page) {
    page.hidden = false;
    page.setAttribute("aria-hidden", "false");
  }
  openEl()?.classList.add("is-on");
  openEl()?.setAttribute("aria-pressed", "true");
  paint();
}

export function bindKnowledge(next: Deps): void {
  deps = next;
  openEl()?.addEventListener("click", () => {
    if (isKnowledgeOpen()) closeKnowledge();
    else openKnowledge();
  });
  document.querySelector(".chat-knowledge-tabs")?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-kb-tab]");
    const nextTab = button?.dataset.kbTab;
    if (nextTab !== "global" && nextTab !== "project" && nextTab !== "agent") return;
    tab = nextTab;
    paint();
  });
  const kindTabs = agentKindTabsEl();
  kindTabs?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-knowledge-kind]");
    const nextKind = button?.dataset.knowledgeKind;
    if (nextKind !== "docs" && nextKind !== "memories") return;
    agentKind = nextKind;
    paintAgentKind();
  });
  kindTabs?.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    agentKind = agentKind === "docs" ? "memories" : "docs";
    paintAgentKind();
    kindTabs.querySelector<HTMLButtonElement>(`[data-knowledge-kind="${agentKind}"]`)?.focus();
  });
  syncKnowledgeLabels();
}
