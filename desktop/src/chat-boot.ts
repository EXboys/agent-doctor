import { setFollowQueueInsertHandler } from "./chat/follow-queue";
import { chatState } from "./chat-state";

import {
  elevatedEl,
  readImageEl,
  modelBtnEl,
  modelMenuEl,
  promptEl,
  actionEl,
  attachEl,
  contextMeterEl,
  contextPopoverEl,
  contextCompactEl,
  mentionMenuEl,
  clearEl,
  restoreBackupEl,
  newSessionEl,
  addProjectEl,
  terminalEl,
  workspaceSelectEl,
  workspaceActivateEl,
  workspaceHintEl,
  themeEl,
  resourcesToggleEl,
  resourcesTabsEl,
  resourcesSearchEl,
  resourcesRefreshEl,
  openResourcesEl,
} from "./chat-dom";

import {
  applyChatTheme,
  renderSessionList,
  ensureRuntimeSession,
  startNewSession,
  addProjectFromAsk,
  clearActiveSession,
  compactActiveSession,
  isViewingRunningSession,
  autoResizePrompt,
  closeModelMenu,
  positionModelMenu,
  openModelMenu,
  refreshWiredProvider,
  flushStorePersist,
  applyI18n,
  pickAttachments,
  setupFileDrop,
  renderActiveMessages,
  toggleResourcesPanel,
  loadAskResources,
  syncWorkspaceActivateButton,
  activateSelectedWorkspace,
  assignActiveSessionWorkspace,
  syncSessionWorkspaceUi,
  openMainWorkspace,
  openMainResources,
  closeContextPopover,
  positionContextPopover,
  toggleContextPopover,
  updateContextMeter,
  ensureListener,
  readInitialRuntime,
  applyVerifyDraftIfAny,
  openTerminal,
  cancelAsk,
  sendAsk,
  restoreChatFromBackup,
  offerBackupRestoreIfNeeded,
  askResources,
  mentionMenu,
} from "./chat";

import { emit, listen } from "@tauri-apps/api/event";
import { startIslandPublisher } from "./island/publish";
import { isAskRuntime } from "./chat/runtime";
import { t } from "./i18n";
import { currentChatTheme, readStoredChatTheme, systemChatTheme } from "./chat/theme";
import { readImageTextEnabled, setReadImageTextEnabled } from "./chat/image-text";
import { wireChatControllers } from "./chat-wire";

export function bootChat(): void {
  setFollowQueueInsertHandler(() => {
    void cancelAsk();
  });
  wireChatControllers();
  const win = window as Window & {
    __AD_ASK_APPLY_RUNTIME__?: (runtime: string) => void;
    __AD_ASK_BOOTED__?: boolean;
  };
  applyChatTheme(readStoredChatTheme() ?? systemChatTheme(), false);
  win.__AD_ASK_APPLY_RUNTIME__ = (runtime) => {
    if (isAskRuntime(runtime)) {
      ensureRuntimeSession(runtime);
    }
  };
  // Paint sessions + transcript first so a hung secondary init never looks like “no history”.
  try {
    renderSessionList();
    renderActiveMessages();
  } catch (error) {
    console.error("Ask: early paint failed", error);
  }
  applyI18n();
  readImageEl.checked = readImageTextEnabled();
  try {
    readInitialRuntime();
  } catch (error) {
    console.error("Ask: runtime session setup failed", error);
  }
  applyI18n();
  renderActiveMessages();
  offerBackupRestoreIfNeeded();
  win.__AD_ASK_BOOTED__ = true;
  try {
    sessionStorage.removeItem("ad-ask-reload");
  } catch {
    /* ignore */
  }
  window.addEventListener("beforeunload", () => {
    flushStorePersist();
  });
  void setupFileDrop();
  void (async () => {
    await loadAskResources();
    syncSessionWorkspaceUi();
    renderSessionList();
    applyVerifyDraftIfAny();
  })();

  window.addEventListener("focus", () => {
    renderSessionList();
    syncSessionWorkspaceUi();
  });
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") {
      renderSessionList();
      syncSessionWorkspaceUi();
    }
  });

  actionEl.addEventListener("click", () => {
    if (isViewingRunningSession()) void cancelAsk();
    else void sendAsk();
  });
  attachEl.addEventListener("click", () => void pickAttachments());
  clearEl.addEventListener("click", clearActiveSession);
  restoreBackupEl?.addEventListener("click", () => {
    restoreChatFromBackup();
  });
  newSessionEl.addEventListener("click", () => startNewSession());
  addProjectEl.addEventListener("click", () => addProjectFromAsk());
  terminalEl.addEventListener("click", () => void openTerminal());
  themeEl.addEventListener("click", () => {
    applyChatTheme(currentChatTheme() === "dark" ? "light" : "dark");
  });
  resourcesToggleEl.addEventListener("click", () => {
    toggleResourcesPanel();
  });
  resourcesRefreshEl.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    void loadAskResources();
  });
  resourcesTabsEl?.addEventListener("click", (event) => {
    const target = event.target as HTMLElement | null;
    const tab = target?.closest<HTMLButtonElement>("[data-res-tab]");
    if (!tab?.dataset.resTab) return;
    const next = tab.dataset.resTab;
    if (next === "all" || next === "skills" || next === "mcp") {
      askResources.setResourcesTab(next);
    }
  });
  resourcesSearchEl?.addEventListener("input", () => {
    askResources.setResourcesQuery(resourcesSearchEl?.value ?? "");
  });
  openResourcesEl.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    void openMainResources();
  });
  workspaceActivateEl.addEventListener("click", () => void activateSelectedWorkspace());
  workspaceSelectEl.addEventListener("change", () => {
    const name = workspaceSelectEl.value.trim();
    if (!name) {
      syncWorkspaceActivateButton();
      return;
    }
    assignActiveSessionWorkspace(name);
    syncWorkspaceActivateButton();
  });
  workspaceHintEl.addEventListener("dblclick", () => void openMainWorkspace());
  elevatedEl.addEventListener("change", () => {
    if (elevatedEl.checked && !window.confirm(t("chat.elevatedConfirm"))) {
      elevatedEl.checked = false;
      return;
    }
    if (elevatedEl.checked) {
      void chatState.permissions?.approvePendingChoices();
    }
  });
  readImageEl.addEventListener("change", () => {
    setReadImageTextEnabled(readImageEl.checked);
  });
  promptEl.addEventListener("input", () => {
    autoResizePrompt();
    mentionMenu.renderMentionMenu();
    updateContextMeter();
  });
  contextMeterEl?.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    toggleContextPopover();
  });
  contextCompactEl?.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    compactActiveSession();
  });
  promptEl.addEventListener("keydown", (event) => {
    if (!mentionMenuEl.hidden) {
      const options = mentionMenu.filteredMentionOptions();
      const slashOpen = mentionMenu.isSlashMenuOpen();

      if (slashOpen && (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
        event.preventDefault();
        mentionMenu.cycleSlashTab(event.key === "ArrowRight" ? 1 : -1);
        return;
      }
      if (slashOpen && event.key === "Tab") {
        event.preventDefault();
        mentionMenu.cycleSlashTab(event.shiftKey ? -1 : 1);
        return;
      }
      if (event.key === "ArrowDown" && options.length > 0) {
        event.preventDefault();
        mentionMenu.mentionMenuIndex = (mentionMenu.mentionMenuIndex + 1) % options.length;
        mentionMenu.renderMentionMenu();
        return;
      }
      if (event.key === "ArrowUp" && options.length > 0) {
        event.preventDefault();
        mentionMenu.mentionMenuIndex =
          (mentionMenu.mentionMenuIndex - 1 + options.length) % options.length;
        mentionMenu.renderMentionMenu();
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        mentionMenu.hideMentionMenu();
        return;
      }
      if (event.key === "Enter" && !event.shiftKey && options[mentionMenu.mentionMenuIndex]) {
        event.preventDefault();
        mentionMenu.applyMentionOption(options[mentionMenu.mentionMenuIndex]);
        return;
      }
      if (!slashOpen && event.key === "Tab" && options[mentionMenu.mentionMenuIndex]) {
        event.preventDefault();
        mentionMenu.applyMentionOption(options[mentionMenu.mentionMenuIndex]);
        return;
      }
    }
    if (event.key !== "Enter") return;
    // Shift+Enter keeps a newline for multi-line prompts.
    if (event.shiftKey) return;
    // IME candidate confirm (Chinese etc.): Enter commits the composition, not send.
    if (event.isComposing || event.keyCode === 229 || promptEl.dataset.composing === "1") return;
    event.preventDefault();
    void sendAsk();
  });
  promptEl.addEventListener("compositionstart", () => {
    promptEl.dataset.composing = "1";
  });
  promptEl.addEventListener("compositionend", () => {
    // Defer clear so the Enter that ends composition doesn't also send.
    window.setTimeout(() => {
      promptEl.dataset.composing = "0";
    }, 0);
  });
  document.addEventListener("click", (event) => {
    if (!(event.target instanceof Node)) return;
    if (mentionMenuEl.contains(event.target) || promptEl.contains(event.target)) return;
    mentionMenu.hideMentionMenu();
    if (!modelBtnEl.contains(event.target) && !modelMenuEl.contains(event.target)) {
      closeModelMenu();
    }
    const wrap = contextMeterEl?.closest(".chat-context-wrap");
    const inMeter = Boolean(wrap?.contains(event.target));
    const inPopover = Boolean(contextPopoverEl?.contains(event.target));
    if (!inMeter && !inPopover) {
      closeContextPopover();
    }
  });

  modelBtnEl.addEventListener("click", (event) => {
    event.stopPropagation();
    closeContextPopover();
    if (chatState.modelMenuOpen) {
      closeModelMenu();
      return;
    }
    openModelMenu();
  });

  window.addEventListener("resize", () => {
    if (chatState.modelMenuOpen) positionModelMenu();
    if (contextPopoverEl && !contextPopoverEl.hidden) positionContextPopover();
  });

  void listen<{ runtime?: string }>("ask-window-focus", (event) => {
    const runtime = event.payload?.runtime;
    if (isAskRuntime(runtime)) {
      ensureRuntimeSession(runtime);
    }
    void (async () => {
      await refreshWiredProvider();
      await loadAskResources();
      applyVerifyDraftIfAny();
    })();
    promptEl.focus();
  });

  void listen("personal-provider-changed", () => {
    void refreshWiredProvider();
  });

  void ensureListener();
  void refreshWiredProvider();
  autoResizePrompt();
  promptEl.focus();
  startIslandPublisher(promptEl, () => ({
    activeId: chatState.store?.activeId ?? "",
    attentionId: chatState.runningChatSessionId || chatState.store?.activeId || "",
    sessions: (chatState.store?.sessions ?? []).map((session) => ({
      id: session.id,
      runtime: session.runtime,
      title: session.title,
      updatedAt: session.updatedAt,
      messages: session.messages ?? [],
      plan: session.plan?.items,
    })),
  }));
  void listen<string>("island-open-session", (event) => {
    const id = event.payload;
    if (!id || !chatState.sessions?.switchSession) return;
    chatState.sessions.switchSession(id);
  });
  void listen<{ sessionId?: string; text?: string }>("island-send-text", (event) => {
    const text = event.payload?.text?.trim() ?? "";
    const sessionId = event.payload?.sessionId || chatState.store?.activeId || "";
    const report = (status: "sent" | "queued" | "busy" | "failed") => {
      void emit("island-send-result", { sessionId, status }).catch(() => {});
    };
    if (!text || !chatState.send || !sessionId) {
      report("failed");
      return;
    }
    const draft = { id: crypto.randomUUID(), sessionId, text, attachments: [], mentions: [] };
    if (chatState.busy) {
      if (chatState.runningChatSessionId && chatState.runningChatSessionId !== sessionId) {
        report("busy");
        return;
      }
      void chatState.send.sendAsk({ draft });
      report("queued");
      return;
    }
    // The run takes the session's own assistant, so it has to be the active one
    // when it starts. Put the person's view back once it is running.
    const viewing = chatState.store?.activeId ?? "";
    chatState.sessions?.switchSession(sessionId, false);
    void chatState.send.sendAsk({ draft });
    report("sent");
    if (viewing && viewing !== sessionId) {
      const started = Date.now();
      const restore = () => {
        if (chatState.runningChatSessionId === sessionId) {
          chatState.sessions?.switchSession(viewing, false);
          return;
        }
        if (Date.now() - started < 3000) window.setTimeout(restore, 50);
      };
      window.setTimeout(restore, 50);
    }
  });
}
