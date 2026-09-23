import type { AskRuntime } from "../ask-resources";
import { getLocale, t } from "../i18n";
import { runtimeDisplayName } from "./runtime";
import { createEmptySession, uid } from "./store";
import { formatTime } from "./copy-ui";
import {
  COMPACT_KEEP_TURNS,
  MAX_SESSIONS,
  type ChatMessage,
  type ChatSession,
  type PendingPermission,
  type SessionStore,
} from "./types";

export type SessionsDeps = {
  sessionListEl: HTMLElement;
  titleEl: HTMLElement;
  promptEl: HTMLTextAreaElement;
  logEl: HTMLElement;
  getStore: () => SessionStore;
  setStore: (store: SessionStore) => void;
  getBusy: () => boolean;
  getRunningChatSessionId: () => string | null;
  getPendingPermissionBatch: () => PendingPermission[];
  getUnseenCompletedSessionIds: () => Set<string>;
  getCurrentRuntime: () => AskRuntime;
  selectedRuntime: () => AskRuntime;
  sessionTitle: (session: ChatSession) => string;
  activeSession: () => ChatSession;
  saveStore: () => void;
  setCurrentRuntime: (runtime: AskRuntime, opts?: { syncSession?: boolean }) => void;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  syncComposerUi: () => void;
  updateContextMeter: () => void;
  closeContextPopover: () => void;
  hideDecisionDock: () => void;
  flushPendingTextSync: () => void;
  scheduleStorePersist: (delayMs?: number) => void;
  updateAssistantMessage: (id: string, content: string, opts?: { persist?: boolean }) => void;
  setAssistantMarkdown: (bubble: HTMLElement, markdown: string) => void;
  syncAssistantCopyButton: (bubble: HTMLElement) => void;
  appendBubble: (
    kind: "assistant" | "user" | "meta" | "permission",
    text: string,
    opts?: { id?: string; persist?: boolean },
  ) => HTMLElement;
  pushActivity: (phase: string, message: string) => void;
  schedulePaintLivePermissionBatch: () => void;
  renderActiveMessages: () => void;
  renderPendingAttachments: () => void;
  isViewingRunningSession: () => boolean;
  getAssistantBubble: () => HTMLElement | null;
  setAssistantBubble: (el: HTMLElement | null) => void;
  getAssistantMessageId: () => string | null;
  setAssistantMessageId: (id: string | null) => void;
  getAssistantRaw: () => string;
  setAssistantRaw: (raw: string) => void;
  getPendingText: () => string;
  setPendingText: (text: string) => void;
  getTurnHadAssistantText: () => boolean;
  setTurnHadAssistantText: (v: boolean) => void;
  getActivityEl: () => HTMLElement | null;
  setActivityEl: (el: HTMLElement | null) => void;
  getLifecycleActivityEl: () => HTMLElement | null;
  setLifecycleActivityEl: (el: HTMLElement | null) => void;
  getToolGroupEl: () => HTMLDetailsElement | null;
  setToolGroupEl: (el: HTMLDetailsElement | null) => void;
  setPendingAttachments: (items: import("./types").ChatAttachment[]) => void;
  touchSession: (session: ChatSession) => void;
  isComposerLocked: () => boolean;
};

export type SessionsApi = ReturnType<typeof createSessionsController>;

export function createSessionsController(deps: SessionsDeps) {
  function renderSessionList(): void {
    deps.sessionListEl.replaceChildren();
    for (const session of deps.getStore().sessions) {
      const isRunning = deps.getBusy() && session.id === deps.getRunningChatSessionId();
      const awaitingConfirm = isRunning && deps.getPendingPermissionBatch().length > 0;
      const doneUnseen = !isRunning && deps.getUnseenCompletedSessionIds().has(session.id);
      const row = document.createElement("div");
      row.className = `chat-session${session.id === deps.getStore().activeId ? " is-active" : ""}${
        awaitingConfirm ? " is-awaiting-confirm" : isRunning ? " is-running" : ""
      }${doneUnseen ? " is-done-unseen" : ""}`;
      row.dataset.sessionId = session.id;

      const main = document.createElement("button");
      main.type = "button";
      main.className = "chat-session-main";

      const title = document.createElement("span");
      title.className = "chat-session-title";
      title.textContent = deps.sessionTitle(session);

      const meta = document.createElement("span");
      meta.className = "chat-session-meta";
      if (awaitingConfirm) {
        meta.textContent = `${t("chat.sessionAwaitingConfirm")} · ${runtimeDisplayName(session.runtime)}`;
      } else if (isRunning) {
        meta.textContent = `${t("chat.sessionRunning")} · ${runtimeDisplayName(session.runtime)}`;
      } else if (doneUnseen) {
        meta.textContent = `${t("chat.sessionDoneUnseen")} · ${runtimeDisplayName(session.runtime)}`;
      } else {
        meta.textContent = `${runtimeDisplayName(session.runtime)} · ${formatTime(session.updatedAt)}`;
      }

      main.append(title, meta);
      main.addEventListener("click", () => {
        switchSession(session.id);
      });

      const del = document.createElement("button");
      del.type = "button";
      del.className = "chat-session-delete";
      del.title = t("chat.deleteSession");
      del.setAttribute("aria-label", t("chat.deleteSession"));
      del.textContent = "×";
      del.addEventListener("click", (event) => {
        event.stopPropagation();
        deleteSession(session.id);
      });

      row.append(main, del);
      deps.sessionListEl.appendChild(row);
    }
  }

  function detachLiveDom(): void {
    deps.flushPendingTextSync();
    if (deps.getAssistantMessageId() && deps.getAssistantRaw()) {
      deps.updateAssistantMessage(deps.getAssistantMessageId()!, deps.getAssistantRaw(), { persist: false });
      deps.scheduleStorePersist();
    }
    deps.setAssistantBubble(null);
    deps.setActivityEl(null);
    deps.setLifecycleActivityEl(null);
    deps.setToolGroupEl(null);
    deps.hideDecisionDock();
  }

  function reattachLiveUi(): void {
    if (!deps.isViewingRunningSession()) return;
    if (deps.getAssistantMessageId()) {
      const messageId = deps.getAssistantMessageId()!;
      const bubble = deps.logEl.querySelector<HTMLElement>(
        `.chat-bubble.assistant[data-message-id="${CSS.escape(messageId)}"]`,
      );
      if (bubble) {
        deps.setAssistantBubble(bubble);
        bubble.classList.add("is-streaming");
        if (deps.getAssistantRaw().trim()) {
          deps.setAssistantMarkdown(bubble, deps.getAssistantRaw());
        }
        deps.syncAssistantCopyButton(bubble);
      } else if (deps.getAssistantRaw().trim() || deps.getTurnHadAssistantText()) {
        const bubble = deps.appendBubble("assistant", deps.getAssistantRaw(), {
          id: deps.getAssistantMessageId() ?? undefined,
          persist: false,
        });
        deps.setAssistantBubble(bubble);
        bubble.classList.add("is-streaming");
      }
    }
    for (const item of deps.getPendingPermissionBatch()) {
      deps.logEl
        .querySelectorAll<HTMLElement>(
          `.chat-permission[data-request-id="${CSS.escape(item.requestId)}"]`,
        )
        .forEach((el) => el.remove());
    }
    if (deps.getPendingPermissionBatch().length > 0) {
      deps.setStatus(t("chat.needYourChoice"), "warn");
      deps.schedulePaintLivePermissionBatch();
    } else {
      deps.setStatus(
        t("chat.running", { runtime: runtimeDisplayName(deps.activeSession().runtime) }),
        "muted",
      );
      deps.pushActivity("think", t("chat.typing"));
    }
    deps.logEl.scrollTop = deps.logEl.scrollHeight;
  }

  function switchSession(id: string): void {
    if (id === deps.getStore().activeId) return;
    const session = deps.getStore().sessions.find((s) => s.id === id);
    if (!session) return;

    const leavingRunning = Boolean(deps.getBusy() && deps.getStore().activeId === deps.getRunningChatSessionId());
    const enteringRunning = Boolean(deps.getBusy() && id === deps.getRunningChatSessionId());

    if (leavingRunning) {
      detachLiveDom();
    } else if (!deps.getBusy()) {
      deps.setAssistantBubble(null);
      deps.setAssistantMessageId(null);
      deps.setAssistantRaw("");
      deps.setPendingText("");
      deps.setTurnHadAssistantText(false);
      deps.setActivityEl(null);
      deps.setLifecycleActivityEl(null);
      deps.setToolGroupEl(null);
    } else {
      // Leaving a non-running chat while another run continues — clear local view only.
      deps.setAssistantBubble(null);
      deps.setActivityEl(null);
      deps.setLifecycleActivityEl(null);
      deps.setToolGroupEl(null);
    }

    deps.getStore().activeId = id;
    deps.saveStore();
    deps.setCurrentRuntime(session.runtime);
    if (deps.getUnseenCompletedSessionIds().delete(id)) {
      // Opened after finishing elsewhere — clear 【完成】 badge.
    }
    deps.renderActiveMessages();
    if (enteringRunning) {
      reattachLiveUi();
    } else if (deps.getBusy() && deps.getRunningChatSessionId()) {
      deps.setStatus(t("chat.otherSessionRunningHint"), "muted");
    } else {
      deps.setStatus("");
    }
    deps.syncComposerUi();
    renderSessionList();
    deps.titleEl.textContent = deps.sessionTitle(session);
    deps.updateContextMeter();
    deps.promptEl.focus();
  }

  function deleteSession(id: string): void {
    if (deps.getBusy() && id === deps.getRunningChatSessionId()) {
      deps.setStatus(t("chat.cannotDeleteRunning"), "warn");
      return;
    }
    if (!window.confirm(t("chat.deleteSessionConfirm"))) return;
    deps.getUnseenCompletedSessionIds().delete(id);
    const remaining = deps.getStore().sessions.filter((s) => s.id !== id);
    if (remaining.length === 0) {
      const session = createEmptySession(deps.getCurrentRuntime());
      deps.setStore({ activeId: session.id, sessions: [session] });
    } else {
      deps.getStore().sessions = remaining;
      if (deps.getStore().activeId === id) {
        deps.getStore().activeId = remaining[0].id;
      }
    }
    deps.saveStore();
    const active = deps.activeSession();
    deps.setCurrentRuntime(active.runtime);
    if (!deps.getBusy()) {
      deps.setAssistantBubble(null);
      deps.setAssistantMessageId(null);
      deps.setAssistantRaw("");
      deps.setPendingText("");
      deps.setTurnHadAssistantText(false);
    } else {
      deps.setAssistantBubble(null);
    }
    deps.setPendingAttachments([]);
    deps.setActivityEl(null);
    deps.setLifecycleActivityEl(null);
    deps.setToolGroupEl(null);
    deps.renderPendingAttachments();
    deps.renderActiveMessages();
    if (deps.isViewingRunningSession()) {
      reattachLiveUi();
    }
    deps.syncComposerUi();
    renderSessionList();
    deps.titleEl.textContent = deps.sessionTitle(active);
    deps.setStatus("");
    deps.promptEl.focus();
  }

  function ensureRuntimeSession(runtime: AskRuntime): void {
    deps.setCurrentRuntime(runtime);
    const active = deps.activeSession();
    if (active.runtime === runtime) {
      renderSessionList();
      return;
    }
    // Prefer the most recently updated session for this agent.
    const existing = deps.getStore().sessions
      .filter((s) => s.runtime === runtime)
      .sort((a, b) => b.updatedAt - a.updatedAt)[0];
    if (existing) {
      switchSession(existing.id);
      return;
    }
    startNewSession();
  }

  function startNewSession(): void {
    if (deps.getBusy() && deps.getStore().activeId === deps.getRunningChatSessionId()) {
      detachLiveDom();
    } else if (!deps.getBusy()) {
      deps.setAssistantBubble(null);
      deps.setAssistantMessageId(null);
      deps.setAssistantRaw("");
      deps.setPendingText("");
      deps.setTurnHadAssistantText(false);
    } else {
      deps.setAssistantBubble(null);
    }
    const session = createEmptySession(deps.selectedRuntime());
    deps.getStore().sessions.unshift(session);
    deps.getStore().activeId = session.id;
    deps.getStore().sessions = deps.getStore().sessions.slice(0, MAX_SESSIONS);
    deps.saveStore();
    deps.setPendingAttachments([]);
    deps.setActivityEl(null);
    deps.setLifecycleActivityEl(null);
    deps.setToolGroupEl(null);
    deps.renderPendingAttachments();
    deps.renderActiveMessages();
    deps.syncComposerUi();
    renderSessionList();
    deps.titleEl.textContent = deps.sessionTitle(session);
    if (deps.getBusy() && deps.getRunningChatSessionId()) {
      deps.setStatus(t("chat.otherSessionRunningHint"), "muted");
    } else {
      deps.setStatus(t("chat.newSessionReady"), "ok");
    }
    deps.updateContextMeter();
    deps.promptEl.focus();
  }

  function clearActiveSession(): void {
    if (deps.isViewingRunningSession()) {
      deps.setStatus(t("chat.cannotClearRunning"), "warn");
      return;
    }
    if (deps.getBusy() && deps.getStore().activeId === deps.getRunningChatSessionId()) return;
    const session = deps.activeSession();
    session.messages = [];
    session.title = "";
    session.runtimeThreadId = null;
    session.updatedAt = Date.now();
    deps.saveStore();
    if (!deps.getBusy()) {
      deps.setAssistantBubble(null);
      deps.setAssistantMessageId(null);
      deps.setAssistantRaw("");
      deps.setTurnHadAssistantText(false);
    } else {
      deps.setAssistantBubble(null);
    }
    deps.setActivityEl(null);
    deps.setPendingAttachments([]);
    deps.renderPendingAttachments();
    deps.renderActiveMessages();
    renderSessionList();
    deps.titleEl.textContent = deps.sessionTitle(session);
    deps.setStatus("");
  }

  function compactActiveSession(): void {
    if (deps.isComposerLocked()) return;
    const session = deps.activeSession();
    const turns = session.messages.filter((m) => m.role === "user" || m.role === "assistant");
    if (turns.length <= COMPACT_KEEP_TURNS) {
      deps.setStatus(t("chat.contextCompactNeedMore"), "muted");
      return;
    }
    const keep = turns.slice(-COMPACT_KEEP_TURNS);
    const dropped = turns.slice(0, -COMPACT_KEEP_TURNS);
    const summaryLines = dropped
      .map((m) => {
        const role =
          m.role === "user"
            ? getLocale() === "zh"
              ? "用户"
              : "User"
            : getLocale() === "zh"
              ? "助手"
              : "Assistant";
        const text = m.content.trim().replace(/\s+/g, " ");
        const short = text.length > 72 ? `${text.slice(0, 72)}…` : text || "(…)";
        return `- ${role}: ${short}`;
      })
      .slice(0, 16);
    const summary: ChatMessage = {
      id: uid(),
      role: "meta",
      content: `${t("chat.contextCompactSummary", { n: String(dropped.length) })}\n${summaryLines.join("\n")}`,
      at: Date.now(),
    };
    const pendingPermissions = session.messages.filter(
      (m) => m.role === "permission" && m.permission?.allowed == null,
    );
    session.messages = [...pendingPermissions, summary, ...keep];
    session.runtimeThreadId = null;
    deps.touchSession(session);
    deps.saveStore();
    deps.closeContextPopover();
    deps.renderActiveMessages();
    renderSessionList();
    deps.updateContextMeter();
    deps.setStatus(t("chat.contextCompactDone"), "ok");
  }


  return {
    renderSessionList,
    detachLiveDom,
    reattachLiveUi,
    switchSession,
    deleteSession,
    ensureRuntimeSession,
    startNewSession,
    clearActiveSession,
    compactActiveSession,
  };
}
