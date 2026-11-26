import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { open } from "@tauri-apps/plugin-dialog";
import { getLocale, t } from "../i18n";
import {
  cancelPromptSession,
  knowledgeAddSource,
  knowledgeContext,
  knowledgeImport,
  knowledgeClear,
  knowledgeDeletePage,
  knowledgePages,
  knowledgePrepare,
  knowledgeReadPage,
  knowledgeSavePage,
  knowledgeSaveOrder,
  listWorkspaceDir,
  resolvePermissionSession,
  startPromptSession,
  type KnowledgePage,
} from "../ipc";
import { renderMarkdown } from "../markdown";
import { isAskRuntime, runtimeDisplayName } from "./runtime";
import {
  htmlToMarkdown,
  joinWikiPage,
  nextNotePath,
  plainLead,
  splitWikiPage,
  upsertIndexLink,
  WIKI_PAGE_MAX_BYTES,
} from "./wiki-edit";
import type { PromptSessionEvent } from "./types";
import { syncKnowledgeFolder } from "./knowledge-sync";
import { wikiRunProgress } from "./wiki-progress";
import { attachPointerReorder } from "./skill-drag";
import { reorderPagePaths, type PageDropPosition } from "./knowledge-order";

/** Chat ignores prompt-session events whose client run id starts with this. */
export const KNOWLEDGE_RUN_PREFIX = "knowledge:";

export type WikiScope = {
  key: string;
  projectPath: string | null;
  projectName: string | null;
};

export type WikiChat = { title: string; text: string; projectName: string | null };

export type WikiDeps = {
  currentAgent: () => string;
  chatTranscript: () => { title: string; text: string } | null;
  /** Chats in the app. The wiki keeps the ones that belong to this scope. */
  chats: () => WikiChat[];
};

type Starter = {
  id: string;
  label: string;
  hint: string;
  checked: boolean;
  path?: string;
  chat?: { title: string; text: string };
};

type Notice = { tone: "ok" | "warn" | "muted"; text: string };

type EditState = {
  path: string;
  isNew: boolean;
  title: string;
  body: string;
  baselineTitle: string;
  baselineBody: string;
  saving: boolean;
  cancelArmed: boolean;
  returnPath: string | null;
};

type EditorNodes = {
  scopeKey: string;
  path: string;
  title: HTMLInputElement;
  body: HTMLElement;
  didFocus: boolean;
};

let editorNodes: EditorNodes | null = null;
let composing = false;

type WikiState = {
  loaded: boolean;
  loading: boolean;
  error: string | null;
  pages: KnowledgePage[];
  selected: string | null;
  query: string;
  page: { path: string; html: string; source: string } | null;
  /** Set while the open page is being written. */
  edit: EditState | null;
  pageError: string | null;
  run: {
    id: string;
    agent: string;
    startedAt: number;
    /** Pages expected from the sources, or the pages already here on a rebuild. */
    total: number;
    done: number;
    before: Set<string>;
  } | null;
  notice: Notice | null;
  /** Second click on 删除 actually removes the page. */
  pendingDelete: string | null;
  /** Second click on 删除知识库 removes this whole scope. */
  pendingClear: boolean;
  starters: Starter[] | null;
  collapsedFolders: Set<string>;
};

const states = new Map<string, WikiState>();
let mounted: { host: HTMLElement; scope: WikiScope; deps: WikiDeps } | null = null;
let runTimer: number | null = null;

function stateFor(key: string): WikiState {
  let state = states.get(key);
  if (!state) {
    state = {
      loaded: false,
      loading: false,
      error: null,
      pages: [],
      selected: null,
      query: "",
      page: null,
      edit: null,
      pageError: null,
      run: null,
      notice: null,
      pendingDelete: null,
      pendingClear: false,
      starters: null,
      collapsedFolders: new Set(),
    };
    states.set(key, state);
  }
  return state;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function button(label: string, className: string, onClick: () => void, disabled = false): HTMLButtonElement {
  const node = el("button", className, label);
  node.type = "button";
  node.disabled = disabled;
  node.addEventListener("click", onClick);
  return node;
}

function agentName(runtime: string): string {
  return isAskRuntime(runtime) ? runtimeDisplayName(runtime) : runtime;
}

function clip(text: string, max: number): string {
  const trimmed = text.trim();
  return trimmed.length > max ? `${trimmed.slice(0, max)}…` : trimmed;
}

function elapsedText(startedAt: number): string {
  const seconds = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (getLocale() === "en") return minutes ? `${minutes}m ${rest}s` : `${rest}s`;
  return minutes ? `${minutes} 分 ${rest} 秒` : `${rest} 秒`;
}

function dateText(ms: number): string {
  if (!ms) return "";
  return new Date(ms).toLocaleString(getLocale() === "en" ? "en-US" : "zh-CN", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

const FOLDER_NAMES_ZH: Record<string, string> = {
  sources: "资料",
  topics: "主题",
  overview: "概览",
  components: "组成部分",
  guides: "使用指南",
  concepts: "概念",
  entities: "人和事",
  people: "人物",
  projects: "项目",
  notes: "笔记",
  decisions: "决定",
};

function folderLabel(name: string): string {
  if (getLocale() === "en") return name.replace(/-/g, " ");
  return FOLDER_NAMES_ZH[name.toLowerCase()] ?? name.replace(/-/g, " ");
}

function folderOf(path: string): string {
  const cut = path.lastIndexOf("/");
  return cut < 0 ? "" : path.slice(0, cut);
}

function stem(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1).replace(/\.md$/i, "");
}

function resolveRelative(from: string, href: string): string {
  const parts = folderOf(from).split("/").filter(Boolean);
  for (const segment of href.split("/")) {
    if (segment === "..") parts.pop();
    else if (segment && segment !== ".") parts.push(segment);
  }
  return parts.join("/");
}

function rerender(): void {
  if (mounted && mounted.host.isConnected) render(mounted);
}

function isMounted(scope: WikiScope): boolean {
  return Boolean(mounted && mounted.scope.key === scope.key && mounted.host.isConnected);
}

async function reloadPages(scope: WikiScope): Promise<void> {
  const state = stateFor(scope.key);
  const { pages } = await knowledgePages({ projectPath: scope.projectPath });
  state.pages = pages;
  state.loaded = true;
  state.error = null;
  if (state.edit?.isNew) return;
  if (!state.selected || !pages.some((page) => page.path === state.selected)) {
    state.selected = pages[0]?.path ?? null;
    state.page = null;
  }
}

async function load(scope: WikiScope): Promise<void> {
  const state = stateFor(scope.key);
  state.loading = true;
  rerender();
  try {
    await reloadPages(scope);
    if (!state.pages.length) await loadStarters(scope, depsFor(scope));
  } catch (error) {
    state.error = String(error);
  } finally {
    state.loading = false;
  }
  rerender();
  if (state.selected) void openPage(scope, state.selected);
}

function depsFor(scope: WikiScope): WikiDeps | null {
  return mounted && mounted.scope.key === scope.key ? mounted.deps : null;
}

async function loadStarters(scope: WikiScope, deps: WikiDeps | null): Promise<void> {
  const state = stateFor(scope.key);
  const starters: Starter[] = [];
  const chats = (deps?.chats() ?? [])
    .filter((chat) => chat.text.trim() && (!scope.projectName || chat.projectName === scope.projectName))
    .slice(0, 30);
  chats.forEach((chat, index) => {
    starters.push({
      id: `chat:${index}:${chat.title}`,
      label: chat.title.trim() || t("chat.wikiStartChat"),
      hint: t("chat.wikiStartChat"),
      checked: true,
      chat: { title: chat.title, text: chat.text },
    });
  });
  if (scope.projectPath) {
    try {
      const entries = await listWorkspaceDir({ root: scope.projectPath });
      for (const entry of entries) {
        starters.push({
          id: `path:${entry.relativePath}`,
          label: entry.name,
          hint: entry.isDir ? t("chat.wikiStartFolder") : t("chat.wikiStartFile"),
          checked: true,
          path: `${scope.projectPath.replace(/\/$/, "")}/${entry.relativePath}`,
        });
      }
    } catch {
      /* The folder list is optional. Chats still show. */
    }
  }
  state.starters = starters;
}

async function generateFromStarters(scope: WikiScope, deps: WikiDeps): Promise<void> {
  const state = stateFor(scope.key);
  const picked = (state.starters ?? []).filter((item) => item.checked);
  if (!picked.length || state.run) return;
  if (!leaveEdit(scope)) return;
  const paths = picked.flatMap((item) => (item.path ? [item.path] : []));
  const chats = picked.flatMap((item) => (item.chat ? [item.chat] : []));
  try {
    let root = await knowledgePrepare({ projectPath: scope.projectPath });
    const sources: string[] = [];
    let skipped = 0;
    let unreadable = 0;
    if (paths.length) {
      const imported = await knowledgeImport({ projectPath: scope.projectPath, paths });
      root = imported.root;
      sources.push(...imported.sources);
      skipped += imported.skipped;
      unreadable += imported.unreadable;
    }
    for (const chat of chats) {
      const saved = await knowledgeAddSource({
        projectPath: scope.projectPath,
        name: `chat-${chat.title}`,
        content: chat.text,
      });
      root = saved.root;
      sources.push(...saved.sources);
    }
    if (!sources.length) {
      setNotice(scope, { tone: "warn", text: t("chat.wikiNothingCopied") });
      return;
    }
    await runIngest(scope, deps, root, sources, "add", { skipped, unreadable });
  } catch (error) {
    setNotice(scope, { tone: "warn", text: String(error) });
  }
}

async function deletePage(scope: WikiScope, path: string, confirmed = false): Promise<void> {
  const state = stateFor(scope.key);
  if (!leaveEdit(scope)) return;
  if (!confirmed && state.pendingDelete !== path) {
    state.pendingDelete = path;
    rerender();
    return;
  }
  state.pendingDelete = null;
  try {
    await knowledgeDeletePage({ projectPath: scope.projectPath, path });
    state.page = null;
    state.selected = null;
    await reloadPages(scope);
    if (!state.pages.length) await loadStarters(scope, depsFor(scope));
    state.notice = { tone: "muted", text: t("chat.wikiDeleted") };
    syncKnowledgeFolder(scope.projectPath);
  } catch (error) {
    state.notice = { tone: "warn", text: String(error) };
  }
  rerender();
  if (state.selected && isMounted(scope)) void openPage(scope, state.selected);
}

function withoutTitle(markdown: string): string {
  return markdown.replace(/^\s*#\s+[^\n]*\n?/, "");
}

async function openPage(scope: WikiScope, path: string): Promise<void> {
  const state = stateFor(scope.key);
  if (state.edit && !leaveEdit(scope)) return;
  state.selected = path;
  if (state.pendingDelete && state.pendingDelete !== path) state.pendingDelete = null;
  state.pageError = null;
  if (state.page?.path !== path) state.page = null;
  rerender();
  try {
    const markdown = await knowledgeReadPage({ projectPath: scope.projectPath, path });
    if (state.selected !== path) return;
    state.page = { path, html: renderMarkdown(withoutTitle(markdown), { wikiLinks: true }), source: markdown };
  } catch {
    if (state.selected !== path) return;
    state.pageError = t("chat.wikiPageMissing");
  }
  rerender();
}

function setNotice(scope: WikiScope, notice: Notice | null): void {
  stateFor(scope.key).notice = notice;
  rerender();
}

type IngestKind = "add" | "rebuild" | "merge";

/** Absolute paths only: some agents (OpenClaw) start in their own folder, not the wiki. */
function ingestPrompt(root: string, sources: string[], kind: IngestKind): string {
  const language = getLocale() === "en" ? "English" : "Simplified Chinese";
  const at = (relative: string) => `${root}/${relative}`;
  const task =
    kind === "rebuild"
      ? `Rebuild the wiki from every source in ${at("raw/")}. Keep good pages, rewrite weak ones, and make sure ${at("wiki/index.md")} lists every page.`
      : kind === "merge"
        ? sources.length
          ? `Merge duplicates into ${sources.map((source) => at(`wiki/${source}`)).join(", ")}. Fold every other page that repeats the same facts into that page.`
          : `Find pages that repeat the same facts. Merge each group into one page.`
        : `Add these new sources to the wiki:\n${sources.map((source) => `- ${at(source)}`).join("\n")}\n\nIf a source repeats a page that already exists, update that page. Do not add another page for the same fact.`;
  return [
    `You maintain the knowledge wiki in ${root}. Your shell may start in a different folder, so always use these full paths. Read ${at("AGENTS.md")} first and follow it exactly.`,
    task,
    "When two pages say the same thing, keep one page, move any extra facts into it, delete the other page, and update the index. Do not leave a second page that only repeats the first.",
    "The user may have edited pages in the app. Keep their wording and structure. Do not revert a page unless a new source contradicts it.",
    "PDF and Word files already have a plain-text copy beside them with `.txt` added to the name (for example `spec.pdf.txt`). Read that copy. Do not try to convert, render, or OCR any file yourself. If a PDF or Word file has no `.txt` copy, it could not be read: skip it.",
    "Only run commands that finish within a few seconds. Never start a command that keeps running in the background.",
    `Write every page in ${language}.`,
    "When you finish, reply with one short sentence saying what you added or changed.",
  ].join("\n\n");
}

function leftText(remainingMs: number): string {
  if (remainingMs < 45_000) return t("chat.wikiLeftSoon");
  const minutes = Math.max(1, Math.round(remainingMs / 60_000));
  if (minutes >= 60) return t("chat.wikiLeftHours", { n: String(Math.max(1, Math.round(minutes / 60))) });
  return t("chat.wikiLeftMinutes", { n: String(minutes) });
}

function progressCaption(run: NonNullable<WikiState["run"]>): {
  percent: number;
  caption: string;
  phase: "reading" | "writing" | "finishing";
} {
  const progress = wikiRunProgress({
    elapsedMs: Date.now() - run.startedAt,
    done: run.done,
    total: run.total,
  });
  if (progress.phase === "reading") {
    return { percent: progress.percent, caption: t("chat.wikiProgressReading"), phase: progress.phase };
  }
  if (progress.phase === "finishing") {
    return { percent: progress.percent, caption: t("chat.wikiProgressFinishing"), phase: progress.phase };
  }
  return {
    percent: progress.percent,
    caption: t("chat.wikiProgressPages", {
      done: String(Math.min(run.done, run.total)),
      total: String(run.total),
      left: leftText(progress.remainingMs ?? 0),
    }),
    phase: progress.phase,
  };
}

function runningText(scope: WikiScope, run: NonNullable<WikiState["run"]>): string {
  const time = elapsedText(run.startedAt);
  if (scope.projectName) {
    return t("chat.wikiRunningProject", { agent: run.agent, name: scope.projectName, time });
  }
  return t("chat.wikiRunningGlobal", { agent: run.agent, time });
}

function paintRunText(): void {
  if (!mounted) return;
  const run = stateFor(mounted.scope.key).run;
  if (!run) return;
  const line = mounted.host.querySelector<HTMLElement>(".kw-notice-text[data-run]");
  if (line) line.textContent = runningText(mounted.scope, run);
  const painted = progressCaption(run);
  const caption = mounted.host.querySelector<HTMLElement>(".kw-progress-caption");
  if (caption) caption.textContent = painted.caption;
  const fill = mounted.host.querySelector<HTMLElement>(".kw-progress-fill");
  const bar = mounted.host.querySelector<HTMLElement>(".kw-progress");
  if (fill) fill.style.width = painted.phase === "reading" ? "" : `${painted.percent}%`;
  if (bar) {
    bar.classList.toggle("is-reading", painted.phase === "reading");
    bar.setAttribute("aria-valuenow", String(painted.percent));
  }
}

let pollingPages = false;

async function refreshRunPages(): Promise<void> {
  if (pollingPages || !mounted) return;
  const scope = mounted.scope;
  const state = stateFor(scope.key);
  const run = state.run;
  if (!run) return;
  pollingPages = true;
  try {
    const { pages } = await knowledgePages({ projectPath: scope.projectPath });
    if (state.run?.id !== run.id) return;
    run.done = pages.filter(
      (page) => !run.before.has(page.path) || page.modifiedMs >= run.startedAt - 1500,
    ).length;
    const changed =
      pages.length !== state.pages.length || pages.some((page, index) => page.path !== state.pages[index]?.path);
    if (changed) {
      state.pages = pages;
      if (!state.edit?.isNew && (!state.selected || !pages.some((page) => page.path === state.selected))) {
        state.selected = pages[0]?.path ?? null;
        state.page = null;
      }
      rerender();
      return;
    }
  } catch {
    /* Keep the last page count. The bar still moves with time. */
  } finally {
    pollingPages = false;
  }
  paintRunText();
}

let pollTick = 0;

function tickRun(): void {
  pollTick += 1;
  if (pollTick % 3 === 0) void refreshRunPages();
  else paintRunText();
}

function syncRunTimer(): void {
  const anyRunning = [...states.values()].some((state) => state.run);
  if (anyRunning && runTimer === null) runTimer = window.setInterval(tickRun, 1000);
  if (!anyRunning && runTimer !== null) {
    window.clearInterval(runTimer);
    runTimer = null;
  }
}

/** Approves file reads and writes for a knowledge run; nobody is watching it to click Allow. */
async function watchRun(clientRunId: string): Promise<UnlistenFn> {
  let backendId: string | null = null;
  return getCurrentWebviewWindow().listen<PromptSessionEvent>("prompt-session-event", (event) => {
    const payload = event.payload;
    if (payload.type === "started") {
      if (payload.client_run_id === clientRunId) backendId = payload.session_id;
      return;
    }
    if (payload.type !== "permission_request" || !backendId || payload.session_id !== backendId) return;
    const typed = payload.input_mode === "line" || payload.input_mode === "secret" || payload.input_mode === "options";
    void resolvePermissionSession({
      sessionId: payload.session_id,
      requestId: payload.request_id,
      allow: !typed,
    }).catch(() => {});
  });
}

async function runIngest(
  scope: WikiScope,
  deps: WikiDeps,
  root: string,
  sources: string[],
  kind: IngestKind,
  imported?: { skipped: number; unreadable: number },
): Promise<void> {
  const state = stateFor(scope.key);
  if (state.run) return;
  state.pendingClear = false;
  const runtime = deps.currentAgent();
  const agent = agentName(runtime);
  const id = `${KNOWLEDGE_RUN_PREFIX}${scope.key}:${Date.now()}`;
  const before = new Set(state.pages.map((page) => page.path));
  state.run = {
    id,
    agent,
    startedAt: Date.now(),
    total: kind === "add" ? Math.max(sources.length + 1, 1) : Math.max(before.size, 1),
    done: 0,
    before,
  };
  state.notice = null;
  syncRunTimer();
  rerender();
  const unlisten = await watchRun(id);
  let notice: Notice;
  try {
    const report = await startPromptSession({
      runtime,
      prompt: ingestPrompt(root, sources, kind),
      cwd: root,
      workspaceName: scope.projectName,
      timeoutSec: 3600,
      dangerouslySkipPermissions: true,
      fullAuto: true,
      clientRunId: id,
    });
    await reloadPages(scope).catch(() => {});
    const added = state.pages.filter((page) => !before.has(page.path));
    if (report.status === "succeeded") {
      notice = {
        tone: "ok",
        text: added.length ? t("chat.wikiDoneAdded", { count: String(added.length) }) : t("chat.wikiDoneUpdated"),
      };
    } else if (report.status === "cancelled") {
      notice = { tone: "muted", text: t("chat.wikiCancelled") };
    } else {
      notice = { tone: "warn", text: t("chat.wikiFailed", { agent }) };
    }
    const firstNew = added.find((page) => page.path !== "index.md" && page.path !== "log.md") ?? added[0];
    if (firstNew) state.selected = firstNew.path;
    state.page = null;
  } catch (error) {
    notice = { tone: "warn", text: `${t("chat.wikiFailed", { agent })}（${clip(String(error), 160)}）` };
  } finally {
    unlisten();
    state.run = null;
    syncRunTimer();
  }
  if (imported?.skipped) notice.text = `${notice.text} ${t("chat.wikiSkipped", { count: String(imported.skipped) })}`;
  if (imported?.unreadable) {
    notice.text = `${notice.text} ${t("chat.wikiUnreadable", { count: String(imported.unreadable) })}`;
  }
  state.notice = notice;
  rerender();
  syncKnowledgeFolder(scope.projectPath);
  if (state.selected && isMounted(scope)) void openPage(scope, state.selected);
}

async function addFromChat(scope: WikiScope, deps: WikiDeps): Promise<void> {
  if (stateFor(scope.key).run) return;
  if (!leaveEdit(scope)) return;
  const transcript = deps.chatTranscript();
  if (!transcript?.text.trim()) {
    setNotice(scope, { tone: "warn", text: t("chat.wikiChatEmpty") });
    return;
  }
  try {
    const saved = await knowledgeAddSource({
      projectPath: scope.projectPath,
      name: `chat-${transcript.title}`,
      content: transcript.text,
    });
    await runIngest(scope, deps, saved.root, saved.sources, "add");
  } catch (error) {
    setNotice(scope, { tone: "warn", text: String(error) });
  }
}

async function addPicked(scope: WikiScope, deps: WikiDeps, directory: boolean): Promise<void> {
  if (stateFor(scope.key).run) return;
  if (!leaveEdit(scope)) return;
  let picked: string | string[] | null;
  try {
    picked = await open({ multiple: true, directory });
  } catch {
    return;
  }
  const paths = (Array.isArray(picked) ? picked : picked ? [picked] : []).filter(Boolean);
  if (!paths.length) return;
  setNotice(scope, { tone: "muted", text: t("chat.wikiCopying") });
  try {
    const imported = await knowledgeImport({ projectPath: scope.projectPath, paths });
    if (!imported.sources.length) {
      setNotice(scope, { tone: "warn", text: t("chat.wikiNothingCopied") });
      return;
    }
    await runIngest(scope, deps, imported.root, imported.sources, "add", imported);
  } catch (error) {
    setNotice(scope, { tone: "warn", text: String(error) });
  }
}

async function clearKnowledge(scope: WikiScope): Promise<void> {
  const state = stateFor(scope.key);
  if (state.run) return;
  if (!leaveEdit(scope)) return;
  if (!state.pendingClear) {
    state.pendingClear = true;
    rerender();
    return;
  }
  state.pendingClear = false;
  try {
    await knowledgeClear({ projectPath: scope.projectPath });
    state.pages = [];
    state.selected = null;
    state.page = null;
    state.edit = null;
    editorNodes = null;
    state.pageError = null;
    state.pendingDelete = null;
    state.starters = null;
    state.notice = { tone: "muted", text: t("chat.wikiDeletedAll") };
    syncKnowledgeFolder(scope.projectPath);
    await loadStarters(scope, depsFor(scope));
  } catch (error) {
    state.notice = { tone: "warn", text: String(error) };
  }
  rerender();
}

async function mergeDuplicates(scope: WikiScope, deps: WikiDeps, focusPath?: string): Promise<void> {
  if (stateFor(scope.key).run) return;
  if (!leaveEdit(scope)) return;
  closePageMenu();
  try {
    const root = await knowledgePrepare({ projectPath: scope.projectPath });
    await runIngest(scope, deps, root, focusPath ? [focusPath] : [], "merge");
  } catch (error) {
    setNotice(scope, { tone: "warn", text: String(error) });
  }
}

let pageMenu: HTMLElement | null = null;
let pageMenuBound = false;

function closePageMenu(): void {
  pageMenu?.remove();
  pageMenu = null;
}

function ensurePageMenuListeners(): void {
  if (pageMenuBound) return;
  pageMenuBound = true;
  document.addEventListener("pointerdown", (event) => {
    if (!pageMenu) return;
    const target = event.target;
    if (target instanceof Node && pageMenu.contains(target)) return;
    closePageMenu();
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") closePageMenu();
  });
}

function openPageMenu(
  x: number,
  y: number,
  scope: WikiScope,
  deps: WikiDeps,
  path: string,
): void {
  ensurePageMenuListeners();
  closePageMenu();
  const menu = document.createElement("div");
  menu.className = "chat-project-menu";
  menu.setAttribute("role", "menu");

  const editItem = document.createElement("button");
  editItem.type = "button";
  editItem.className = "chat-project-menu-item";
  editItem.setAttribute("role", "menuitem");
  editItem.textContent = t("chat.wikiEdit");
  editItem.addEventListener("click", (event) => {
    event.stopPropagation();
    closePageMenu();
    void editFromMenu(scope, path);
  });

  const merge = document.createElement("button");
  merge.type = "button";
  merge.className = "chat-project-menu-item";
  merge.setAttribute("role", "menuitem");
  merge.textContent = t("chat.wikiMergePage");
  merge.addEventListener("click", (event) => {
    event.stopPropagation();
    void mergeDuplicates(scope, deps, path);
  });

  const remove = document.createElement("button");
  remove.type = "button";
  remove.className = "chat-project-menu-item";
  remove.setAttribute("role", "menuitem");
  remove.textContent = t("chat.wikiMenuDelete");
  let armed = false;
  remove.addEventListener("click", (event) => {
    event.stopPropagation();
    if (!armed) {
      armed = true;
      remove.textContent = t("chat.wikiDeleteConfirm");
      return;
    }
    closePageMenu();
    void deletePage(scope, path, true);
  });

  menu.append(editItem, merge, remove);
  document.body.append(menu);
  const rect = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(8, Math.min(x, window.innerWidth - rect.width - 8))}px`;
  menu.style.top = `${Math.max(8, Math.min(y, window.innerHeight - rect.height - 8))}px`;
  pageMenu = menu;
}

async function rebuild(scope: WikiScope, deps: WikiDeps): Promise<void> {
  if (stateFor(scope.key).run) return;
  if (!leaveEdit(scope)) return;
  stateFor(scope.key).pendingClear = false;
  try {
    const root = await knowledgePrepare({ projectPath: scope.projectPath });
    await runIngest(scope, deps, root, [], "rebuild");
  } catch (error) {
    setNotice(scope, { tone: "warn", text: String(error) });
  }
}

function toolbar(m: { scope: WikiScope; deps: WikiDeps }, state: WikiState): HTMLElement {
  const bar = el("div", "kw-bar");
  const busy = Boolean(state.run);
  const sources = el("div", "kw-sources");
  sources.append(
    button(t("chat.wikiFromChat"), "kw-btn", () => void addFromChat(m.scope, m.deps), busy),
    button(t("chat.wikiAddFiles"), "kw-btn", () => void addPicked(m.scope, m.deps, false), busy),
    button(t("chat.wikiAddFolder"), "kw-btn", () => void addPicked(m.scope, m.deps, true), busy),
    button(t("chat.wikiNewPage"), "kw-btn", () => startNewPage(m.scope), busy),
  );
  bar.append(sources);
  if (state.pages.length) {
    const actions = el("div", "kw-bar-end");
    if (state.pages.length > 1) {
      actions.append(button(t("chat.wikiMerge"), "kw-btn", () => void mergeDuplicates(m.scope, m.deps), busy));
    }
    actions.append(button(t("chat.wikiRebuild"), "kw-btn kw-btn-solid", () => void rebuild(m.scope, m.deps), busy));
    const wipe = button(
      state.pendingClear ? t("chat.wikiDeleteAllConfirm") : t("chat.wikiDeleteAll"),
      "kw-btn kw-btn-danger",
      () => void clearKnowledge(m.scope),
      busy,
    );
    wipe.classList.toggle("is-armed", state.pendingClear);
    actions.append(wipe);
    bar.append(actions);
  }
  return bar;
}

function noticeLine(scope: WikiScope, state: WikiState): HTMLElement | null {
  if (state.run) {
    const run = state.run;
    const line = el("div", "kw-notice kw-notice-run");
    line.dataset.tone = "muted";
    const row = el("div", "kw-notice-row");
    const text = el("span", "kw-notice-text", runningText(scope, run));
    text.dataset.run = "1";
    row.append(el("span", "kw-spinner"), text, button(t("chat.wikiStop"), "kw-btn kw-btn-quiet", () => void cancelPromptSession(run.id)));
    const painted = progressCaption(run);
    const bar = el("div", "kw-progress");
    bar.classList.toggle("is-reading", painted.phase === "reading");
    bar.setAttribute("role", "progressbar");
    bar.setAttribute("aria-valuemin", "0");
    bar.setAttribute("aria-valuemax", "100");
    bar.setAttribute("aria-valuenow", String(painted.percent));
    const fill = el("div", "kw-progress-fill");
    if (painted.phase !== "reading") fill.style.width = `${painted.percent}%`;
    bar.append(fill);
    line.append(row, bar, el("p", "kw-progress-caption", painted.caption));
    return line;
  }
  if (!state.notice) return null;
  const line = el("div", "kw-notice");
  line.dataset.tone = state.notice.tone;
  line.append(el("span", "kw-notice-text", state.notice.text));
  return line;
}

async function reorderKnowledgePages(
  scope: WikiScope,
  state: WikiState,
  targetCategory: string,
  targetPaths: string[],
  from: string,
  to: string,
  position: PageDropPosition,
): Promise<void> {
  const moved = state.pages.find((page) => page.path === from);
  const target = state.pages.find((page) => page.path === to);
  if (!moved || !target) return;
  const sourceCategory = pageCategory(moved);
  const seeded = targetPaths.includes(from) ? targetPaths : [...targetPaths, from];
  const ordered = reorderPagePaths(seeded, from, to, position);
  if (
    sourceCategory === targetCategory &&
    ordered.every((path, index) => path === targetPaths[index])
  ) {
    return;
  }
  const next = state.pages.filter((page) => page.path !== from);
  let targetIndex = next.findIndex((page) => page.path === to);
  if (targetIndex < 0) return;
  if (position === "after") targetIndex += 1;
  next.splice(targetIndex, 0, { ...moved, category: targetCategory });
  state.pages = next;
  rerender();
  try {
    await knowledgeSaveOrder({
      projectPath: scope.projectPath,
      category: targetCategory,
      pagePaths: ordered,
      movedPath: sourceCategory === targetCategory ? null : from,
    });
  } catch {
    state.notice = { tone: "warn", text: t("chat.wikiOrderFailed") };
    await reloadPages(scope).catch(() => {});
    rerender();
  }
}

type PageGroup = {
  key: string;
  label: string;
  category: string;
  pages: KnowledgePage[];
  fixed: boolean;
};

function pageCategory(page: KnowledgePage): string {
  return page.category ?? folderOf(page.path);
}

function groupedPages(pages: KnowledgePage[]): PageGroup[] {
  const groups = new Map<string, PageGroup>();
  const ensure = (key: string, label: string, category: string, fixed = false): PageGroup => {
    let group = groups.get(key);
    if (!group) {
      group = { key, label, category, pages: [], fixed };
      groups.set(key, group);
    }
    return group;
  };
  for (const page of pages) {
    if (page.path === "index.md") {
      ensure("!overview", t("chat.wikiCategoryOverview"), "", true).pages.push(page);
      continue;
    }
    if (page.path === "log.md") {
      ensure("~maintenance", t("chat.wikiCategoryMaintenance"), "", true).pages.push(page);
      continue;
    }
    const category = pageCategory(page);
    if (!category) {
      ensure("}other", t("chat.wikiCategoryOther"), "").pages.push(page);
      continue;
    }
    ensure(category, category.split("/").map(folderLabel).join(" / "), category).pages.push(page);
  }
  const rank = (group: PageGroup): number => {
    if (group.key === "!overview") return 0;
    if (group.key === "}other") return 2;
    if (group.key === "~maintenance") return 3;
    return 1;
  };
  return [...groups.values()].sort(
    (a, b) => rank(a) - rank(b) || a.label.localeCompare(b.label),
  );
}

function navList(m: { scope: WikiScope; deps: WikiDeps }, state: WikiState): HTMLElement {
  const list = el("div", "kw-tree");
  if (state.edit?.isNew) {
    const row = el("div", "kw-tree-row");
    const item = button(state.edit.title.trim() || t("chat.wikiNewPage"), "kw-tree-item", () => {
      editorNodes?.title.focus();
    });
    item.dataset.newPage = "1";
    item.setAttribute("aria-current", "page");
    row.append(item);
    list.append(row);
  }
  const q = state.query.trim().toLowerCase();
  const pages = q
    ? state.pages.filter((page) => `${page.title} ${page.path}`.toLowerCase().includes(q))
    : state.pages;
  if (!pages.length) {
    if (!state.edit?.isNew) list.append(el("p", "kw-tree-empty", t("chat.resourcesNoMatch")));
    return list;
  }
  const groups = groupedPages(pages);
  const movableCount = groups
    .filter((group) => !group.fixed)
    .reduce((count, group) => count + group.pages.length, 0);
  for (const group of groups) {
    const section = el("section", "kw-tree-section");
    const heading = button("", "kw-tree-folder", () => {
      if (q) return;
      if (state.collapsedFolders.has(group.key)) state.collapsedFolders.delete(group.key);
      else state.collapsedFolders.add(group.key);
      rerender();
    });
    const arrow = el("span", "kw-tree-arrow", "⌄");
    const name = el("span", "kw-tree-folder-name", group.label);
    const count = el("span", "kw-tree-folder-count", String(group.pages.length));
    const collapsed = !q && state.collapsedFolders.has(group.key);
    heading.setAttribute("aria-expanded", collapsed ? "false" : "true");
    heading.append(arrow, name, count);
    section.append(heading);
    const pageList = el("div", "kw-tree-group");
    pageList.hidden = collapsed;
    const paths = group.pages.map((page) => page.path);
    for (const page of group.pages) {
      const row = el("div", "kw-tree-row");
      row.dataset.page = page.path;
      row.dataset.id = page.path;
      const sortable = !q && !group.fixed && movableCount > 1;
      if (sortable) {
        row.dataset.category = group.category;
        const grip = button("⠿", "kw-tree-grip", () => {});
        grip.title = t("chat.wikiReorder");
        grip.setAttribute("aria-label", t("chat.wikiReorder"));
        row.append(grip);
        attachPointerReorder(
          list,
          grip,
          page.path,
          (from, to, position) => {
            const targetPage = state.pages.find((item) => item.path === to);
            if (!targetPage) return;
            const targetCategory = pageCategory(targetPage);
            const targetPaths =
              groupedPages(state.pages)
                .find((item) => !item.fixed && item.category === targetCategory)
                ?.pages.map((item) => item.path) ?? paths;
            void reorderKnowledgePages(
              m.scope,
              state,
              targetCategory,
              targetPaths,
              from,
              to,
              position,
            );
          },
          {
            rowSelector: ".kw-tree-row[data-page][data-category]",
            bodyClass: "chat-knowledge-dragging",
          },
        );
      }
      const item = button(page.title, "kw-tree-item", () => void openPage(m.scope, page.path));
      item.title = page.title;
      item.addEventListener("contextmenu", (event) => {
        event.preventDefault();
        openPageMenu(event.clientX, event.clientY, m.scope, m.deps, page.path);
      });
      item.setAttribute("aria-current", page.path === state.selected ? "page" : "false");
      row.append(item);
      pageList.append(row);
    }
    section.append(pageList);
    list.append(section);
  }
  return list;
}

function draftOf(scopeKey: string, state: WikiState): { title: string; body: string } | null {
  const edit = state.edit;
  if (!edit) return null;
  const live = editorNodes && editorNodes.scopeKey === scopeKey && editorNodes.path === edit.path ? editorNodes : null;
  return {
    title: (live ? live.title.value : edit.title).trim(),
    body: (live ? htmlToMarkdown(live.body) : edit.body).replace(/\s+$/, ""),
  };
}

function editIsDirty(scopeKey: string, state: WikiState): boolean {
  const edit = state.edit;
  const draft = draftOf(scopeKey, state);
  if (!edit || !draft) return false;
  if (edit.isNew) return Boolean(draft.title || draft.body.trim());
  return draft.title !== edit.baselineTitle.trim() || draft.body !== edit.baselineBody.replace(/\s+$/, "");
}

function canSave(scopeKey: string, state: WikiState): boolean {
  const draft = draftOf(scopeKey, state);
  return Boolean(draft?.title) && editIsDirty(scopeKey, state);
}

/** False when the open page still has unsaved words. Callers stay on that page. */
function leaveEdit(scope: WikiScope): boolean {
  const state = stateFor(scope.key);
  const edit = state.edit;
  if (!edit) return true;
  if (editIsDirty(scope.key, state)) {
    state.notice = { tone: "warn", text: t("chat.wikiUnsaved") };
    rerender();
    return false;
  }
  if (edit.isNew) state.selected = edit.returnPath;
  state.edit = null;
  editorNodes = null;
  return true;
}

function syncSave(scopeKey: string): void {
  const state = stateFor(scopeKey);
  const edit = state.edit;
  if (!edit || !mounted || mounted.scope.key !== scopeKey) return;
  const save = mounted.host.querySelector<HTMLButtonElement>(".kw-save");
  if (save) save.disabled = edit.saving || !canSave(scopeKey, state);
  if (edit.cancelArmed) {
    edit.cancelArmed = false;
    const cancel = mounted.host.querySelector<HTMLButtonElement>(".kw-cancel");
    if (cancel) cancel.textContent = t("chat.wikiCancel");
  }
  const label = mounted.host.querySelector<HTMLElement>("[data-new-page]");
  if (label && edit.isNew) label.textContent = edit.title.trim() || t("chat.wikiNewPage");
}

function activeScope(scopeKey: string): WikiScope | null {
  return mounted && mounted.scope.key === scopeKey ? mounted.scope : null;
}

function ensureEditor(scopeKey: string, edit: EditState): EditorNodes {
  if (editorNodes && editorNodes.scopeKey === scopeKey && editorNodes.path === edit.path) return editorNodes;
  const title = document.createElement("input");
  title.type = "text";
  title.className = "kw-title-input";
  title.value = edit.title;
  title.placeholder = t("chat.wikiTitlePlaceholder");
  title.setAttribute("aria-label", t("chat.wikiTitlePlaceholder"));
  title.autocomplete = "off";
  title.spellcheck = false;
  const body = document.createElement("div");
  body.className = "kw-editor kw-doc chat-md";
  body.contentEditable = "true";
  body.spellcheck = true;
  body.dataset.placeholder = t("chat.wikiBodyPlaceholder");
  body.setAttribute("role", "textbox");
  body.setAttribute("aria-multiline", "true");
  body.setAttribute("aria-label", t("chat.wikiBodyPlaceholder"));
  body.innerHTML = renderMarkdown(edit.body, { wikiLinks: true });
  if (!edit.isNew) {
    const normalized = htmlToMarkdown(body).replace(/\s+$/, "");
    edit.baselineBody = normalized;
    edit.body = normalized;
  }
  const remember = () => {
    const current = stateFor(scopeKey).edit;
    if (!current || current.saving) return;
    current.title = title.value;
    syncSave(scopeKey);
  };
  title.addEventListener("input", remember);
  body.addEventListener("input", () => {
    if (!composing) remember();
  });
  body.addEventListener("compositionstart", () => {
    composing = true;
  });
  body.addEventListener("compositionend", () => {
    composing = false;
    remember();
  });
  const onKey = (event: KeyboardEvent) => {
    if (event.isComposing || composing) return;
    const scope = activeScope(scopeKey);
    if (!scope) return;
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s" && !event.altKey) {
      event.preventDefault();
      event.stopPropagation();
      void saveEdit(scope);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      cancelEdit(scope);
    }
  };
  title.addEventListener("keydown", (event) => {
    onKey(event);
    if (event.key === "Enter" && !event.isComposing && !composing) {
      event.preventDefault();
      body.focus();
    }
  });
  body.addEventListener("keydown", onKey);
  body.addEventListener("paste", (event) => {
    if (stateFor(scopeKey).edit?.saving) return;
    const text = event.clipboardData?.getData("text/plain");
    if (text == null) return;
    event.preventDefault();
    if (document.execCommand("insertText", false, text)) return;
    const selection = document.getSelection();
    if (!selection || selection.rangeCount === 0) return;
    const range = selection.getRangeAt(0);
    range.deleteContents();
    range.insertNode(document.createTextNode(text));
    range.collapse(false);
    remember();
  });
  body.addEventListener("drop", (event) => event.preventDefault());
  body.addEventListener("click", (event) => {
    if ((event.target as HTMLElement).closest("a")) event.preventDefault();
  });
  editorNodes = { scopeKey, path: edit.path, title, body, didFocus: false };
  return editorNodes;
}

function startEdit(scope: WikiScope): void {
  const state = stateFor(scope.key);
  if (state.run) return;
  const source = state.page;
  if (!source || source.path !== state.selected) return;
  if (state.edit && !state.edit.isNew && state.edit.path === source.path) return;
  if (!leaveEdit(scope)) return;
  const split = splitWikiPage(source.source);
  const known = state.pages.find((page) => page.path === source.path)?.title ?? "";
  const title = split.title || known;
  state.edit = {
    path: source.path,
    isNew: false,
    title,
    body: split.body,
    baselineTitle: title,
    baselineBody: "",
    saving: false,
    cancelArmed: false,
    returnPath: source.path,
  };
  state.pendingDelete = null;
  state.notice = null;
  rerender();
}

function startNewPage(scope: WikiScope): void {
  const state = stateFor(scope.key);
  if (state.run) return;
  if (state.edit?.isNew) {
    editorNodes?.title.focus();
    return;
  }
  if (!leaveEdit(scope)) return;
  state.edit = {
    path: nextNotePath(state.pages.map((page) => page.path)),
    isNew: true,
    title: "",
    body: "",
    baselineTitle: "",
    baselineBody: "",
    saving: false,
    cancelArmed: false,
    returnPath: state.selected,
  };
  state.selected = null;
  state.pendingDelete = null;
  state.notice = null;
  rerender();
}

function cancelEdit(scope: WikiScope): void {
  const state = stateFor(scope.key);
  const edit = state.edit;
  if (!edit || edit.saving) return;
  if (editIsDirty(scope.key, state) && !edit.cancelArmed) {
    edit.cancelArmed = true;
    const cancel = mounted?.host.querySelector<HTMLButtonElement>(".kw-cancel");
    if (cancel) {
      cancel.textContent = t("chat.wikiDiscard");
      return;
    }
    rerender();
    return;
  }
  const back = edit.returnPath;
  const wasNew = edit.isNew;
  state.edit = null;
  editorNodes = null;
  state.notice = null;
  if (wasNew) state.selected = back;
  rerender();
  if (wasNew && state.selected && state.page?.path !== state.selected && isMounted(scope)) void openPage(scope, state.selected);
}

async function editFromMenu(scope: WikiScope, path: string): Promise<void> {
  const state = stateFor(scope.key);
  if (state.run) return;
  if (state.edit && !state.edit.isNew && state.edit.path === path) return;
  if (!leaveEdit(scope)) return;
  if (state.selected !== path || state.page?.path !== path) await openPage(scope, path);
  const next = stateFor(scope.key);
  if (next.page?.path === path && next.selected === path) startEdit(scope);
}

async function touchIndex(scope: WikiScope, path: string, title: string, summary: string, relabel: boolean): Promise<void> {
  if (path === "index.md" || path === "log.md") return;
  try {
    const index = await knowledgeReadPage({ projectPath: scope.projectPath, path: "index.md" });
    const next = upsertIndexLink(index, path, title, summary, relabel);
    if (next === index) return;
    await knowledgeSavePage({ projectPath: scope.projectPath, path: "index.md", content: next });
  } catch {
    /* The page itself is already saved. A missing index still lists it for the next question. */
  }
}

async function saveEdit(scope: WikiScope): Promise<void> {
  const state = stateFor(scope.key);
  const edit = state.edit;
  if (!edit || edit.saving) return;
  const draft = draftOf(scope.key, state);
  if (!draft) return;
  if (!draft.title) {
    state.notice = { tone: "warn", text: t("chat.wikiNeedTitle") };
    editorNodes?.title.focus();
    rerender();
    return;
  }
  if (!editIsDirty(scope.key, state)) return;
  const content = joinWikiPage(draft.title, draft.body);
  if (new TextEncoder().encode(content).length > WIKI_PAGE_MAX_BYTES) {
    state.notice = { tone: "warn", text: t("chat.wikiTooLong") };
    rerender();
    return;
  }
  edit.saving = true;
  state.notice = null;
  rerender();
  const relabel = edit.isNew || draft.title !== edit.baselineTitle.trim();
  const summary = edit.isNew ? plainLead(draft.body) : "";
  try {
    await knowledgeSavePage({ projectPath: scope.projectPath, path: edit.path, content });
    await touchIndex(scope, edit.path, draft.title, summary, relabel);
  } catch {
    edit.saving = false;
    state.notice = { tone: "warn", text: t("chat.wikiSaveFailed") };
    rerender();
    return;
  }
  const path = edit.path;
  state.edit = null;
  editorNodes = null;
  state.selected = path;
  state.page = { path, source: content, html: renderMarkdown(withoutTitle(content), { wikiLinks: true }) };
  state.notice = { tone: "ok", text: t("chat.wikiSaved") };
  await reloadPages(scope).catch(() => {});
  syncKnowledgeFolder(scope.projectPath);
  rerender();
  if (isMounted(scope)) await openPage(scope, path);
}

function textOffset(root: HTMLElement, target: Node, offset: number): number {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  let count = 0;
  let current = walker.nextNode();
  while (current) {
    if (current === target) return count + offset;
    count += current.textContent?.length ?? 0;
    current = walker.nextNode();
  }
  return count;
}

function captureSelection(root: HTMLElement): { start: number; end: number } | null {
  const selection = document.getSelection();
  if (!selection || selection.rangeCount === 0) return null;
  const range = selection.getRangeAt(0);
  if (!root.contains(range.startContainer) || !root.contains(range.endContainer)) return null;
  if (range.startContainer.nodeType !== Node.TEXT_NODE || range.endContainer.nodeType !== Node.TEXT_NODE) return null;
  return {
    start: textOffset(root, range.startContainer, range.startOffset),
    end: textOffset(root, range.endContainer, range.endOffset),
  };
}

function restoreSelection(root: HTMLElement, selection: { start: number; end: number }): void {
  const point = (offset: number): { node: Node; offset: number } | null => {
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    let left = offset;
    let last: Node | null = null;
    let current = walker.nextNode();
    while (current) {
      const len = current.textContent?.length ?? 0;
      if (left <= len) return { node: current, offset: left };
      left -= len;
      last = current;
      current = walker.nextNode();
    }
    if (!last) return null;
    return { node: last, offset: last.textContent?.length ?? 0 };
  };
  const start = point(selection.start);
  const end = point(selection.end);
  if (!start || !end) return;
  const range = document.createRange();
  range.setStart(start.node, start.offset);
  range.setEnd(end.node, end.offset);
  const selected = document.getSelection();
  selected?.removeAllRanges();
  selected?.addRange(range);
}

function editView(scope: WikiScope, state: WikiState): HTMLElement {
  const edit = state.edit;
  const view = el("article", "kw-page is-editing");
  if (!edit) return view;
  const nodes = ensureEditor(scope.key, edit);
  const head = el("header", "kw-page-head");
  head.append(nodes.title);
  const meta = el("div", "kw-page-meta");
  meta.append(el("span", "kw-edit-hint", t("chat.wikiEditHint")));
  meta.append(
    button(
      edit.saving ? t("chat.wikiSaving") : t("chat.wikiSave"),
      "kw-btn kw-btn-solid kw-save",
      () => void saveEdit(scope),
      edit.saving || !canSave(scope.key, state),
    ),
    button(
      edit.cancelArmed ? t("chat.wikiDiscard") : t("chat.wikiCancel"),
      "kw-btn kw-btn-quiet kw-cancel",
      () => cancelEdit(scope),
      edit.saving,
    ),
  );
  head.append(meta);
  view.append(head, nodes.body);
  nodes.title.readOnly = edit.saving;
  nodes.body.contentEditable = edit.saving ? "false" : "true";
  return view;
}

function pageView(m: { scope: WikiScope }, state: WikiState): HTMLElement {
  if (state.edit && (state.edit.isNew || state.edit.path === state.selected)) return editView(m.scope, state);
  const view = el("article", "kw-page");
  const info = state.pages.find((page) => page.path === state.selected);
  if (!info) return view;
  const head = el("header", "kw-page-head");
  head.append(el("h3", "kw-page-title", info.title));
  const meta = el("div", "kw-page-meta");
  const updated = dateText(info.modifiedMs);
  if (updated) meta.append(el("span", "", t("chat.wikiUpdated", { time: updated })));
  const actions = el("span", "kw-page-actions");
  if (state.page?.path === info.path && !state.pageError) {
    actions.append(button(t("chat.wikiEdit"), "kw-btn kw-edit", () => startEdit(m.scope), Boolean(state.run)));
  }
  const removing = state.pendingDelete === info.path;
  const remove = button(removing ? t("chat.wikiDeleteConfirm") : t("chat.wikiDelete"), "kw-delete", () => {
    void deletePage(m.scope, info.path);
  });
  remove.classList.toggle("is-armed", removing);
  actions.append(remove);
  meta.append(actions);
  head.append(meta);
  view.append(head);
  if (state.pageError) {
    view.append(el("p", "kw-hint", state.pageError));
    return view;
  }
  if (!state.page || state.page.path !== info.path) {
    view.append(el("p", "kw-hint", t("chat.wikiLoading")));
    return view;
  }
  const doc = el("div", "kw-doc chat-md");
  doc.innerHTML = state.page.html;
  doc.addEventListener("click", (event) => {
    const link = (event.target as HTMLElement).closest<HTMLAnchorElement>("a[data-wiki-page], a[data-wiki-name]");
    if (!link) return;
    event.preventDefault();
    const target = link.dataset.wikiPage
      ? resolveRelative(info.path, link.dataset.wikiPage)
      : state.pages.find((page) => {
          const name = (link.dataset.wikiName ?? "").toLowerCase();
          return page.title.toLowerCase() === name || stem(page.path).toLowerCase() === name;
        })?.path;
    if (target && state.pages.some((page) => page.path === target)) void openPage(m.scope, target);
  });
  view.append(doc);
  return view;
}

function starterView(m: { scope: WikiScope; deps: WikiDeps }, state: WikiState): HTMLElement {
  const box = el("div", "kw-start");
  box.append(el("strong", "", t("chat.wikiEmptyTitle")), el("p", "", t("chat.wikiEmptyHint")));
  if (!state.starters) {
    box.append(el("p", "kw-hint", t("chat.wikiStartScanning")));
    return box;
  }
  if (!state.starters.length) {
    box.append(el("p", "kw-hint", t("chat.wikiStartNone")));
    return box;
  }
  const list = el("div", "kw-start-list");
  for (const item of state.starters) {
    const row = el("label", "kw-start-row");
    const input = document.createElement("input");
    input.type = "checkbox";
    input.checked = item.checked;
    input.addEventListener("change", () => {
      item.checked = input.checked;
      const next = box.querySelector<HTMLButtonElement>(".kw-generate");
      if (next) next.disabled = !state.starters?.some((entry) => entry.checked);
    });
    const text = el("span", "kw-start-label", item.label);
    row.append(input, text, el("span", "kw-start-hint", item.hint));
    list.append(row);
  }
  box.append(list);
  const picked = state.starters.some((item) => item.checked);
  box.append(
    button(t("chat.wikiGenerate"), "kw-btn kw-btn-solid kw-generate", () => void generateFromStarters(m.scope, m.deps), !picked),
  );
  return box;
}

const NAV_WIDTH_KEY = "agent-doctor.kw-nav-width";
const NAV_WIDTH_DEFAULT = 210;

function clampNavWidth(width: number): number {
  return Math.round(Math.min(420, Math.max(150, width)));
}

function navResizer(body: HTMLElement): HTMLElement {
  const saved = Number(localStorage.getItem(NAV_WIDTH_KEY));
  if (saved) body.style.setProperty("--kw-nav-w", `${clampNavWidth(saved)}px`);
  const handle = el("div", "kw-resizer");
  handle.setAttribute("role", "separator");
  handle.setAttribute("aria-orientation", "vertical");
  handle.title = t("chat.wikiResize");
  handle.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    handle.setPointerCapture(event.pointerId);
    body.classList.add("is-resizing");
    const left = body.getBoundingClientRect().left;
    const move = (e: PointerEvent) => {
      body.style.setProperty("--kw-nav-w", `${clampNavWidth(e.clientX - left)}px`);
    };
    const end = (e: PointerEvent) => {
      handle.releasePointerCapture(e.pointerId);
      handle.removeEventListener("pointermove", move);
      body.classList.remove("is-resizing");
      localStorage.setItem(NAV_WIDTH_KEY, String(clampNavWidth(e.clientX - left)));
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", end, { once: true });
  });
  handle.addEventListener("dblclick", () => {
    localStorage.removeItem(NAV_WIDTH_KEY);
    body.style.setProperty("--kw-nav-w", `${NAV_WIDTH_DEFAULT}px`);
  });
  return handle;
}

function render(m: { host: HTMLElement; scope: WikiScope; deps: WikiDeps }): void {
  const state = stateFor(m.scope.key);
  const viewPath = state.edit?.path ?? state.page?.path ?? "";
  const oldPage = m.host.querySelector<HTMLElement>(".kw-page");
  const oldTree = m.host.querySelector<HTMLElement>(".kw-tree");
  const keepPage = oldPage?.dataset.path === viewPath ? oldPage.scrollTop : 0;
  const keepTree = oldTree?.scrollTop ?? 0;
  const live =
    editorNodes && state.edit && editorNodes.scopeKey === m.scope.key && editorNodes.path === state.edit.path
      ? editorNodes
      : null;
  const bodyFocused = Boolean(
    live && document.activeElement && (document.activeElement === live.body || live.body.contains(document.activeElement)),
  );
  const titleFocused = Boolean(live && document.activeElement === live.title);
  const selection = bodyFocused && live && !composing ? captureSelection(live.body) : null;
  const titleRange = titleFocused && live ? ([live.title.selectionStart ?? 0, live.title.selectionEnd ?? 0] as const) : null;
  if (!state.edit) editorNodes = null;
  closePageMenu();
  const root = el("div", "kw");
  root.append(toolbar(m, state));
  const notice = noticeLine(m.scope, state);
  if (notice) root.append(notice);
  if (!state.loaded) {
    root.append(el("p", "kw-hint", state.error ?? t("chat.wikiLoading")));
  } else if (state.edit?.isNew && !state.pages.length && !state.run) {
    root.append(editView(m.scope, state));
  } else if (!state.pages.length && !state.run) {
    root.append(starterView(m, state));
  } else if (state.pages.length) {
    const body = el("div", "kw-body");
    const nav = el("aside", "kw-nav");
    const search = el("input", "chat-knowledge-input kw-search");
    search.type = "search";
    search.placeholder = t("chat.wikiSearch");
    search.value = state.query;
    let tree = navList(m, state);
    search.addEventListener("input", () => {
      state.query = search.value;
      const next = navList(m, state);
      tree.replaceWith(next);
      tree = next;
    });
    nav.append(search, tree);
    body.append(nav, navResizer(body), pageView(m, state));
    root.append(body);
  }
  m.host.replaceChildren(root);
  const page = root.querySelector<HTMLElement>(".kw-page");
  if (page) {
    page.dataset.path = viewPath;
    page.scrollTop = keepPage;
  }
  const tree = root.querySelector<HTMLElement>(".kw-tree");
  if (tree) tree.scrollTop = keepTree;
  const nodes =
    editorNodes && state.edit && editorNodes.scopeKey === m.scope.key && editorNodes.path === state.edit.path
      ? editorNodes
      : null;
  if (!nodes || composing) return;
  if (!nodes.didFocus) {
    nodes.didFocus = true;
    (state.edit?.isNew ? nodes.title : nodes.body).focus();
    return;
  }
  if (selection) restoreSelection(nodes.body, selection);
  else if (titleRange) {
    nodes.title.focus();
    nodes.title.setSelectionRange(titleRange[0], titleRange[1]);
  }
}

export function mountWiki(host: HTMLElement, scope: WikiScope, deps: WikiDeps): void {
  mounted = { host, scope, deps };
  const state = stateFor(scope.key);
  render(mounted);
  if (!state.loaded && !state.loading) void load(scope);
  else if (state.selected && state.page?.path !== state.selected) void openPage(scope, state.selected);
}

/** Moves notes written before the wiki into a `notes.md` page. */
export async function migrateLegacyNotes(
  scope: WikiScope,
  notes: { title: string; body: string }[],
): Promise<boolean> {
  if (!notes.length) return true;
  let existing = "";
  try {
    existing = await knowledgeReadPage({ projectPath: scope.projectPath, path: "notes.md" });
  } catch {
    existing = `# ${t("chat.wikiLegacyTitle")}\n`;
  }
  const blocks = notes.map((note) => {
    const title = note.title.trim();
    return title ? `## ${title}\n\n${note.body.trim()}` : `- ${note.body.trim()}`;
  });
  try {
    await knowledgeSavePage({
      projectPath: scope.projectPath,
      path: "notes.md",
      content: `${existing.trimEnd()}\n\n${blocks.join("\n\n")}\n`,
    });
  } catch {
    return false;
  }
  syncKnowledgeFolder(scope.projectPath);
  stateFor(scope.key).loaded = false;
  return true;
}

/** Catalog and location for one knowledge base. The assistant opens a page only when the message needs it. */
export async function wikiPromptSection(label: string, projectPath: string | null): Promise<string | null> {
  try {
    const context = await knowledgeContext({ projectPath });
    if (!context?.index.trim()) return null;
    const zh = getLocale() !== "en";
    const sourcesOnly = Boolean(context.rawDir) && context.wiki === context.rawDir;
    if (sourcesOnly) {
      const lead = zh
        ? `这些资料还没整理成页面，在 ${context.rawDir}。只有某个文件名和这句话对得上时，才打开那一个文件。不要把文件夹读完。`
        : `These files are not pages yet. They are in ${context.rawDir}. Open one file only when its name matches this message. Do not read the whole folder.`;
      return `${label}\n${lead}\n${context.index}`;
    }
    const where = zh
      ? `页面在 ${context.wiki}。下面是目录。只有这句话用得上某一页时，才打开那一页。用不上就直接回答，不要把文件夹翻一遍。`
      : `Pages are in ${context.wiki}. The catalog is below. Open a page only when this message needs it. Otherwise answer directly, and do not browse the folder.`;
    return `${label}\n${where}\n${context.index}`;
  } catch {
    return null;
  }
}
