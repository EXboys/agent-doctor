import { rowActivityInfo, stepActivityInfo, toolActivityInfo, type ToolActivityInfo } from "./format";
import { t } from "../i18n";
import type { ChatMessage } from "./types";

export type ReplyFileChange = {
  path: string;
  name: string;
  additions: number;
  deletions: number;
  addedLines: string[];
  deletedLines: string[];
  lineStart?: number;
  lineEnd?: number;
};

function fileName(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] || path;
}

function absorb(byPath: Map<string, ReplyFileChange>, info: ToolActivityInfo): void {
  if ((info.kind !== "write" && info.kind !== "edit") || !info.path.trim()) return;
  const path = info.path.trim();
  const seen = byPath.get(path) ?? {
    path,
    name: fileName(path),
    additions: 0,
    deletions: 0,
    addedLines: [],
    deletedLines: [],
  };
  seen.additions += info.additions;
  seen.deletions += info.deletions;
  if (info.addedLines.length) seen.addedLines.push(...info.addedLines);
  if (info.deletedLines.length) seen.deletedLines.push(...info.deletedLines);
  if (info.lineStart) seen.lineStart = info.lineStart;
  if (info.lineEnd) seen.lineEnd = info.lineEnd;
  byPath.set(path, seen);
}

function fileInfo(info: ToolActivityInfo): ToolActivityInfo | null {
  if ((info.kind === "write" || info.kind === "edit") && info.path.trim()) return info;
  return null;
}

function infoForToolMessage(message: ChatMessage): ToolActivityInfo | null {
  if (message.step && message.step.status !== "failed") {
    const fromStep = fileInfo(stepActivityInfo(message.step));
    if (fromStep) return fromStep;
  }
  if (message.content.trim()) {
    const fromText = fileInfo(toolActivityInfo(message.content));
    if (fromText) return fromText;
  }
  if (message.step?.command) return fileInfo(toolActivityInfo(message.step.command));
  return null;
}

/** Files written or edited after the latest user message. A new message starts a fresh turn, so the previous card goes away. */
export function latestReplyFileChanges(messages: readonly ChatMessage[]): ReplyFileChange[] {
  let start = 0;
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (messages[i]?.role === "user") {
      start = i + 1;
      break;
    }
  }
  const byPath = new Map<string, ReplyFileChange>();
  for (let i = start; i < messages.length; i += 1) {
    const message = messages[i];
    if (message?.role !== "tool") continue;
    const info = infoForToolMessage(message);
    if (info) absorb(byPath, info);
  }
  return [...byPath.values()];
}

/** The folded process row already knows the file, even when the saved step does not. */
function visibleReplyFileChanges(logEl: HTMLElement): ReplyFileChange[] {
  const children = [...logEl.children];
  let start = 0;
  for (let i = children.length - 1; i >= 0; i -= 1) {
    if (children[i]?.classList.contains("chat-msg-user")) {
      start = i + 1;
      break;
    }
  }
  const byPath = new Map<string, ReplyFileChange>();
  for (const child of children.slice(start)) {
    const rows = child.classList.contains("chat-tool-row")
      ? [child as HTMLElement]
      : [...child.querySelectorAll<HTMLElement>(".chat-tool-row")];
    for (const row of rows) absorb(byPath, rowActivityInfo(row));
  }
  return [...byPath.values()];
}

function openFile(file: ReplyFileChange): void {
  window.dispatchEvent(
    new CustomEvent("chat-open-workspace-file", {
      detail: {
        path: file.path,
        additions: file.additions,
        deletions: file.deletions,
        addedLines: file.addedLines,
        deletedLines: file.deletedLines,
        lineStart: file.lineStart,
        lineEnd: file.lineEnd,
      },
    }),
  );
}

const PREVIEW_FILES = 5;

function renderReplyChangeCard(files: ReplyFileChange[], open: boolean): HTMLElement {
  const card = document.createElement("section");
  card.className = "chat-reply-changes";
  const head = document.createElement("div");
  head.className = "chat-reply-changes-head";
  const title = document.createElement("span");
  title.textContent =
    files.length === 1 ? t("chat.replyFileOne") : t("chat.replyFileMany", { count: String(files.length) });
  const review = document.createElement("button");
  review.type = "button";
  review.className = "chat-reply-changes-review";
  review.textContent = t("chat.replyFileReview");
  review.addEventListener("click", () => {
    const first = files[0];
    if (first) openFile(first);
  });
  head.append(title, review);
  card.append(head);
  const extra = files.length - PREVIEW_FILES;
  files.forEach((file, index) => {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "chat-reply-changes-file";
    row.title = file.path;
    const mark = document.createElement("span");
    mark.className = "chat-reply-changes-mark";
    mark.textContent = "#";
    mark.setAttribute("aria-hidden", "true");
    const name = document.createElement("span");
    name.className = "chat-reply-changes-name";
    name.textContent = file.name;
    row.append(mark, name);
    if (file.additions || file.deletions) {
      const stats = document.createElement("span");
      stats.className = "chat-reply-changes-stats";
      const add = document.createElement("span");
      add.className = "chat-reply-changes-add";
      add.textContent = `+${file.additions}`;
      const del = document.createElement("span");
      del.className = "chat-reply-changes-del";
      del.textContent = `-${file.deletions}`;
      stats.append(add, del);
      row.append(stats);
    }
    row.addEventListener("click", () => openFile(file));
    if (index >= PREVIEW_FILES) {
      row.classList.add("is-extra");
      row.hidden = !open;
    }
    card.append(row);
  });
  if (extra > 0) {
    const more = document.createElement("button");
    more.type = "button";
    more.className = "chat-reply-changes-more";
    more.setAttribute("aria-expanded", open ? "true" : "false");
    const paint = (shown: boolean) => {
      more.textContent = shown
        ? t("chat.replyFileFold")
        : t("chat.replyFileMore", { count: String(extra) });
      more.setAttribute("aria-expanded", shown ? "true" : "false");
      card.classList.toggle("is-open", shown);
      card.querySelectorAll<HTMLElement>(".chat-reply-changes-file.is-extra").forEach((row) => {
        row.hidden = !shown;
      });
    };
    paint(open);
    more.addEventListener("click", () => paint(!card.classList.contains("is-open")));
    card.append(more);
  }
  return card;
}

/** One card at the end of the latest reply. Hidden while that reply is still running, so it does not jump through the transcript. */
export function mountReplyChangeCard(
  logEl: HTMLElement,
  messages: readonly ChatMessage[],
  opts?: { running?: boolean },
): void {
  const wasOpen = logEl.querySelector(".chat-reply-changes")?.classList.contains("is-open") ?? false;
  logEl.querySelector(".chat-reply-changes")?.remove();
  if (opts?.running) return;
  const fromMessages = latestReplyFileChanges(messages);
  const files = fromMessages.length ? fromMessages : visibleReplyFileChanges(logEl);
  if (!files.length) return;
  const card = renderReplyChangeCard(files, wasOpen);
  logEl.append(card);
}
