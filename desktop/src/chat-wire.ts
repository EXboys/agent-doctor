import { focusMainTab } from "./ipc";
import { applyBackgroundEvent } from "./chat/background-run";
import { isChatRunning, runningCount } from "./chat/live-runs";
import { chatState } from "./chat-state";
import { normalizePlanItems, paintChatPlan } from "./plan";

import {
  elevatedEl,
  modelBtnEl,
  modelLabelEl,
  modelMenuEl,
  modelWrapEl,
  promptEl,
  voiceEl,
  mainEl,
  dialogModeEl,
  voiceModeEl,
  islandEl,
  islandTitleEl,
  islandDetailEl,
  attachmentsEl,
  composerBoxEl,
  composerEl,
  contextMeterEl,
  contextRingFillEl,
  contextLabelEl,
  contextPopoverEl,
  contextPopoverTitleEl,
  contextPopoverBodyEl,
  contextCompactEl,
  sessionListEl,
  logEl,
  decisionDockEl,
  decisionKickerEl,
  decisionTitleEl,
  decisionDetailEl,
  decisionActionsEl,
  cwdEl,
  workspaceSelectEl,
  workspaceActivateEl,
  workspaceHintEl,
  titleEl,
  shellEl,
} from "./chat-dom";

import {
  saveStore,
  sessionTitle,
  buildPromptWithHistory,
  contextUsagePercent,
  expireLivePermissionCards,
  pushPermissionCard,
  schedulePaintLivePermissionBatch,
  renderPermissionCard,
  renderPermissionGroup,
  collapseResolvedPermissionsBeforeAssistant,
  markPermissionResolved,
  renderSessionList,
  isViewingRunningSession,
  isComposerLocked,
  sessionById,
  runTargetSession,
  isEventForCurrentRun,
  resolveEventChatId,
  captureRunningView,
  restoreRunningView,
  addProjectFromAsk,
  removeProjectFromAsk,
  syncAskChrome,
  settleRunRouting,
  autoResizePrompt,
  selectedRuntime,
  closeModelMenu,
  setCurrentRuntime,
  flushStorePersist,
  scheduleStorePersist,
  flushSessionListRender,
  scheduleSessionListRender,
  activeSession,
  touchSession,
  syncRestoreBackupButton,
  setStatus,
  syncComposerUi,
  setBusy,
  dismissLifecycleActivity,
  finishToolGroup,
  clearEphemeralActivity,
  appendStderrLine,
  pushActivity,
  settleActivity,
  flushPendingTextSync,
  hideDecisionDock,
  allowQuickRepliesAgain,
  clearQuickReplies,
  showQuickReplies,
  sealAssistantBubble,
  appendAssistantChunk,
  renderPendingAttachments,
  persistMessage,
  updateAssistantMessage,
  syncAssistantCopyButton,
  setAssistantMarkdown,
  appendBubble,
  renderActiveMessages,
  queueAssistantText,
  setDisplayedCwd,
  resolveSendWorkspace,
  syncSessionWorkspaceUi,
  closeContextPopover,
  updateContextMeter,
  ensureListener,
  noteVerifyBrowserSignal,
  applyVerifyEvidenceFromAssistant,
  reportVerifyMcpIfNeeded,
  applyVerifyMcpFooter,
  sendAsk,
  askResources,
  mentionMenu,
} from "./chat";

import { createPermissionsController } from "./chat/permissions";
import { createSessionsController } from "./chat/sessions";
import { createBubblesController } from "./chat/bubbles";
import { createStreamController } from "./chat/stream";
import { createSendController } from "./chat/send";
import { createActivityController } from "./chat/activity";
import { createThinkingController } from "./chat/thinking";
import { createRunningIndicator } from "./chat/running";
import { t } from "./i18n";
import { createModelPickerController } from "./chat/model-picker";
import { createDecisionController } from "./chat/decision";
import { createAttachmentsController } from "./chat/attachments";
import { createContextMeterController } from "./chat/context-meter";
import { createShellUiController } from "./chat/shell-ui";
import { createBackupUiController } from "./chat/backup-ui";
import { createVoiceInputController } from "./chat/voice";
import { createHostedController } from "./chat/hosted";
import { readImageTextEnabled } from "./chat/image-text";
import { mountChatFailureBubble } from "./chat/chat-failure-ui";
import {
  formatChatFailureLine,
  type ChatFailureExplain,
} from "./friendly-error";
import { markChatFailureBubbleShown } from "./chat/turn-errors";

export function wireChatControllers(): void {
  const chatFailureHandlers = {
    setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => setStatus(text, tone),
    selectedRuntime: () => selectedRuntime(),
  };
  const showChatFailureBubble = (explain: ChatFailureExplain) => {
    if (!markChatFailureBubbleShown()) return;
    mountChatFailureBubble(logEl, explain, chatFailureHandlers);
    setStatus(formatChatFailureLine(explain), "error");
  };
  chatState.backupUi = createBackupUiController({
    logEl,
    titleEl,
    getStore: () => chatState.store,
    setStore: (next) => {
      chatState.store = next;
    },
    activeSession: () => activeSession(),
    sessionTitle: (session) => sessionTitle(session),
    flushStorePersist: () => flushStorePersist(),
    renderActiveMessages: () => renderActiveMessages(),
    renderSessionList: () => renderSessionList(),
    setStatus: (text, tone) => setStatus(text, tone),
    syncRestoreBackupButton: () => syncRestoreBackupButton(),
    appendBubble: (kind, text, opts) => appendBubble(kind, text, opts),
  });

  chatState.activity = createActivityController({
    logEl,
    isViewingRunningSession: () => isViewingRunningSession(),
    flushPendingTextSync: () => flushPendingTextSync(),
    sealAssistantBubble: () => sealAssistantBubble(),
    collapseResolvedPermissionsBeforeAssistant: (anchor) =>
      collapseResolvedPermissionsBeforeAssistant(anchor),
    getActivityEl: () => chatState.activityEl,
    setActivityEl: (el) => {
      chatState.activityEl = el;
    },
    getLifecycleActivityEl: () => chatState.lifecycleActivityEl,
    setLifecycleActivityEl: (el) => {
      chatState.lifecycleActivityEl = el;
    },
    getToolGroupEl: () => chatState.toolGroupEl,
    setToolGroupEl: (el) => {
      chatState.toolGroupEl = el;
    },
    rememberTool: (text) => chatState.bubbles.rememberTool(text),
    toolRecordsForTurn: () => {
      const session = chatState.busy ? runTargetSession() : activeSession();
      const messages = session.messages;
      let start = 0;
      for (let i = messages.length - 1; i >= 0; i -= 1) {
        if (messages[i]?.role === "user") {
          start = i + 1;
          break;
        }
      }
      return messages.slice(start).filter((message) => message.role === "tool").map((message) => message.content);
    },
    onChatConnectionFailure: (explain) => showChatFailureBubble(explain),
  });

  chatState.modelPicker = createModelPickerController({
    modelBtnEl,
    modelLabelEl,
    modelMenuEl,
    modelWrapEl,
    composerBoxEl,
    composerEl,
    isComposerLocked: () => isComposerLocked(),
    selectedRuntime: () => selectedRuntime(),
    getWiredProvider: () => chatState.wiredProvider,
    setWiredProvider: (provider) => {
      chatState.wiredProvider = provider;
    },
    getModelMenuOpen: () => chatState.modelMenuOpen,
    setModelMenuOpen: (open) => {
      chatState.modelMenuOpen = open;
    },
    setStatus: (text, tone) => setStatus(text, tone),
    updateContextMeter: () => updateContextMeter(),
  });

  chatState.decision = createDecisionController({
    decisionDockEl,
    decisionKickerEl,
    decisionTitleEl,
    decisionDetailEl,
    decisionActionsEl,
    logEl,
    promptEl,
    getBusy: () => chatState.busy,
    setStatus: (text, tone) => setStatus(text, tone),
    autoResizePrompt: () => autoResizePrompt(),
    sendAsk: () => sendAsk(),
  });

  chatState.attachments = createAttachmentsController({
    attachmentsEl,
    composerBoxEl,
    isComposerLocked: () => isComposerLocked(),
    getPendingAttachments: () => chatState.pendingAttachments,
    setPendingAttachments: (items) => {
      chatState.pendingAttachments = items;
    },
    setStatus: (text, tone) => setStatus(text, tone),
  });

  chatState.voiceInput = createVoiceInputController({
    voiceBtnEl: voiceEl,
    promptEl,
    isComposerLocked: () => isComposerLocked(),
    isHostedActive: () => chatState.hosted?.isActive() ?? false,
    setStatus: (text, tone) => setStatus(text, tone),
    autoResizePrompt: () => autoResizePrompt(),
  });

  chatState.contextMeter = createContextMeterController({
    contextMeterEl,
    contextRingFillEl,
    contextLabelEl,
    contextPopoverEl,
    contextPopoverTitleEl,
    contextPopoverBodyEl,
    contextCompactEl,
    composerBoxEl,
    composerEl,
    promptEl,
    activeSession: () => activeSession(),
    contextUsagePercent: (session, draft) => contextUsagePercent(session, draft),
    isComposerLocked: () => isComposerLocked(),
    closeModelMenu: () => closeModelMenu(),
  });

  chatState.shellUi = createShellUiController({
    shellEl,
    cwdEl,
    workspaceSelectEl,
    workspaceActivateEl,
    workspaceHintEl,
    askResources,
    setStatus: (text, tone) => setStatus(text, tone),
    autoResizePrompt: () => autoResizePrompt(),
    setWorkspaceCwd: (cwd) => {
      chatState.workspaceCwd = cwd;
    },
    getWorkspaceDoc: () => chatState.workspaceDoc,
    setWorkspaceDoc: (doc) => {
      chatState.workspaceDoc = doc;
    },
    setDisplayedCwd: (cwd) => setDisplayedCwd(cwd),
  });

  chatState.bubbles = createBubblesController({
    logEl,
    titleEl,
    getBusy: () => chatState.busy,
    getStore: () => chatState.store,
    isViewingRunningSession: () => isViewingRunningSession(),
    runTargetSession: () => runTargetSession(),
    activeSession: () => activeSession(),
    selectedRuntime: () => selectedRuntime(),
    sessionTitle: (session) => sessionTitle(session),
    touchSession: (session) => touchSession(session),
    saveStore: () => saveStore(),
    flushStorePersist: () => flushStorePersist(),
    scheduleStorePersist: (delayMs) => scheduleStorePersist(delayMs),
    scheduleSessionListRender: (delayMs) => scheduleSessionListRender(delayMs),
    renderSessionList: () => renderSessionList(),
    updateContextMeter: () => updateContextMeter(),
    pushActivity: (phase, message) => pushActivity(phase, message),
    settleActivity: () => settleActivity(),
    finishToolGroup: (collapse) => finishToolGroup(collapse),
    dismissLifecycleActivity: () => dismissLifecycleActivity(),
    collapseResolvedPermissionsBeforeAssistant: (anchor) =>
      collapseResolvedPermissionsBeforeAssistant(anchor),
    renderPermissionCard: (message, interactive) => renderPermissionCard(message, interactive),
    renderPermissionGroup: (messages, interactive) => renderPermissionGroup(messages, interactive),
    getPendingPermissionBatch: () => chatState.permissions.pendingPermissionBatch,
    getAssistantBubble: () => chatState.assistantBubble,
    setAssistantBubble: (el) => {
      chatState.assistantBubble = el;
    },
    getAssistantMessageId: () => chatState.assistantMessageId,
    setAssistantMessageId: (id) => {
      chatState.assistantMessageId = id;
    },
    getAssistantRaw: () => chatState.assistantRaw,
    setAssistantRaw: (raw) => {
      chatState.assistantRaw = raw;
    },
    getPendingText: () => chatState.pendingText,
    setPendingText: (value) => {
      chatState.pendingText = value;
    },
    getTurnHadAssistantText: () => chatState.turnHadAssistantText,
    setTurnHadAssistantText: (value) => {
      chatState.turnHadAssistantText = value;
    },
    getActivityEl: () => chatState.activityEl,
    setActivityEl: (el) => {
      chatState.activityEl = el;
    },
    getToolGroupEl: () => chatState.toolGroupEl,
    setToolGroupEl: (el) => {
      chatState.toolGroupEl = el;
    },
    getLifecycleActivityEl: () => chatState.lifecycleActivityEl,
    setLifecycleActivityEl: (el) => {
      chatState.lifecycleActivityEl = el;
    },
    addProject: () => addProjectFromAsk(),
    openProvider: () => {
      void focusMainTab({ tab: "provider" }).catch((error) => {
        setStatus(String(error), "error");
      });
    },
  });

  chatState.running = createRunningIndicator(composerBoxEl);

  chatState.thinking = createThinkingController({
    logEl,
    isViewingRunningSession: () => isViewingRunningSession(),
    persistMessage: (content) => chatState.bubbles.persistMessage("thinking", content),
    updateMessage: (id, content) => {
      updateAssistantMessage(id, content, { persist: false });
      scheduleStorePersist();
    },
    endReply: () => {
      flushPendingTextSync();
      sealAssistantBubble();
    },
    beforeBlock: () => {
      dismissLifecycleActivity();
      settleActivity();
      finishToolGroup(true);
    },
  });

  chatState.permissions = createPermissionsController({
    logEl,
    isViewingRunningSession: () => isViewingRunningSession(),
    runTargetSession: () => runTargetSession(),
    touchSession: (session) => touchSession(session),
    saveStore: () => saveStore(),
    flushSessionListRender: () => flushSessionListRender(),
    setStatus: (text, tone) => setStatus(text, tone),
    hideDecisionDock: () => hideDecisionDock(),
    persistMessage: (role, content, opts) => persistMessage(role, content, opts),
    flushPendingTextSync: () => flushPendingTextSync(),
    sealAssistantBubble: () => sealAssistantBubble(),
    settleActivity: () => settleActivity(),
    dismissLifecycleActivity: () => dismissLifecycleActivity(),
    finishToolGroup: (collapse) => finishToolGroup(collapse),
    getActivityEl: () => chatState.activityEl,
    setActivityEl: (el) => {
      chatState.activityEl = el;
    },
    getToolGroupEl: () => chatState.toolGroupEl,
    setToolGroupEl: (el) => {
      chatState.toolGroupEl = el;
    },
    getAssistantBubble: () => chatState.assistantBubble,
    setAssistantBubble: (el) => {
      chatState.assistantBubble = el;
    },
    getAssistantMessageId: () => chatState.assistantMessageId,
    setAssistantMessageId: (id) => {
      chatState.assistantMessageId = id;
    },
    getAssistantRaw: () => chatState.assistantRaw,
    setAssistantRaw: (raw) => {
      chatState.assistantRaw = raw;
    },
    autoApprove: () => elevatedEl.checked,
    resendBefore: (messageId) => {
      const session = chatState.store?.sessions.find((item) =>
        item.messages.some((message) => message.id === messageId),
      );
      if (!session) return null;
      const index = session.messages.findIndex((message) => message.id === messageId);
      const asked = session.messages
        .slice(0, index)
        .reverse()
        .find((message) => message.role === "user" && message.content.trim());
      if (!asked) return null;
      return () => {
        void chatState.send?.sendAsk({
          draft: {
            id: crypto.randomUUID(),
            sessionId: session.id,
            text: asked.content,
            attachments: asked.attachments ?? [],
            mentions: [],
          },
        });
      };
    },
  });

  chatState.sessions = createSessionsController({
    sessionListEl,
    titleEl,
    promptEl,
    logEl,
    getStore: () => chatState.store,
    setStore: (next) => {
      chatState.store = next;
    },
    getBusy: () => chatState.busy,
    getRunningChatSessionId: () => chatState.runningChatSessionId,
    getPendingPermissionBatch: () => chatState.permissions.pendingPermissionBatch,
    getUnseenCompletedSessionIds: () => chatState.unseenCompletedSessionIds,
    getCurrentRuntime: () => chatState.currentRuntime,
    selectedRuntime: () => selectedRuntime(),
    sessionTitle: (session) => sessionTitle(session),
    activeSession: () => activeSession(),
    saveStore: () => saveStore(),
    setCurrentRuntime: (runtime, opts) => setCurrentRuntime(runtime, opts),
    setStatus: (text, tone) => setStatus(text, tone),
    syncComposerUi: () => syncComposerUi(),
    updateContextMeter: () => updateContextMeter(),
    closeContextPopover: () => closeContextPopover(),
    hideDecisionDock: () => hideDecisionDock(),
    flushPendingTextSync: () => flushPendingTextSync(),
    scheduleStorePersist: (delayMs) => scheduleStorePersist(delayMs),
    updateAssistantMessage: (id, content, opts) => updateAssistantMessage(id, content, opts),
    setAssistantMarkdown: (bubble, markdown) => setAssistantMarkdown(bubble, markdown),
    syncAssistantCopyButton: (bubble) => syncAssistantCopyButton(bubble),
    appendBubble: (kind, text, opts) => appendBubble(kind, text, opts),
    pushActivity: (phase, message) => pushActivity(phase, message),
    schedulePaintLivePermissionBatch: () => schedulePaintLivePermissionBatch(),
    renderActiveMessages: () => renderActiveMessages(),
    renderPendingAttachments: () => renderPendingAttachments(),
    isViewingRunningSession: () => isViewingRunningSession(),
    getAssistantBubble: () => chatState.assistantBubble,
    setAssistantBubble: (el) => {
      chatState.assistantBubble = el;
    },
    getAssistantMessageId: () => chatState.assistantMessageId,
    setAssistantMessageId: (id) => {
      chatState.assistantMessageId = id;
    },
    getAssistantRaw: () => chatState.assistantRaw,
    setAssistantRaw: (raw) => {
      chatState.assistantRaw = raw;
    },
    getPendingText: () => chatState.pendingText,
    setPendingText: (value) => {
      chatState.pendingText = value;
    },
    getTurnHadAssistantText: () => chatState.turnHadAssistantText,
    setTurnHadAssistantText: (value) => {
      chatState.turnHadAssistantText = value;
    },
    getActivityEl: () => chatState.activityEl,
    setActivityEl: (el) => {
      chatState.activityEl = el;
    },
    getLifecycleActivityEl: () => chatState.lifecycleActivityEl,
    setLifecycleActivityEl: (el) => {
      chatState.lifecycleActivityEl = el;
    },
    getToolGroupEl: () => chatState.toolGroupEl,
    setToolGroupEl: (el) => {
      chatState.toolGroupEl = el;
    },
    setPendingAttachments: (items) => {
      chatState.pendingAttachments = items;
    },
    touchSession: (session) => touchSession(session),
    isComposerLocked: () => isComposerLocked(),
    getWorkspaceDoc: () => chatState.workspaceDoc,
    syncSessionWorkspaceUi: () => syncSessionWorkspaceUi(),
    captureRunningView: () => captureRunningView(),
    restoreRunningView: () => restoreRunningView(),
    addProject: () => addProjectFromAsk(),
    removeProject: (name) => {
      void removeProjectFromAsk(name);
    },
    syncAskChrome: (showNewWorkspace) => syncAskChrome(showNewWorkspace),
  });

  chatState.stream = createStreamController({
    getStore: () => chatState.store,
    getRunningChatSessionId: () => chatState.runningChatSessionId,
    setRunningBackendSessionId: (id) => {
      chatState.runningBackendSessionId = id;
    },
    getAssistantBubble: () => chatState.assistantBubble,
    setAssistantBubble: (el) => {
      chatState.assistantBubble = el;
    },
    getAssistantMessageId: () => chatState.assistantMessageId,
    setAssistantMessageId: (id) => {
      chatState.assistantMessageId = id;
    },
    getAssistantRaw: () => chatState.assistantRaw,
    setAssistantRaw: (raw) => {
      chatState.assistantRaw = raw;
    },
    getPendingText: () => chatState.pendingText,
    setPendingText: (value) => {
      chatState.pendingText = value;
    },
    getTurnHadAssistantText: () => chatState.turnHadAssistantText,
    setTurnHadAssistantText: (value) => {
      chatState.turnHadAssistantText = value;
    },
    getUnseenCompletedSessionIds: () => chatState.unseenCompletedSessionIds,
    isEventForCurrentRun: (sessionId) => isEventForCurrentRun(sessionId),
    resolveEventChatId: (payload) => resolveEventChatId(payload),
    isForegroundChat: (chatId) => chatState.store.activeId === chatId,
    applyBackgroundEvent: (chatId, payload) => {
      applyBackgroundEvent(
        {
          sessionById: (id) => sessionById(id),
          touchSession: (session) => touchSession(session),
          scheduleStorePersist: () => scheduleStorePersist(),
          scheduleSessionListRender: () => scheduleSessionListRender(),
          setStatus: (text, tone) => setStatus(text, tone),
          getActiveId: () => chatState.store.activeId,
          endRun: (id) => setBusy(false, id),
          noteUnseen: (id) => {
            chatState.unseenCompletedSessionIds.add(id);
          },
          onPlan: (id, planEvent) => {
            const steps = normalizePlanItems(planEvent.items);
            const session = chatState.store.sessions.find((item) => item.id === id);
            if (!session) return;
            session.plan = steps.length ? { items: steps, at: Date.now() } : undefined;
            touchSession(session);
            flushStorePersist();
            if (chatState.store.activeId === id) paintChatPlan(logEl, session.plan);
          },
          autoApprove: () => elevatedEl.checked,
        },
        chatId,
        payload,
      );
    },
    setDisplayedCwd: (cwd) => setDisplayedCwd(cwd),
    pushActivity: (phase, message) => pushActivity(phase, message),
    flushSessionListRender: () => flushSessionListRender(),
    noteVerifyBrowserSignal: (text, source) => noteVerifyBrowserSignal(text, source),
    queueAssistantText: (text) => queueAssistantText(text),
    thinking: {
      append: (text) => {
        chatState.running?.setText(t("chat.thinkingLive"));
        chatState.thinking.append(text);
      },
      seal: () => chatState.thinking.seal(),
      isLive: () => chatState.thinking.isLive(),
    },
    appendStderrLine: (line) => appendStderrLine(line),
    pushPermissionCard: (payload) => pushPermissionCard(payload),
    markPermissionResolved: (requestId, allowed) => markPermissionResolved(requestId, allowed),
    isViewingRunningSession: () => isViewingRunningSession(),
    flushPendingTextSync: () => flushPendingTextSync(),
    appendAssistantChunk: (chunk) => appendAssistantChunk(chunk),
    clearEphemeralActivity: (dropStderr) => clearEphemeralActivity(dropStderr),
    sealAssistantBubble: () => sealAssistantBubble(),
    expireLivePermissionCards: () => expireLivePermissionCards(),
    hideDecisionDock: () => hideDecisionDock(),
    appendBubble: (kind, text, opts) => appendBubble(kind, text, opts),
    appendChatFailure: (explain) => showChatFailureBubble(explain),
    reportVerifyMcpIfNeeded: () => reportVerifyMcpIfNeeded(),
    applyVerifyMcpFooter: () => applyVerifyMcpFooter(),
    flushStorePersist: () => flushStorePersist(),
    setBusy: (next, chatSessionId) => setBusy(next, chatSessionId),
    settleRunRouting: (chatSessionId) => settleRunRouting(chatSessionId),
    setStatus: (text, tone) => setStatus(text, tone),
    showQuickReplies: (sourceText) => {
      if (chatState.hosted?.isActive()) {
        hideDecisionDock();
        return;
      }
      showQuickReplies(sourceText);
    },
    renderSessionList: () => renderSessionList(),
    onPlan: (items) => {
      const steps = normalizePlanItems(items);
      const id = chatState.runningChatSessionId;
      if (!id) return;
      const session = chatState.store.sessions.find((item) => item.id === id);
      if (!session) return;
      session.plan = steps.length ? { items: steps, at: Date.now() } : undefined;
      touchSession(session);
      flushStorePersist();
      if (chatState.store.activeId === id) paintChatPlan(logEl, session.plan);
    },
    onTurnCompleted: (text, status) => chatState.hosted?.noteTurnCompleted(text, status),
    onPermissionNeeded: (payload) => chatState.hosted?.notePermission(payload),
  });

  chatState.send = createSendController({
    promptEl,
    elevatedEl,
    askResources,
    mentionMenu,
    getStore: () => chatState.store,
    getBusy: () => chatState.busy,
    getBusyGen: () => chatState.busyGen,
    isChatRunning: (id) => isChatRunning(id),
    runningCount: () => runningCount(),
    getRunningChatSessionId: () => chatState.runningChatSessionId,
    getPendingAttachments: () => chatState.pendingAttachments,
    setPendingAttachments: (items) => {
      chatState.pendingAttachments = items;
    },
    resolveSendWorkspace: (session) => resolveSendWorkspace(session),
    getVerifyMcpTurn: () => chatState.verifyMcpTurn,
    setVerifyMcpTurn: (v) => {
      chatState.verifyMcpTurn = v;
    },
    getVerifySawBrowserNavigate: () => chatState.verifySawBrowserNavigate,
    setVerifySawBrowserNavigate: (v) => {
      chatState.verifySawBrowserNavigate = v;
    },
    getVerifyMcpReported: () => chatState.verifyMcpReported,
    setVerifyMcpReported: (v) => {
      chatState.verifyMcpReported = v;
    },
    getVerifyTurnText: () => chatState.verifyTurnText,
    setVerifyTurnText: (v) => {
      chatState.verifyTurnText = v;
    },
    getAssistantBubble: () => chatState.assistantBubble,
    setAssistantBubble: (el) => {
      chatState.assistantBubble = el;
    },
    getAssistantMessageId: () => chatState.assistantMessageId,
    setAssistantMessageId: (id) => {
      chatState.assistantMessageId = id;
    },
    getAssistantRaw: () => chatState.assistantRaw,
    setAssistantRaw: (raw) => {
      chatState.assistantRaw = raw;
    },
    getPendingText: () => chatState.pendingText,
    setPendingText: (value) => {
      chatState.pendingText = value;
    },
    getTurnHadAssistantText: () => chatState.turnHadAssistantText,
    setTurnHadAssistantText: (value) => {
      chatState.turnHadAssistantText = value;
    },
    setStatus: (text, tone) => setStatus(text, tone),
    selectedRuntime: () => selectedRuntime(),
    activeSession: () => activeSession(),
    ensureListener: () => ensureListener(),
    clearQuickReplies: () => clearQuickReplies(),
    allowQuickRepliesAgain: () => allowQuickRepliesAgain(),
    setBusy: (next, chatSessionId) => setBusy(next, chatSessionId),
    pushActivity: (phase, message) => pushActivity(phase, message),
    persistMessage: (role, content, opts) => persistMessage(role, content, opts),
    appendBubble: (kind, text, opts) => appendBubble(kind, text, opts),
    appendChatFailure: (explain) => showChatFailureBubble(explain),
    autoResizePrompt: () => autoResizePrompt(),
    renderPendingAttachments: () => renderPendingAttachments(),
    buildPromptWithHistory: (text, picked, chatSessionId, pictures) =>
      buildPromptWithHistory(text, picked, chatSessionId, pictures),
    setDisplayedCwd: (cwd) => setDisplayedCwd(cwd),
    sessionById: (id) => sessionById(id),
    runTargetSession: () => runTargetSession(),
    touchSession: (session) => touchSession(session),
    saveStore: () => saveStore(),
    applyVerifyMcpFooter: () => applyVerifyMcpFooter(),
    applyVerifyEvidenceFromAssistant: () => applyVerifyEvidenceFromAssistant(),
    reportVerifyMcpIfNeeded: () => reportVerifyMcpIfNeeded(),
    expireLivePermissionCards: () => expireLivePermissionCards(),
    settleRunRouting: (chatSessionId) => settleRunRouting(chatSessionId),
    renderSessionList: () => renderSessionList(),
    readImageTextEnabled: () => readImageTextEnabled(),
    refreshComposer: () => syncComposerUi(),
    providerTag: () => {
      const wired = chatState.wiredProvider;
      if (!wired) return "";
      let host = wired.url;
      try {
        host = new URL(wired.url).host;
      } catch {
        // keep the raw address
      }
      return `${wired.id}|${host}`;
    },
  });

  chatState.hosted = createHostedController({
    mainEl,
    dialogModeEl,
    voiceModeEl,
    islandEl,
    islandTitleEl,
    islandDetailEl,
    promptEl,
    setStatus: (text, tone) => setStatus(text, tone),
  latestActivity: () => chatState.latestActivityText,
  sendAsk: (opts) => sendAsk(opts),
    stopDictation: () => chatState.voiceInput.stopListening(),
    syncDictation: () => chatState.voiceInput.syncEnabled(),
    hideChoice: () => hideDecisionDock(),
  });
}
