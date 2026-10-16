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
  workspaceAddEl,
  workspaceActivateEl,
  workspaceHintEl,
  resourcesTabsEl,
  resourcesSearchEl,
  resourcesRefreshEl,
  openResourcesEl,
} from "./chat-dom";

import {
  renderSessionList,
  saveStore,
  ensureRuntimeSession,
  addProjectFromAsk,
  compactActiveSession,
  isViewingRunningSession,
  autoResizePrompt,
  setStatus,
  setActiveSessionAutoApprove,
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
  flushPendingTextSync,
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
import {
  teamupsChannelConnected,
  teamupsChannelNote,
  teamupsChannelPref,
  teamupsChannelReply,
  teamupsChannelSetEnabled,
} from "./ipc";
import { applyChannelAnswer, choiceVerdict, pendingChoiceCount, takePendingChoices } from "./chat/channel-ask";
import {
  assistantForChannelTurn,
  settleChannelAnswer,
  channelConfirmAction,
  channelQueueLength,
  enqueueChannelJob,
  setChannelSessionIdle,
  takeNextChannelJob,
  unshiftChannelJob,
  type ChannelQueued,
} from "./chat/channel-lane";
import { channelSessionTitle, placeChannelSession } from "./chat/channel-route";
import { createChannelLedger } from "./chat/channel-ledger";
import { createEmptySession } from "./chat/store";
import { defaultWorkspaceName } from "./chat/session-workspace";
import { MAX_SESSIONS } from "./chat/types";
import type { SendAskResult } from "./chat/send";
import { bindLayoutResize } from "./chat/layout-resize";
import { bindTuneMenu } from "./chat/tune-menu";
import { bindKnowledge } from "./chat/knowledge-page";
import { bindResourcesPage } from "./chat/resources-page";
import { bindSchedule } from "./chat/schedule-page";
import { orderedProjectNames } from "./chat/project-order";
import { syncKnowledgeFolder } from "./chat/knowledge-sync";
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
    syncKnowledgeFolder(null);
    const projectName = sessionWorkspaceName(activeSession(), chatState.workspaceDoc);
    const projectPath = projectName ? chatState.workspaceDoc?.workspaces[projectName]?.path?.trim() : "";
    if (projectPath) syncKnowledgeFolder(projectPath);
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
  workspaceAddEl.addEventListener("click", () => addProjectFromAsk());
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
    setActiveSessionAutoApprove(elevatedEl.checked);
  });
  readImageEl.addEventListener("change", () => {
    setReadImageTextEnabled(readImageEl.checked);
  });
  promptEl.addEventListener("input", () => {
    autoResizePrompt();
    // Menu and meter rewrite the DOM. Doing that while the input method is
    // still painting marked text makes those characters jump or disappear.
    if (promptEl.dataset.composing === "1") return;
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
  promptEl.addEventListener("focus", () => {
    closeContextPopover();
  });
  promptEl.addEventListener("compositionstart", () => {
    promptEl.dataset.composing = "1";
  });
  promptEl.addEventListener("compositionend", () => {
    // Defer clear so the Enter that ends composition doesn't also send.
    window.setTimeout(() => {
      promptEl.dataset.composing = "0";
      autoResizePrompt();
      mentionMenu.renderMentionMenu();
      updateContextMeter();
    }, 0);
  });
  document.addEventListener("click", (event) => {
    if (!(event.target instanceof Node)) return;
    if (promptEl.contains(event.target)) {
      closeContextPopover();
      return;
    }
    if (mentionMenuEl.contains(event.target)) return;
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
  // A reload (or a recovered window) must not drop a turn that is still running.
  void chatState.send.adoptLiveRuns();
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
  let channelEnabled = true;
  let channelSignedIn = false;
  const channelSwitch = () => document.querySelector<HTMLInputElement>("#chat-settings-channel-switch");
  const paintChannel = () => {
    const el = document.querySelector<HTMLElement>("#chat-settings-channel");
    const input = channelSwitch();
    if (input) {
      input.checked = channelEnabled;
      input.setAttribute("aria-checked", channelEnabled ? "true" : "false");
      input.setAttribute("aria-label", t("chat.settingsChannelSwitch"));
    }
    if (!el) return;
    const live = channelEnabled && channelSignedIn && channelOn;
    el.classList.toggle("is-on", live);
    if (!channelEnabled) el.textContent = t("chat.settingsChannelClosed");
    else if (!channelSignedIn) el.textContent = t("chat.settingsChannelSignedOut");
    else el.textContent = channelOn ? t("chat.settingsChannelOn") : t("chat.settingsChannelOff");
  };
  const loadChannelPref = () => {
    void teamupsChannelPref()
      .then((pref) => {
        channelEnabled = pref.enabled;
        channelSignedIn = pref.signed_in;
        paintChannel();
      })
      .catch(() => paintChannel());
  };
  channelSwitch()?.addEventListener("change", () => {
    const input = channelSwitch();
    if (!input) return;
    const on = input.checked;
    channelEnabled = on;
    paintChannel();
    void teamupsChannelSetEnabled(on).catch(() => {
      channelEnabled = !on;
      paintChannel();
    });
  });
  void listen<boolean>("teamups-channel-status", (event) => {
    channelOn = event.payload === true;
    paintChannel();
  });
  void listen("teamups-account-changed", () => {
    loadChannelPref();
  });
  void teamupsChannelConnected()
    .then((on) => {
      channelOn = on;
      paintChannel();
    })
    .catch(() => paintChannel());
  loadChannelPref();
  const channelLedger = createChannelLedger(
    (() => {
      try {
        return window.localStorage;
      } catch {
        return null;
      }
    })(),
  );
  function finishChannel(id: string, text: string, ok: boolean): void {
    channelLedger.finish(id, text, ok);
    void teamupsChannelReply(id, text, ok, true).catch(() => {});
  }
  setChannelSessionIdle((sessionId) => {
    const next = takeNextChannelJob(sessionId);
    if (!next) return;
    void runChannelTurn(next);
  });
  void listen<{ id?: string; text?: string; chat?: string }>("teamups-channel-job", (event) => {
    const id = event.payload?.id?.trim() ?? "";
    const text = event.payload?.text?.trim() ?? "";
    const chat = event.payload?.chat?.trim() ?? "";
    void runChannelJob(id, text, chat);
  });
  function sessionForChannel(chat: string, text: string): string {
    const store = chatState.store;
    const placed = placeChannelSession(store.sessions, chat, () => {
      const session = createEmptySession(
        selectedRuntime(),
        defaultWorkspaceName(chatState.workspaceDoc),
      );
      session.title = channelSessionTitle(text);
      return session;
    });
    if (placed.created) {
      store.sessions = placed.sessions.slice(0, MAX_SESSIONS);
      saveStore();
      renderSessionList();
    }
    return placed.session.id;
  }
  function tellChannelQueued(job: ChannelQueued): void {
    if (job.noticed) return;
    job.noticed = true;
    teamupsChannelNote(job.id, `queued in ${job.sessionId}`);
    if (chatState.store.activeId === job.sessionId) {
      setStatus(t("chat.channelQueued"), "muted");
    }
    void teamupsChannelReply(job.id, t("chat.channelQueued"), true, false).catch(() => {});
  }
  async function runChannelTurn(job: ChannelQueued): Promise<void> {
    teamupsChannelNote(job.id, `start in ${job.sessionId}`);
    void teamupsChannelReply(job.id, t("chat.channelStarted"), true, false).catch(() => {});
    const started = Date.now();
    const replyText = () => {
      const session = chatState.store.sessions.find((item) => item.id === job.sessionId);
      return assistantForChannelTurn(session?.messages ?? [], started, job.text);
    };
    let sent = "";
    const pump = window.setInterval(() => {
      const text = replyText();
      if (!text || text === sent) return;
      sent = text;
      void teamupsChannelReply(job.id, text, true, false).catch(() => {});
    }, 400);
    let result: SendAskResult = "empty";
    channelLedger.start(job.id);
    try {
      result = await chatState.send.sendAsk({
        lane: "channel",
        draft: {
          id: crypto.randomUUID(),
          sessionId: job.sessionId,
          text: job.text,
          attachments: [],
          mentions: [],
          channelJobId: job.id,
        },
      });
    } catch (error) {
      window.clearInterval(pump);
      teamupsChannelNote(job.id, `failed ${String(error)}`);
      finishChannel(job.id, t("chat.channelFailed"), false);
      return;
    }
    window.clearInterval(pump);
    if (result === "blocked") {
      finishChannel(job.id, t("chat.handsBlockedChannel"), false);
      return;
    }
    if (result === "queued") {
      channelLedger.unstart(job.id);
      tellChannelQueued(job);
      unshiftChannelJob(job);
      return;
    }
    if (result !== "started") {
      teamupsChannelNote(job.id, `not started ${result}`);
      finishChannel(job.id, t("chat.channelFailed"), false);
      return;
    }
    const answer = await settleChannelAnswer(replyText, { flush: flushPendingTextSync });
    const session = chatState.store.sessions.find((item) => item.id === job.sessionId);
    const stopped = Boolean(session?.interrupted && session.interrupted.at >= started);
    teamupsChannelNote(job.id, stopped ? "stopped" : `done chars=${answer.length}`);
    const final = answer || (stopped ? t("chat.channelFailed") : t("chat.channelDone"));
    finishChannel(job.id, final, Boolean(answer) || !stopped);
  }
  async function runChannelJob(id: string, text: string, chat: string): Promise<void> {
    teamupsChannelNote(id, "reached chat window");
    if (!id || !text || !chatState.send || !chatState.store) {
      teamupsChannelNote(id, "chat window not ready");
      void teamupsChannelReply(id, t("chat.channelNotReady"), false).catch(() => {});
      return;
    }
    const seen = channelLedger.seen(id);
    if (seen.kind === "here") {
      teamupsChannelNote(id, "sent again, already here");
      return;
    }
    if (seen.kind === "done") {
      teamupsChannelNote(id, "sent again, reply again");
      void teamupsChannelReply(id, seen.reply || t("chat.channelDone"), seen.ok, true).catch(() => {});
      return;
    }
    if (seen.kind === "cut") {
      teamupsChannelNote(id, "sent again, was cut off");
      finishChannel(id, t("chat.channelCut"), false);
      return;
    }
    channelLedger.take(id);
    try {
      const sessionId = sessionForChannel(chat, text);
      const job: ChannelQueued = { id, sessionId, text, noticed: false };
      const busy = isChatRunning(sessionId) || channelQueueLength(sessionId) > 0;
      const action = channelConfirmAction({
        confirmToken: choiceVerdict(text) !== null,
        pendingChoices: pendingChoiceCount(sessionId),
        busy,
      });
      if (action === "confirm") {
        const taken = await takePendingChoices(sessionId, text);
        if (taken === 0) {
          finishChannel(id, t("chat.channelAlreadyConfirmed"), true);
          return;
        }
        const allow = choiceVerdict(text) === true;
        const reply =
          taken > 1
            ? t(allow ? "chat.channelAllowedAll" : "chat.channelDeniedAll")
            : t(allow ? "chat.channelAllowed" : "chat.channelDenied");
        finishChannel(id, reply, true);
        return;
      }
      if (action === "repeat") {
        finishChannel(id, t("chat.channelAlreadyConfirmed"), true);
        return;
      }
      if (action === "queue") {
        tellChannelQueued(job);
        enqueueChannelJob(job);
        return;
      }
      await runChannelTurn(job);
    } catch (error) {
      teamupsChannelNote(id, `failed ${String(error)}`);
      finishChannel(id, t("chat.channelFailed"), false);
    }
  }
  void listen<{ requestId?: string; text?: string }>("teamups-channel-answer", (event) => {
    const requestId = event.payload?.requestId?.trim() ?? "";
    const text = event.payload?.text?.trim() ?? "";
    if (!requestId || !text) return;
    void applyChannelAnswer(requestId, text).catch(() => {});
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
