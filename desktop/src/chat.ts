import { renderFollowQueue } from "./chat/follow-queue";
import { syncSidebarCollapseLabels } from "./chat/session-sidebar";
import { syncAccountBarLabels } from "./chat/account-bar";
import { syncTuneLabels } from "./chat/tune-menu";
import { syncKnowledgeLabels } from "./chat/knowledge-page";
import { KNOWLEDGE_RUN_PREFIX } from "./chat/knowledge-wiki";
import { syncResourcesPageLabels } from "./chat/resources-page";
import { syncScheduleLabels } from "./chat/schedule-page";
import type { PromptSessionEvent, ToolStep } from "./chat/types";
import {
  beginLiveRun,
  bindBackendSession,
  chatIdForBackend,
  endLiveRun,
  isChatRunning,
  liveRun,
  liveRuns,
} from "./chat/live-runs";
import { chatState, initChatStore } from "./chat-state";
import {
  elevatedEl,
  elevatedLabelEl,
  elevatedWrapEl,
  readImageWrapEl,
  modelBtnEl,
  promptEl,
  followQueueEl,
  actionEl,
  attachEl,
  composerBoxEl,
  composerEl,
  contextMeterEl,
  contextCompactEl,
  mentionsEl,
  mentionMenuEl,
  restoreBackupEl,
  newWorkspaceEl,
  sessionListEl,
  statusEl,
  cwdEl,
  titleEl,
  themeEl,
  shellEl,
  resourcesPanelEl,
  resourcesToggleEl,
  resourcesCountEl,
  resourcesTabsEl,
  resourcesSearchEl,
  skillsListEl,
  skillsEmptyEl,
  mcpListEl,
  mcpEmptyEl,
  mainEl,
  filesToggleEl,
  filesPanelEl,
  filesListEl,
  filesViewEl,
  filesCloseEl,
  filesLayoutEl,
  filesTreeEl,
  filesBreadcrumbEl,
  filesNameEl,
  filesPlaceholderEl,
  filesPreviewEl,
  filesEditorEl,
  filesCodeEl,
  filesImageEl,
  filesSheetEl,
  filesNoticeEl,
  filesEmptyEl,
} from "./chat-dom";
import { createFilesPanel } from "./chat/files-panel";
import { setFileLinkRoot } from "./chat/file-links";
import { bootChat } from "./chat-boot";
import {
  AskMentionMenuController,
  AskResourcesController,
  type AskRuntime,
  type WorkspaceDoc,
} from "./ask-resources";
import { getLocale, t, type MessageKey } from "./i18n";
import { withErrorDetail } from "./friendly-error";

import { registerProjectFromAsk } from "./chat/register-project";
import { defaultWorkspaceName, sessionWorkspaceName, sessionWorkspacePath } from "./chat/session-workspace";
import {
  ASK_VERIFY_DRAFT_KEY,
  MAX_SESSIONS,
  type ChatAttachment,
  type ChatMessage,
  type ChatRole,
  type ChatSession,
  type ChatTheme,
  type PermissionMeta,
} from "./chat/types";
import {
  applyChatTheme as applyChatThemeBase,
  currentChatTheme,
} from "./chat/theme";
import { emit } from "@tauri-apps/api/event";
import {
  ASK_ACTIVE_RUNTIME_EVENT,
  isAskRuntime,
  runtimeFromLocation,
} from "./chat/runtime";
import {
  createEmptySession,
  persistStore,
  sessionTitle as sessionTitleBase,
} from "./chat/store";
import {
  buildPromptWithHistory as buildPromptWithHistoryBase,
  contextUsagePercent as contextUsagePercentBase,
} from "./chat/context";
import type { PictureTurn } from "./chat/picture-route";
import {
  looksLikeBrowserMcpVerifyEvidence,
  looksLikeBrowserToolCall,
  withTimeoutChat,
} from "./chat/verify";
import { openSession, removeWorkspace } from "./ipc";

export function applyChatTheme(theme: ChatTheme, persist = true): void {
  applyChatThemeBase(theme, themeEl, persist);
}

export function saveStore(): void {
  chatState.store = persistStore(chatState.store);
}

export function sessionTitle(session: ChatSession): string {
  return sessionTitleBase(session, t("chat.untitled"));
}

export function buildPromptWithHistory(
  userText: string,
  picked: ChatAttachment[],
  sessionId?: string,
  pictures?: PictureTurn,
): string {
  const session = sessionById(sessionId) ?? (chatState.busy ? runTargetSession() : activeSession());
  return buildPromptWithHistoryBase(userText, picked, session, pictures);
}

export function contextUsagePercent(session: ChatSession, draft = ""): number {
  return contextUsagePercentBase(session, draft, session.model || chatState.wiredProvider?.model);
}

export function expireLivePermissionCards(): void {
  chatState.permissions.expireLivePermissionCards();
}
export function pushPermissionCard(payload: {
  session_id: string;
  request_id: string;
  tool_name: string;
  detail: string;
  input_mode?: string;
  input_json?: string;
}): void {
  chatState.permissions.pushPermissionCard(payload);
}
export function schedulePaintLivePermissionBatch(): void {
  chatState.permissions.schedulePaintLivePermissionBatch();
}
export function renderPermissionCard(message: ChatMessage, interactive: boolean): HTMLElement {
  return chatState.permissions.renderPermissionCard(message, interactive);
}
export function renderPermissionGroup(messages: ChatMessage[], interactive: boolean): HTMLDetailsElement {
  return chatState.permissions.renderPermissionGroup(messages, interactive);
}
export function collapseResolvedPermissionsBeforeAssistant(anchor: HTMLElement): void {
  chatState.permissions.collapseResolvedPermissionsBeforeAssistant(anchor);
}
export function markPermissionResolved(requestId: string, allowed: boolean): void {
  chatState.permissions.markPermissionResolved(requestId, allowed);
}
export function renderSessionList(): void {
  chatState.sessions.renderSessionList();
}
export function ensureRuntimeSession(runtime: AskRuntime): void {
  chatState.sessions.ensureRuntimeSession(runtime);
}
export function startNewSession(workspaceName?: string | null): void {
  chatState.sessions.startNewSession(workspaceName);
}

export function resolveSendWorkspace(session: ChatSession): {
  cwd: string | null;
  workspaceName: string | null;
} {
  const doc = chatState.workspaceDoc;
  return {
    workspaceName: sessionWorkspaceName(session, doc),
    cwd: sessionWorkspacePath(session, doc),
  };
}

export function syncSessionWorkspaceUi(session?: ChatSession): void {
  chatState.shellUi?.syncWorkspaceForSession(session ?? activeSession());
}

/** Pin the open chat to a registered project (clears resume thread when the project changes). */
export function assignActiveSessionWorkspace(name: string): boolean {
  const doc = chatState.workspaceDoc;
  if (!doc?.workspaces[name]) return false;
  if (chatState.busy && isViewingRunningSession()) {
    setStatus(t("chat.cannotChangeWorkspaceRunning"), "warn");
    syncSessionWorkspaceUi();
    return false;
  }
  const session = activeSession();
  if (session.workspaceName === name) {
    syncSessionWorkspaceUi(session);
    return true;
  }
  session.workspaceName = name;
  session.runtimeThreadId = null;
  session.interrupted = null;
  touchSession(session);
  saveStore();
  const path = doc.workspaces[name]?.path?.trim();
  if (path) setDisplayedCwd(path);
  syncSessionWorkspaceUi(session);
  renderSessionList();
  setStatus(t("chat.sessionWorkspaceChanged", { name }), "ok");
  return true;
}
export function clearActiveSession(): void {
  chatState.sessions.clearActiveSession();
}
export function compactActiveSession(): void {
  chatState.sessions.compactActiveSession();
}
/** True when the open chat has a run in flight. Other chats may be running too. */
export function isViewingRunningSession(): boolean {
  return isChatRunning(chatState.store?.activeId);
}

/** Bind a backend prompt-session id to the chat that started it. */
export function resolveEventChatId(payload: PromptSessionEvent): string | null {
  if (payload.type === "started") {
    const clientId = payload.client_run_id?.trim();
    if (clientId?.startsWith(KNOWLEDGE_RUN_PREFIX)) return null;
    if (clientId && isChatRunning(clientId)) {
      bindBackendSession(clientId, payload.session_id);
      return clientId;
    }
    const waiting = [...liveRuns()].filter((run) => !run.backendSessionId);
    if (waiting.length === 1) {
      bindBackendSession(waiting[0].chatSessionId, payload.session_id);
      return waiting[0].chatSessionId;
    }
  }
  const mapped = chatIdForBackend(payload.session_id);
  return mapped && isChatRunning(mapped) ? mapped : null;
}

/** Composer/send locks only while a run is busy and that chat is open. */
export function isComposerLocked(): boolean {
  return Boolean(chatState.busy && isViewingRunningSession());
}

export function sessionById(id: string | null | undefined): ChatSession | undefined {
  if (!id) return undefined;
  return chatState.store.sessions.find((s) => s.id === id);
}

/** Session that owns the in-flight run (falls back to the open chat). */
export function runTargetSession(): ChatSession {
  const running = sessionById(chatState.runningChatSessionId);
  if (running) return running;
  return activeSession();
}

/** Ignore stale events from a previous backend session. */
export function isEventForCurrentRun(sessionId: string | undefined): boolean {
  if (sessionId && chatIdForBackend(sessionId)) return true;
  return isChatRunning(chatState.runningChatSessionId) && !sessionId;
}

export function settleRunRouting(chatSessionId?: string | null): void {
  const id = chatSessionId ?? chatState.runningChatSessionId;
  if (!id || id === chatState.store.activeId) {
    chatState.runningChatSessionId = null;
    chatState.runningBackendSessionId = null;
  }
}

/** Copy the on-screen run into its slot before leaving that chat. */
export function captureRunningView(): void {
  const id = chatState.store.activeId;
  const run = liveRun(id);
  if (!run) return;
  run.assistantMessageId = chatState.assistantMessageId;
  run.assistantRaw = chatState.assistantRaw;
  run.pendingText = chatState.pendingText;
  run.turnHadAssistantText = chatState.turnHadAssistantText;
  if (chatState.runningBackendSessionId) run.backendSessionId = chatState.runningBackendSessionId;
  run.pendingPermissions = [...(chatState.permissions?.pendingPermissionBatch ?? [])];
  const thinking = chatState.thinking?.exportState?.();
  if (thinking) {
    run.thinkingMessageId = thinking.messageId;
    run.thinkingText = thinking.text;
  }
}

/** Put a background run back on screen. */
export function restoreRunningView(): void {
  const id = chatState.store.activeId;
  const run = liveRun(id);
  if (!run) return;
  chatState.assistantMessageId = run.assistantMessageId;
  chatState.assistantRaw = run.assistantRaw;
  chatState.pendingText = run.pendingText;
  chatState.turnHadAssistantText = run.turnHadAssistantText;
  chatState.runningBackendSessionId = run.backendSessionId;
  chatState.runningChatSessionId = id;
  chatState.busy = true;
  if (chatState.permissions) {
    chatState.permissions.pendingPermissionBatch = [...run.pendingPermissions];
  }
  chatState.thinking?.importState?.({
    messageId: run.thinkingMessageId,
    text: run.thinkingText,
  });
}

/** Expire leftover Allow/Deny cards when the ask ends (in place — do not reshuffle the log). */

export function autoResizePrompt(): void {
  promptEl.style.height = "auto";
  const styles = window.getComputedStyle(promptEl);
  const maxHeight = Number.parseFloat(styles.maxHeight);
  const minHeight = Number.parseFloat(styles.minHeight);
  const content = promptEl.scrollHeight;
  let next = content;
  if (Number.isFinite(minHeight)) next = Math.max(next, minHeight);
  const capped = Number.isFinite(maxHeight) && content > maxHeight;
  if (capped) next = maxHeight;
  promptEl.style.height = `${next}px`;
  promptEl.style.overflowY = capped ? "auto" : "hidden";
}

export function selectedRuntime(): AskRuntime {
  return chatState.currentRuntime;
}

export function updateRuntimeLabel(): void {
  chatState.modelPicker.updateRuntimeLabel();
}

export function closeModelMenu(): void {
  chatState.modelPicker.closeModelMenu();
}
export function positionModelMenu(): void {
  chatState.modelPicker.positionModelMenu();
}
export function renderModelPickerLabel(): void {
  chatState.modelPicker.renderModelPickerLabel();
}
export function openModelMenu(): void {
  chatState.modelPicker.openModelMenu();
}
export async function refreshWiredProvider(): Promise<void> {
  await chatState.modelPicker.refreshWiredProvider();
}
export function setCurrentRuntime(runtime: AskRuntime, opts?: { syncSession?: boolean }): void {
  const changed = chatState.currentRuntime !== runtime;
  chatState.currentRuntime = runtime;
  updateElevatedLabel();
  updateRuntimeLabel();
  if (opts?.syncSession) {
    const session = activeSession();
    if (session.messages.length === 0) {
      session.runtime = runtime;
      saveStore();
    }
  }
  if (changed) {
    publishActiveRuntime();
  }
  void loadAskResources();
}

export function publishActiveRuntime(): void {
  void emit(ASK_ACTIVE_RUNTIME_EVENT, { runtime: chatState.currentRuntime }).catch(() => {});
}

// Repair historical “one token = one message” fragmentation from early Codex streaming.

export function flushStorePersist(): void {
  if (chatState.storePersistTimer) {
    window.clearTimeout(chatState.storePersistTimer);
    chatState.storePersistTimer = 0;
  }
  saveStore();
}

/** Avoid syncing the full transcript to disk on every streaming token or tool step. */
export function scheduleStorePersist(delayMs = 500): void {
  if (chatState.storePersistTimer) window.clearTimeout(chatState.storePersistTimer);
  chatState.storePersistTimer = window.setTimeout(() => {
    chatState.storePersistTimer = 0;
    saveStore();
  }, delayMs);
}

export function flushSessionListRender(): void {
  if (chatState.sessionListRenderTimer) {
    window.clearTimeout(chatState.sessionListRenderTimer);
    chatState.sessionListRenderTimer = 0;
  }
  renderSessionList();
}

export function scheduleSessionListRender(delayMs = 400): void {
  if (chatState.sessionListRenderTimer) window.clearTimeout(chatState.sessionListRenderTimer);
  chatState.sessionListRenderTimer = window.setTimeout(() => {
    chatState.sessionListRenderTimer = 0;
    renderSessionList();
  }, delayMs);
}

export function activeSession(): ChatSession {
  let session = chatState.store.sessions.find((s) => s.id === chatState.store.activeId);
  if (!session) {
    session = createEmptySession(selectedRuntime());
    chatState.store.sessions.unshift(session);
    chatState.store.activeId = session.id;
    saveStore();
  }
  return session;
}

export function touchSession(session: ChatSession): void {
  session.updatedAt = Date.now();
  chatState.store.sessions = [
    session,
    ...chatState.store.sessions.filter((s) => s.id !== session.id),
  ].slice(0, MAX_SESSIONS);
}

export function applyI18n(): void {
  document.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    if (el === actionEl || el === themeEl) return;
    const key = el.dataset.i18n as MessageKey | undefined;
    if (key) el.textContent = t(key);
  });
  promptEl.placeholder = t(isComposerLocked() ? "chat.placeholderBusy" : "chat.placeholder");
  attachEl.title = t("chat.attach");
  attachEl.setAttribute("aria-label", t("chat.attach"));
  newWorkspaceEl.title = t("chat.addProjectPick");
  if (readImageWrapEl) {
    readImageWrapEl.title = t("chat.readImageTextHint");
  }
  chatState.voiceInput?.applyI18n();
  chatState.hosted?.applyI18n();
  if (resourcesSearchEl) {
    resourcesSearchEl.placeholder = t("chat.resourcesSearch");
  }
  document.documentElement.lang = getLocale() === "zh" ? "zh-CN" : "en";
  applyChatTheme(currentChatTheme(), false);
  updateElevatedLabel();
  updateRuntimeLabel();
  syncActionButton();
  askFilesPanel.applyI18n();
  syncSidebarCollapseLabels();
  syncAccountBarLabels();
  syncTuneLabels();
  syncKnowledgeLabels();
  syncResourcesPageLabels();
  syncScheduleLabels();
  if (contextCompactEl) {
    contextCompactEl.textContent = t("chat.contextCompact");
    contextCompactEl.title = t("chat.contextCompactHint");
  }
  if (contextMeterEl) {
    contextMeterEl.title = t("chat.contextMeterTitle");
  }
  // Paint history list before optional resource chips — chips must not block sessions.
  renderSessionList();
  titleEl.textContent = sessionTitle(activeSession());
  try {
    updateResourcesSummary();
    askResources.renderResourceChips();
  } catch (error) {
    console.warn("Ask: resources UI update failed", error);
  }
  syncRestoreBackupButton();
  updateContextMeter();
  // Transcript labels are painted in the language of that moment. Relabel them
  // when the app language arrives or changes; static chrome alone is not enough.
  try {
    renderActiveMessages();
  } catch (error) {
    console.warn("Ask: relabel transcript failed", error);
  }
}

export function syncRestoreBackupButton(): void {
  if (!restoreBackupEl) return;
  // Always show the control; empty backup just no-ops with a status message.
  restoreBackupEl.hidden = false;
}

export function updateElevatedLabel(): void {
  const runtime = selectedRuntime();
  elevatedWrapEl.hidden = false;
  const detail =
    runtime === "codex"
      ? t("chat.elevatedCodex")
      : runtime === "hermes"
        ? t("chat.elevatedHermes")
        : runtime === "openclaw"
          ? t("chat.elevatedOpenclaw")
          : runtime === "deepseek-harness"
            ? t("chat.elevatedDeepseekHarness")
            : t("chat.elevatedClaude");
  elevatedWrapEl.title = `${detail} — ${t("chat.permissionHint")}`;
  elevatedEl.disabled = false;
  elevatedLabelEl.textContent = detail;
}

export function setStatus(text: string, tone: "ok" | "warn" | "error" | "muted" = "muted"): void {
  if (!statusEl) return;
  statusEl.textContent = text;
  statusEl.classList.remove("is-ok", "is-warn", "is-error");
  if (tone === "ok") statusEl.classList.add("is-ok");
  if (tone === "warn") statusEl.classList.add("is-warn");
  if (tone === "error") statusEl.classList.add("is-error");
}

export function syncActionButton(): void {
  if (isViewingRunningSession()) {
    actionEl.textContent = t("chat.stop");
    actionEl.classList.remove("btn-primary");
    actionEl.classList.add("btn-danger");
    actionEl.dataset.mode = "stop";
    actionEl.setAttribute("aria-label", t("chat.stop"));
  } else {
    actionEl.textContent = t("chat.send");
    actionEl.classList.remove("btn-danger");
    actionEl.classList.add("btn-primary");
    actionEl.dataset.mode = "send";
    actionEl.setAttribute("aria-label", t("chat.send"));
  }
}

export function syncComposerUi(): void {
  const viewingRun = isChatRunning(chatState.store?.activeId);
  chatState.busy = viewingRun;
  if (viewingRun) {
    chatState.runningChatSessionId = chatState.store.activeId;
  } else {
    chatState.runningChatSessionId = null;
    chatState.runningBackendSessionId = null;
  }
  const locked = isComposerLocked();
  promptEl.disabled = false;
  promptEl.readOnly = false;
  promptEl.placeholder = t(locked ? "chat.placeholderBusy" : "chat.placeholder");
  elevatedEl.disabled = false;
  modelBtnEl.disabled = locked || chatState.personalProviders.length === 0 && !chatState.wiredProvider;
  chatState.modelPicker?.renderModelPickerLabel();
  if (locked) {
    closeModelMenu();
    closeContextPopover();
  }
  newWorkspaceEl.disabled = false;
  attachEl.disabled = false;
  renderFollowQueue(followQueueEl, chatState.store.activeId, locked);
  if (locked && chatState.voiceInput?.isListening()) {
    void chatState.voiceInput.stopListening();
  }
  chatState.voiceInput?.syncEnabled();
  sessionListEl.classList.remove("is-busy");
  chatState.running?.setRunning(locked);
  syncActionButton();
  updateElevatedLabel();
  renderModelPickerLabel();
  updateContextMeter();
}

export function setBusy(next: boolean, chatSessionId?: string | null): void {
  const id = chatSessionId || chatState.store.activeId;
  if (next) {
    const run = beginLiveRun(id);
    chatState.busyGen += 1;
    run.busyGen = chatState.busyGen;
    run.assistantMessageId = null;
    run.assistantRaw = "";
    run.pendingText = "";
    run.turnHadAssistantText = false;
    run.pendingPermissions = [];
    run.thinkingMessageId = null;
    run.thinkingText = "";
    if (id === chatState.store.activeId) {
      chatState.busy = true;
      chatState.runningChatSessionId = id;
      syncComposerUi();
    }
    flushSessionListRender();
    return;
  }
  const viewing = id === chatState.store.activeId && isChatRunning(id);
  endLiveRun(id);
  if (viewing) {
    chatState.busy = false;
    chatState.runningChatSessionId = null;
    settleActivity();
    finishToolGroup(false);
    if (chatState.assistantBubble?.isConnected) {
      chatState.assistantBubble.classList.remove("is-streaming");
      syncAssistantCopyButton(chatState.assistantBubble);
    }
    chatState.assistantBubble = null;
    chatState.assistantMessageId = null;
    chatState.assistantRaw = "";
    chatState.pendingText = "";
    chatState.turnHadAssistantText = false;
    chatState.activityEl = null;
    chatState.lifecycleActivityEl = null;
    chatState.toolGroupEl = null;
    syncComposerUi();
  }
  flushStorePersist();
  flushSessionListRender();
}

export function dismissLifecycleActivity(): void {
  chatState.activity.dismissLifecycleActivity();
}
export function finishToolGroup(collapse = true): void {
  chatState.activity.finishToolGroup(collapse);
}
export function clearEphemeralActivity(dropStderr = false): void {
  chatState.activity.clearEphemeralActivity(dropStderr);
}
export function appendStderrLine(line: string): void {
  chatState.activity.appendStderrLine(line);
}
export function pushActivity(phase: string, message: string): void {
  chatState.latestActivityText = message.trim();
  chatState.running?.setText(message);
  chatState.activity.pushActivity(phase, message);
}
export function pushToolStep(step: ToolStep): void {
  chatState.activity.pushToolStep(step);
}
export function settleActivity(): void {
  chatState.activity.settleActivity();
}

/** Apply queued assistant text immediately (before inserting later events). */
export function flushPendingTextSync(): void {
  chatState.bubbles.flushPendingTextSync();
}

export function hideDecisionDock(): void {
  chatState.decision.hideDecisionDock();
}
export function clearQuickReplies(): void {
  chatState.decision.clearQuickReplies();
}
export function allowQuickRepliesAgain(): void {
  chatState.decision.allowQuickRepliesAgain();
}
export function showQuickReplies(sourceText: string): void {
  chatState.decision.showQuickReplies(sourceText);
}

export function sealAssistantBubble(): void {
  chatState.bubbles.sealAssistantBubble();
}

export function appendAssistantChunk(chunk: string): void {
  chatState.bubbles.appendAssistantChunk(chunk);
}

export function renderPendingAttachments(): void {
  chatState.attachments.renderPendingAttachments();
}
export async function pickAttachments(): Promise<void> {
  await chatState.attachments.pickAttachments();
}
export async function setupFileDrop(): Promise<void> {
  await chatState.attachments.setupFileDrop();
}

export function persistMessage(
  role: ChatRole,
  content: string,
  opts?: { id?: string; attachments?: ChatAttachment[]; permission?: PermissionMeta },
): ChatMessage {
  return chatState.bubbles.persistMessage(role, content, opts);
}

export function updateAssistantMessage(id: string, content: string, opts?: { persist?: boolean }): void {
  chatState.bubbles.updateAssistantMessage(id, content, opts);
}

export function syncAssistantCopyButton(bubble: HTMLElement): void {
  chatState.bubbles.syncAssistantCopyButton(bubble);
}

export function setAssistantMarkdown(bubble: HTMLElement, markdown: string): void {
  chatState.bubbles.setAssistantMarkdown(bubble, markdown);
}

export function appendBubble(
  kind: ChatRole,
  text: string,
  opts?: { id?: string; persist?: boolean; attachments?: ChatAttachment[] },
): HTMLElement {
  return chatState.bubbles.appendBubble(kind, text, opts);
}

export function renderActiveMessages(): void {
  chatState.bubbles.renderActiveMessages();
}

export function queueAssistantText(text: string): void {
  chatState.bubbles.queueAssistantText(text);
}

export function displayCwd(): string {
  const live = cwdEl.dataset.cwd?.trim() || cwdEl.textContent?.trim();
  if (live && live !== "—") return live;
  return chatState.workspaceCwd?.trim() || "—";
}

const askFilesPanel = createFilesPanel({
  mainEl,
  toggleEl: filesToggleEl,
  panelEl: filesPanelEl,
  listEl: filesListEl,
  viewEl: filesViewEl,
  closeEl: filesCloseEl,
  layoutEl: filesLayoutEl,
  treeEl: filesTreeEl,
  breadcrumbEl: filesBreadcrumbEl,
  fileNameEl: filesNameEl,
  placeholderEl: filesPlaceholderEl,
  previewEl: filesPreviewEl,
  editorEl: filesEditorEl,
  codeEl: filesCodeEl,
  imageEl: filesImageEl,
  sheetEl: filesSheetEl,
  noticeEl: filesNoticeEl,
  emptyEl: filesEmptyEl,
  getRoot: () => {
    const cwd = displayCwd();
    if (cwd !== "—") return cwd;
    const fromSession = sessionWorkspacePath(activeSession(), chatState.workspaceDoc);
    return fromSession?.trim() || "—";
  },
  setStatus,
});
setFileLinkRoot(() => {
  const cwd = displayCwd();
  if (cwd !== "—") return cwd;
  return sessionWorkspacePath(activeSession(), chatState.workspaceDoc)?.trim() || "—";
});

export function setDisplayedCwd(cwd: string): void {
  const value = cwd.trim() || "—";
  cwdEl.dataset.cwd = value;
  cwdEl.textContent = value;
  cwdEl.title = value;
  askResources.updateResourcesSummary();
}

export const askResources = new AskResourcesController(
  {
    shellEl,
    resourcesPanelEl,
    resourcesToggleEl,
    resourcesCountEl,
    resourcesTabsEl,
    resourcesSearchEl,
    skillsListEl,
    mcpListEl,
    skillsEmptyEl,
    mcpEmptyEl,
    mentionsEl,
  },
  selectedRuntime,
);

export const mentionMenu = new AskMentionMenuController(
  { promptEl, mentionMenuEl },
  () => askResources.mentionCandidates(),
  (mention) => askResources.upsertMention(mention),
  autoResizePrompt,
  (open) => {
    composerBoxEl.classList.toggle("is-mention-open", open);
    composerEl.classList.toggle("is-mention-open", open);
  },
);

export function updateResourcesSummary(): void {
  chatState.shellUi.updateResourcesSummary();
}
export function toggleResourcesPanel(): void {
  chatState.shellUi.toggleResourcesPanel();
}
export async function loadAskResources(): Promise<void> {
  await chatState.shellUi.loadAskResources();
  renderSessionList();
  syncSessionWorkspaceUi();
}

/** New-workspace button stays once a project exists. Skills and tools stay available. */
export function syncAskChrome(showNewWorkspace: boolean): void {
  newWorkspaceEl.hidden = !showNewWorkspace;
  resourcesToggleEl.hidden = false;
}

export async function removeProjectFromAsk(name: string): Promise<void> {
  try {
    await removeWorkspace({ name });
    const store = chatState.store;
    if (store) {
      for (const session of store.sessions) {
        if (session.workspaceName === name) session.workspaceName = undefined;
      }
      saveStore();
    }
    await loadAskResources();
    const active = activeSession();
    const stillListed = !active.workspaceName || Boolean(chatState.workspaceDoc?.workspaces[active.workspaceName]);
    if (!stillListed) {
      startNewSession(defaultWorkspaceName(chatState.workspaceDoc));
    } else {
      renderSessionList();
      syncSessionWorkspaceUi();
    }
    setStatus(t("chat.removeProjectDone", { name }), "ok");
  } catch (error) {
    setStatus(withErrorDetail(t("chat.removeProjectFailed"), error), "error");
  }
}

export function addProjectFromAsk(): void {
  void registerProjectFromAsk({
    getWorkspaceDoc: () => chatState.workspaceDoc,
    setStatus: (text, tone) => setStatus(text, tone),
    loadAskResources: () => loadAskResources(),
    renderSessionList: () => renderSessionList(),
    syncSessionWorkspaceUi: () => syncSessionWorkspaceUi(),
    startNewSession: (name) => startNewSession(name),
  });
}
export function syncWorkspaceActivateButton(doc: WorkspaceDoc | null = chatState.workspaceDoc): void {
  chatState.shellUi.syncWorkspaceActivateButton(doc);
}
export async function activateSelectedWorkspace(): Promise<void> {
  await chatState.shellUi.activateSelectedWorkspace();
  renderSessionList();
}
export async function openMainWorkspace(): Promise<void> {
  await chatState.shellUi.openMainWorkspace();
}
export async function openMainResources(): Promise<void> {
  await chatState.shellUi.openMainResources();
}

/** Rough token estimate — CJK denser than ASCII. */

export function closeContextPopover(): void {
  chatState.contextMeter.closeContextPopover();
}
export function positionContextPopover(): void {
  chatState.contextMeter.positionContextPopover();
}
export function toggleContextPopover(): void {
  chatState.contextMeter.toggleContextPopover();
}
export function updateContextMeter(): void {
  chatState.contextMeter.updateContextMeter();
}

export async function ensureListener(): Promise<void> {
  await chatState.stream.ensureListener();
}

export function readInitialRuntime(): void {
  const runtime = runtimeFromLocation();
  if (isAskRuntime(runtime)) {
    ensureRuntimeSession(runtime);
  } else {
    setCurrentRuntime(activeSession().runtime);
  }
  publishActiveRuntime();
}

/** OpenClaw often replies with the page title and never streams the tool name. */

export function noteVerifyBrowserSignal(text: string, source: "status" | "assistant" | "tool"): void {
  if (!chatState.verifyMcpTurn || chatState.verifySawBrowserNavigate || !text) return;
  if (source === "assistant") {
    chatState.verifyTurnText += `${text}\n`;
  }
  if (source === "status") {
    // Wiring notes like "browser MCP ready" are not tool calls.
    if (looksLikeBrowserToolCall(text)) chatState.verifySawBrowserNavigate = true;
    return;
  }
  if (looksLikeBrowserToolCall(text) || looksLikeBrowserMcpVerifyEvidence(text)) {
    chatState.verifySawBrowserNavigate = true;
  }
}

export function applyVerifyEvidenceFromAssistant(): void {
  if (!chatState.verifyMcpTurn || chatState.verifySawBrowserNavigate) return;
  const corpus = [chatState.verifyTurnText, chatState.assistantRaw, chatState.pendingText]
    .filter(Boolean)
    .join("\n");
  if (looksLikeBrowserToolCall(corpus) || looksLikeBrowserMcpVerifyEvidence(corpus)) {
    chatState.verifySawBrowserNavigate = true;
  }
}

export function reportVerifyMcpIfNeeded(): void {
  if (!chatState.verifyMcpTurn || chatState.verifyMcpReported) return;
  applyVerifyEvidenceFromAssistant();
  chatState.verifyMcpReported = true;
  appendBubble(
    "meta",
    chatState.verifySawBrowserNavigate ? t("chat.verifyMcpOk") : t("chat.verifyMcpFail"),
    { persist: false },
  );
}

export function applyVerifyMcpFooter(): void {
  if (!chatState.verifyMcpTurn) return;
  applyVerifyEvidenceFromAssistant();
  setStatus(
    chatState.verifySawBrowserNavigate ? t("chat.verifyMcpOk") : t("chat.verifyMcpFail"),
    chatState.verifySawBrowserNavigate ? "ok" : "error",
  );
}

export function applyVerifyDraftIfAny(): void {
  const raw = localStorage.getItem(ASK_VERIFY_DRAFT_KEY);
  if (!raw?.trim()) {
    return;
  }
  localStorage.removeItem(ASK_VERIFY_DRAFT_KEY);

  let prompt = raw.trim();
  let autoSend = false;
  try {
    const parsed = JSON.parse(raw) as { prompt?: string; autoSend?: boolean };
    if (typeof parsed.prompt === "string" && parsed.prompt.trim()) {
      prompt = parsed.prompt.trim();
      autoSend = Boolean(parsed.autoSend);
    }
  } catch {
    // Legacy plain-string draft.
  }

  promptEl.value = prompt;
  autoResizePrompt();
  setStatus(t("chat.verifyDraftReady"), "ok");
  if (autoSend) {
    window.setTimeout(() => {
      if (!chatState.busy && promptEl.value.trim()) {
        void sendAsk({ verifyMcp: true });
      }
    }, 450);
  }
}

export async function openTerminal(): Promise<void> {
  try {
    await withTimeoutChat(
      openSession({
        runtime: selectedRuntime(),
        cwd: null,
        prompt: null,
        terminal: true,
      }),
      20_000,
      new Error(t("chat.terminalFailed", { error: t("runtime.openTimeout") })),
    );
    setStatus(t("chat.terminalOpened"), "ok");
  } catch (error) {
    setStatus(withErrorDetail(t("chat.terminalFailed"), error), "error");
  }
}

export async function cancelAsk(): Promise<void> {
  await chatState.send.cancelAsk();
}

export async function sendAsk(opts?: { verifyMcp?: boolean; fromVoice?: boolean }): Promise<void> {
  await chatState.send.sendAsk(opts);
}

export function restoreChatFromBackup(): boolean {
  return chatState.backupUi!.restoreChatFromBackup();
}
export function offerBackupRestoreIfNeeded(): void {
  chatState.backupUi!.offerBackupRestoreIfNeeded();
}
export function showBootFailure(error: unknown): void {
  // backupUi is wired first in wireChatControllers; if boot dies earlier, still surface UI.
  if (chatState.backupUi) {
    chatState.backupUi.showBootFailure(error);
    return;
  }
  console.error("Ask: boot failed before backup UI wired", error);
  const log = document.querySelector<HTMLElement>("#chat-log");
  if (log) {
    log.textContent =
      "对话页加载失败。请完全退出 Agent Doctor 后重试；若仍白屏，可清除本机对话缓存。";
  }
}

initChatStore();

try {
  bootChat();
} catch (error) {
  showBootFailure(error);
}
