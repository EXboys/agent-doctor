import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { open } from "@tauri-apps/plugin-dialog";
import { getLocale, t } from "../i18n";
import {
  cancelPromptSession,
  knowledgeAddSource,
  knowledgeContext,
  knowledgeImport,
  knowledgeDeletePage,
  knowledgePages,
  knowledgePrepare,
  knowledgeReadPage,
  knowledgeSavePage,
  listWorkspaceDir,
  resolvePermissionSession,
  startPromptSession,
  type KnowledgePage,
} from "../ipc";
import { renderMarkdown } from "../markdown";
import { isAskRuntime, runtimeDisplayName } from "./runtime";
import type { PromptSessionEvent } from "./types";

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

type WikiState = {
  loaded: boolean;
  loading: boolean;
  error: string | null;
  pages: KnowledgePage[];
  selected: string | null;
  query: string;
  page: { path: string; html: string } | null;
  pageError: string | null;
  run: { id: string; agent: string; startedAt: number } | null;
  notice: Notice | null;
  /** Second click on 删除 actually removes the page. */
  pendingDelete: string | null;
  starters: Starter[] | null;
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
      pageError: null,
      run: null,
      notice: null,
      pendingDelete: null,
      starters: null,
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
    await runIngest(scope, deps, root, sources, false, { skipped, unreadable });
  } catch (error) {
    setNotice(scope, { tone: "warn", text: String(error) });
  }
}

async function deletePage(scope: WikiScope, path: string): Promise<void> {
  const state = stateFor(scope.key);
  if (state.pendingDelete !== path) {
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
  state.selected = path;
  if (state.pendingDelete && state.pendingDelete !== path) state.pendingDelete = null;
  state.pageError = null;
  if (state.page?.path !== path) state.page = null;
  rerender();
  try {
    const markdown = await knowledgeReadPage({ projectPath: scope.projectPath, path });
    if (state.selected !== path) return;
    state.page = { path, html: renderMarkdown(withoutTitle(markdown), { wikiLinks: true }) };
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

/** Absolute paths only: some agents (OpenClaw) start in their own folder, not the wiki. */
function ingestPrompt(root: string, sources: string[], rebuild: boolean): string {
  const language = getLocale() === "en" ? "English" : "Simplified Chinese";
  const at = (relative: string) => `${root}/${relative}`;
  const task = rebuild
    ? `Rebuild the wiki from every source in ${at("raw/")}. Keep good pages, rewrite weak ones, merge duplicates, and make sure ${at("wiki/index.md")} lists every page.`
    : `Add these new sources to the wiki:\n${sources.map((source) => `- ${at(source)}`).join("\n")}`;
  return [
    `You maintain the knowledge wiki in ${root}. Your shell may start in a different folder, so always use these full paths. Read ${at("AGENTS.md")} first and follow it exactly.`,
    task,
    "PDF and Word files already have a plain-text copy beside them with `.txt` added to the name (for example `spec.pdf.txt`). Read that copy. Do not try to convert, render, or OCR any file yourself. If a PDF or Word file has no `.txt` copy, it could not be read: skip it.",
    "Only run commands that finish within a few seconds. Never start a command that keeps running in the background.",
    `Write every page in ${language}.`,
    "When you finish, reply with one short sentence saying what you added or changed.",
  ].join("\n\n");
}

function paintRunText(): void {
  if (!mounted) return;
  const run = stateFor(mounted.scope.key).run;
  const line = mounted.host.querySelector<HTMLElement>(".kw-notice-text[data-run]");
  if (run && line) line.textContent = t("chat.wikiRunning", { agent: run.agent, time: elapsedText(run.startedAt) });
}

function syncRunTimer(): void {
  const anyRunning = [...states.values()].some((state) => state.run);
  if (anyRunning && runTimer === null) runTimer = window.setInterval(paintRunText, 1000);
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
  rebuild: boolean,
  imported?: { skipped: number; unreadable: number },
): Promise<void> {
  const state = stateFor(scope.key);
  if (state.run) return;
  const runtime = deps.currentAgent();
  const agent = agentName(runtime);
  const id = `${KNOWLEDGE_RUN_PREFIX}${scope.key}:${Date.now()}`;
  const before = new Set(state.pages.map((page) => page.path));
  state.run = { id, agent, startedAt: Date.now() };
  state.notice = null;
  syncRunTimer();
  rerender();
  const unlisten = await watchRun(id);
  let notice: Notice;
  try {
    const report = await startPromptSession({
      runtime,
      prompt: ingestPrompt(root, sources, rebuild),
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
  if (state.selected && isMounted(scope)) void openPage(scope, state.selected);
}

async function addFromChat(scope: WikiScope, deps: WikiDeps): Promise<void> {
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
    await runIngest(scope, deps, saved.root, saved.sources, false);
  } catch (error) {
    setNotice(scope, { tone: "warn", text: String(error) });
  }
}

async function addPicked(scope: WikiScope, deps: WikiDeps, directory: boolean): Promise<void> {
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
    await runIngest(scope, deps, imported.root, imported.sources, false, imported);
  } catch (error) {
    setNotice(scope, { tone: "warn", text: String(error) });
  }
}

async function rebuild(scope: WikiScope, deps: WikiDeps): Promise<void> {
  try {
    const root = await knowledgePrepare({ projectPath: scope.projectPath });
    await runIngest(scope, deps, root, [], true);
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
  );
  bar.append(sources);
  if (state.pages.length) {
    bar.append(button(t("chat.wikiRebuild"), "kw-btn kw-btn-solid", () => void rebuild(m.scope, m.deps), busy));
  }
  return bar;
}

function noticeLine(state: WikiState): HTMLElement | null {
  if (state.run) {
    const run = state.run;
    const line = el("div", "kw-notice");
    line.dataset.tone = "muted";
    const text = el("span", "kw-notice-text", t("chat.wikiRunning", { agent: run.agent, time: elapsedText(run.startedAt) }));
    text.dataset.run = "1";
    line.append(el("span", "kw-spinner"), text, button(t("chat.wikiStop"), "kw-btn kw-btn-quiet", () => void cancelPromptSession(run.id)));
    return line;
  }
  if (!state.notice) return null;
  const line = el("div", "kw-notice");
  line.dataset.tone = state.notice.tone;
  line.append(el("span", "kw-notice-text", state.notice.text));
  return line;
}

function navList(m: { scope: WikiScope }, state: WikiState): HTMLElement {
  const list = el("div", "kw-tree");
  const q = state.query.trim().toLowerCase();
  const pages = q
    ? state.pages.filter((page) => `${page.title} ${page.path}`.toLowerCase().includes(q))
    : state.pages;
  if (!pages.length) {
    list.append(el("p", "kw-tree-empty", t("chat.resourcesNoMatch")));
    return list;
  }
  const groups = new Map<string, KnowledgePage[]>();
  for (const page of pages) {
    const folder = folderOf(page.path);
    groups.set(folder, [...(groups.get(folder) ?? []), page]);
  }
  const folders = [...groups.keys()].sort((a, b) => (a === "" ? -1 : b === "" ? 1 : a.localeCompare(b)));
  for (const folder of folders) {
    if (folder) list.append(el("p", "kw-tree-folder", folder.split("/").map(folderLabel).join(" / ")));
    for (const page of groups.get(folder) ?? []) {
      const item = button(page.title, "kw-tree-item", () => void openPage(m.scope, page.path));
      item.title = page.path;
      if (folder) item.classList.add("is-nested");
      item.setAttribute("aria-current", page.path === state.selected ? "page" : "false");
      list.append(item);
    }
  }
  return list;
}

function pageView(m: { scope: WikiScope }, state: WikiState): HTMLElement {
  const view = el("article", "kw-page");
  const info = state.pages.find((page) => page.path === state.selected);
  if (!info) return view;
  const head = el("header", "kw-page-head");
  head.append(el("h3", "kw-page-title", info.title));
  const meta = el("div", "kw-page-meta");
  const updated = dateText(info.modifiedMs);
  if (updated) meta.append(el("span", "", t("chat.wikiUpdated", { time: updated })));
  const removing = state.pendingDelete === info.path;
  const remove = button(removing ? t("chat.wikiDeleteConfirm") : t("chat.wikiDelete"), "kw-delete", () => {
    void deletePage(m.scope, info.path);
  });
  remove.classList.toggle("is-armed", removing);
  meta.append(remove);
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
  const root = el("div", "kw");
  root.append(toolbar(m, state));
  const notice = noticeLine(state);
  if (notice) root.append(notice);
  if (!state.loaded) {
    root.append(el("p", "kw-hint", state.error ?? t("chat.wikiLoading")));
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
  const oldPage = m.host.querySelector<HTMLElement>(".kw-page");
  const oldTree = m.host.querySelector<HTMLElement>(".kw-tree");
  const keepPage = oldPage?.dataset.path === state.page?.path ? oldPage?.scrollTop : 0;
  const keepTree = oldTree?.scrollTop ?? 0;
  m.host.replaceChildren(root);
  const page = root.querySelector<HTMLElement>(".kw-page");
  if (page) {
    page.dataset.path = state.page?.path ?? "";
    page.scrollTop = keepPage ?? 0;
  }
  const tree = root.querySelector<HTMLElement>(".kw-tree");
  if (tree) tree.scrollTop = keepTree;
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
  stateFor(scope.key).loaded = false;
  return true;
}

/** One knowledge base as prompt text, or null when it has no pages. */
export async function wikiPromptSection(label: string, projectPath: string | null): Promise<string | null> {
  try {
    const context = await knowledgeContext({ projectPath });
    if (!context) return null;
    if (context.full) return `${label}\n${context.full}`;
    const where =
      getLocale() === "en"
        ? `(pages live in ${context.wiki}; open the ones you need)`
        : `（页面在 ${context.wiki}，需要细节时打开对应页面）`;
    return `${label} ${where}\n${context.index}`;
  } catch {
    return null;
  }
}
