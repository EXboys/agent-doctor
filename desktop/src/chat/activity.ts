import { explainChatFailure, formatChatFailureLine } from "../friendly-error";
import type { ChatFailureExplain } from "../friendly-error";
import { getLocale, t } from "../i18n";
import {
  activityKind,
  isQuietPhase,
  isQuietStderr,
  splitToolActivity,
  toolSignature,
} from "./format";
import { clearEmptyChatStart } from "./empty-start";
import { pushChatTurnError } from "./turn-errors";

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
  /** Tool notes saved since the latest user message. */
  toolRecordsForTurn: () => string[];
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

export function applyToolRow(row: HTMLElement, text: string): void {
  const { summary, detail } = splitToolActivity(text);
  row.dataset.summary = summary;
  row.dataset.detail = detail;
  row.dataset.signature = toolSignature(text);
  row.dataset.kind = "tool";
  const name = row.querySelector<HTMLElement>(".chat-tool-name");
  const preview = row.querySelector<HTMLElement>(".chat-tool-preview");
  const cmd = row.querySelector<HTMLElement>(".chat-tool-cmd");
  const chevron = row.querySelector<HTMLElement>(".chat-tool-chevron");
  if (name) name.textContent = summary;
  const firstLine = detail.split("\n")[0]?.trim() ?? "";
  if (preview) {
    preview.textContent = firstLine;
    preview.hidden = !firstLine;
  }
  if (cmd) {
    cmd.textContent = detail;
    cmd.hidden = !detail;
  }
  if (chevron) chevron.hidden = !detail;
  if (!detail && row instanceof HTMLDetailsElement) row.open = false;
}

export function createToolRowElement(text: string): HTMLDetailsElement {
  const row = document.createElement("details");
  row.className = "chat-activity is-done kind-tool chat-tool-row";
  row.innerHTML = `<summary class="chat-tool-row-summary"><span class="chat-tool-step" aria-hidden="true"></span><span class="chat-tool-name chat-activity-text"></span><span class="chat-tool-preview" hidden></span><span class="chat-tool-chevron" hidden aria-hidden="true"></span></summary><pre class="chat-tool-cmd" hidden></pre>`;
  row.querySelector("summary")?.addEventListener("click", (event) => {
    if (!row.dataset.detail) event.preventDefault();
  });
  applyToolRow(row, text);
  return row;
}

/** Collapsed history of one turn's tool calls. Click a row for the command. */
export function renderToolHistoryGroup(texts: string[]): HTMLDetailsElement {
  const group = document.createElement("details");
  group.className = "chat-tool-group";
  group.open = false;
  const count = texts.length;
  const summary = document.createElement("summary");
  summary.className = "chat-tool-group-summary";
  summary.innerHTML = `<span class="chat-tool-group-icon" aria-hidden="true">$</span><span class="chat-tool-group-label"></span><span class="chat-tool-group-chevron" aria-hidden="true"></span>`;
  const label = summary.querySelector<HTMLElement>(".chat-tool-group-label");
  if (label) {
    label.textContent =
      getLocale() === "zh"
        ? t("chat.permissionGroupTools", { count: String(count) })
        : `${count} tool${count === 1 ? "" : "s"} used`;
  }
  const list = document.createElement("div");
  list.className = "chat-tool-list";
  for (const text of texts) list.appendChild(createToolRowElement(text));
  group.append(summary, list);
  return group;
}

export function createActivityController(deps: ActivityDeps) {
  window.setInterval(() => {
    const label = elapsedLabel();
    for (const el of deps.logEl.querySelectorAll<HTMLElement>(
      ".chat-activity.is-live > .chat-activity-elapsed, .chat-thinking.is-live .chat-activity-elapsed",
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
    const count = group.querySelectorAll(
      ".chat-tool-list > .chat-tool-row, .chat-permission-stack > .chat-tool-row",
    ).length;
    const label = group.querySelector<HTMLElement>(".chat-tool-group-label");
    if (!label) return;
    if (getLocale() === "zh") {
      label.textContent = live ? `正在调用工具 · ${count}` : `已调用 ${count} 个工具`;
    } else {
      label.textContent = live
        ? `Using tools · ${count}`
        : `${count} tool${count === 1 ? "" : "s"} used`;
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
      last.classList.add("is-live");
      updateToolGroupSummary(last, true);
      return last;
    }
    const group = document.createElement("details");
    group.className = "chat-tool-group is-live";
    group.open = false;
    group.innerHTML = `
    <summary class="chat-tool-group-summary">
      <span class="chat-tool-group-icon" aria-hidden="true">$</span>
      <span class="chat-tool-group-label"></span>
      <span class="chat-tool-group-chevron" aria-hidden="true"></span>
    </summary>
    <div class="chat-tool-list"></div>
  `;
    deps.logEl.appendChild(group);
    deps.setToolGroupEl(group);
    updateToolGroupSummary(group, true);
    return group;
  }

  function finishToolGroup(collapse = true): void {
    const group = deps.getToolGroupEl();
    if (!group) return;
    const activity = deps.getActivityEl();
    if (activity && group.contains(activity)) settleActivity();
    updateToolGroupSummary(group, false);
    if (collapse) group.open = false;
    deps.setToolGroupEl(null);
  }

  function toolTextsInLog(): string[] {
    const texts: string[] = [];
    deps.logEl.querySelectorAll<HTMLElement>(".chat-tool-row, .chat-activity.kind-tool").forEach((row) => {
      const summary = row.dataset.summary || row.querySelector(".chat-tool-name")?.textContent || "";
      const detail = row.dataset.detail || row.querySelector(".chat-tool-cmd")?.textContent || "";
      const text = detail.trim() ? `${summary.trim()}\n${detail.trim()}` : summary.trim();
      if (text) texts.push(text);
    });
    return texts;
  }

  /** Drop ephemeral progress rows so they don't litter the transcript. */
  function clearEphemeralActivity(dropStderr = false): void {
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
  function ensureTurnToolsVisible(lastAssistant: HTMLElement | null, seen: string[] = []): void {
    const stored = deps.toolRecordsForTurn().filter((text) => text.trim());
    const texts = stored.length > 0 ? stored : seen.filter((text) => text.trim());
    if (!lastAssistant) {
      if (texts.length === 0 || deps.logEl.querySelector(":scope > .chat-tool-group")) return;
      deps.logEl.appendChild(renderToolHistoryGroup(texts));
      return;
    }
    const block = lastAssistant.closest(".chat-msg-assistant") ?? lastAssistant;
    const parent = block.parentElement;
    if (!parent) return;
    const previous = block.previousElementSibling;
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
      if (stray instanceof HTMLDetailsElement) stray.open = false;
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
      if (!deps.isViewingRunningSession()) return;
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
        last.dataset.summary === parts.summary &&
        !last.dataset.detail &&
        Boolean(parts.detail);
      if (last && (last.dataset.signature === signature || canFillDetail)) {
        if (canFillDetail) applyToolRow(last, text);
        last.classList.add("is-live");
        last.classList.remove("is-done");
        deps.setActivityEl(last);
        updateToolGroupSummary(group, true);
        return;
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
    settleActivity,
  };
}
