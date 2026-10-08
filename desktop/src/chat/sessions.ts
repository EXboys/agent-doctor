import type { AskRuntime, WorkspaceDoc } from "../ask-resources";
import { getLocale, t } from "../i18n";
import { runtimeDisplayName } from "./runtime";
import { createEmptySession, uid } from "./store";
import {
  defaultWorkspaceName,
  listProjectGroupsForSidebar,
} from "./session-workspace";
import { attachProjectReorderPointer, consumeProjectDragClick } from "./project-drag";
import {
  moveProjectInOrder,
  orderedProjectNames,
  reorderProjectKeys,
  saveProjectOrder,
} from "./project-order";
import {
  SESSIONS_PREVIEW_COUNT,
  shouldUseProjectSidebar,
  sidebarGroupKey,
  visibleSessionsForGroup,
} from "./session-sidebar";
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
  getWorkspaceDoc: () => WorkspaceDoc | null;
  syncSessionWorkspaceUi: () => void;
};

export type SessionsApi = ReturnType<typeof createSessionsController>;

export function createSessionsController(deps: SessionsDeps) {
  /** Whole project folder collapsed (Cursor-style). */
  const collapsedProjects = new Set<string>();
  /** Per-project list expanded past SESSIONS_PREVIEW_COUNT. */
  const expandedSessionLists = new Set<string>();

  function commitProjectReorder(
    from: string,
    to: string,
    position: "before" | "after" = "before",
  ): void {
    const doc = deps.getWorkspaceDoc();
    if (!doc) return;
    const order = orderedProjectNames(doc);
    saveProjectOrder(reorderProjectKeys(order, from, to, position));
    renderSessionList();
  }

  function nudgeProjectOrder(name: string, delta: -1 | 1): void {
    const doc = deps.getWorkspaceDoc();
    if (!doc) return;
    const order = orderedProjectNames(doc);
    const next = moveProjectInOrder(order, name, delta);
    if (next === order) return;
    saveProjectOrder(next);
    renderSessionList();
  }

  function createProjectGrip(): HTMLElement {
    const grip = document.createElement("span");
    grip.className = "chat-session-group-grip";
    grip.title = t("chat.projectReorder");
    grip.setAttribute("aria-label", t("chat.projectReorder"));
    grip.innerHTML = `<svg width="8" height="12" viewBox="0 0 8 12" fill="currentColor" aria-hidden="true"><circle cx="2" cy="2" r="1"/><circle cx="6" cy="2" r="1"/><circle cx="2" cy="6" r="1"/><circle cx="6" cy="6" r="1"/><circle cx="2" cy="10" r="1"/><circle cx="6" cy="10" r="1"/></svg>`;
    return grip;
  }

  function sessionMetaLine(session: ChatSession, isRunning: boolean, awaitingConfirm: boolean, doneUnseen: boolean): string {
    if (awaitingConfirm) {
      return `${t("chat.sessionAwaitingConfirm")} · ${runtimeDisplayName(session.runtime)}`;
    }
    if (isRunning) {
      return `${t("chat.sessionRunning")} · ${runtimeDisplayName(session.runtime)}`;
    }
    if (doneUnseen) {
      return `${t("chat.sessionDoneUnseen")} · ${runtimeDisplayName(session.runtime)}`;
    }
    return `${runtimeDisplayName(session.runtime)} · ${formatTime(session.updatedAt)}`;
  }

  function renderSessionRow(session: ChatSession, layout: "default" | "compact" = "default"): HTMLElement {
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

    const metaText = sessionMetaLine(session, isRunning, awaitingConfirm, doneUnseen);

    if (layout === "compact") {
      row.classList.add("is-compact");
      const line = document.createElement("span");
      line.className = "chat-session-line";
      const dot = document.createElement("span");
      dot.className = "chat-session-dot";
      dot.setAttribute("aria-hidden", "true");
      const when = document.createElement("span");
      when.className = "chat-session-when";
      when.textContent = awaitingConfirm || isRunning || doneUnseen ? "…" : formatTime(session.updatedAt);
      title.classList.add("chat-session-title-inline");
      line.append(dot, title, when);
      const agent = document.createElement("span");
      agent.className = "chat-session-agent";
      agent.textContent =
        awaitingConfirm || isRunning || doneUnseen
          ? metaText
          : runtimeDisplayName(session.runtime);
      main.append(line, agent);
      main.title = metaText;
    } else {
      const meta = document.createElement("span");
      meta.className = "chat-session-meta";
      meta.textContent = metaText;
      main.append(title, meta);
    }
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
    return row;
  }

  function groupFoldIcon(collapsed: boolean): HTMLElement {
    const wrap = document.createElement("span");
    wrap.className = "chat-session-group-icon";
    wrap.setAttribute("aria-hidden", "true");
    const folder = document.createElement("span");
    folder.className = "chat-session-group-icon-folder";
    folder.innerHTML = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none"><path d="M4 7h6l2 2h8v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V7z" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/></svg>`;
    const chevron = document.createElement("span");
    chevron.className = `chat-session-group-icon-chevron${collapsed ? " is-collapsed" : ""}`;
    chevron.innerHTML = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none"><path d="M8 10l4 4 4-4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
    wrap.append(folder, chevron);
    return wrap;
  }

  function renderSessionList(): void {
    deps.sessionListEl.replaceChildren();
    const doc = deps.getWorkspaceDoc();
    const sessions = deps.getStore().sessions;
    const workspaceCount = doc ? Object.keys(doc.workspaces).length : 0;
    const groups = listProjectGroupsForSidebar(sessions, doc, t("chat.sessionGroupOther"));
    if (!shouldUseProjectSidebar(workspaceCount)) {
      for (const session of deps.getStore().sessions) {
        deps.sessionListEl.appendChild(renderSessionRow(session));
      }
      return;
    }

    for (const group of groups) {
      const key = sidebarGroupKey(group.name);
      const projectCollapsed = collapsedProjects.has(key);
      const block = document.createElement("section");
      block.className = `chat-session-group${projectCollapsed ? " is-collapsed" : ""}`;
      block.dataset.groupKey = key;
      if (group.name) {
        block.dataset.projectName = group.name;
      }

      const head = document.createElement("div");
      head.className = "chat-session-group-head";

      const fold = document.createElement("button");
      fold.type = "button";
      fold.className = "chat-session-group-fold";
      fold.setAttribute("aria-expanded", projectCollapsed ? "false" : "true");
      fold.title = projectCollapsed ? t("chat.sessionGroupExpand") : t("chat.sessionGroupFold");
      fold.append(groupFoldIcon(projectCollapsed));
      fold.addEventListener("click", (event) => {
        event.stopPropagation();
        if (consumeProjectDragClick()) return;
        if (projectCollapsed) collapsedProjects.delete(key);
        else collapsedProjects.add(key);
        renderSessionList();
      });

      const grip = createProjectGrip();
      const label = document.createElement("span");
      label.className = "chat-session-group-label";
      label.textContent = group.label;

      head.append(fold, grip, label);

      if (group.name) {
        const projectName = group.name;
        attachProjectReorderPointer(deps.sessionListEl, head, projectName, commitProjectReorder);

        const reorder = document.createElement("span");
        reorder.className = "chat-session-group-reorder";
        const up = document.createElement("button");
        up.type = "button";
        up.className = "chat-session-group-nudge";
        up.title = t("chat.projectMoveUp");
        up.setAttribute("aria-label", t("chat.projectMoveUp"));
        up.textContent = "↑";
        up.addEventListener("click", (event) => {
          event.stopPropagation();
          nudgeProjectOrder(projectName, -1);
        });
        const down = document.createElement("button");
        down.type = "button";
        down.className = "chat-session-group-nudge";
        down.title = t("chat.projectMoveDown");
        down.setAttribute("aria-label", t("chat.projectMoveDown"));
        down.textContent = "↓";
        down.addEventListener("click", (event) => {
          event.stopPropagation();
          nudgeProjectOrder(projectName, 1);
        });
        reorder.append(up, down);

        const add = document.createElement("button");
        add.type = "button";
        add.className = "chat-session-group-new";
        add.title = t("chat.newSessionInProject");
        add.setAttribute("aria-label", t("chat.newSessionInProject"));
        add.textContent = "+";
        add.addEventListener("click", (event) => {
          event.stopPropagation();
          startNewSession(group.name);
        });
        head.append(reorder, add);
      }

      block.append(head);

      if (!projectCollapsed) {
        const { visible, hiddenCount } = visibleSessionsForGroup(
          group.sessions,
          key,
          expandedSessionLists,
        );
        const list = document.createElement("div");
        list.className = "chat-session-group-list";
        list.setAttribute("role", "list");
        for (const session of visible) {
          list.appendChild(renderSessionRow(session, "compact"));
        }
        block.append(list);

        if (hiddenCount > 0) {
          const more = document.createElement("button");
          more.type = "button";
          more.className = "chat-session-group-more";
          more.textContent = t("chat.sessionShowMore", { n: String(hiddenCount) });
          more.addEventListener("click", () => {
            expandedSessionLists.add(key);
            renderSessionList();
          });
          block.append(more);
        } else if (
          group.sessions.length > SESSIONS_PREVIEW_COUNT &&
          expandedSessionLists.has(key)
        ) {
          const less = document.createElement("button");
          less.type = "button";
          less.className = "chat-session-group-more";
          less.textContent = t("chat.sessionShowLess");
          less.addEventListener("click", () => {
            expandedSessionLists.delete(key);
            renderSessionList();
          });
          block.append(less);
        }
      }

      deps.sessionListEl.appendChild(block);
    }
  }

  /** The conversation just opened from an agent card belongs at the top of the list. */
  function bringSessionToFront(id: string): void {
    const sessions = deps.getStore().sessions;
    const index = sessions.findIndex((session) => session.id === id);
    if (index > 0) {
      const [session] = sessions.splice(index, 1);
      sessions.unshift(session);
      deps.saveStore();
      renderSessionList();
    }
    const row = deps.sessionListEl.querySelector<HTMLElement>(
      `.chat-session[data-session-id="${CSS.escape(id)}"]`,
    );
    row?.scrollIntoView({ block: "nearest" });
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

  function switchSession(id: string, focus = true): void {
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
    deps.syncSessionWorkspaceUi();
    renderSessionList();
    deps.titleEl.textContent = deps.sessionTitle(session);
    deps.updateContextMeter();
    if (focus) deps.promptEl.focus();
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
    deps.syncSessionWorkspaceUi();
    renderSessionList();
    deps.titleEl.textContent = deps.sessionTitle(active);
    deps.setStatus("");
    deps.promptEl.focus();
  }

  function ensureRuntimeSession(runtime: AskRuntime): void {
    deps.setCurrentRuntime(runtime);
    const active = deps.activeSession();
    if (active.runtime === runtime) {
      bringSessionToFront(active.id);
      return;
    }
    // Prefer the most recently updated session for this agent.
    const existing = deps.getStore().sessions
      .filter((s) => s.runtime === runtime)
      .sort((a, b) => b.updatedAt - a.updatedAt)[0];
    if (existing) {
      switchSession(existing.id);
      bringSessionToFront(existing.id);
      return;
    }
    startNewSession();
  }

  function startNewSession(workspaceName?: string | null): void {
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
    const pinned =
      workspaceName?.trim() ||
      defaultWorkspaceName(deps.getWorkspaceDoc()) ||
      undefined;
    const session = createEmptySession(deps.selectedRuntime(), pinned);
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
    deps.syncSessionWorkspaceUi();
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
    session.interrupted = null;
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
    session.interrupted = null;
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
