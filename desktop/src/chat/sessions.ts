import type { AskRuntime, WorkspaceDoc } from "../ask-resources";
import { setAgentBrandIcon } from "../agent-brand";
import { getLocale, t } from "../i18n";
import { listInstalledAskAgents } from "../ipc";
import { isAskRuntime, runtimeDisplayName } from "./runtime";
import { createEmptySession, uid } from "./store";
import {
  defaultWorkspaceName,
  listProjectGroupsForSidebar,
  sessionWorkspaceName,
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
  sidebarSessionRank,
  visibleSessionsForGroup,
} from "./session-sidebar";
import { formatTime } from "./copy-ui";
import { leaveChatOverlayPages } from "./overlay-pages";
import { closeChatSettings } from "./settings-page";
import { backgroundChoiceCount, isChatRunning, runningCount } from "./live-runs";
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
  captureRunningView: () => void;
  restoreRunningView: () => void;
  addProject: () => void;
  removeProject: (name: string) => void;
  syncAskChrome: (hasProjects: boolean) => void;
};

export type SessionsApi = ReturnType<typeof createSessionsController>;

export function createSessionsController(deps: SessionsDeps) {
  /** Whole project folder collapsed (Cursor-style). */
  const collapsedProjects = new Set<string>();
  /** Per-project list expanded past SESSIONS_PREVIEW_COUNT. */
  const expandedSessionLists = new Set<string>();
  /** Chats opened from the island, so they sit at the top until a newer update. */
  const surfacedAt = new Map<string, number>();

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
    const isRunning = isChatRunning(session.id);
    const awaitingConfirm =
      isRunning &&
      (session.id === deps.getStore().activeId
        ? deps.getPendingPermissionBatch().length > 0
        : backgroundChoiceCount(session.id) > 0);
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
    } else {
      const meta = document.createElement("span");
      meta.className = "chat-session-meta";
      meta.textContent = metaText;
      main.append(title, meta);
    }
    const del = document.createElement("button");
    del.type = "button";
    del.className = "chat-session-delete";
    del.title = t("chat.deleteSession");
    del.setAttribute("aria-label", t("chat.deleteSession"));
    del.innerHTML = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M4 7h16M9 7V4h6v3m3 0-1 13H7L6 7m4 4v5m4-5v5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
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

  let projectMenu: HTMLElement | null = null;
  /** Bumped whenever a menu closes, so a slow install scan cannot open late. */
  let agentPickToken = 0;
  let installedAgentsCache: { at: number; agents: InstalledAgent[] } | null = null;

  type InstalledAgent = { id: AskRuntime; label: string };
  type MenuAnchor = HTMLElement | { x: number; y: number };

  function closeProjectMenu(): void {
    projectMenu?.remove();
    projectMenu = null;
    agentPickToken += 1;
  }

  window.addEventListener("focus", () => {
    void refreshInstalledAgents();
  });
  void refreshInstalledAgents();

  function openProjectMenu(x: number, y: number, name: string, sessions: ChatSession[]): void {
    closeProjectMenu();
    const menu = document.createElement("div");
    menu.className = "chat-project-menu";
    menu.setAttribute("role", "menu");

    const addItem = (label: string, onPick: () => void) => {
      const item = document.createElement("button");
      item.type = "button";
      item.className = "chat-project-menu-item";
      item.setAttribute("role", "menuitem");
      item.textContent = label;
      item.addEventListener("click", (event) => {
        event.stopPropagation();
        closeProjectMenu();
        onPick();
      });
      menu.append(item);
    };

    addItem(t("chat.projectMenuNew"), () => {
      void beginNewSessionInProject(name, { x, y });
    });
    addItem(t("chat.projectMenuRemove"), () => {
      if (sessions.some((session) => isChatRunning(session.id))) {
        deps.setStatus(t("chat.removeProjectBusy"), "warn");
        return;
      }
      if (!window.confirm(t("chat.removeProjectConfirm", { name }))) return;
      deps.removeProject(name);
    });

    document.body.append(menu);
    const rect = menu.getBoundingClientRect();
    const left = Math.min(x, window.innerWidth - rect.width - 8);
    const top = Math.min(y, window.innerHeight - rect.height - 8);
    menu.style.left = `${Math.max(8, left)}px`;
    menu.style.top = `${Math.max(8, top)}px`;
    projectMenu = menu;
  }

  document.addEventListener("pointerdown", (event) => {
    if (!projectMenu) return;
    const target = event.target;
    if (!(target instanceof Node) || projectMenu.contains(target)) return;
    if (
      projectMenu.dataset.kind === "agent-pick" &&
      target instanceof Element &&
      target.closest(".chat-session-group-new")
    ) {
      return;
    }
    closeProjectMenu();
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") closeProjectMenu();
  });

  function renderEmptyProjects(): HTMLElement {
    const card = document.createElement("div");
    card.className = "chat-empty-projects";
    const title = document.createElement("p");
    title.className = "chat-empty-projects-title";
    title.textContent = t("chat.emptyProjectsTitle");
    const hint = document.createElement("p");
    hint.className = "chat-empty-projects-hint";
    hint.textContent = t("chat.emptyProjectsHint");
    const button = document.createElement("button");
    button.type = "button";
    button.className = "chat-empty-projects-action";
    button.textContent = t("chat.emptyProjectsAction");
    button.addEventListener("click", () => deps.addProject());
    card.append(title, hint, button);
    return card;
  }

  function renderSessionList(): void {
    deps.sessionListEl.replaceChildren();
    const doc = deps.getWorkspaceDoc();
    const sessions = deps.getStore().sessions;
    const workspaceCount = doc ? Object.keys(doc.workspaces).length : 0;
    deps.syncAskChrome(workspaceCount > 0 || doc == null);
    const groups = listProjectGroupsForSidebar(sessions, doc, t("chat.sessionGroupOther"));
    if (doc && workspaceCount === 0) {
      deps.sessionListEl.appendChild(renderEmptyProjects());
      for (const session of sessions) {
        if (!session.messages.some((message) => message.role === "user" || message.role === "assistant")) {
          continue;
        }
        deps.sessionListEl.appendChild(renderSessionRow(session));
      }
      return;
    }
    if (!shouldUseProjectSidebar(workspaceCount)) {
      for (const session of deps.getStore().sessions) {
        deps.sessionListEl.appendChild(renderSessionRow(session));
      }
      return;
    }

    for (const group of groups) {
      group.sessions.sort(
        (a, b) =>
          sidebarSessionRank(b.updatedAt, surfacedAt.get(b.id) ?? 0) -
          sidebarSessionRank(a.updatedAt, surfacedAt.get(a.id) ?? 0),
      );
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
        head.addEventListener("contextmenu", (event) => {
          event.preventDefault();
          event.stopPropagation();
          openProjectMenu(event.clientX, event.clientY, projectName, group.sessions);
        });
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
        add.setAttribute("aria-haspopup", "menu");
        add.addEventListener("click", (event) => {
          event.stopPropagation();
          void beginNewSessionInProject(projectName, add);
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
        if (hiddenCount > 0) {
          const more = document.createElement("button");
          more.type = "button";
          more.className = "chat-session-group-more";
          more.textContent = t("chat.sessionShowMore");
          more.addEventListener("click", () => {
            expandedSessionLists.add(key);
            renderSessionList();
          });
          list.append(more);
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
          list.append(less);
        }
        block.append(list);
      }

      deps.sessionListEl.appendChild(block);
    }
  }

  /**
   * Show this chat at the top of its project without changing its last-activity time.
   * Same temporary pin the island uses, so a session hidden under「更多」becomes visible.
   */
  function bringSessionToFront(id: string): void {
    const sessions = deps.getStore().sessions;
    const index = sessions.findIndex((session) => session.id === id);
    if (index < 0) return;
    const session = sessions[index]!;
    surfacedAt.set(id, Date.now());
    collapsedProjects.delete(
      sidebarGroupKey(sessionWorkspaceName(session, deps.getWorkspaceDoc())),
    );
    if (index > 0) {
      sessions.splice(index, 1);
      sessions.unshift(session);
      deps.saveStore();
    }
    renderSessionList();
    deps.sessionListEl
      .querySelector<HTMLElement>(`.chat-session[data-session-id="${CSS.escape(id)}"]`)
      ?.scrollIntoView({ block: "nearest" });
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

  /** The chat opened from the island belongs at the top, still showing its last activity time. */
  function openSessionFromIsland(id: string): void {
    if (!deps.getStore().sessions.some((session) => session.id === id)) return;
    bringSessionToFront(id);
    switchSession(id);
    deps.sessionListEl
      .querySelector<HTMLElement>(`.chat-session[data-session-id="${CSS.escape(id)}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }

  function switchSession(id: string, focus = true): void {
    const hadOverlay = leaveChatOverlayPages();
    closeChatSettings();
    if (id === deps.getStore().activeId) {
      if (hadOverlay) {
        deps.renderActiveMessages();
        deps.syncComposerUi();
        deps.syncSessionWorkspaceUi();
        renderSessionList();
        if (focus) deps.promptEl.focus();
      }
      return;
    }
    const session = deps.getStore().sessions.find((s) => s.id === id);
    if (!session) return;

    const leavingRunning = isChatRunning(deps.getStore().activeId);
    const enteringRunning = isChatRunning(id);

    if (leavingRunning) {
      deps.captureRunningView();
      detachLiveDom();
    } else {
      deps.setAssistantBubble(null);
      deps.setAssistantMessageId(null);
      deps.setAssistantRaw("");
      deps.setPendingText("");
      deps.setTurnHadAssistantText(false);
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
      deps.restoreRunningView();
      reattachLiveUi();
    } else if (runningCount() > 0) {
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
    if (isChatRunning(id)) {
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
      bringSessionToFront(existing.id);
      switchSession(existing.id);
      return;
    }
    startNewSession();
  }

  async function refreshInstalledAgents(): Promise<InstalledAgent[] | null> {
    try {
      const listed = await listInstalledAskAgents();
      const agents: InstalledAgent[] = [];
      const seen = new Set<string>();
      for (const agent of listed) {
        if (!isAskRuntime(agent.id) || seen.has(agent.id)) continue;
        seen.add(agent.id);
        agents.push({
          id: agent.id,
          label: agent.label.trim() || runtimeDisplayName(agent.id),
        });
      }
      installedAgentsCache = { at: Date.now(), agents };
      return agents;
    } catch (error) {
      console.warn("Ask: failed to list installed agents", error);
      return null;
    }
  }

  function placeMenu(menu: HTMLElement, anchor: MenuAnchor): void {
    document.body.append(menu);
    const menuRect = menu.getBoundingClientRect();
    let left: number;
    let top: number;
    if (anchor instanceof HTMLElement) {
      const rect = anchor.getBoundingClientRect();
      left = rect.right - menuRect.width;
      top = rect.bottom + 6;
      if (top + menuRect.height > window.innerHeight - 8) {
        top = rect.top - menuRect.height - 6;
      }
    } else {
      left = anchor.x;
      top = anchor.y;
    }
    left = Math.min(Math.max(8, left), Math.max(8, window.innerWidth - menuRect.width - 8));
    top = Math.min(Math.max(8, top), Math.max(8, window.innerHeight - menuRect.height - 8));
    menu.style.left = `${Math.round(left)}px`;
    menu.style.top = `${Math.round(top)}px`;
  }

  function openAgentPickMenu(anchor: MenuAnchor, projectName: string, agents: InstalledAgent[]): void {
    closeProjectMenu();
    const menu = document.createElement("div");
    menu.className = "chat-project-menu";
    menu.dataset.kind = "agent-pick";
    menu.dataset.project = projectName;
    menu.setAttribute("role", "menu");
    menu.setAttribute("aria-label", t("chat.pickAgent"));

    const heading = document.createElement("div");
    heading.className = "chat-project-menu-label";
    heading.textContent = t("chat.pickAgent");
    menu.append(heading);

    for (const agent of agents) {
      const item = document.createElement("button");
      item.type = "button";
      item.className = "chat-project-menu-item chat-agent-pick-item";
      item.setAttribute("role", "menuitem");
      const icon = document.createElement("span");
      icon.className = "chat-agent-pick-icon";
      setAgentBrandIcon(icon, agent.id);
      const label = document.createElement("span");
      label.textContent = agent.label;
      item.append(icon, label);
      item.addEventListener("click", (event) => {
        event.stopPropagation();
        closeProjectMenu();
        startNewSession(projectName, agent.id);
      });
      menu.append(item);
    }

    placeMenu(menu, anchor);
    projectMenu = menu;
  }

  /** One installed agent starts immediately. Several agents open a picker first. */
  async function beginNewSessionInProject(projectName: string, anchor: MenuAnchor): Promise<void> {
    if (
      projectMenu?.dataset.kind === "agent-pick" &&
      projectMenu.dataset.project === projectName
    ) {
      closeProjectMenu();
      return;
    }
    const token = ++agentPickToken;
    // The list is loaded with the window. A click uses it at once; a miss
    // falls through to the same lookup, which only checks that the program
    // exists and does not start it.
    const agents = installedAgentsCache?.agents ?? (await refreshInstalledAgents());
    if (token !== agentPickToken) return;
    if (!agents) {
      startNewSession(projectName);
      return;
    }
    if (agents.length === 0) {
      deps.setStatus(t("chat.noInstalledAgent"), "warn");
      return;
    }
    if (agents.length === 1) {
      startNewSession(projectName, agents[0].id);
      return;
    }
    openAgentPickMenu(anchor, projectName, agents);
  }

  function startNewSession(workspaceName?: string | null, runtime?: AskRuntime): void {
    leaveChatOverlayPages();
    closeChatSettings();
    const chosen = runtime ?? deps.selectedRuntime();
    if (chosen !== deps.getCurrentRuntime()) {
      deps.setCurrentRuntime(chosen);
    }
    if (isChatRunning(deps.getStore().activeId)) {
      deps.captureRunningView();
      detachLiveDom();
    } else {
      deps.setAssistantBubble(null);
      deps.setAssistantMessageId(null);
      deps.setAssistantRaw("");
      deps.setPendingText("");
      deps.setTurnHadAssistantText(false);
    }
    const pinned =
      workspaceName?.trim() ||
      defaultWorkspaceName(deps.getWorkspaceDoc()) ||
      undefined;
    const session = createEmptySession(chosen, pinned);
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
    if (runningCount() > 0) {
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


  if (deps.sessionListEl.dataset.sessionNavBound !== "1") {
    deps.sessionListEl.dataset.sessionNavBound = "1";
    deps.sessionListEl.addEventListener("click", (event) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      if (
        target.closest(
          ".chat-session-delete, .chat-session-group-fold, .chat-session-group-new, .chat-session-group-nudge, .chat-session-group-more",
        )
      ) {
        return;
      }
      const main = target.closest(".chat-session-main");
      if (!main) return;
      const id = main.closest<HTMLElement>(".chat-session[data-session-id]")?.dataset.sessionId;
      if (!id) return;
      switchSession(id);
    });
  }

  return {
    renderSessionList,
    detachLiveDom,
    reattachLiveUi,
    switchSession,
    openSessionFromIsland,
    deleteSession,
    ensureRuntimeSession,
    startNewSession,
    clearActiveSession,
    compactActiveSession,
  };
}
