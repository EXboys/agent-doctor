import { getLocale } from "../i18n";
import {
  activityKind,
  cleanToolLabel,
  isQuietPhase,
  isQuietStderr,
  toolSignature,
} from "./format";

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
};

export type ActivityApi = ReturnType<typeof createActivityController>;

export function createActivityController(deps: ActivityDeps) {
  /** Remove the transient lifecycle row once a more meaningful event replaces it. */
  function dismissLifecycleActivity(): void {
    const lifecycle = deps.getLifecycleActivityEl();
    if (!lifecycle) return;
    if (deps.getActivityEl() === lifecycle) deps.setActivityEl(null);
    lifecycle.remove();
    deps.setLifecycleActivityEl(null);
  }

  function updateToolGroupSummary(group: HTMLDetailsElement, live: boolean): void {
    const count = group.querySelectorAll(".chat-activity.kind-tool").length;
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
      last.open = true;
      last.classList.add("is-live");
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

  /** Drop ephemeral progress rows so they don't litter the transcript. */
  function clearEphemeralActivity(dropStderr = false): void {
    settleActivity();
    finishToolGroup(true);
    for (const row of deps.logEl.querySelectorAll<HTMLElement>(".chat-activity")) {
      const kind = row.dataset.kind ?? "";
      if (kind === "tool") continue;
      if (kind === "error" && !(dropStderr && row.dataset.stderr === "1")) continue;
      row.remove();
    }
    deps.setLifecycleActivityEl(null);
    const assistants = deps.logEl.querySelectorAll<HTMLElement>(".chat-bubble.assistant");
    const lastAssistant = assistants[assistants.length - 1];
    if (lastAssistant) deps.collapseResolvedPermissionsBeforeAssistant(lastAssistant);
  }

  function appendStderrLine(line: string): void {
    if (!deps.isViewingRunningSession()) return;
    const text = line.trim();
    if (!text || isQuietStderr(text)) return;
    const last = deps.logEl.lastElementChild as HTMLElement | null;
    if (last?.dataset.kind === "error" && last.dataset.stderr === "1") {
      const label = last.querySelector<HTMLElement>(".chat-activity-text");
      if (label) {
        label.textContent = `${label.textContent}\n${text}`;
        deps.logEl.scrollTop = deps.logEl.scrollHeight;
        return;
      }
    }
    pushActivity("error", text);
    settleActivity();
    const row = deps.logEl.lastElementChild as HTMLElement | null;
    if (row?.dataset.kind === "error") {
      row.dataset.stderr = "1";
    }
  }

  /** Render progress / tool calls inline in the chat stream (not a side panel). */
  function pushActivity(phase: string, message: string): void {
    if (!deps.isViewingRunningSession()) return;
    const text = message.trim() || phase;
    if (!text) return;

    // The permission card that follows carries this state and its resolution.
    if (phase === "permission") return;

    // Quiet lifecycle chatter — skip.
    if (isQuietPhase(phase) || phase === "writing") return;

    const kind = activityKind(phase);

    if (kind === "tool") {
      dismissLifecycleActivity();
      deps.flushPendingTextSync();
      deps.sealAssistantBubble();

      const group = ensureToolGroup();
      const list = group.querySelector<HTMLElement>(".chat-tool-list")!;
      const signature = toolSignature(text);
      const last = list.querySelector<HTMLElement>(".chat-activity.kind-tool:last-child");
      if (last?.dataset.signature === signature) {
        last.classList.add("is-live");
        last.classList.remove("is-done");
        deps.setActivityEl(last);
        updateToolGroupSummary(group, true);
        return;
      }

      settleActivity();
      const row = document.createElement("div");
      row.className = "chat-activity is-live kind-tool";
      row.dataset.phase = phase;
      row.dataset.kind = kind;
      row.dataset.signature = signature;
      row.innerHTML = `<span class="chat-tool-step" aria-hidden="true"></span><code class="chat-activity-text chat-tool-cmd"></code>`;
      row.querySelector<HTMLElement>(".chat-activity-text")!.textContent = cleanToolLabel(text);
      list.appendChild(row);
      deps.setActivityEl(row);
      updateToolGroupSummary(group, true);
      deps.logEl.scrollTop = deps.logEl.scrollHeight;
      return;
    }

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
      row.innerHTML = `<span class="chat-spinner" aria-hidden="true"></span><span class="chat-activity-text"></span>`;
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
