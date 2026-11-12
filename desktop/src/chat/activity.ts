import { perfMark } from "./perf-probe";
import { explainChatFailure } from "../friendly-error";
import type { ChatFailureExplain } from "../friendly-error";
import { getLocale, t } from "../i18n";
import {
  activityKind,
  appendRunLog,
  HAND_MISSING_PHASE,
  handMissingNotice,
  isMachineToolDump,
  isQuietPhase,
  isRuntimeLog,
  isTransportDump,
  mergeToolStep,
  stripLogAnsi,
  readRowStep,
  repeatRuns,
  rowActivityInfo,
  splitToolActivity,
  type ToolActivityInfo,
  toolSignature,
  toolStepText,
} from "./format";
import { clearEmptyChatStart } from "./empty-start";
import { rememberFileChange } from "./file-changes";
import { interleaveLines } from "./file-diff";
import { fileLinkRoot } from "./file-links";
import type { ToolStep } from "./types";
import {
  clipStepForPage,
  earlierToolRange,
  laterToolRange,
  MAX_SHOWN_TOOL_ROWS,
  MAX_TOOL_DOM_ROWS,
  toolRecordKey,
  turnSegmentRecords,
  type ToolRecord,
} from "./tool-records";
import { markTurnNoticeShown, pushChatTurnError, pushChatTurnReport } from "./turn-errors";
import {
  placeProcessBlock,
  sealProcessContainer,
  sealWorkGroup,
  wrapProcessRows,
} from "./work-trail";

const PAGE_LOG_CHARS = 8_000;

function capPageText(text: string): string {
  return text.length > PAGE_LOG_CHARS ? `${text.slice(0, PAGE_LOG_CHARS)}\n…` : text;
}

export type ActivityDeps = {
  logEl: HTMLElement;
  isViewingRunningSession: () => boolean;
  flushPendingTextSync: () => void;
  sealAssistantBubble: () => void;
  collapseResolvedPermissionsBeforeAssistant: (anchor: HTMLElement) => void;
  getActivityEl: () => HTMLElement | null;
  setActivityEl: (el: HTMLElement | null) => void;
  getLifecycleActivityEl: () => HTMLElement | null;
  setLifecycleActivityEl: (el: HTMLElement | null) => void;
  getToolGroupEl: () => HTMLDetailsElement | null;
  setToolGroupEl: (el: HTMLDetailsElement | null) => void;
  /** Keep the command on the session so it survives a redraw. */
  rememberTool: (text: string) => void;
  /** Keep a structured step on the session, merged by id. */
  rememberToolStep: (step: ToolStep) => void;
  /** Keep raw output on this turn's saved log so a redraw stays collapsed. */
  rememberRuntimeLog: (text: string) => void;
  /** Keep a sentence the runtime reported for a person. */
  rememberTurnNotice: (text: string) => void;
  /** Tool notes saved since the latest user message. */
  toolRecordsForTurn: () => ToolRecord[];
  /** Put the latest reply's file card back at the end after a new step lands. */
  syncReplyChangeCard?: () => void;
  /** First critical connection failure this turn (GLM / geo block). */
  onChatConnectionFailure?: (explain: ChatFailureExplain) => void;
};

export type ActivityApi = ReturnType<typeof createActivityController>;

let turnStartedAt = 0;
/** The first model request of this turn has already started the clock. */
let modelRequestMarked = false;
/** This thinking stretch. The turn clock keeps the earlier web and tool time. */
let thinkingStartedAt = 0;

const LIVE_ELAPSED =
  ".chat-activity.is-live > .chat-activity-elapsed, .chat-work.is-live > .chat-work-summary .chat-activity-elapsed, .chat-running:not([hidden]) .chat-running-time";
const THINKING_ELAPSED = ".chat-thinking.is-live .chat-activity-elapsed";

function paintLiveElapsed(): void {
  if (typeof document === "undefined" || typeof document.querySelectorAll !== "function") return;
  const label = elapsedLabel();
  const thinking = thinkingClockActive();
  for (const el of document.querySelectorAll<HTMLElement>(LIVE_ELAPSED)) {
    el.textContent = thinking && el.classList.contains("chat-running-time") ? thinkingElapsedLabel() : label;
  }
  paintThinkingElapsed();
}

function paintThinkingElapsed(): void {
  if (typeof document === "undefined" || typeof document.querySelectorAll !== "function") return;
  const label = thinkingElapsedLabel();
  for (const el of document.querySelectorAll<HTMLElement>(THINKING_ELAPSED)) el.textContent = label;
}

/** A new user message starts clean, so the previous turn's minutes do not carry over. */
export function markTurnStarted(): void {
  turnStartedAt = Date.now();
  modelRequestMarked = false;
  thinkingStartedAt = 0;
  paintLiveElapsed();
}

/**
 * The line the person just sent. If the module clock is still pointing at an
 * older run — the session had already stopped — this message is the start.
 */
export function noteTurnUserAt(at: number): void {
  if (!Number.isFinite(at) || at <= 0) return;
  if (turnStartedAt && at <= turnStartedAt) return;
  turnStartedAt = at;
  modelRequestMarked = false;
  thinkingStartedAt = 0;
  paintLiveElapsed();
}

/** No chat is running. A stopped session must not keep a start time. */
export function clearTurnClock(): void {
  turnStartedAt = 0;
  modelRequestMarked = false;
  thinkingStartedAt = 0;
  paintLiveElapsed();
}

/**
 * The clock used to start when the runtime process launched, so the model
 * request already showed that earlier time. The first request of this turn
 * starts the clock; a later request keeps counting.
 */
export function markModelRequest(): void {
  if (modelRequestMarked) return;
  modelRequestMarked = true;
  turnStartedAt = Date.now();
  paintLiveElapsed();
}

export function elapsedLabel(): string {
  if (!turnStartedAt) return "";
  return durationLabel(Date.now() - turnStartedAt);
}

export function durationLabel(ms: number): string {
  const sec = Math.floor(ms / 1000);
  if (sec < 1) return "";
  const zh = getLocale() === "zh";
  if (sec < 60) return zh ? `${sec} 秒` : `${sec}s`;
  const min = Math.floor(sec / 60);
  const rest = sec % 60;
  return zh ? `${min} 分 ${rest} 秒` : `${min}m ${rest}s`;
}

/** The open thinking block counts from its own first line, not from the turn. */
export function markThinkingStartedAt(at: number): void {
  thinkingStartedAt = at;
  paintThinkingElapsed();
}

export function clearThinkingStarted(): void {
  if (!thinkingStartedAt) return;
  thinkingStartedAt = 0;
  paintThinkingElapsed();
}

export function thinkingClockActive(): boolean {
  return thinkingStartedAt > 0;
}

export function thinkingElapsedLabel(): string {
  if (!thinkingStartedAt) return "";
  return durationLabel(Date.now() - thinkingStartedAt);
}

/** Folder of a file, relative to the project when it is inside it. */
function shortFolder(path: string): string {
  const root = fileLinkRoot().replace(/\/+$/, "");
  let rel = path;
  if (root && root !== "—" && path.startsWith(`${root}/`)) rel = path.slice(root.length + 1);
  const cut = rel.lastIndexOf("/");
  return cut > 0 ? `${rel.slice(0, cut)}/` : "";
}

/** `cat -n` and `N|text` dumps. A plain file is left as prose lines. */
function parseNumberedLine(line: string): { no: string; text: string } | null {
  const match = line.match(/^\s*(\d+)(?:\t|\s*\|)\s?(.*)$/);
  if (!match) return null;
  return { no: String(Number(match[1])), text: match[2] ?? "" };
}

/** One row per source line. A wrapped fragment with no number stays on the line above it. */
function readRows(body: string): { no: string; text: string }[] {
  const lines = body.replace(/\n$/, "").split("\n");
  const parsed = lines.map(parseNumberedLine);
  const numbered = parsed.filter(Boolean).length >= Math.max(2, Math.ceil(lines.length * 0.6));
  if (!numbered) return lines.map((text) => ({ no: "", text }));
  const rows: { no: string; text: string }[] = [];
  for (let i = 0; i < lines.length; i += 1) {
    const hit = parsed[i];
    if (hit) {
      rows.push({ no: hit.no, text: hit.text });
      continue;
    }
    const prev = rows[rows.length - 1];
    if (prev) prev.text += lines[i] ?? "";
    else rows.push({ no: "", text: lines[i] ?? "" });
  }
  return rows;
}

/** A read opens as lines of text. A command stays a single block of output. */
function paintToolBody(cmd: HTMLElement, body: string, asRead: boolean): void {
  const read = asRead && Boolean(body);
  cmd.classList.toggle("is-read", read);
  if (!read) {
    cmd.replaceChildren();
    cmd.textContent = body;
    return;
  }
  cmd.replaceChildren();
  for (const line of readRows(body)) {
    const row = document.createElement("div");
    row.className = "chat-tool-read-line";
    if (line.no) {
      const no = document.createElement("span");
      no.className = "chat-tool-read-no";
      no.textContent = line.no;
      row.append(no);
    }
    const text = document.createElement("span");
    text.className = "chat-tool-read-text";
    text.textContent = line.text;
    row.append(text);
    cmd.append(row);
  }
}

function paintDiffLines(box: HTMLElement, deleted: string[], added: string[]): void {
  box.replaceChildren();
  for (const line of interleaveLines(deleted, added)) {
    if (line.kind === "same") continue;
    const row = document.createElement("div");
    row.className = "chat-tool-diff-line";
    row.dataset.kind = line.kind;
    row.textContent = line.text.length ? line.text : " ";
    box.append(row);
  }
}

function paintSubagentFace(row: HTMLElement, info: ToolActivityInfo): void {
  const face = row.querySelector<HTMLElement>(".chat-subagent");
  if (!face) return;
  const on = info.kind === "subagent";
  face.hidden = !on;
  if (!on) return;
  const kind = face.querySelector<HTMLElement>(".chat-subagent-kind");
  const agent = face.querySelector<HTMLElement>(".chat-subagent-name");
  const task = face.querySelector<HTMLElement>(".chat-subagent-task");
  if (kind) kind.textContent = t("chat.subagentKind");
  if (agent) {
    agent.textContent = info.agentName ?? "";
    agent.hidden = !info.agentName;
  }
  if (task) {
    task.textContent = info.agentTask ?? "";
    task.hidden = !info.agentTask;
  }
}

export function applyToolRow(row: HTMLElement, text: string, step?: ToolStep): void {
  if (text) {
    const { summary, detail } = splitToolActivity(text);
    row.dataset.summary = summary;
    row.dataset.detail = capPageText(detail);
    row.dataset.signature = toolSignature(text);
  }
  if (step) {
    step = clipStepForPage(step);
    row.dataset.step = JSON.stringify(step);
    row.dataset.toolId = step.id;
    row.dataset.toolKey = `id:${step.id}`;
  }
  const current = readRowStep(row);
  const info = rowActivityInfo(row);
  row.dataset.kind = "tool";
  row.dataset.toolKind = info.kind;
  row.dataset.filePath = info.path;
  row.dataset.additions = String(info.additions);
  row.dataset.deletions = String(info.deletions);
  const name = row.querySelector<HTMLElement>(".chat-tool-name");
  const preview = row.querySelector<HTMLElement>(".chat-tool-preview");
  const subagent = info.kind === "subagent";
  row.classList.toggle("is-subagent", subagent);
  paintSubagentFace(row, info);
  const folder = row.querySelector<HTMLElement>(".chat-tool-folder");
  const cmd = row.querySelector<HTMLElement>(".chat-tool-cmd");
  const diff = row.querySelector<HTMLElement>(".chat-tool-diff");
  const changes = row.querySelector<HTMLElement>(".chat-tool-changes");
  const status = row.querySelector<HTMLElement>(".chat-tool-status");
  const chevron = row.querySelector<HTMLElement>(".chat-tool-chevron");
  if (name) name.textContent = info.label;
  if (info.path && (info.kind === "write" || info.kind === "edit")) {
    rememberFileChange(info.path, row.dataset.toolId || info.path, {
      additions: info.additions,
      deletions: info.deletions,
      addedLines: info.addedLines,
      deletedLines: info.deletedLines,
      lineStart: info.lineStart,
      lineEnd: info.lineEnd,
    });
  }
  const firstLine = info.target;
  const fileAction = info.kind === "read" || info.kind === "write" || info.kind === "edit";
  if (folder) {
    folder.textContent = fileAction && info.path ? shortFolder(info.path) : "";
    folder.hidden = !folder.textContent;
    folder.title = info.path;
  }
  if (preview) {
    preview.textContent = firstLine;
    preview.hidden = !firstLine;
    preview.classList.toggle("is-file", fileAction && Boolean(info.path));
    if (preview instanceof HTMLButtonElement) {
      preview.disabled = !fileAction || !info.path;
      if (preview.dataset.fileBound !== "1") {
        preview.dataset.fileBound = "1";
        preview.addEventListener("click", (event) => {
          event.preventDefault();
          event.stopPropagation();
          const current = rowActivityInfo(row);
          window.dispatchEvent(
            new CustomEvent("chat-open-workspace-file", {
              detail: {
                path: current.path,
                lineStart: current.lineStart,
                lineEnd: current.lineEnd,
                additions: current.additions,
                deletions: current.deletions,
                addedLines: current.addedLines,
                deletedLines: current.deletedLines,
              },
            }),
          );
        });
      }
    }
  }
  if (changes) {
    changes.replaceChildren();
    if (info.additions) {
      const add = document.createElement("span");
      add.className = "chat-tool-add";
      add.textContent = `+${info.additions}`;
      changes.append(add);
    }
    if (info.deletions) {
      const del = document.createElement("span");
      del.className = "chat-tool-del";
      del.textContent = `−${info.deletions}`;
      changes.append(del);
    }
    changes.hidden = !info.additions && !info.deletions;
  }

  let body = "";
  if (subagent) {
    body = "";
  } else if (current) {
    const head = current.command ? `$ ${current.command}` : (current.query ?? "");
    if (!fileAction || info.kind === "read") {
      const output = current.output && !isMachineToolDump(current.output) ? current.output : "";
      body = [info.kind === "read" ? "" : head, output].filter(Boolean).join("\n\n");
    }
  } else {
    const legacy = row.dataset.detail ?? "";
    const marker = legacy.indexOf("\n@@diff\n");
    body = (marker < 0 ? legacy : legacy.slice(0, marker)).trim();
    if (fileAction && info.addedLines.length + info.deletedLines.length > 0) body = "";
  }
  if (cmd) {
    paintToolBody(cmd, body, info.kind === "read");
    cmd.hidden = !body;
  }
  const hasDiff = fileAction && info.kind !== "read" && info.addedLines.length + info.deletedLines.length > 0;
  if (diff) {
    if (hasDiff) paintDiffLines(diff, info.deletedLines, info.addedLines);
    else diff.replaceChildren();
    diff.hidden = !hasDiff;
  }
  const failed = current?.status === "failed";
  row.classList.toggle("is-failed", failed);
  if (status) {
    status.textContent = failed ? t("chat.toolFailed") : "";
    status.hidden = !failed;
  }
  if (current) {
    const running = current.status === "running";
    row.classList.toggle("is-live", running);
    row.classList.toggle("is-done", !running);
  }
  const expandable = Boolean(body) || hasDiff;
  row.dataset.expandable = expandable ? "1" : "";
  if (chevron) chevron.hidden = !expandable;
  if (!expandable && row instanceof HTMLDetailsElement) row.open = false;
}

export function createToolRowElement(text: string, step?: ToolStep): HTMLDetailsElement {
  const row = document.createElement("details");
  row.className = "chat-activity is-done kind-tool chat-tool-row";
  row.innerHTML = `<summary class="chat-tool-row-summary"><span class="chat-tool-step" aria-hidden="true"></span><span class="chat-tool-name chat-activity-text"></span><span class="chat-subagent" hidden><span class="chat-subagent-mark" aria-hidden="true"></span><span class="chat-subagent-kind"></span><span class="chat-subagent-name" hidden></span><span class="chat-subagent-task" hidden></span></span><span class="chat-tool-repeat" hidden></span><button type="button" class="chat-tool-preview" hidden></button><span class="chat-tool-folder" hidden></span><span class="chat-tool-changes" hidden></span><span class="chat-tool-status" hidden></span><button type="button" class="chat-tool-copy" hidden title=""></button><span class="chat-tool-chevron" hidden aria-hidden="true"></span></summary><pre class="chat-tool-cmd" hidden></pre><div class="chat-tool-diff" hidden></div>`;
  row.querySelector("summary")?.addEventListener("click", (event) => {
    if (!row.dataset.expandable) event.preventDefault();
  });
  bindToolRowCopy(row, text);
  applyToolRow(row, text, step);
  return row;
}

/** Full stored output for one row, for the copy button. The page body shows
 * a clipped slice; this keeps what the person would collect. */
const fullToolOutputs = new WeakMap<HTMLElement, string>();

function bindToolRowCopy(row: HTMLDetailsElement, text: string): void {
  if (text.trim().length < 400) return;
  fullToolOutputs.set(row, text);
  const button = row.querySelector<HTMLButtonElement>(".chat-tool-copy");
  if (!button) return;
  button.hidden = false;
  button.title = t("chat.toolCopyOutput");
  button.textContent = t("chat.toolCopyOutput");
  button.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    const full = fullToolOutputs.get(row) ?? "";
    void navigator.clipboard
      .writeText(full)
      .then(() => {
        button.textContent = t("chat.copied");
        window.setTimeout(() => {
          button.textContent = t("chat.toolCopyOutput");
        }, 1200);
      })
      .catch(() => {});
  });
}

export { toolRecordKey, type ToolRecord } from "./tool-records";

/**
 * A tool group is bookkeeping, not a fold: it holds one segment's steps for the
 * scroll pager and shows them as flat rows. The work trail around it folds.
 */
const heldToolRecords = new WeakMap<HTMLDetailsElement, ToolRecord[]>();
let ignoreLogScroll = 0;

function holdToolRecords(group: HTMLDetailsElement, records: ToolRecord[]): void {
  heldToolRecords.set(group, records);
}

function mountToolRecord(record: ToolRecord): HTMLDetailsElement {
  const row = typeof record === "string" ? createToolRowElement(record) : createToolRowElement(record.text, record.step);
  row.dataset.toolKey = toolRecordKey(record);
  return row;
}

/** Where this row sits in the saved list. Repeated blank rows share a key, so the key alone is not a position. */
function recordIndexOf(row: HTMLElement, records: ToolRecord[]): number {
  const raw = row.dataset.recordIndex;
  if (raw) {
    const index = Number(raw);
    const key = row.dataset.toolKey || "";
    if (Number.isInteger(index) && index >= 0 && index < records.length && toolRecordKey(records[index]!) === key) {
      return index;
    }
  }
  const key = row.dataset.toolKey || "";
  return records.findIndex((record) => toolRecordKey(record) === key);
}

function stampRecord(row: HTMLElement, index: number): void {
  row.dataset.recordIndex = String(index);
}

function withoutLogJump(work: () => void): void {
  ignoreLogScroll += 1;
  work();
  queueMicrotask(() => {
    ignoreLogScroll = Math.max(0, ignoreLogScroll - 1);
  });
}

/** Programmatic scroll should not look like the reader left the latest line. */
export function withoutChatLogJump(work: () => void): void {
  withoutLogJump(work);
}

const CHAT_ROW = new Set([
  "chat-msg",
  "chat-work",
  "chat-work-stream",
  "chat-thinking",
  "chat-tool-group",
  "chat-bubble",
  "chat-activity",
  "chat-permission",
  "chat-reply-changes",
  "chat-plan",
]);
const sizedRows = new WeakSet<HTMLElement>();
const watchedLogs = new WeakSet<HTMLElement>();

function isChatRow(node: HTMLElement): boolean {
  for (const name of node.classList) {
    if (CHAT_ROW.has(name)) return true;
  }
  return false;
}

/**
 * Record each row's real height so an off-screen stand-in matches it.
 * Rows already recorded are left alone unless `refresh` is set.
 */
export function lockChatRowHeights(nodes: ArrayLike<Element>, refresh = false): void {
  perfMark("row-measure");
  const pending: HTMLElement[] = [];
  for (let i = 0; i < nodes.length; i += 1) {
    const node = nodes[i];
    if (!(node instanceof HTMLElement) || !node.isConnected || !isChatRow(node)) continue;
    if (!refresh && sizedRows.has(node)) continue;
    if (!refresh && node.querySelector(".is-streaming")) continue;
    pending.push(node);
  }
  if (pending.length === 0) return;
  for (const node of pending) node.style.setProperty("content-visibility", "visible");
  for (const node of pending) {
    const height = node.getBoundingClientRect().height;
    if (height > 0) {
      node.style.setProperty("contain-intrinsic-block-size", `auto ${height}px`);
      sizedRows.add(node);
    }
  }
  for (const node of pending) node.style.removeProperty("content-visibility");
}

/** Measure rows the first time they join the transcript, including ones appended while scrolled up. */
function watchChatLogRowHeights(logEl: HTMLElement): void {
  if (watchedLogs.has(logEl)) return;
  watchedLogs.add(logEl);
  const observer = new MutationObserver((records) => {
    const added: HTMLElement[] = [];
    for (const record of records) {
      for (const node of record.addedNodes) {
        if (node instanceof HTMLElement) added.push(node);
      }
    }
    if (added.length > 0) lockChatRowHeights(added);
  });
  observer.observe(logEl, { childList: true });
}

/** Jump to the latest line. Opening a chat or sending a message starts here. */
export function pinChatLog(logEl: HTMLElement): void {
  lockChatRowHeights(logEl.children);
  logEl.dataset.pin = "1";
  withoutLogJump(() => {
    logEl.scrollTop = logEl.scrollHeight;
  });
}

/** Stay put when the reader has scrolled up to look at earlier steps. */
export function followChatLog(logEl: HTMLElement): void {
  if (logEl.dataset.pin === "0") return;
  pinChatLog(logEl);
}

/** Let a jump to an earlier message hold its ground while the tail keeps streaming. */
export function releaseChatFollow(logEl: HTMLElement): void {
  logEl.dataset.pin = "0";
}

/**
 * The holder for one segment's rows. Always open, with its summary hidden: a
 * closed details would hide the rows, and a details without a summary grows a
 * default disclosure marker.
 */
function createToolGroupElement(): HTMLDetailsElement {
  const group = document.createElement("details");
  group.className = "chat-tool-group";
  group.open = true;
  group.innerHTML = `<summary hidden></summary><div class="chat-tool-list"></div>`;
  group.addEventListener("toggle", () => {
    // A manual open wins: stop auto-collapsing this group for the rest of the turn.
    if (group.open && group.classList.contains("is-live")) group.dataset.openPinned = "1";
  });
  return group;
}

/** One segment's tool calls, a row each, in order. Click a row for the command. */
export function renderToolHistoryGroup(records: ToolRecord[]): HTMLDetailsElement {
  const group = createToolGroupElement();
  holdToolRecords(group, records);
  const list = group.querySelector<HTMLElement>(".chat-tool-list")!;
  const page = records.slice(-MAX_SHOWN_TOOL_ROWS);
  const offset = records.length - page.length;
  page.forEach((record, index) => {
    const row = mountToolRecord(record);
    stampRecord(row, offset + index);
    list.appendChild(row);
  });
  foldRepeatedToolRows(list);
  return group;
}

function rowFace(row: HTMLElement): string {
  const name = row.querySelector(".chat-tool-name")?.textContent ?? "";
  const preview = row.querySelector<HTMLElement>(".chat-tool-preview");
  const shown = preview && !preview.hidden ? (preview.textContent ?? "") : "";
  const failed = row.classList.contains("is-failed") ? "failed" : "";
  const live = row.classList.contains("is-live") ? "live" : "";
  return [live, failed, name, shown].join("\n");
}

/** Consecutive rows that say the same thing become one line with a count. */
export function foldRepeatedToolRows(list: HTMLElement): void {
  const rows = [...list.querySelectorAll<HTMLElement>(":scope > .chat-tool-row")];
  const counts = repeatRuns(rows.map(rowFace));
  rows.forEach((row, index) => {
    const count = counts[index] ?? 0;
    row.hidden = count === 0;
    const badge = row.querySelector<HTMLElement>(".chat-tool-repeat");
    if (!badge) return;
    if (count > 1) {
      badge.hidden = false;
      badge.textContent = t("chat.toolTimes", { count: String(count) });
    } else {
      badge.hidden = true;
      badge.textContent = "";
    }
  });
}

export function createActivityController(deps: ActivityDeps) {
  watchChatLogRowHeights(deps.logEl);
  window.setInterval(() => {
    const label = elapsedLabel();
    for (const el of deps.logEl.querySelectorAll<HTMLElement>(
      ".chat-activity.is-live > .chat-activity-elapsed, .chat-work.is-live > .chat-work-summary .chat-activity-elapsed",
    )) {
      el.textContent = label;
    }
    const thinking = thinkingElapsedLabel();
    for (const el of deps.logEl.querySelectorAll<HTMLElement>(THINKING_ELAPSED)) el.textContent = thinking;
  }, 1000);

  /** Remove the transient lifecycle row once a more meaningful event replaces it. */
  function dismissLifecycleActivity(): void {
    const lifecycle = deps.getLifecycleActivityEl();
    if (!lifecycle) return;
    if (deps.getActivityEl() === lifecycle) deps.setActivityEl(null);
    lifecycle.remove();
    deps.setLifecycleActivityEl(null);
  }

  /** Every tool chip since the latest user message, including those folded into a work trail. */
  function turnToolGroups(): HTMLDetailsElement[] {
    const groups: HTMLDetailsElement[] = [];
    for (
      let node = deps.logEl.lastElementChild as HTMLElement | null;
      node && !node.classList.contains("chat-msg-user");
      node = node.previousElementSibling as HTMLElement | null
    ) {
      if (node.classList.contains("chat-tool-group")) groups.push(node as HTMLDetailsElement);
      else groups.push(...node.querySelectorAll<HTMLDetailsElement>(".chat-tool-group"));
    }
    return groups;
  }

  /** This turn's steps that no chip other than `own` already shows: the chip after the latest reply. */
  function segmentRecords(own: HTMLDetailsElement | null): ToolRecord[] {
    const shown: string[] = [];
    for (const group of turnToolGroups()) {
      if (group === own) continue;
      for (const record of heldToolRecords.get(group) ?? []) shown.push(toolRecordKey(record));
    }
    return turnSegmentRecords(deps.toolRecordsForTurn(), shown);
  }

  function liveRecords(group: HTMLDetailsElement): ToolRecord[] {
    const records = segmentRecords(group);
    holdToolRecords(group, records);
    return records;
  }

  function recordsFor(group: HTMLDetailsElement): ToolRecord[] {
    if (group === deps.getToolGroupEl()) return liveRecords(group);
    return heldToolRecords.get(group) ?? [];
  }

  /** The on-screen list still ends at the newest step, or the one just before it. */
  function windowShowsTail(records: ToolRecord[], list: HTMLElement): boolean {
    const last = list.querySelector<HTMLElement>(":scope > .chat-tool-row:last-child");
    if (!last || records.length === 0) return true;
    const index = recordIndexOf(last, records);
    if (index >= 0) return index >= records.length - 2;
    return false;
  }

  function dropRow(row: HTMLElement): void {
    if (deps.getActivityEl() === row) deps.setActivityEl(null);
    row.remove();
  }

  /** While following the task, keep only the latest page of rows. */
  function trimLiveToolRows(group: HTMLDetailsElement): void {
    const list = group.querySelector<HTMLElement>(".chat-tool-list");
    if (!list) return;
    const rows = [...list.querySelectorAll<HTMLElement>(":scope > .chat-tool-row")];
    const extra = rows.length - MAX_SHOWN_TOOL_ROWS;
    if (extra <= 0) return;
    for (const row of rows.slice(0, extra)) dropRow(row);
  }

  /** Drop rows that sit outside the chat, so a long scroll never paints every step.
   * One layout read for the whole batch: interleaving reads with removals used
   * to reflow per row. */
  function dropOffscreenRows(list: HTMLElement, edge: "top" | "bottom"): void {
    const log = deps.logEl;
    const logRect = log.getBoundingClientRect();
    const rows = [...list.querySelectorAll<HTMLElement>(":scope > .chat-tool-row")];
    const extra = rows.length - MAX_TOOL_DOM_ROWS;
    if (extra <= 0) return;
    const victims = edge === "top" ? rows.slice(0, extra) : rows.slice(-extra).reverse();
    const rects = victims.map((row) => row.getBoundingClientRect());
    let shift = 0;
    let count = 0;
    for (let i = 0; i < victims.length; i += 1) {
      const rect = rects[i]!;
      const outside =
        edge === "top" ? rect.bottom < logRect.top + 4 : rect.top > logRect.bottom - 4;
      if (!outside) break;
      shift += rect.height;
      count += 1;
    }
    if (count === 0) return;
    for (let i = 0; i < count; i += 1) dropRow(victims[i]!);
    if (edge === "top" && shift) {
      withoutLogJump(() => {
        log.scrollTop = Math.max(0, log.scrollTop - shift);
      });
    }
  }

  /** One page at a time. A scroll event must not stack another page before the first settles. */
  let toolPageLock = 0;

  function mountedRecordIndexes(list: HTMLElement): Set<string> {
    const mounted = new Set<string>();
    for (const row of list.querySelectorAll<HTMLElement>(":scope > .chat-tool-row")) {
      if (row.dataset.recordIndex) mounted.add(row.dataset.recordIndex);
    }
    return mounted;
  }

  function releaseToolPage(): void {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        toolPageLock = Math.max(0, toolPageLock - 1);
      });
    });
  }

  /**
   * Keep the reader on the row they were looking at. Setting scrollTop inside
   * the scroll event is ignored on Windows, so the new rows stayed on screen
   * and the next event loaded another page.
   */
  function settleToolPage(beforeHeight: number, beforeTop: number): void {
    const log = deps.logEl;
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        const delta = log.scrollHeight - beforeHeight;
        if (delta) {
          withoutLogJump(() => {
            log.scrollTop = beforeTop + delta;
          });
        }
        requestAnimationFrame(() => {
          toolPageLock = Math.max(0, toolPageLock - 1);
        });
      });
    });
  }

  function revealEarlier(group: HTMLDetailsElement): void {
    perfMark("tool-page");
    if (toolPageLock > 0) return;
    const records = recordsFor(group);
    const list = group.querySelector<HTMLElement>(".chat-tool-list");
    const anchor = list?.querySelector<HTMLElement>(":scope > .chat-tool-row");
    if (!list || !anchor?.dataset.toolKey) return;
    const start = recordIndexOf(anchor, records);
    if (start < 0) return;
    const range = earlierToolRange(start);
    if (!range) return;
    const mounted = mountedRecordIndexes(list);
    const indexes: number[] = [];
    for (let i = range.from; i < range.to; i += 1) {
      if (!mounted.has(String(i))) indexes.push(i);
    }
    if (indexes.length === 0) return;
    toolPageLock += 1;
    const log = deps.logEl;
    const beforeHeight = log.scrollHeight;
    const beforeTop = log.scrollTop;
    for (let n = indexes.length - 1; n >= 0; n -= 1) {
      const i = indexes[n]!;
      const row = mountToolRecord(records[i]!);
      stampRecord(row, i);
      list.insertBefore(row, list.firstChild);
    }
    dropOffscreenRows(list, "bottom");
    syncToolGroup(group, group === deps.getToolGroupEl());
    settleToolPage(beforeHeight, beforeTop);
  }

  function revealLater(group: HTMLDetailsElement): void {
    if (toolPageLock > 0) return;
    const records = recordsFor(group);
    const list = group.querySelector<HTMLElement>(".chat-tool-list");
    const last = list?.querySelector<HTMLElement>(":scope > .chat-tool-row:last-child");
    if (!list || !last?.dataset.toolKey) return;
    const lastIndex = recordIndexOf(last, records);
    if (lastIndex < 0) return;
    const range = laterToolRange(lastIndex, records.length);
    if (!range) return;
    const mounted = mountedRecordIndexes(list);
    const indexes: number[] = [];
    for (let i = range.from; i < range.to; i += 1) {
      if (!mounted.has(String(i))) indexes.push(i);
    }
    if (indexes.length === 0) return;
    toolPageLock += 1;
    for (const i of indexes) {
      const row = mountToolRecord(records[i]!);
      stampRecord(row, i);
      list.appendChild(row);
    }
    dropOffscreenRows(list, "top");
    syncToolGroup(group, group === deps.getToolGroupEl());
    releaseToolPage();
  }

  // Streaming pins the log every frame; coalesce scroll work to one run per
  // frame. Page reveals wait for the scroll to settle: mounting pages
  // mid-scroll (and compensating scrollTop so the reader holds still) made a
  // long transcript impossible to scroll — the wheel raced the pages, and
  // every race round re-rendered markdown and reflowed the log. While the
  // wheel is moving the mounted pages are enough; the edges fill in when it
  // settles.
  const SCROLL_SETTLE_MS = 150;
  const GROUP_REVEAL_COOLDOWN_MS = 250;
  const groupRevealAt = new WeakMap<HTMLDetailsElement, number>();
  let logScrollRaf = 0;
  let logScrollIdleTimer = 0;
  deps.logEl.addEventListener(
    "scroll",
    () => {
      if (ignoreLogScroll > 0) return;
      if (logScrollRaf) return;
      logScrollRaf = window.requestAnimationFrame(() => {
        logScrollRaf = 0;
        const log = deps.logEl;
        const gap = log.scrollHeight - log.scrollTop - log.clientHeight;
        log.dataset.pin = gap < 64 ? "1" : "0";
        if (toolPageLock > 0) return;
        if (logScrollIdleTimer) window.clearTimeout(logScrollIdleTimer);
        logScrollIdleTimer = window.setTimeout(() => {
          logScrollIdleTimer = 0;
          revealAtEdges();
        }, SCROLL_SETTLE_MS);
      });
    },
    { passive: true },
  );

  /** Fill pages for every group whose list edge sits in a trigger band. One
   * page per pass; a per-group cooldown keeps a band the reader keeps
   * hovering from paging again and again. */
  function revealAtEdges(): void {
    if (toolPageLock > 0) return;
    const log = deps.logEl;
    const logRect = log.getBoundingClientRect();
    for (const group of log.querySelectorAll<HTMLDetailsElement>(".chat-tool-group")) {
      if (!group.open || toolPageLock > 0) break;
      const list = group.querySelector<HTMLElement>(".chat-tool-list");
      if (!list) continue;
      const rect = list.getBoundingClientRect();
      if (rect.height < 1) continue;
      const nearTop = rect.top > logRect.top - 24 && rect.top < logRect.top + 200;
      const nearBottom = rect.bottom > logRect.bottom - 200 && rect.bottom < logRect.bottom + 24;
      if (!nearTop && !nearBottom) {
        groupRevealAt.delete(group);
        continue;
      }
      const now = performance.now();
      const last = groupRevealAt.get(group) ?? 0;
      if (now - last < GROUP_REVEAL_COOLDOWN_MS) continue;
      groupRevealAt.set(group, now);
      if (nearTop) revealEarlier(group);
      if (toolPageLock === 0 && nearBottom) revealLater(group);
    }
  }

  /** A live group past this many visible rows folds into its summary bar. */
  const LIVE_GROUP_COLLAPSE_AFTER = 6;

  /** Fold repeated rows and mark whether the group is still taking steps. */
  function syncToolGroup(group: HTMLDetailsElement, live: boolean): void {
    const list = group.querySelector<HTMLElement>(".chat-tool-list");
    if (list) {
      foldRepeatedToolRows(list);
      const summary = group.querySelector<HTMLElement>("summary");
      if (live && summary) {
        // Long tool runs (the built-in doctor alternates every round) used to
        // paint one wall of rows until the reply arrived; fold them instead.
        const rows = list.querySelectorAll<HTMLElement>(":scope > .chat-tool-row:not([hidden])").length;
        const pinned = group.dataset.openPinned === "1";
        if (rows > LIVE_GROUP_COLLAPSE_AFTER && !pinned) {
          summary.hidden = false;
          summary.textContent = t("chat.workLiveCount", { count: String(rows) });
          if (group.open) group.open = false;
        } else if (rows <= LIVE_GROUP_COLLAPSE_AFTER && !pinned) {
          // Too few rows to fold: keep the bare list, as CLI turns always had.
          summary.hidden = true;
          if (!group.open) group.open = true;
        }
      }
    }
    group.classList.toggle("is-live", live);
  }

  function ensureToolGroup(): HTMLDetailsElement {
    const existing = deps.getToolGroupEl();
    if (existing?.isConnected) return existing;
    // Reuse a trailing unfinished tool chip instead of stacking duplicate rows.
    const last = deps.logEl.lastElementChild as HTMLElement | null;
    if (
      last instanceof HTMLDetailsElement &&
      last.classList.contains("chat-tool-group") &&
      !last.classList.contains("chat-permission-group") &&
      !last.classList.contains("chat-turn-tools")
    ) {
      deps.setToolGroupEl(last);
      last.open = true;
      syncToolGroup(last, true);
      return last;
    }
    const group = createToolGroupElement();
    group.classList.add("is-live");
    placeProcessBlock(processStreamEl(), group);
    deps.setToolGroupEl(group);
    return group;
  }

  function finishToolGroup(_collapse = true): void {
    const group = deps.getToolGroupEl();
    if (!group) return;
    liveRecords(group);
    const activity = deps.getActivityEl();
    if (activity && group.contains(activity)) settleActivity();
    syncToolGroup(group, false);
    deps.setToolGroupEl(null);
    sealWorkGroup(group);
  }

  function toolTextsInLog(): ToolRecord[] {
    const records: ToolRecord[] = [];
    deps.logEl.querySelectorAll<HTMLElement>(".chat-tool-row, .chat-activity.kind-tool").forEach((row) => {
      const summary = row.dataset.summary || row.querySelector(".chat-tool-name")?.textContent || "";
      const detail = row.dataset.detail || row.querySelector(".chat-tool-cmd")?.textContent || "";
      const text = detail.trim() ? `${summary.trim()}\n${detail.trim()}` : summary.trim();
      const step = readRowStep(row);
      if (step) records.push({ text, step });
      else if (text) records.push(text);
    });
    return records;
  }

  /** Drop ephemeral progress rows so they don't litter the transcript. */
  function clearEphemeralActivity(dropStderr = false): void {
    sealProcessStream();
    const keptTools = toolTextsInLog();
    settleActivity();
    finishToolGroup(false);
    for (const row of deps.logEl.querySelectorAll<HTMLElement>(".chat-activity")) {
      const kind = row.dataset.kind ?? "";
      if (kind === "tool" || row.closest(".chat-tool-group")) continue;
      if (kind === "log" && row.dataset.stderr === "1") continue;
      if (kind === "error" && !(dropStderr && row.dataset.stderr === "1")) continue;
      row.remove();
    }
    deps.setLifecycleActivityEl(null);
    const assistants = deps.logEl.querySelectorAll<HTMLElement>(".chat-bubble.assistant");
    const lastAssistant = assistants[assistants.length - 1];
    if (lastAssistant) deps.collapseResolvedPermissionsBeforeAssistant(lastAssistant);
    ensureTurnToolsVisible(lastAssistant ?? null, keptTools);
    deps.syncReplyChangeCard?.();
  }

  function isToolGroupEl(el: Element | null): el is HTMLElement {
    return Boolean(
      el?.classList.contains("chat-tool-group") || el?.classList.contains("chat-turn-tools"),
    );
  }

  /** If the live chip was dropped, put the saved tool list back above the reply. */
  function ensureTurnToolsVisible(lastAssistant: HTMLElement | null, seen: ToolRecord[] = []): void {
    // Interleaved rendering already places each tool row at its chronological
    // spot (chat-work rows under the log). The re-insert is only for the old
    // shape where the whole reply rendered as one bubble with no rows.
    if (deps.logEl.querySelector(":scope > .chat-tool-group, :scope > .chat-work")) return;
    const filled = (record: ToolRecord) =>
      typeof record === "string" ? Boolean(record.trim()) : Boolean(record.step || record.text.trim());
    // Only the steps no chip shows yet; the chips above earlier replies keep theirs.
    // The rows read off the page stand in only when this turn lost its chips.
    const stored = segmentRecords(null).filter(filled);
    const texts = stored.length > 0 || turnToolGroups().length > 0 ? stored : seen.filter(filled);
    if (!lastAssistant) {
      if (texts.length === 0 || deps.logEl.querySelector(":scope > .chat-tool-group, :scope > .chat-work")) return;
      deps.logEl.appendChild(renderToolHistoryGroup(texts));
      return;
    }
    const block = lastAssistant.closest(".chat-msg-assistant") ?? lastAssistant;
    const parent = block.parentElement;
    if (!parent) return;
    const previous = block.previousElementSibling;
    if (
      previous?.classList.contains("chat-work") ||
      previous?.classList.contains("chat-work-stream")
    ) {
      return;
    }
    if (isToolGroupEl(previous)) {
      const hasRows = previous.querySelector(".chat-tool-row, .chat-activity.kind-tool");
      if (hasRows || texts.length === 0) return;
      previous.remove();
    }
    // Only this turn's top-level chips count as stray; earlier turns keep theirs.
    const stray = turnToolGroups()
      .filter((group) => group.parentElement === parent && group !== block.previousElementSibling)
      .pop();
    if (stray) {
      parent.insertBefore(stray, block);
      // A plain group must stay open or its rows vanish; only the permission fold closes.
      if (stray.classList.contains("chat-turn-tools")) stray.open = false;
      return;
    }
    if (texts.length === 0) return;
    parent.insertBefore(renderToolHistoryGroup(texts), block);
  }

  /** A connection the runtime keeps retrying: say so now instead of after the retries. */
  function noteConnectionFailure(text: string): void {
    const explain = explainChatFailure(text);
    if (
      explain &&
      (explain.kind === "glm_codex_url" ||
        explain.kind === "geo_blocked" ||
        explain.kind === "session_expired" ||
        explain.kind === "account_model") &&
      deps.onChatConnectionFailure
    ) {
      deps.onChatConnectionFailure(explain);
    }
  }

  /** Raw process output: kept in this turn's collapsed log, never shown as a reply or a verdict. */
  function appendStderrLine(line: string): void {
    const text = stripLogAnsi(line).trim();
    if (!text) return;
    pushChatTurnError(text);
    deps.rememberRuntimeLog(text);
    if (!deps.isViewingRunningSession()) return;
    noteConnectionFailure(text);
    appendRunLog(deps.logEl, text);
    followChatLog(deps.logEl);
  }

  /** The runtime's own reason for a failed turn. Words already written for a person are shown. */
  function appendTurnError(message: string, plain: boolean): void {
    const text = stripLogAnsi(message).trim();
    if (!text) return;
    pushChatTurnReport(text);
    if (!plain) {
      appendStderrLine(text);
      return;
    }
    markTurnNoticeShown();
    deps.rememberTurnNotice(text);
    if (!deps.isViewingRunningSession()) return;
    const row = document.createElement("div");
    row.className = "chat-activity kind-log";
    row.dataset.kind = "log";
    row.innerHTML = `<span class="chat-activity-icon" aria-hidden="true"></span><span class="chat-activity-text"></span>`;
    row.querySelector<HTMLElement>(".chat-activity-text")!.textContent = text;
    deps.logEl.appendChild(row);
    followChatLog(deps.logEl);
  }

  /** One plain sentence in the chat; the original error stays in the collapsed log. */
  function showHandMissing(message: string): void {
    const { sentence, detail } = handMissingNotice(message);
    deps.rememberTurnNotice(sentence);
    if (detail) deps.rememberRuntimeLog(detail);
    if (!deps.isViewingRunningSession()) return;
    clearEmptyChatStart(deps.logEl);
    const row = document.createElement("div");
    row.className = "chat-activity kind-log";
    row.dataset.kind = "log";
    row.innerHTML = `<span class="chat-activity-icon" aria-hidden="true"></span><span class="chat-activity-text"></span>`;
    row.querySelector<HTMLElement>(".chat-activity-text")!.textContent = sentence;
    deps.logEl.appendChild(row);
    if (detail) appendRunLog(deps.logEl, detail);
    followChatLog(deps.logEl);
  }

  /** Render progress / tool calls inline in the chat stream (not a side panel). */
  function pushActivity(phase: string, message: string): void {
    if (phase === HAND_MISSING_PHASE) {
      showHandMissing(message);
      return;
    }
    const text = message.trim() || phase;
    if (!text) return;
    // A status that is really a log or an HTTP body stays in the collapsed record.
    if (
      phase !== "tool" &&
      phase !== "command" &&
      phase !== "permission" &&
      (isRuntimeLog(text) || isTransportDump(text))
    ) {
      appendStderrLine(text);
      return;
    }
    if (deps.isViewingRunningSession()) clearEmptyChatStart(deps.logEl);

    // The permission card that follows carries this state and its resolution.
    if (phase === "permission") return;

    // Quiet lifecycle chatter — skip.
    if (isQuietPhase(phase) || phase === "writing") return;

    const kind = activityKind(phase);

    if (kind === "tool") {
      deps.rememberTool(text);
      showToolRow(phase, text);
      return;
    }

    if (!deps.isViewingRunningSession()) return;

    // Waiting/requesting/thinking are one evolving state, not transcript entries.
    if (kind !== "error") {
      const lifecycle = deps.getLifecycleActivityEl();
      if (lifecycle?.isConnected) {
        lifecycle.dataset.phase = phase;
        lifecycle.className = `chat-activity is-live kind-${kind}`;
        const label = lifecycle.querySelector<HTMLElement>(".chat-activity-text");
        if (label) label.textContent = text;
        deps.setActivityEl(lifecycle);
        followChatLog(deps.logEl);
        return;
      }
    } else {
      dismissLifecycleActivity();
    }
    appendActivityRow(phase, kind, text);
  }

  /** Structured step: fill the row with the same id, or start a new one. */
  /** The status line for a tool arrives first; its step then takes over that row. */
  function pushToolStep(step: ToolStep): void {
    deps.rememberToolStep(step);
    if (!deps.isViewingRunningSession()) return;
    const known = [...deps.logEl.querySelectorAll<HTMLElement>(".chat-tool-row")].find(
      (row) => row.dataset.toolId === step.id,
    );
    if (known) {
      applyToolRow(known, "", mergeToolStep(readRowStep(known), step));
      if (step.status !== "running" && deps.getActivityEl() === known) settleActivity();
      deps.syncReplyChangeCard?.();
      return;
    }
    if (!step.kind) {
      deps.syncReplyChangeCard?.();
      return;
    }
    // Flat, time-ordered: every tool row appends directly to the container.
    const row = createToolRowElement(toolStepText(step));
    row.dataset.kind = "tool";
    row.dataset.toolId = step.id;
    row.classList.add("is-live");
    row.classList.remove("is-done");
    applyToolRow(row, "", step);
    processStreamEl().appendChild(row);
    deps.syncReplyChangeCard?.();
  }

  function showToolRow(phase: string, text: string): HTMLElement | null {
    if (!deps.isViewingRunningSession()) return null;
    dismissLifecycleActivity();
    deps.flushPendingTextSync();
    deps.sealAssistantBubble();

    const group = ensureToolGroup();
    const list = group.querySelector<HTMLElement>(".chat-tool-list")!;
    liveRecords(group);
    const parts = splitToolActivity(text);
    const signature = toolSignature(text);
    const last = list.querySelector<HTMLElement>(".chat-activity.kind-tool:last-child");
    const canFillDetail =
      last &&
      !last.dataset.toolId &&
      last.dataset.summary === parts.summary &&
      !last.dataset.detail &&
      Boolean(parts.detail);
    if (last && !last.dataset.toolId && (last.dataset.signature === signature || canFillDetail)) {
      if (canFillDetail) applyToolRow(last, text);
      last.classList.add("is-live");
      last.classList.remove("is-done");
      deps.setActivityEl(last);
      syncToolGroup(group, true);
      if (!last.dataset.toolKey) last.dataset.toolKey = `t:${signature}`;
      return last;
    }

    const records = liveRecords(group);
    if (deps.logEl.dataset.pin === "0" && !windowShowsTail(records, list)) {
      syncToolGroup(group, true);
      return null;
    }

    settleActivity();
    const row = createToolRowElement(text);
    row.classList.add("is-live");
    row.classList.remove("is-done");
    row.dataset.phase = phase;
    row.dataset.toolKey = `t:${signature}`;
    stampRecord(row, Math.max(0, records.length - 1));
    list.appendChild(row);
    if (deps.logEl.dataset.pin !== "0") trimLiveToolRows(group);
    else dropOffscreenRows(list, "top");
    deps.setActivityEl(row);
    syncToolGroup(group, true);
    followChatLog(deps.logEl);
    deps.syncReplyChangeCard?.();
    return row;
  }

  /** Every process row of this turn (status/tool/thinking) collects here and
   *  folds once at completion. Rows stay FLAT and time-ordered: no nested
   *  tool groups, so thinking and tools interleave like ZCode. */
  let processStream: HTMLElement | null = null;

  /** Only fold on completion: a running turn shows every step live, never
   *  collapsed mid-turn. */

  function processStreamEl(): HTMLElement {
    if (!processStream?.isConnected) {
      processStream = wrapProcessRows([], true);
    }
    // The stream follows the newest text segment: re-appending moves it below
    // the bubble that was just sealed, so the next rows continue the timeline
    // instead of jumping above an earlier paragraph.
    placeProcessBlock(deps.logEl, processStream);
    return processStream;
  }



  function placeInProcessStream(el: HTMLElement): void {
    processStreamEl().append(el);
  }

  /** Turn done: fold the whole process container into one collapsed trail. */


  function sealProcessStream(): void {
    if (!processStream?.isConnected) {
      processStream = null;
      return;
    }
    sealProcessContainer(processStream);
    processStream = null;
  }

  function appendActivityRow(phase: string, kind: string, text: string): void {
    const softPhase = kind === "write";
    const activity = deps.getActivityEl();
    const shouldAppend =
      !activity ||
      (!softPhase && (activity.dataset.kind === "tool" || activity.dataset.phase !== phase));

    if (shouldAppend) {
      deps.flushPendingTextSync();
      deps.sealAssistantBubble();
      // The text segment sealed above closes this tool container: the next
      // reply segment opens a fresh one after it, keeping rows chronological.
      sealProcessStream();
      settleActivity();
      const row = document.createElement("div");
      row.className = `chat-activity is-live kind-${kind}`;
      row.dataset.phase = phase;
      row.dataset.kind = kind;
      row.innerHTML = `<span class="chat-spinner" aria-hidden="true"></span><span class="chat-activity-icon" aria-hidden="true"></span><span class="chat-activity-text"></span><span class="chat-activity-elapsed"></span>`;
      row.querySelector<HTMLElement>(".chat-activity-elapsed")!.textContent = elapsedLabel();
      const label = row.querySelector<HTMLElement>(".chat-activity-text")!;
      label.textContent = text;
      processStreamEl().appendChild(row);
      deps.setActivityEl(row);
      if (kind !== "error") deps.setLifecycleActivityEl(row);
    } else if (activity) {
      activity.dataset.phase = phase;
      activity.dataset.kind = kind;
      activity.className = `chat-activity is-live kind-${kind}`;
      const label = activity.querySelector<HTMLElement>(".chat-activity-text");
      if (label) label.textContent = text;
    }
    followChatLog(deps.logEl);
  }

  function settleActivity(): void {
    const activity = deps.getActivityEl();
    if (!activity) return;
    activity.classList.remove("is-live");
    activity.classList.add("is-done");
    const spinner = activity.querySelector(".chat-spinner");
    spinner?.remove();
    deps.setActivityEl(null);
  }

  return {
    dismissLifecycleActivity,
    finishToolGroup,
    sealProcessStream,
    placeInProcessStream,
    clearEphemeralActivity,
    appendStderrLine,
    appendTurnError,
    pushActivity,
    pushToolStep,
    settleActivity,
  };
}
