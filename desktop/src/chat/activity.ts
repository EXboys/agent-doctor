import { explainChatFailure, formatChatFailureLine } from "../friendly-error";
import type { ChatFailureExplain } from "../friendly-error";
import { getLocale, t } from "../i18n";
import {
  activityKind,
  isQuietPhase,
  isQuietStderr,
  mergeToolStep,
  readRowStep,
  rowActivityInfo,
  splitToolActivity,
  summarizeToolActivities,
  toolSignature,
  toolStepText,
} from "./format";
import { clearEmptyChatStart } from "./empty-start";
import { rememberFileChange } from "./file-changes";
import { fileLinkRoot } from "./file-links";
import type { ToolStep } from "./types";
import { pushChatTurnError } from "./turn-errors";
import { placeProcessBlock, sealWorkTrail } from "./work-trail";

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
  /** Tool notes saved since the latest user message. */
  toolRecordsForTurn: () => ToolRecord[];
  /** First critical connection failure this turn (GLM / geo block). */
  onChatConnectionFailure?: (explain: ChatFailureExplain) => void;
};

export type ActivityApi = ReturnType<typeof createActivityController>;

let turnStartedAt = 0;

/** The live row counts from here, the way the agent's own window does. */
export function markTurnStarted(): void {
  turnStartedAt = Date.now();
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

/** Folder of a file, relative to the project when it is inside it. */
function shortFolder(path: string): string {
  const root = fileLinkRoot().replace(/\/+$/, "");
  let rel = path;
  if (root && root !== "—" && path.startsWith(`${root}/`)) rel = path.slice(root.length + 1);
  const cut = rel.lastIndexOf("/");
  return cut > 0 ? `${rel.slice(0, cut)}/` : "";
}

function paintDiffLines(box: HTMLElement, deleted: string[], added: string[]): void {
  box.replaceChildren();
  const add = (kind: "delete" | "add", text: string) => {
    const line = document.createElement("div");
    line.className = "chat-tool-diff-line";
    line.dataset.kind = kind;
    line.textContent = `${kind === "add" ? "+" : "−"} ${text}`;
    box.append(line);
  };
  for (const text of deleted) add("delete", text);
  for (const text of added) add("add", text);
}

export function applyToolRow(row: HTMLElement, text: string, step?: ToolStep): void {
  if (text) {
    const { summary, detail } = splitToolActivity(text);
    row.dataset.summary = summary;
    row.dataset.detail = detail;
    row.dataset.signature = toolSignature(text);
  }
  if (step) {
    row.dataset.step = JSON.stringify(step);
    row.dataset.toolId = step.id;
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
  if (current) {
    const head = current.command ? `$ ${current.command}` : (current.query ?? "");
    if (!fileAction || info.kind === "read") {
      body = [info.kind === "read" ? "" : head, current.output ?? ""].filter(Boolean).join("\n\n");
    }
  } else {
    const legacy = row.dataset.detail ?? "";
    const marker = legacy.indexOf("\n@@diff\n");
    body = (marker < 0 ? legacy : legacy.slice(0, marker)).trim();
    if (fileAction && info.addedLines.length + info.deletedLines.length > 0) body = "";
  }
  if (cmd) {
    cmd.textContent = body;
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
  row.innerHTML = `<summary class="chat-tool-row-summary"><span class="chat-tool-step" aria-hidden="true"></span><span class="chat-tool-name chat-activity-text"></span><button type="button" class="chat-tool-preview" hidden></button><span class="chat-tool-folder" hidden></span><span class="chat-tool-changes" hidden></span><span class="chat-tool-status" hidden></span><span class="chat-tool-chevron" hidden aria-hidden="true"></span></summary><pre class="chat-tool-cmd" hidden></pre><div class="chat-tool-diff" hidden></div>`;
  row.querySelector("summary")?.addEventListener("click", (event) => {
    if (!row.dataset.expandable) event.preventDefault();
  });
  applyToolRow(row, text, step);
  return row;
}

export type ToolRecord = string | { text: string; step?: ToolStep };

/** Collapsed history of one turn's tool calls. Click a row for the command. */
export function renderToolHistoryGroup(records: ToolRecord[]): HTMLDetailsElement {
  const group = document.createElement("details");
  group.className = "chat-tool-group";
  group.open = true;
  const count = records.length;
  const summary = document.createElement("summary");
  summary.className = "chat-tool-group-summary";
  summary.innerHTML = `<span class="chat-tool-group-icon" aria-hidden="true">$</span><span class="chat-tool-group-label"></span><span class="chat-tool-group-chevron" aria-hidden="true"></span>`;
  const label = summary.querySelector<HTMLElement>(".chat-tool-group-label");
  if (label) {
    label.textContent = t("chat.permissionGroupTools", { count: String(count) });
  }
  const list = document.createElement("div");
  list.className = "chat-tool-list";
  for (const record of records) {
    list.appendChild(
      typeof record === "string"
        ? createToolRowElement(record)
        : createToolRowElement(record.text, record.step),
    );
  }
  group.append(summary, list);
  return group;
}

export function createActivityController(deps: ActivityDeps) {
  window.setInterval(() => {
    const label = elapsedLabel();
    for (const el of deps.logEl.querySelectorAll<HTMLElement>(
      ".chat-activity.is-live > .chat-activity-elapsed, .chat-thinking.is-live .chat-activity-elapsed, .chat-work.is-live > .chat-work-summary .chat-activity-elapsed",
    )) {
      el.textContent = label;
    }
  }, 1000);

  /** Remove the transient lifecycle row once a more meaningful event replaces it. */
  function dismissLifecycleActivity(): void {
    const lifecycle = deps.getLifecycleActivityEl();
    if (!lifecycle) return;
    if (deps.getActivityEl() === lifecycle) deps.setActivityEl(null);
    lifecycle.remove();
    deps.setLifecycleActivityEl(null);
  }

  function updateToolGroupSummary(group: HTMLDetailsElement, live: boolean): void {
    const rows = group.querySelectorAll<HTMLElement>(
      ".chat-tool-list > .chat-tool-row, .chat-permission-stack > .chat-tool-row",
    );
    const label = group.querySelector<HTMLElement>(".chat-tool-group-label");
    if (!label) return;
    const summary = summarizeToolActivities([...rows].map(rowActivityInfo));
    label.textContent =
      summary || (live
        ? t("chat.toolsLive", { count: String(rows.length) })
        : t("chat.permissionGroupTools", { count: String(rows.length) }));
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
      last.classList.add("is-live");
      last.open = true;
      updateToolGroupSummary(last, true);
      return last;
    }
    const group = document.createElement("details");
    group.className = "chat-tool-group is-live";
    group.open = true;
    group.innerHTML = `
    <summary class="chat-tool-group-summary">
      <span class="chat-tool-group-icon" aria-hidden="true">$</span>
      <span class="chat-tool-group-label"></span>
      <span class="chat-tool-group-chevron" aria-hidden="true"></span>
    </summary>
    <div class="chat-tool-list"></div>
  `;
    placeProcessBlock(deps.logEl, group);
    deps.setToolGroupEl(group);
    updateToolGroupSummary(group, true);
    return group;
  }

  function finishToolGroup(_collapse = true): void {
    const group = deps.getToolGroupEl();
    if (!group) return;
    const activity = deps.getActivityEl();
    if (activity && group.contains(activity)) settleActivity();
    updateToolGroupSummary(group, false);
    group.open = true;
    deps.setToolGroupEl(null);
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
    sealWorkTrail(deps.logEl);
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
  }

  function isToolGroupEl(el: Element | null): el is HTMLElement {
    return Boolean(
      el?.classList.contains("chat-tool-group") || el?.classList.contains("chat-turn-tools"),
    );
  }

  /** If the live chip was dropped, put the saved tool list back above the reply. */
  function ensureTurnToolsVisible(lastAssistant: HTMLElement | null, seen: ToolRecord[] = []): void {
    const filled = (record: ToolRecord) =>
      typeof record === "string" ? Boolean(record.trim()) : Boolean(record.step || record.text.trim());
    const stored = deps.toolRecordsForTurn().filter(filled);
    const texts = stored.length > 0 ? stored : seen.filter(filled);
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
    const stray = [...parent.querySelectorAll<HTMLElement>(":scope > .chat-tool-group, :scope > .chat-turn-tools")].find(
      (group) => group !== block.previousElementSibling,
    );
    if (stray) {
      parent.insertBefore(stray, block);
      if (stray instanceof HTMLDetailsElement) stray.open = true;
      return;
    }
    if (texts.length === 0) return;
    parent.insertBefore(renderToolHistoryGroup(texts), block);
  }

  function appendStderrLine(line: string): void {
    if (!deps.isViewingRunningSession()) return;
    const text = line.trim();
    if (!text || isQuietStderr(text)) return;
    pushChatTurnError(text);
    const explain = explainChatFailure(text);
    const display = explain ? formatChatFailureLine(explain) : text;
    if (
      explain &&
      (explain.kind === "glm_codex_url" || explain.kind === "geo_blocked") &&
      deps.onChatConnectionFailure
    ) {
      deps.onChatConnectionFailure(explain);
    }
    const last = deps.logEl.lastElementChild as HTMLElement | null;
    if (last?.dataset.kind === "log" && last.dataset.stderr === "1") {
      const label = last.querySelector<HTMLElement>(".chat-activity-text");
      if (label) {
        label.textContent = `${label.textContent}\n${display}`;
        deps.logEl.scrollTop = deps.logEl.scrollHeight;
        return;
      }
    }
    const row = document.createElement("div");
    row.className = "chat-activity kind-log";
    row.dataset.kind = "log";
    row.dataset.stderr = "1";
    const label = document.createElement("span");
    label.className = "chat-activity-text";
    label.textContent = display;
    row.appendChild(label);
    deps.logEl.appendChild(row);
    deps.logEl.scrollTop = deps.logEl.scrollHeight;
  }

  /** Render progress / tool calls inline in the chat stream (not a side panel). */
  function pushActivity(phase: string, message: string): void {
    const text = message.trim() || phase;
    if (!text) return;
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
        deps.logEl.scrollTop = deps.logEl.scrollHeight;
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
      const group = known.closest<HTMLDetailsElement>(".chat-tool-group");
      if (group) updateToolGroupSummary(group, group === deps.getToolGroupEl());
      if (step.status !== "running" && deps.getActivityEl() === known) settleActivity();
      return;
    }
    if (!step.kind) return;
    const group = deps.getToolGroupEl();
    const last = group?.isConnected
      ? group.querySelector<HTMLElement>(".chat-tool-list > .chat-tool-row:last-child")
      : null;
    const row = last && !last.dataset.toolId ? last : showToolRow("tool", toolStepText(step));
    if (!row) return;
    applyToolRow(row, "", step);
    const owner = row.closest<HTMLDetailsElement>(".chat-tool-group");
    if (owner) updateToolGroupSummary(owner, true);
  }

  function showToolRow(phase: string, text: string): HTMLElement | null {
    if (!deps.isViewingRunningSession()) return null;
    dismissLifecycleActivity();
    deps.flushPendingTextSync();
    deps.sealAssistantBubble();

    const group = ensureToolGroup();
    const list = group.querySelector<HTMLElement>(".chat-tool-list")!;
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
      updateToolGroupSummary(group, true);
      return last;
    }

    settleActivity();
    const row = createToolRowElement(text);
    row.classList.add("is-live");
    row.classList.remove("is-done");
    row.dataset.phase = phase;
    list.appendChild(row);
    deps.setActivityEl(row);
    updateToolGroupSummary(group, true);
    deps.logEl.scrollTop = deps.logEl.scrollHeight;
    return row;
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
      settleActivity();
      const row = document.createElement("div");
      row.className = `chat-activity is-live kind-${kind}`;
      row.dataset.phase = phase;
      row.dataset.kind = kind;
      row.innerHTML = `<span class="chat-spinner" aria-hidden="true"></span><span class="chat-activity-text"></span><span class="chat-activity-elapsed"></span>`;
      row.querySelector<HTMLElement>(".chat-activity-elapsed")!.textContent = elapsedLabel();
      const label = row.querySelector<HTMLElement>(".chat-activity-text")!;
      label.textContent = text;
      deps.logEl.appendChild(row);
      deps.setActivityEl(row);
      if (kind !== "error") deps.setLifecycleActivityEl(row);
    } else if (activity) {
      activity.dataset.phase = phase;
      activity.dataset.kind = kind;
      activity.className = `chat-activity is-live kind-${kind}`;
      const label = activity.querySelector<HTMLElement>(".chat-activity-text");
      if (label) label.textContent = text;
    }
    deps.logEl.scrollTop = deps.logEl.scrollHeight;
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
    clearEphemeralActivity,
    appendStderrLine,
    pushActivity,
    pushToolStep,
    settleActivity,
  };
}
