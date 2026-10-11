import { setFollowQueueInsertHandler } from "./chat/follow-queue";
import { isChatRunning, MAX_PARALLEL_RUNS, runningCount } from "./chat/live-runs";
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
  newWorkspaceEl,
  workspaceSelectEl,
  workspaceActivateEl,
  workspaceHintEl,
  resourcesTabsEl,
  resourcesSearchEl,
  resourcesRefreshEl,
  openResourcesEl,
} from "./chat-dom";

import {
  renderSessionList,
  ensureRuntimeSession,
  addProjectFromAsk,
  compactActiveSession,
  isViewingRunningSession,
  autoResizePrompt,
  setStatus,
  closeModelMenu,
  positionModelMenu,
  openModelMenu,
  refreshWiredProvider,
  flushStorePersist,
  applyI18n,
  publishActiveRuntime,
  selectedRuntime,
  activeSession,
  pickAttachments,
  setupFileDrop,
  renderActiveMessages,
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
  cancelAsk,
  sendAsk,
  offerBackupRestoreIfNeeded,
  askResources,
  mentionMenu,
} from "./chat";

import { emit, listen } from "@tauri-apps/api/event";
import { bindLocaleSync } from "./locale-sync";
import { startIslandPublisher } from "./island/publish";
import {
  ASK_RUNTIME_QUERY_EVENT,
  ASK_SELECT_RUNTIME_EVENT,
  isAskRuntime,
} from "./chat/runtime";
import { getLocale, t } from "./i18n";
import { applyThemePreference, readThemePreference, watchSystemTheme } from "./chat/theme";
import { bindChatSettings, refreshConfirmCopy } from "./chat/settings-page";
import { bindSidebarCollapse } from "./chat/session-sidebar";
import { bindAccountBar } from "./chat/account-bar";
import { teamupsChannelConnected, teamupsChannelReply } from "./ipc";
import { bindLayoutResize } from "./chat/layout-resize";
import { bindTuneMenu } from "./chat/tune-menu";
import { bindKnowledge } from "./chat/knowledge-page";
import { bindResourcesPage } from "./chat/resources-page";
import { bindSchedule } from "./chat/schedule-page";
import { orderedProjectNames } from "./chat/project-order";
import { sessionWorkspaceName } from "./chat/session-workspace";
import { readImageTextEnabled, setReadImageTextEnabled } from "./chat/image-text";
import { wireChatControllers } from "./chat-wire";

function transcriptOf(session: ReturnType<typeof activeSession>): { title: string; text: string } | null {
  const lines = session.messages
    .filter((message) => (message.role === "user" || message.role === "assistant") && message.content.trim())
    .map((message) => `## ${message.role === "user" ? "User" : "Assistant"}\n\n${message.content.trim()}`);
  if (!lines.length) return null;
  const title = session.title.trim() || "聊天";
  return { title, text: `# ${title}\n\n${lines.join("\n\n")}\n` };
}

export function bootChat(): void {
  setFollowQueueInsertHandler(() => {
    void cancelAsk();
  });
  wireChatControllers();
  const win = window as Window & {
    __AD_ASK_APPLY_RUNTIME__?: (runtime: string) => void;
    __AD_ASK_BOOTED__?: boolean;
  };
  applyThemePreference(readThemePreference(), false);
  watchSystemTheme();
  bindChatSettings(() => applyI18n());
  bindSidebarCollapse();
  bindLayoutResize();
  bindAccountBar();
  bindTuneMenu();
  bindKnowledge({
    projects: () => (chatState.workspaceDoc ? orderedProjectNames(chatState.workspaceDoc) : []),
    currentProject: () => sessionWorkspaceName(activeSession(), chatState.workspaceDoc),
    currentAgent: () => selectedRuntime(),
    projectPath: (name) => chatState.workspaceDoc?.workspaces[name]?.path?.trim() || null,
    chatTranscript: () => transcriptOf(activeSession()),
    chats: () =>
      (chatState.store?.sessions ?? []).flatMap((session) => {
        const text = transcriptOf(session);
        return text ? [{ ...text, projectName: session.workspaceName?.trim() || null }] : [];
      }),
  });
  bindSchedule({
    activeSession,
    sessions: () => chatState.store?.sessions ?? [],
    workspaceDoc: () => chatState.workspaceDoc,
    switchSession: (id) => chatState.sessions.switchSession(id),
    fillPrompt: (text) => {
      promptEl.value = text;
      autoResizePrompt();
      promptEl.focus();
      const end = promptEl.value.length;
      promptEl.setSelectionRange(end, end);
    },
    setStatus,
  });
  bindResourcesPage({
    projects: () => (chatState.workspaceDoc ? orderedProjectNames(chatState.workspaceDoc) : []),
    currentProject: () => sessionWorkspaceName(activeSession(), chatState.workspaceDoc),
    currentAgent: () => selectedRuntime(),
    askResources,
    reload: () => loadAskResources(),
    openManage: () => {
      void openMainResources();
    },
    closeSidePanel: () => askResources.setResourcesOpen(false),
  });
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
  try {
    applyI18n();
  } catch (error) {
    console.error("Ask: applyI18n failed", error);
  }
  readImageEl.checked = readImageTextEnabled();
  try {
    readInitialRuntime();
  } catch (error) {
    console.error("Ask: runtime session setup failed", error);
  }
  try {
    applyI18n();
  } catch (error) {
    console.error("Ask: applyI18n failed", error);
  }
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
  newWorkspaceEl.addEventListener("click", () => addProjectFromAsk());
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

  void listen(ASK_RUNTIME_QUERY_EVENT, () => {
    publishActiveRuntime();
  });

  void listen<{ runtime?: string }>(ASK_SELECT_RUNTIME_EVENT, (event) => {
    const runtime = event.payload?.runtime;
    if (isAskRuntime(runtime)) {
      ensureRuntimeSession(runtime);
    }
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
    if (runningCount() > 0) setStatus(t("chat.providerChangedMidTurn"), "warn");
  });

  void listen("workspace-changed", () => {
    void loadAskResources();
  });

  bindLocaleSync(() => {
    applyI18n();
    document.querySelectorAll<HTMLButtonElement>("[data-settings-locale]").forEach((button) => {
      const on = button.dataset.settingsLocale === getLocale();
      button.classList.toggle("is-on", on);
      button.setAttribute("aria-selected", on ? "true" : "false");
    });
    refreshConfirmCopy();
    paintChannel();
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
    if (!id || !chatState.sessions?.openSessionFromIsland) return;
    chatState.sessions.openSessionFromIsland(id);
  });
  let channelOn = false;
  const paintChannel = () => {
    const el = document.querySelector<HTMLElement>("#chat-settings-channel");
    if (!el) return;
    el.textContent = channelOn ? t("chat.settingsChannelOn") : t("chat.settingsChannelOff");
  };
  void listen<boolean>("teamups-channel-status", (event) => {
    channelOn = event.payload === true;
    paintChannel();
  });
  void teamupsChannelConnected()
    .then((on) => {
      channelOn = on;
      paintChannel();
    })
    .catch(() => paintChannel());
  void listen<{ id?: string; text?: string }>("teamups-channel-job", (event) => {
    const id = event.payload?.id?.trim() ?? "";
    const text = event.payload?.text?.trim() ?? "";
    void runChannelJob(id, text);
  });
  async function runChannelJob(id: string, text: string): Promise<void> {
    const sessionId = chatState.store?.activeId ?? "";
    const fail = (message: string) => {
      void teamupsChannelReply(id, message, false).catch(() => {});
    };
    if (!id || !text || !chatState.send || !sessionId) {
      fail(t("chat.channelNotReady"));
      return;
    }
    if (isChatRunning(sessionId) || runningCount() >= MAX_PARALLEL_RUNS) {
      fail(t("chat.channelBusy"));
      return;
    }
    const started = Date.now();
    try {
      await chatState.send.sendAsk({
        draft: { id: crypto.randomUUID(), sessionId, text, attachments: [], mentions: [] },
      });
    } catch {
      fail(t("chat.channelFailed"));
      return;
    }
    const session = chatState.store?.sessions.find((item) => item.id === sessionId);
    const reply = [...(session?.messages ?? [])]
      .reverse()
      .find((message) => message.role === "assistant" && message.at >= started - 2000);
    void teamupsChannelReply(id, reply?.content.trim() || t("chat.channelDone"), true).catch(() => {});
  }
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
    if (isChatRunning(sessionId)) {
      void chatState.send.sendAsk({ draft });
      report("queued");
      return;
    }
    if (runningCount() >= MAX_PARALLEL_RUNS) {
      report("busy");
      return;
    }
    // Start this chat without leaving the one on screen. Both can run together.
    void chatState.send.sendAsk({ draft });
    report("sent");
  });
}
