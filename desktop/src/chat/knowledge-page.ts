import { getLocale, t, tNamed } from "../i18n";
import { listAgentMemory, listWorkspaceDir, readWorkspaceFile } from "../ipc";
import { isAskRuntime, runtimeDisplayName } from "./runtime";

const STORAGE_KEY = "agent-doctor-ask-knowledge";

const AGENTS = ["codex", "claude-code", "hermes", "openclaw", "deepseek-harness"] as const;

type Tab = "global" | "project" | "agent";
type Note = { id: string; title: string; body: string };
type Memory = { id: string; text: string };
type Bucket = { notes: Note[]; memories: Memory[] };
type Library = {
  global: Bucket;
  projects: Record<string, Bucket>;
};

type Deps = {
  projects: () => string[];
  currentProject: () => string | null;
  currentAgent: () => string;
  projectPath: (projectName: string) => string | null;
  lastUserPrompt: () => string | null;
};

const DEFAULT_MEMORY_MARKERS = [
  "# Project memory (OpenClaw workspace)",
  "Managed by Agent Doctor workspace.",
];

const emptyBucket = (): Bucket => ({ notes: [], memories: [] });

function emptyLibrary(): Library {
  return { global: emptyBucket(), projects: {} };
}

function asText(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function asBucket(value: unknown): Bucket {
  const raw = value && typeof value === "object" ? (value as { notes?: unknown; memories?: unknown }) : {};
  const notes = Array.isArray(raw.notes)
    ? raw.notes
        .map((item) => {
          const row = item && typeof item === "object" ? (item as Note) : null;
          if (!row) return null;
          const title = asText(row.title).trim();
          const body = asText(row.body).trim();
          const id = asText(row.id).trim();
          if (!id || (!title && !body)) return null;
          return { id, title, body };
        })
        .filter((item): item is Note => Boolean(item))
    : [];
  const memories = Array.isArray(raw.memories)
    ? raw.memories
        .map((item) => {
          const row = item && typeof item === "object" ? (item as Memory) : null;
          if (!row) return null;
          const text = asText(row.text).trim();
          const id = asText(row.id).trim();
          if (!id || !text) return null;
          return { id, text };
        })
        .filter((item): item is Memory => Boolean(item))
    : [];
  return { notes, memories };
}

function loadLibrary(): Library {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return emptyLibrary();
    const parsed = JSON.parse(raw) as { global?: unknown; projects?: unknown };
    const projects: Record<string, Bucket> = {};
    if (parsed.projects && typeof parsed.projects === "object") {
      for (const [name, bucket] of Object.entries(parsed.projects)) {
        if (name.trim()) projects[name] = asBucket(bucket);
      }
    }
    return { global: asBucket(parsed.global), projects };
  } catch {
    return emptyLibrary();
  }
}

function saveLibrary(library: Library): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(library));
  } catch {
    /* The page still shows what was just written. */
  }
}

function newId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function agentLabel(runtime: string): string {
  return isAskRuntime(runtime) ? runtimeDisplayName(runtime) : runtime;
}

function clip(text: string, max: number): string {
  const trimmed = text.trim();
  return trimmed.length > max ? `${trimmed.slice(0, max)}…` : trimmed;
}

function bucketLines(bucket: Bucket | undefined): string[] {
  if (!bucket) return [];
  const lines: string[] = [];
  for (const memory of bucket.memories.slice(0, 12)) {
    const text = clip(memory.text, 400);
    if (text) lines.push(text);
  }
  for (const note of bucket.notes.slice(0, 6)) {
    const title = clip(note.title, 80);
    const body = clip(note.body, 800);
    const text = [title, body].filter(Boolean).join("：");
    if (text) lines.push(text);
  }
  return lines;
}

export function knowledgePromptBlock(project: string | null, _runtime: string): string {
  const library = loadLibrary();
  const zh = getLocale() !== "en";
  const parts: string[] = [];
  const push = (title: string, lines: string[]) => {
    if (!lines.length) return;
    parts.push(`${title}\n${lines.map((line) => `- ${line}`).join("\n")}`);
  };
  push(zh ? "全局（所有项目、所有助手都要遵守）" : "Global (every project and every agent)", bucketLines(library.global));
  if (project?.trim()) {
    push(
      zh ? `项目「${project.trim()}」（只在这个项目遵守）` : `Project “${project.trim()}” (this project only)`,
      bucketLines(library.projects[project.trim()]),
    );
  }
  if (!parts.length) return "";
  const lead = zh
    ? "下面是知识库，回答时要遵守。"
    : "Follow this knowledge base when you answer.";
  return `${lead}\n\n${parts.join("\n\n")}`;
}

export function appendKnowledgeToPrompt(prompt: string, project: string | null, runtime: string): string {
  const block = knowledgePromptBlock(project, runtime);
  return block ? `${block}\n\n${prompt}` : prompt;
}

const shellEl = () => document.querySelector<HTMLElement>("#chat-shell");
const mainEl = () => document.querySelector<HTMLElement>("#chat-main");
const pageEl = () => document.querySelector<HTMLElement>("#chat-knowledge");
const openEl = () => document.querySelector<HTMLButtonElement>("#chat-knowledge-open");

let deps: Deps = {
  projects: () => [],
  currentProject: () => null,
  currentAgent: () => "codex",
  projectPath: () => null,
  lastUserPrompt: () => null,
};
let workspaceLoadGen = 0;
let agentMemoryFetchId = 0;
let tab: Tab = "global";
let projectPick = "";
let agentPick: string = AGENTS[0];
let library = emptyLibrary();

type Draft =
  | { kind: "note"; id: string | null; title: string; body: string }
  | { kind: "memory"; id: string | null; text: string };
let draft: Draft | null = null;
let confirmDelete: { kind: "note" | "memory"; id: string } | null = null;

export function isKnowledgeOpen(): boolean {
  return mainEl()?.classList.contains("is-knowledge") ?? false;
}

export function closeKnowledge(): void {
  shellEl()?.classList.remove("is-knowledge");
  mainEl()?.classList.remove("is-knowledge");
  const page = pageEl();
  if (page) {
    page.hidden = true;
    page.setAttribute("aria-hidden", "true");
  }
  openEl()?.classList.remove("is-on");
  openEl()?.setAttribute("aria-pressed", "false");
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

function bucketFor(scope: Tab, id: string): Bucket {
  if (scope === "global") return library.global;
  library.projects[id] ??= emptyBucket();
  return library.projects[id];
}

function activeId(): string | null {
  if (tab === "global") return "global";
  if (tab === "project") return projectPick || null;
  return agentPick || null;
}

function needsConfirm(): boolean {
  return tab === "global" || tab === "project";
}

function el(tag: string, className: string, text?: string): HTMLElement {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function paintInherit(lines: string[]): void {
  const host = document.querySelector<HTMLElement>("#chat-knowledge-inherit");
  if (!host) return;
  host.replaceChildren();
  if (!lines.length || tab === "global") {
    host.hidden = true;
    return;
  }
  host.hidden = false;
  const block = el("div", "chat-knowledge-from");
  block.append(el("p", "chat-knowledge-from-label", t("chat.knowledgeFromGlobal")));
  const list = document.createElement("ul");
  list.className = "chat-knowledge-list";
  for (const line of lines) list.append(el("li", "", line));
  block.append(list);
  host.append(block);
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
  for (const item of items) {
    const id = item;
    const label = tab === "agent" ? agentLabel(id) : id;
    const button = document.createElement("button");
    button.type = "button";
    button.className = "chat-knowledge-pick";
    button.textContent = label;
    const on = id === (tab === "project" ? projectPick : agentPick);
    button.setAttribute("aria-pressed", on ? "true" : "false");
    button.addEventListener("click", () => {
      draft = null;
      confirmDelete = null;
      if (tab === "project") projectPick = id;
      else agentPick = id;
      paint();
    });
    host.append(button);
  }
}

function memoryRow(memory: Memory, editable: boolean): HTMLElement {
  const row = el("li", "chat-knowledge-row");
  if (draft?.kind === "memory" && draft.id === memory.id) {
    const input = document.createElement("input");
    input.className = "chat-knowledge-input";
    input.value = draft.text;
    input.placeholder = t("chat.knowledgeMemPh");
    input.addEventListener("input", () => {
      if (draft?.kind === "memory") draft.text = input.value;
    });
    row.append(input);
    if (!needsConfirm()) row.append(saveButton(), cancelButton());
    return row;
  }
  row.append(el("span", "chat-knowledge-text", memory.text));
  if (editable) {
    const edit = document.createElement("button");
    edit.type = "button";
    edit.className = "chat-knowledge-textbtn";
    edit.textContent = t("chat.knowledgeEdit");
    edit.addEventListener("click", () => {
      draft = { kind: "memory", id: memory.id, text: memory.text };
      confirmDelete = null;
      paint();
    });
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "chat-knowledge-textbtn";
    remove.textContent = t("chat.knowledgeDelete");
    remove.addEventListener("click", () => requestDelete("memory", memory.id));
    row.append(edit, remove);
  }
  return row;
}

function noteBlock(note: Note, editable: boolean): HTMLElement {
  const row = el("li", "chat-knowledge-note");
  if (draft?.kind === "note" && draft.id === note.id) {
    row.append(noteFields(draft.title, draft.body));
    if (!needsConfirm()) row.append(actionRow());
    return row;
  }
  if (note.title) row.append(el("strong", "chat-knowledge-note-title", note.title));
  if (note.body) row.append(el("p", "chat-knowledge-note-body", note.body));
  if (editable) {
    const actions = el("div", "chat-knowledge-row-actions");
    const edit = document.createElement("button");
    edit.type = "button";
    edit.className = "chat-knowledge-textbtn";
    edit.textContent = t("chat.knowledgeEdit");
    edit.addEventListener("click", () => {
      draft = { kind: "note", id: note.id, title: note.title, body: note.body };
      confirmDelete = null;
      paint();
    });
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "chat-knowledge-textbtn";
    remove.textContent = t("chat.knowledgeDelete");
    remove.addEventListener("click", () => requestDelete("note", note.id));
    actions.append(edit, remove);
    row.append(actions);
  }
  return row;
}

function noteFields(title: string, body: string): HTMLElement {
  const wrap = el("div", "chat-knowledge-fields");
  const titleInput = document.createElement("input");
  titleInput.className = "chat-knowledge-input";
  titleInput.value = title;
  titleInput.placeholder = t("chat.knowledgeTitlePh");
  titleInput.addEventListener("input", () => {
    if (draft?.kind === "note") draft.title = titleInput.value;
  });
  const bodyInput = document.createElement("textarea");
  bodyInput.className = "chat-knowledge-area";
  bodyInput.value = body;
  bodyInput.placeholder = t("chat.knowledgeBodyPh");
  bodyInput.addEventListener("input", () => {
    if (draft?.kind === "note") draft.body = bodyInput.value;
  });
  wrap.append(titleInput, bodyInput);
  return wrap;
}

function saveButton(): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "chat-knowledge-save";
  button.textContent = t("chat.knowledgeSave");
  button.addEventListener("click", () => commitDraft());
  return button;
}

function cancelButton(): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "chat-knowledge-textbtn";
  button.textContent = t("chat.knowledgeCancel");
  button.addEventListener("click", () => {
    draft = null;
    confirmDelete = null;
    paint();
  });
  return button;
}

function actionRow(): HTMLElement {
  const row = el("div", "chat-knowledge-row-actions");
  row.append(saveButton(), cancelButton());
  return row;
}

function requestDelete(kind: "note" | "memory", id: string): void {
  draft = null;
  if (!needsConfirm()) {
    removeItem(kind, id);
    return;
  }
  confirmDelete = { kind, id };
  paint();
}

function removeItem(kind: "note" | "memory", id: string): void {
  const scopeId = activeId();
  if (!scopeId) return;
  const bucket = bucketFor(tab, scopeId);
  if (kind === "note") bucket.notes = bucket.notes.filter((item) => item.id !== id);
  else bucket.memories = bucket.memories.filter((item) => item.id !== id);
  confirmDelete = null;
  saveLibrary(library);
  paint();
}

function pushMemoryText(text: string): void {
  const trimmed = text.trim();
  if (!trimmed) return;
  const scopeId = activeId();
  if (!scopeId) return;
  const bucket = bucketFor(tab, scopeId);
  bucket.memories.unshift({ id: newId(), text: trimmed });
  saveLibrary(library);
  paint();
}

function isUsefulWorkspaceMemory(raw: string): boolean {
  const lines = raw
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  const meaningful = lines.filter((line) => !DEFAULT_MEMORY_MARKERS.some((mark) => line.includes(mark)));
  return meaningful.join("").length > 0;
}

function clearSyncHost(): void {
  const host = document.querySelector<HTMLElement>("#chat-knowledge-workspace");
  if (!host) return;
  host.hidden = true;
  host.replaceChildren();
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

function splitAgentSnippets(snippets: { label: string; body: string }[]): {
  docs: { label: string; body: string }[];
  memories: { label: string; body: string }[];
} {
  const docs: { label: string; body: string }[] = [];
  const memories: { label: string; body: string }[] = [];
  for (const snippet of snippets) {
    if (isAgentMemorySnippet(snippet.label)) memories.push(snippet);
    else docs.push(snippet);
  }
  return { docs, memories };
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

function agentReadonlyBlock(
  title: string,
  hint: string,
  snippets: { label: string; body: string }[],
  emptyText: string,
): HTMLElement {
  const section = el("section", "chat-knowledge-block");
  const head = el("div", "chat-knowledge-block-head");
  head.append(el("h3", "", title));
  section.append(head);
  section.append(el("p", "chat-knowledge-block-hint", hint));
  const list = document.createElement("ul");
  list.className = "chat-knowledge-list";
  fillAgentReadonlyList(list, snippets, emptyText);
  section.append(list);
  return section;
}

function paintReadonlySyncBlock(
  host: HTMLElement,
  title: string,
  hint: string,
  snippets: { label: string; body: string }[],
  emptyNote?: string | null,
): void {
  host.replaceChildren();
  host.hidden = false;
  const section = el("section", "chat-knowledge-block chat-knowledge-workspace");
  const head = el("div", "chat-knowledge-block-head");
  head.append(el("h3", "", title));
  section.append(head);
  section.append(el("p", "chat-knowledge-workspace-hint", hint));
  const list = document.createElement("ul");
  list.className = "chat-knowledge-list";
  if (!snippets.length) {
    list.append(el("li", "chat-knowledge-empty", emptyNote?.trim() || t("chat.knowledgeEmptyMem")));
  } else {
    for (const snippet of snippets) {
      const row = el("li", "chat-knowledge-note");
      row.append(el("strong", "chat-knowledge-note-title", snippet.label));
      row.append(el("p", "chat-knowledge-note-body", snippet.body));
      list.append(row);
    }
  }
  section.append(list);
  host.append(section);
}

async function refreshWorkspaceMemory(root: string | null): Promise<void> {
  const host = document.querySelector<HTMLElement>("#chat-knowledge-workspace");
  if (!host) return;
  const gen = ++workspaceLoadGen;
  if (!root?.trim()) {
    host.hidden = true;
    host.replaceChildren();
    return;
  }
  host.hidden = false;
  host.replaceChildren(el("p", "chat-knowledge-workspace-hint", t("chat.knowledgeWorkspaceLoading")));
  const snippets: { label: string; body: string }[] = [];
  const rootPath = root.trim();
  try {
    const memory = await readWorkspaceFile({ root: rootPath, relative: "MEMORY.md" });
    if (isUsefulWorkspaceMemory(memory.content)) {
      snippets.push({ label: "MEMORY.md", body: memory.content.trim() });
    }
  } catch {
    /* no file */
  }
  try {
    const entries = await listWorkspaceDir({ root: rootPath, relative: "memory" });
    const files = entries
      .filter((entry) => !entry.isDir && entry.name.toLowerCase().endsWith(".md"))
      .sort((a, b) => b.name.localeCompare(a.name))
      .slice(0, 8);
    for (const file of files) {
      try {
        const payload = await readWorkspaceFile({ root: rootPath, relative: file.relativePath });
        if (isUsefulWorkspaceMemory(payload.content)) {
          snippets.push({ label: file.relativePath, body: payload.content.trim() });
        }
      } catch {
        /* skip */
      }
    }
  } catch {
    /* no memory dir */
  }
  if (gen !== workspaceLoadGen) return;
  if (!snippets.length) {
    host.hidden = true;
    host.replaceChildren();
    return;
  }
  paintReadonlySyncBlock(host, t("chat.knowledgeWorkspaceTitle"), t("chat.knowledgeWorkspaceHint"), snippets);
}

async function refreshAgentNativeMemory(
  runtime: string,
  workspaceName: string | null,
  docsBlock: HTMLElement,
  memBlock: HTMLElement,
): Promise<void> {
  const fetchId = ++agentMemoryFetchId;
  const runtimeAtStart = runtime;
  clearSyncHost();
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
  if (fetchId !== agentMemoryFetchId || tab !== "agent" || agentPick !== runtimeAtStart) return;
  const { docs, memories } = splitAgentSnippets(snippets);
  const memEmpty = emptyHint?.trim() || t("chat.knowledgeEmptyMem");
  fillAgentReadonlyList(docsList, docs, t("chat.knowledgeEmptyDocs"));
  fillAgentReadonlyList(memList, memories, memEmpty);
}

function paintQuickBar(): void {
  const host = document.querySelector<HTMLElement>("#chat-knowledge-quick");
  if (!host) return;
  if (tab === "agent") {
    host.hidden = true;
    host.replaceChildren();
    return;
  }
  const scopeId = activeId();
  if (!scopeId) {
    host.hidden = true;
    host.replaceChildren();
    return;
  }
  host.hidden = false;
  host.replaceChildren();
  const area = document.createElement("textarea");
  area.className = "chat-knowledge-area chat-knowledge-quick-area";
  area.placeholder = t("chat.knowledgeQuickPh");
  area.rows = 2;
  const actions = el("div", "chat-knowledge-quick-actions");
  const fromChat = document.createElement("button");
  fromChat.type = "button";
  fromChat.className = "chat-knowledge-textbtn";
  fromChat.textContent = t("chat.knowledgeQuickFromChat");
  fromChat.addEventListener("click", () => {
    const last = deps.lastUserPrompt()?.trim();
    if (last) area.value = last;
    area.focus();
  });
  const save = document.createElement("button");
  save.type = "button";
  save.className = "chat-knowledge-save";
  save.textContent = t("chat.knowledgeQuickSave");
  save.addEventListener("click", () => {
    pushMemoryText(area.value);
    area.value = "";
  });
  actions.append(fromChat, save);
  host.append(area, actions);
}

function commitDraft(): void {
  if (!draft) return;
  const scopeId = activeId();
  if (!scopeId) return;
  const bucket = bucketFor(tab, scopeId);
  if (draft.kind === "memory") {
    const text = draft.text.trim();
    if (!text) return;
    if (draft.id) {
      const id = draft.id;
      const found = bucket.memories.find((item) => item.id === id);
      if (found) found.text = text;
    } else {
      bucket.memories.unshift({ id: newId(), text });
    }
  } else {
    const title = draft.title.trim();
    const body = draft.body.trim();
    if (!title && !body) return;
    if (draft.id) {
      const id = draft.id;
      const found = bucket.notes.find((item) => item.id === id);
      if (found) {
        found.title = title;
        found.body = body;
      }
    } else {
      bucket.notes.unshift({ id: newId(), title, body });
    }
  }
  draft = null;
  saveLibrary(library);
  paint();
}

function confirmBar(message: string, yesLabel: string, onYes: () => void): HTMLElement {
  const bar = el("div", "chat-knowledge-confirm");
  bar.append(el("p", "", message));
  const yes = document.createElement("button");
  yes.type = "button";
  yes.className = "chat-knowledge-save";
  yes.textContent = yesLabel;
  yes.addEventListener("click", onYes);
  bar.append(yes, cancelButton());
  return bar;
}

function paintContent(): void {
  const host = document.querySelector<HTMLElement>("#chat-knowledge-content");
  const scope = document.querySelector<HTMLElement>("#chat-knowledge-scope");
  if (!host || !scope) return;
  host.replaceChildren();
  const projects = deps.projects();
  if (tab === "project" && projects.length === 0) {
    scope.textContent = t("chat.knowledgeEmptyProjects");
    paintInherit([]);
    clearSyncHost();
    return;
  }
  const scopeId = activeId();
  if (!scopeId) return;
  if (tab === "project" && !projects.includes(projectPick)) projectPick = projects[0] ?? "";
  const name =
    tab === "agent" ? agentLabel(agentPick) : tab === "project" ? projectPick : "";
  scope.textContent =
    tab === "global"
      ? t("chat.knowledgeScopeGlobal")
      : tab === "project"
        ? tNamed("chat.knowledgeScopeProject", name)
        : tNamed("chat.knowledgeScopeAgent", name);
  paintInherit(tab === "agent" ? [] : bucketLines(library.global));
  paintQuickBar();
  if (tab === "agent") {
    draft = null;
    confirmDelete = null;
    clearSyncHost();
    const loadingText = t("chat.knowledgeAgentSyncLoading");
    const docsBlock = agentReadonlyBlock(
      t("chat.knowledgeDocs"),
      t("chat.knowledgeAgentDocsHint"),
      [],
      loadingText,
    );
    const memBlock = agentReadonlyBlock(
      t("chat.knowledgeMemories"),
      t("chat.knowledgeAgentMemHint"),
      [],
      loadingText,
    );
    host.append(docsBlock, memBlock);
    const workspaceName = deps.currentProject() ?? projectPick ?? null;
    void refreshAgentNativeMemory(agentPick, workspaceName, docsBlock, memBlock);
  } else {
    const bucket = bucketFor(tab, scopeId);
    host.append(sectionNotes(bucket), sectionMemories(bucket));
  }
  if (tab === "project") {
    const root = projectPick ? deps.projectPath(projectPick) : null;
    void refreshWorkspaceMemory(root);
  } else {
    clearSyncHost();
  }
  if (confirmDelete) {
    const message = tab === "global" ? t("chat.knowledgeDeleteGlobal") : t("chat.knowledgeDeleteProject");
    host.append(
      confirmBar(message, t("chat.knowledgeDelete"), () => removeItem(confirmDelete!.kind, confirmDelete!.id)),
    );
  }
}

function sectionNotes(bucket: Bucket): HTMLElement {
  const section = el("section", "chat-knowledge-block");
  const head = el("div", "chat-knowledge-block-head");
  head.append(el("h3", "", t("chat.knowledgeDocs")));
  const add = document.createElement("button");
  add.type = "button";
  add.className = "chat-knowledge-add";
  add.textContent = t("chat.knowledgeAdd");
  add.addEventListener("click", () => {
    draft = { kind: "note", id: null, title: "", body: "" };
    confirmDelete = null;
    paint();
  });
  head.append(add);
  section.append(head);
  const list = document.createElement("ul");
  list.className = "chat-knowledge-list";
  if (draft?.kind === "note" && draft.id === null) {
    const row = el("li", "chat-knowledge-note");
    row.append(noteFields(draft.title, draft.body));
    if (needsConfirm()) {
      row.append(confirmBar(tab === "global" ? t("chat.knowledgeConfirmGlobal") : t("chat.knowledgeConfirmProject"), t("chat.knowledgeSave"), commitDraft));
    } else {
      row.append(actionRow());
    }
    list.append(row);
  }
  if (bucket.notes.length === 0 && !(draft?.kind === "note" && draft.id === null)) {
    list.append(el("li", "chat-knowledge-empty", t("chat.knowledgeEmptyDocs")));
  }
  for (const note of bucket.notes) {
    const row = noteBlock(note, true);
    if (draft?.kind === "note" && draft.id === note.id && needsConfirm()) {
      row.append(confirmBar(tab === "global" ? t("chat.knowledgeConfirmGlobal") : t("chat.knowledgeConfirmProject"), t("chat.knowledgeSave"), commitDraft));
    }
    list.append(row);
  }
  section.append(list);
  return section;
}

function sectionMemories(bucket: Bucket): HTMLElement {
  const section = el("section", "chat-knowledge-block");
  const head = el("div", "chat-knowledge-block-head");
  head.append(el("h3", "", t("chat.knowledgeMemories")));
  const add = document.createElement("button");
  add.type = "button";
  add.className = "chat-knowledge-add";
  add.textContent = t("chat.knowledgeAdd");
  add.addEventListener("click", () => {
    draft = { kind: "memory", id: null, text: "" };
    confirmDelete = null;
    paint();
  });
  head.append(add);
  section.append(head);
  const list = document.createElement("ul");
  list.className = "chat-knowledge-list";
  if (draft?.kind === "memory" && draft.id === null) {
    const row = el("li", "chat-knowledge-row");
    const input = document.createElement("input");
    input.className = "chat-knowledge-input";
    input.value = draft.text;
    input.placeholder = t("chat.knowledgeMemPh");
    input.addEventListener("input", () => {
      if (draft?.kind === "memory") draft.text = input.value;
    });
    row.append(input);
    if (needsConfirm()) {
      row.append(confirmBar(tab === "global" ? t("chat.knowledgeConfirmGlobal") : t("chat.knowledgeConfirmProject"), t("chat.knowledgeSave"), commitDraft));
    } else {
      row.append(saveButton(), cancelButton());
    }
    list.append(row);
  }
  if (bucket.memories.length === 0 && !(draft?.kind === "memory" && draft.id === null)) {
    list.append(el("li", "chat-knowledge-empty", t("chat.knowledgeEmptyMem")));
  }
  for (const memory of bucket.memories) {
    const row = memoryRow(memory, true);
    if (draft?.kind === "memory" && draft.id === memory.id && needsConfirm()) {
      row.append(confirmBar(tab === "global" ? t("chat.knowledgeConfirmGlobal") : t("chat.knowledgeConfirmProject"), t("chat.knowledgeSave"), commitDraft));
    }
    list.append(row);
  }
  section.append(list);
  return section;
}

function paint(): void {
  document.querySelectorAll<HTMLButtonElement>("[data-kb-tab]").forEach((button) => {
    const on = button.dataset.kbTab === tab;
    button.setAttribute("aria-selected", on ? "true" : "false");
    button.classList.toggle("is-on", on);
  });
  const projects = deps.projects();
  if (tab === "project") {
    if (!projectPick || !projects.includes(projectPick)) {
      projectPick = deps.currentProject() && projects.includes(deps.currentProject()!) ? deps.currentProject()! : projects[0] ?? "";
    }
  }
  if (!AGENTS.includes(agentPick as (typeof AGENTS)[number])) agentPick = deps.currentAgent();
  if (!AGENTS.includes(agentPick as (typeof AGENTS)[number])) agentPick = AGENTS[0];
  paintPicks();
  paintContent();
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
  library = loadLibrary();
  draft = null;
  confirmDelete = null;
  const current = deps.currentAgent();
  if (AGENTS.includes(current as (typeof AGENTS)[number])) agentPick = current;
  const project = deps.currentProject();
  if (project) projectPick = project;
  closeSettings();
  closeFiles();
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
  library = loadLibrary();
  openEl()?.addEventListener("click", () => {
    if (isKnowledgeOpen()) closeKnowledge();
    else openKnowledge();
  });
  document.querySelector(".chat-knowledge-tabs")?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-kb-tab]");
    const nextTab = button?.dataset.kbTab;
    if (nextTab !== "global" && nextTab !== "project" && nextTab !== "agent") return;
    tab = nextTab;
    draft = null;
    confirmDelete = null;
    paint();
  });
  syncKnowledgeLabels();
}
