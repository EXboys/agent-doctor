import { t } from "../i18n";
import { chatState } from "../chat-state";
import { formatTime } from "./copy-ui";
import {
  isKnowledgePageOpen,
  isResourcesMainPageOpen,
  isSchedulePageOpen,
} from "./overlay-pages";
import { projectDisplayLabel, sessionWorkspaceName } from "./session-workspace";
import type { ChatSession } from "./types";

type Scope = "all" | "projects" | "chats";
type Row =
  | { kind: "chat"; id: string; title: string; meta: string }
  | { kind: "project"; name: string; title: string; meta: string }
  | { kind: "place"; id: "knowledge" | "schedule" | "skills" | "project"; title: string; meta: string };

const RECENT_LIMIT = 6;

let scope: Scope = "all";
let rows: Row[] = [];
let selected = 0;

const layerEl = () => document.querySelector<HTMLElement>("#chat-search");
const inputEl = () => document.querySelector<HTMLInputElement>("#chat-search-input");
const resultsEl = () => document.querySelector<HTMLElement>("#chat-search-results");
const openEl = () => document.querySelector<HTMLButtonElement>("#chat-search-open");

function shortcutLabel(): string {
  return document.documentElement.dataset.os === "macos" ? "⌘K" : "Ctrl K";
}

function queryText(): string {
  return inputEl()?.value.trim().toLowerCase() ?? "";
}

function includes(value: string, query: string): boolean {
  return value.toLowerCase().includes(query);
}

function projects(): { name: string; label: string }[] {
  const doc = chatState.workspaceDoc;
  if (!doc) return [];
  return Object.keys(doc.workspaces)
    .map((name) => ({ name, label: projectDisplayLabel(name, doc) }))
    .sort((left, right) => left.label.localeCompare(right.label, "zh"));
}

function chats(): ChatSession[] {
  return [...chatState.store.sessions].sort((left, right) => right.updatedAt - left.updatedAt);
}

function buildRows(): Row[] {
  const query = queryText();
  const doc = chatState.workspaceDoc;
  const next: Row[] = [];
  const wantProjects = scope === "all" || scope === "projects";
  const wantChats = scope === "all" || scope === "chats";

  if (!query && scope === "all") {
    for (const session of chats().slice(0, RECENT_LIMIT)) {
      next.push({
        kind: "chat",
        id: session.id,
        title: session.title.trim() || t("chat.searchChats"),
        meta: formatTime(session.updatedAt),
      });
    }
    next.push(
      { kind: "place", id: "knowledge", title: t("chat.knowledge"), meta: "" },
      { kind: "place", id: "schedule", title: t("chat.schedule"), meta: "" },
      { kind: "place", id: "skills", title: t("chat.resourcesPanelTitle"), meta: "" },
    );
    return next;
  }

  if (wantProjects) {
    for (const project of projects()) {
      if (query && !includes(project.label, query) && !includes(project.name, query)) continue;
      const count = chats().filter(
        (session) => sessionWorkspaceName(session, doc) === project.name,
      ).length;
      next.push({
        kind: "project",
        name: project.name,
        title: project.label,
        meta: count > 0 ? "" : t("chat.searchNoChats"),
      });
    }
  }

  if (wantChats) {
    for (const session of chats()) {
      const title = session.title.trim() || t("chat.searchChats");
      if (query && !includes(title, query)) continue;
      if (!query && next.filter((row) => row.kind === "chat").length >= RECENT_LIMIT) break;
      next.push({
        kind: "chat",
        id: session.id,
        title,
        meta: formatTime(session.updatedAt),
      });
    }
  }
  return next;
}

function sectionHeading(row: Row, index: number): string | null {
  const previous = rows[index - 1];
  if (row.kind === "place") {
    return previous?.kind === "place" ? null : t("chat.searchGo");
  }
  if (!queryText() && scope === "all" && row.kind === "chat") {
    return index === 0 ? t("chat.searchRecent") : null;
  }
  if (row.kind === "project") return previous?.kind === "project" ? null : t("chat.searchProjects");
  if (row.kind === "chat") return previous?.kind === "chat" ? null : t("chat.searchChats");
  return null;
}

function iconPath(row: Row): string {
  if (row.kind === "project") {
    return "M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z";
  }
  if (row.kind === "chat") {
    return "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z";
  }
  if (row.id === "knowledge") {
    return "M12 7v14M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z";
  }
  if (row.id === "schedule") {
    return "M8 2v4M16 2v4M3 10h18M5 4h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z";
  }
  return "M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.8-3.8a6 6 0 0 1-7.9 7.9l-6.9 6.9a2.1 2.1 0 0 1-3-3l6.9-6.9a6 6 0 0 1 7.9-7.9l-3.8 3.8z";
}

function rowIcon(row: Row): SVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("width", "16");
  svg.setAttribute("height", "16");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("fill", "none");
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("stroke", "currentColor");
  path.setAttribute("stroke-width", "2");
  path.setAttribute("stroke-linecap", "round");
  path.setAttribute("stroke-linejoin", "round");
  path.setAttribute("d", iconPath(row));
  svg.append(path);
  return svg;
}

function paint(): void {
  const results = resultsEl();
  if (!results) return;
  rows = buildRows();
  if (selected >= rows.length) selected = Math.max(0, rows.length - 1);
  results.replaceChildren();
  if (rows.length === 0) {
    const empty = document.createElement("p");
    empty.className = "chat-search-empty";
    empty.textContent = t("chat.searchEmpty");
    results.append(empty);
    return;
  }
  rows.forEach((row, index) => {
    const heading = sectionHeading(row, index);
    if (heading) {
      const label = document.createElement("p");
      label.className = "chat-search-heading";
      label.textContent = heading;
      results.append(label);
    }
    const button = document.createElement("button");
    button.type = "button";
    button.className = `chat-search-row${index === selected ? " is-on" : ""}`;
    button.append(rowIcon(row));
    const title = document.createElement("span");
    title.className = "chat-search-row-title";
    title.textContent = row.title;
    button.append(title);
    if (row.meta) {
      const meta = document.createElement("span");
      meta.className = "chat-search-row-meta";
      meta.textContent = row.meta;
      button.append(meta);
    }
    button.addEventListener("click", () => {
      selected = index;
      activate(row);
    });
    button.addEventListener("pointerenter", () => {
      if (selected === index) return;
      selected = index;
      results.querySelectorAll(".chat-search-row").forEach((item, itemIndex) => {
        item.classList.toggle("is-on", itemIndex === selected);
      });
    });
    results.append(button);
  });
  results.querySelector(".chat-search-row.is-on")?.scrollIntoView({ block: "nearest" });
}

function clickIfClosed(selector: string, alreadyOpen: boolean): void {
  if (alreadyOpen) return;
  document.querySelector<HTMLButtonElement>(selector)?.click();
}

function openProject(name: string): void {
  const doc = chatState.workspaceDoc;
  const inProject = chats().filter((session) => sessionWorkspaceName(session, doc) === name);
  const latest = inProject[0];
  if (latest) {
    chatState.sessions.switchSession(latest.id);
    return;
  }
  chatState.sessions.startNewSession(name);
}

function activate(row: Row): void {
  closeSearch();
  if (row.kind === "chat") {
    chatState.sessions.switchSession(row.id);
    return;
  }
  if (row.kind === "project") {
    openProject(row.name);
    return;
  }
  if (row.id === "knowledge") clickIfClosed("#chat-knowledge-open", isKnowledgePageOpen());
  else if (row.id === "schedule") clickIfClosed("#chat-schedule-open", isSchedulePageOpen());
  else if (row.id === "skills") clickIfClosed("#chat-resources-toggle", isResourcesMainPageOpen());
  else document.querySelector<HTMLButtonElement>("#chat-new-workspace")?.click();
}

function setScope(next: Scope): void {
  scope = next;
  selected = 0;
  document.querySelectorAll<HTMLButtonElement>("[data-search-scope]").forEach((button) => {
    const on = button.dataset.searchScope === next;
    button.classList.toggle("is-on", on);
    button.setAttribute("aria-selected", on ? "true" : "false");
  });
  paint();
}

export function closeSearch(): void {
  const layer = layerEl();
  if (!layer || layer.hidden) return;
  layer.hidden = true;
  openEl()?.setAttribute("aria-expanded", "false");
}

function openSearch(): void {
  const layer = layerEl();
  const input = inputEl();
  if (!layer || !input) return;
  layer.hidden = false;
  openEl()?.setAttribute("aria-expanded", "true");
  input.placeholder = t("chat.searchPlaceholder");
  selected = 0;
  paint();
  input.focus();
  input.select();
}

function toggleSearch(): void {
  if (layerEl()?.hidden === false) closeSearch();
  else openSearch();
}

function move(step: number): void {
  if (rows.length === 0) return;
  selected = (selected + step + rows.length) % rows.length;
  paint();
}

export function bindSidebarSearch(): void {
  const kbd = document.querySelector("#chat-search-kbd");
  if (kbd) kbd.textContent = shortcutLabel();
  openEl()?.addEventListener("click", () => toggleSearch());
  layerEl()?.addEventListener("click", (event) => {
    if (event.target === layerEl()) closeSearch();
  });
  document.querySelector(".chat-search-scopes")?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-search-scope]");
    const next = button?.dataset.searchScope;
    if (next === "all" || next === "projects" || next === "chats") setScope(next);
  });
  inputEl()?.addEventListener("input", () => {
    selected = 0;
    paint();
  });
  layerEl()?.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeSearch();
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      move(1);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      move(-1);
      return;
    }
    if (event.key === "Enter") {
      const row = rows[selected];
      if (!row) return;
      event.preventDefault();
      activate(row);
    }
  });
  window.addEventListener("keydown", (event) => {
    if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "k") return;
    if (event.altKey || event.shiftKey) return;
    event.preventDefault();
    toggleSearch();
  });
}
