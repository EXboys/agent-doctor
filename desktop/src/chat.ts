import {
  AskMentionMenuController,
  AskResourcesController,
  type AskRuntime,
  type WorkspaceDoc,
} from "./ask-resources";
import { getLocale, t, type MessageKey } from "./i18n";
import { withErrorDetail } from "./friendly-error";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { PersonalProviderListItem } from "./types";
import {
  ASK_VERIFY_DRAFT_KEY,
  MAX_SESSIONS,
  type ChatAttachment,
  type ChatMessage,
  type ChatRole,
  type ChatSession,
  type ChatTheme,
  type PermissionMeta,
  type SessionStore,
} from "./chat/types";
import {
  applyChatTheme as applyChatThemeBase,
  currentChatTheme,
  ensureChatThemeButton,
  readStoredChatTheme,
  systemChatTheme,
} from "./chat/theme";
import { isAskRuntime, runtimeFromLocation } from "./chat/runtime";
import {
  createEmptySession,
  loadStore as loadStoreFromDisk,
  persistStore,
  sessionTitle as sessionTitleBase,
} from "./chat/store";
import { shortCwdLabel } from "./chat/format";
import {
  buildPromptWithHistory as buildPromptWithHistoryBase,
  type ImageReading,
  contextUsagePercent as contextUsagePercentBase,
} from "./chat/context";
import {
  looksLikeBrowserMcpVerifyEvidence,
  looksLikeBrowserToolCall,
  withTimeoutChat,
} from "./chat/verify";
import { createPermissionsController, type PermissionsApi } from "./chat/permissions";
import { createSessionsController, type SessionsApi } from "./chat/sessions";
import { createBubblesController, type BubblesApi } from "./chat/bubbles";
import { createStreamController, type StreamApi } from "./chat/stream";
import { createSendController, type SendApi } from "./chat/send";
import { createActivityController, type ActivityApi } from "./chat/activity";
import { createModelPickerController, type ModelPickerApi } from "./chat/model-picker";
import { createDecisionController, type DecisionApi } from "./chat/decision";
import { createAttachmentsController, type AttachmentsApi } from "./chat/attachments";
import { createContextMeterController, type ContextMeterApi } from "./chat/context-meter";
import { createShellUiController, type ShellUiApi } from "./chat/shell-ui";
import { createBackupUiController, type BackupUiApi } from "./chat/backup-ui";
import { createVoiceInputController, type VoiceInputApi } from "./chat/voice";
import { readImageTextEnabled, setReadImageTextEnabled } from "./chat/image-text";


const elevatedEl = document.querySelector<HTMLInputElement>("#chat-elevated")!;
const elevatedLabelEl = document.querySelector<HTMLElement>("#chat-elevated-label")!;
const elevatedWrapEl = elevatedEl.closest("label") as HTMLLabelElement;
const readImageEl = document.querySelector<HTMLInputElement>("#chat-read-image")!;
const readImageWrapEl = document.querySelector<HTMLElement>("#chat-read-image-wrap");
const modelBtnEl = document.querySelector<HTMLButtonElement>("#chat-model-btn")!;
const modelLabelEl = document.querySelector<HTMLElement>("#chat-model-label")!;
const modelMenuEl = document.querySelector<HTMLElement>("#chat-model-menu")!;
const modelWrapEl = modelBtnEl.closest(".chat-model-wrap") as HTMLElement;
const promptEl = document.querySelector<HTMLTextAreaElement>("#chat-prompt")!;
const actionEl = document.querySelector<HTMLButtonElement>("#chat-action")!;
const attachEl = document.querySelector<HTMLButtonElement>("#chat-attach")!;
const voiceEl = document.querySelector<HTMLButtonElement>("#chat-voice")!;
const attachmentsEl = document.querySelector<HTMLElement>("#chat-attachments")!;
const composerBoxEl = document.querySelector<HTMLElement>(".chat-composer-box")!;
const composerEl = document.querySelector<HTMLElement>(".chat-composer")!;
const contextMeterEl = document.querySelector<HTMLButtonElement>("#chat-context-meter");
const contextRingFillEl = document.querySelector<SVGCircleElement>("#chat-context-ring-fill");
const contextLabelEl = document.querySelector<HTMLElement>("#chat-context-label");
const contextPopoverEl = document.querySelector<HTMLElement>("#chat-context-popover");
const contextPopoverTitleEl = document.querySelector<HTMLElement>("#chat-context-popover-title");
const contextPopoverBodyEl = document.querySelector<HTMLElement>("#chat-context-popover-body");
const contextCompactEl = document.querySelector<HTMLButtonElement>("#chat-context-compact");
const mentionsEl = document.querySelector<HTMLElement>("#chat-mentions")!;
const mentionMenuEl = document.querySelector<HTMLElement>("#chat-mention-menu")!;
const clearEl = document.querySelector<HTMLButtonElement>("#chat-clear")!;
const restoreBackupEl = document.querySelector<HTMLButtonElement>("#chat-restore-backup");
const newSessionEl = document.querySelector<HTMLButtonElement>("#chat-new")!;
const terminalEl = document.querySelector<HTMLButtonElement>("#chat-terminal")!;
const sessionListEl = document.querySelector<HTMLElement>("#chat-sessions")!;
const logEl = document.querySelector<HTMLElement>("#chat-log")!;
const statusEl = document.querySelector<HTMLElement>("#chat-status")!;
const decisionDockEl = document.querySelector<HTMLElement>("#chat-decision-dock")!;
const decisionKickerEl = document.querySelector<HTMLElement>("#chat-decision-kicker")!;
const decisionTitleEl = document.querySelector<HTMLElement>("#chat-decision-title")!;
const decisionDetailEl = document.querySelector<HTMLElement>("#chat-decision-detail")!;
const decisionActionsEl = document.querySelector<HTMLElement>("#chat-decision-actions")!;
const cwdEl = document.querySelector<HTMLElement>("#chat-cwd")!;
const workspaceSelectEl = document.querySelector<HTMLSelectElement>("#chat-workspace-select")!;
const workspaceActivateEl = document.querySelector<HTMLButtonElement>("#chat-workspace-activate")!;
const workspaceHintEl = document.querySelector<HTMLElement>("#chat-workspace-hint")!;
const titleEl = document.querySelector<HTMLElement>("#chat-title")!;
const themeEl = ensureChatThemeButton();

function applyChatTheme(theme: ChatTheme, persist = true): void {
  applyChatThemeBase(theme, themeEl, persist);
}

function saveStore(): void {
  store = persistStore(store);
}

function sessionTitle(session: ChatSession): string {
  return sessionTitleBase(session, t("chat.untitled"));
}

function buildPromptWithHistory(
  userText: string,
  attachments: ChatAttachment[],
  sessionId?: string,
  readings?: ImageReading[],
): string {
  const session = sessionById(sessionId) ?? (busy ? runTargetSession() : activeSession());
  return buildPromptWithHistoryBase(userText, attachments, session, readings);
}

function contextUsagePercent(session: ChatSession, draft = ""): number {
  return contextUsagePercentBase(session, draft, wiredProvider?.model);
}

const shellEl = document.querySelector<HTMLElement>("#chat-shell")!;
const resourcesPanelEl = document.querySelector<HTMLElement>("#chat-resources-panel")!;
const resourcesToggleEl = document.querySelector<HTMLButtonElement>("#chat-resources-toggle")!;
const resourcesLabelEl = document.querySelector<HTMLElement>("#chat-resources-label")!;
const resourcesCountEl = document.querySelector<HTMLElement>("#chat-resources-count");
const resourcesTabsEl = document.querySelector<HTMLElement>("#chat-resources-tabs");
const resourcesSearchEl = document.querySelector<HTMLInputElement>("#chat-resources-search");
const resourcesRefreshEl = document.querySelector<HTMLButtonElement>("#chat-resources-refresh")!;
const openResourcesEl = document.querySelector<HTMLButtonElement>("#chat-open-resources")!;
const skillsListEl = document.querySelector<HTMLElement>("#chat-skills-list")!;
const skillsEmptyEl = document.querySelector<HTMLElement>("#chat-skills-empty")!;
const mcpListEl = document.querySelector<HTMLElement>("#chat-mcp-list")!;
const mcpEmptyEl = document.querySelector<HTMLElement>("#chat-mcp-empty")!;

/** Locked by main-page Ask entry (`#runtime=` / ask-window-focus). Not switched in-chat. */
let currentRuntime: AskRuntime = "claude-code";
let wiredProvider: PersonalProviderListItem | null = null;
let modelMenuOpen = false;


let store: SessionStore = (() => {
  try {
    return loadStoreFromDisk(currentRuntime);
  } catch (error) {
    console.error("Ask: failed to load chat store", error);
    const session = createEmptySession(currentRuntime);
    return { activeId: session.id, sessions: [session] };
  }
})();
let busy = false;
let busyGen = 0;
/** Frontend chat session id for the in-flight ask (null when idle). */
let runningChatSessionId: string | null = null;
/** Backend prompt-session id for the in-flight ask. */
let runningBackendSessionId: string | null = null;
/** Sessions that finished while the user was looking elsewhere — show 【完成】 until opened. */
const unseenCompletedSessionIds = new Set<string>();
let permissions!: PermissionsApi;
let sessions!: SessionsApi;
let bubbles!: BubblesApi;
let stream!: StreamApi;
let send!: SendApi;
let activity!: ActivityApi;
let modelPicker!: ModelPickerApi;
let decision!: DecisionApi;
let attachments!: AttachmentsApi;
let voiceInput!: VoiceInputApi;
let contextMeter!: ContextMeterApi;
let shellUi!: ShellUiApi;
let backupUi: BackupUiApi | undefined;

function expireLivePermissionCards(): void {
  permissions.expireLivePermissionCards();
}
function pushPermissionCard(payload: {
  session_id: string;
  request_id: string;
  tool_name: string;
  detail: string;
}): void {
  permissions.pushPermissionCard(payload);
}
function schedulePaintLivePermissionBatch(): void {
  permissions.schedulePaintLivePermissionBatch();
}
function renderPermissionCard(message: ChatMessage, interactive: boolean): HTMLElement {
  return permissions.renderPermissionCard(message, interactive);
}
function renderPermissionGroup(messages: ChatMessage[], interactive: boolean): HTMLDetailsElement {
  return permissions.renderPermissionGroup(messages, interactive);
}
function collapseResolvedPermissionsBeforeAssistant(anchor: HTMLElement): void {
  permissions.collapseResolvedPermissionsBeforeAssistant(anchor);
}
function markPermissionResolved(requestId: string, allowed: boolean): void {
  permissions.markPermissionResolved(requestId, allowed);
}
function renderSessionList(): void {
  sessions.renderSessionList();
}
function ensureRuntimeSession(runtime: AskRuntime): void {
  sessions.ensureRuntimeSession(runtime);
}
function startNewSession(): void {
  sessions.startNewSession();
}
function clearActiveSession(): void {
  sessions.clearActiveSession();
}
function compactActiveSession(): void {
  sessions.compactActiveSession();
}
let assistantBubble: HTMLElement | null = null;
let assistantMessageId: string | null = null;
let assistantRaw = "";
let activityEl: HTMLElement | null = null;
let lifecycleActivityEl: HTMLElement | null = null;
let toolGroupEl: HTMLDetailsElement | null = null;
let pendingText = "";
let pendingAttachments: ChatAttachment[] = [];
/** True once any assistant text was rendered this turn (avoids result-fallback duplicates). */
let turnHadAssistantText = false;
let workspaceCwd: string | null = null;
let workspaceDoc: WorkspaceDoc | null = null;

/** True when the open chat owns the in-flight (or just-finishing) run. */
function isViewingRunningSession(): boolean {
  return Boolean(runningChatSessionId && store.activeId === runningChatSessionId);
}

/** Composer/send locks only while a run is busy and that chat is open. */
function isComposerLocked(): boolean {
  return Boolean(busy && isViewingRunningSession());
}

function sessionById(id: string | null | undefined): ChatSession | undefined {
  if (!id) return undefined;
  return store.sessions.find((s) => s.id === id);
}

/** Session that owns the in-flight run (falls back to the open chat). */
function runTargetSession(): ChatSession {
  const running = sessionById(runningChatSessionId);
  if (running) return running;
  return activeSession();
}

/** Ignore stale events from a previous backend session. */
function isEventForCurrentRun(sessionId: string | undefined): boolean {
  // Prefer backend session id so late events still apply after invoke()>finally
  // clears `busy` a tick before the matching `completed`/delta is handled.
  if (runningBackendSessionId) {
    return !sessionId || sessionId === runningBackendSessionId;
  }
  return busy;
}

function settleRunRouting(): void {
  runningChatSessionId = null;
  runningBackendSessionId = null;
}

/** Expire leftover Allow/Deny cards when the ask ends (in place — do not reshuffle the log). */


function autoResizePrompt(): void {
  promptEl.style.height = "auto";
  const styles = window.getComputedStyle(promptEl);
  const maxHeight = Number.parseFloat(styles.maxHeight);
  const minHeight = Number.parseFloat(styles.minHeight);
  let next = promptEl.scrollHeight;
  if (Number.isFinite(minHeight)) next = Math.max(next, minHeight);
  if (Number.isFinite(maxHeight)) next = Math.min(next, maxHeight);
  promptEl.style.height = `${next}px`;
}

function selectedRuntime(): AskRuntime {
  return currentRuntime;
}


function updateRuntimeLabel(): void {
  modelPicker.updateRuntimeLabel();
}

function closeModelMenu(): void {
  modelPicker.closeModelMenu();
}
function positionModelMenu(): void {
  modelPicker.positionModelMenu();
}
function renderModelPickerLabel(): void {
  modelPicker.renderModelPickerLabel();
}
function openModelMenu(): void {
  modelPicker.openModelMenu();
}
async function refreshWiredProvider(): Promise<void> {
  await modelPicker.refreshWiredProvider();
}
function setCurrentRuntime(runtime: AskRuntime, opts?: { syncSession?: boolean }): void {
  currentRuntime = runtime;
  updateElevatedLabel();
  updateRuntimeLabel();
  if (opts?.syncSession) {
    const session = activeSession();
    if (session.messages.length === 0) {
      session.runtime = runtime;
      saveStore();
    }
  }
  void loadAskResources();
}


/** Repair historical “one token = one message” fragmentation from early Codex streaming. */


let storePersistTimer = 0;
let sessionListRenderTimer = 0;

function flushStorePersist(): void {
  if (storePersistTimer) {
    window.clearTimeout(storePersistTimer);
    storePersistTimer = 0;
  }
  saveStore();
}

/** Avoid syncing the full transcript to disk on every streaming token or tool step. */
function scheduleStorePersist(delayMs = 500): void {
  if (storePersistTimer) window.clearTimeout(storePersistTimer);
  storePersistTimer = window.setTimeout(() => {
    storePersistTimer = 0;
    saveStore();
  }, delayMs);
}

function flushSessionListRender(): void {
  if (sessionListRenderTimer) {
    window.clearTimeout(sessionListRenderTimer);
    sessionListRenderTimer = 0;
  }
  renderSessionList();
}

function scheduleSessionListRender(delayMs = 400): void {
  if (sessionListRenderTimer) window.clearTimeout(sessionListRenderTimer);
  sessionListRenderTimer = window.setTimeout(() => {
    sessionListRenderTimer = 0;
    renderSessionList();
  }, delayMs);
}


function activeSession(): ChatSession {
  let session = store.sessions.find((s) => s.id === store.activeId);
  if (!session) {
    session = createEmptySession(selectedRuntime());
    store.sessions.unshift(session);
    store.activeId = session.id;
    saveStore();
  }
  return session;
}

function touchSession(session: ChatSession): void {
  session.updatedAt = Date.now();
  store.sessions = [
    session,
    ...store.sessions.filter((s) => s.id !== session.id),
  ].slice(0, MAX_SESSIONS);
}


function applyI18n(): void {
  document.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    if (el === actionEl || el === themeEl) return;
    const key = el.dataset.i18n as MessageKey | undefined;
    if (key) el.textContent = t(key);
  });
  promptEl.placeholder = t("chat.placeholder");
  attachEl.title = t("chat.attach");
  attachEl.setAttribute("aria-label", t("chat.attach"));
  if (readImageWrapEl) {
    readImageWrapEl.title = t("chat.readImageTextHint");
  }
  voiceInput?.applyI18n();
  if (resourcesSearchEl) {
    resourcesSearchEl.placeholder = t("chat.resourcesSearch");
  }
  document.documentElement.lang = getLocale() === "zh" ? "zh-CN" : "en";
  applyChatTheme(currentChatTheme(), false);
  updateElevatedLabel();
  updateRuntimeLabel();
  syncActionButton();
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
}

function syncRestoreBackupButton(): void {
  if (!restoreBackupEl) return;
  // Always show the control; empty backup just no-ops with a status message.
  restoreBackupEl.hidden = false;
}

function updateElevatedLabel(): void {
  const runtime = selectedRuntime();
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
  if (runtime === "deepseek-harness") {
    elevatedLabelEl.textContent = t("chat.elevatedDeepseekHarness");
    elevatedEl.checked = false;
    elevatedEl.disabled = true;
    return;
  }
  elevatedEl.disabled = isComposerLocked();
  elevatedLabelEl.textContent = detail;
}

function setStatus(text: string, tone: "ok" | "warn" | "error" | "muted" = "muted"): void {
  statusEl.textContent = text;
  statusEl.classList.remove("is-ok", "is-warn", "is-error");
  if (tone === "ok") statusEl.classList.add("is-ok");
  if (tone === "warn") statusEl.classList.add("is-warn");
  if (tone === "error") statusEl.classList.add("is-error");
}

function syncActionButton(): void {
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

function syncComposerUi(): void {
  const locked = isComposerLocked();
  promptEl.disabled = locked;
  elevatedEl.disabled = locked || selectedRuntime() === "deepseek-harness";
  modelBtnEl.disabled = locked || !wiredProvider;
  if (locked) {
    closeModelMenu();
    closeContextPopover();
  }
  newSessionEl.disabled = false;
  attachEl.disabled = locked;
  if (locked && voiceInput?.isListening()) {
    void voiceInput.stopListening();
  }
  voiceInput?.syncEnabled();
  sessionListEl.classList.remove("is-busy");
  syncActionButton();
  updateElevatedLabel();
  renderModelPickerLabel();
  updateContextMeter();
}

function setBusy(next: boolean, chatSessionId?: string | null): void {
  if (next) {
    busyGen += 1;
    busy = true;
    runningChatSessionId = chatSessionId ?? store.activeId;
    syncComposerUi();
    flushSessionListRender();
    return;
  }
  const wasViewing = isViewingRunningSession();
  busy = false;
  // Do not clear runningBackendSessionId here — late prompt-session-event
  // handlers (completed / trailing deltas) must still match the run.
  syncComposerUi();
  if (wasViewing) {
    settleActivity();
    finishToolGroup(true);
    if (assistantBubble?.isConnected) {
      assistantBubble.classList.remove("is-streaming");
      syncAssistantCopyButton(assistantBubble);
    }
  }
  assistantBubble = null;
  assistantMessageId = null;
  assistantRaw = "";
  pendingText = "";
  turnHadAssistantText = false;
  activityEl = null;
  lifecycleActivityEl = null;
  toolGroupEl = null;
  flushStorePersist();
  flushSessionListRender();
}


function dismissLifecycleActivity(): void {
  activity.dismissLifecycleActivity();
}
function finishToolGroup(collapse = true): void {
  activity.finishToolGroup(collapse);
}
function clearEphemeralActivity(): void {
  activity.clearEphemeralActivity();
}
function appendStderrLine(line: string): void {
  activity.appendStderrLine(line);
}
function pushActivity(phase: string, message: string): void {
  activity.pushActivity(phase, message);
}
function settleActivity(): void {
  activity.settleActivity();
}

/** Apply queued assistant text immediately (before inserting later events). */
function flushPendingTextSync(): void {
  bubbles.flushPendingTextSync();
}

function hideDecisionDock(): void {
  decision.hideDecisionDock();
}
function clearQuickReplies(): void {
  decision.clearQuickReplies();
}
function showQuickReplies(sourceText: string): void {
  decision.showQuickReplies(sourceText);
}

function sealAssistantBubble(): void {
  bubbles.sealAssistantBubble();
}

function appendAssistantChunk(chunk: string): void {
  bubbles.appendAssistantChunk(chunk);
}


function renderPendingAttachments(): void {
  attachments.renderPendingAttachments();
}
async function pickAttachments(): Promise<void> {
  await attachments.pickAttachments();
}
async function setupFileDrop(): Promise<void> {
  await attachments.setupFileDrop();
}

function persistMessage(
  role: ChatRole,
  content: string,
  opts?: { id?: string; attachments?: ChatAttachment[]; permission?: PermissionMeta },
): ChatMessage {
  return bubbles.persistMessage(role, content, opts);
}

function updateAssistantMessage(id: string, content: string, opts?: { persist?: boolean }): void {
  bubbles.updateAssistantMessage(id, content, opts);
}


function syncAssistantCopyButton(bubble: HTMLElement): void {
  bubbles.syncAssistantCopyButton(bubble);
}


function setAssistantMarkdown(bubble: HTMLElement, markdown: string): void {
  bubbles.setAssistantMarkdown(bubble, markdown);
}

function appendBubble(
  kind: ChatRole,
  text: string,
  opts?: { id?: string; persist?: boolean; attachments?: ChatAttachment[] },
): HTMLElement {
  return bubbles.appendBubble(kind, text, opts);
}

function renderActiveMessages(): void {
  bubbles.renderActiveMessages();
}

function queueAssistantText(text: string): void {
  bubbles.queueAssistantText(text);
}

function displayCwd(): string {
  const live = cwdEl.dataset.cwd?.trim() || cwdEl.textContent?.trim();
  if (live && live !== "—") return live;
  return workspaceCwd?.trim() || "—";
}

function setDisplayedCwd(cwd: string): void {
  const value = cwd.trim() || "—";
  cwdEl.dataset.cwd = value;
  cwdEl.textContent = value;
  cwdEl.title = value;
  askResources.updateResourcesSummary();
}

const askResources = new AskResourcesController(
  {
    shellEl,
    resourcesPanelEl,
    resourcesToggleEl,
    resourcesLabelEl,
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
  displayCwd,
  shortCwdLabel,
);

const mentionMenu = new AskMentionMenuController(
  { promptEl, mentionMenuEl },
  () => askResources.mentionCandidates(),
  (mention) => askResources.upsertMention(mention),
  autoResizePrompt,
  (open) => {
    composerBoxEl.classList.toggle("is-mention-open", open);
    composerEl.classList.toggle("is-mention-open", open);
  },
);

function updateResourcesSummary(): void {
  shellUi.updateResourcesSummary();
}
function toggleResourcesPanel(): void {
  shellUi.toggleResourcesPanel();
}
async function loadAskResources(): Promise<void> {
  await shellUi.loadAskResources();
}
function syncWorkspaceActivateButton(doc: WorkspaceDoc | null = workspaceDoc): void {
  shellUi.syncWorkspaceActivateButton(doc);
}
async function activateSelectedWorkspace(): Promise<void> {
  await shellUi.activateSelectedWorkspace();
}
async function openMainWorkspace(): Promise<void> {
  await shellUi.openMainWorkspace();
}
async function openMainResources(): Promise<void> {
  await shellUi.openMainResources();
}

/** Rough token estimate — CJK denser than ASCII. */


function closeContextPopover(): void {
  contextMeter.closeContextPopover();
}
function positionContextPopover(): void {
  contextMeter.positionContextPopover();
}
function toggleContextPopover(): void {
  contextMeter.toggleContextPopover();
}
function updateContextMeter(): void {
  contextMeter.updateContextMeter();
}

async function ensureListener(): Promise<void> {
  await stream.ensureListener();
}


function readInitialRuntime(): void {
  const runtime = runtimeFromLocation();
  if (isAskRuntime(runtime)) {
    ensureRuntimeSession(runtime);
  } else {
    setCurrentRuntime(activeSession().runtime);
  }
}


/** When true, this Ask turn is a browser MCP pathway verify. */
let verifyMcpTurn = false;
let verifySawBrowserNavigate = false;
let verifyMcpReported = false;
let verifyTurnText = "";


/** OpenClaw often replies with the page title and never streams the tool name. */

function noteVerifyBrowserSignal(text: string, source: "status" | "assistant" | "tool"): void {
  if (!verifyMcpTurn || verifySawBrowserNavigate || !text) return;
  if (source === "assistant") {
    verifyTurnText += `${text}\n`;
  }
  if (source === "status") {
    // Wiring notes like "browser MCP ready" are not tool calls.
    if (looksLikeBrowserToolCall(text)) verifySawBrowserNavigate = true;
    return;
  }
  if (looksLikeBrowserToolCall(text) || looksLikeBrowserMcpVerifyEvidence(text)) {
    verifySawBrowserNavigate = true;
  }
}

function applyVerifyEvidenceFromAssistant(): void {
  if (!verifyMcpTurn || verifySawBrowserNavigate) return;
  const corpus = [verifyTurnText, assistantRaw, pendingText].filter(Boolean).join("\n");
  if (looksLikeBrowserToolCall(corpus) || looksLikeBrowserMcpVerifyEvidence(corpus)) {
    verifySawBrowserNavigate = true;
  }
}

function reportVerifyMcpIfNeeded(): void {
  if (!verifyMcpTurn || verifyMcpReported) return;
  applyVerifyEvidenceFromAssistant();
  verifyMcpReported = true;
  appendBubble(
    "meta",
    verifySawBrowserNavigate ? t("chat.verifyMcpOk") : t("chat.verifyMcpFail"),
    { persist: false },
  );
}

function applyVerifyMcpFooter(): void {
  if (!verifyMcpTurn) return;
  applyVerifyEvidenceFromAssistant();
  setStatus(
    verifySawBrowserNavigate ? t("chat.verifyMcpOk") : t("chat.verifyMcpFail"),
    verifySawBrowserNavigate ? "ok" : "error",
  );
}

function applyVerifyDraftIfAny(): void {
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
      if (!busy && promptEl.value.trim()) {
        void sendAsk({ verifyMcp: true });
      }
    }, 450);
  }
}

async function openTerminal(): Promise<void> {
  try {
    await withTimeoutChat(
      invoke("open_session_command", {
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

async function cancelAsk(): Promise<void> {
  await send.cancelAsk();
}

async function sendAsk(opts?: { verifyMcp?: boolean }): Promise<void> {
  await send.sendAsk(opts);
}


function restoreChatFromBackup(): boolean {
  return backupUi!.restoreChatFromBackup();
}
function offerBackupRestoreIfNeeded(): void {
  backupUi!.offerBackupRestoreIfNeeded();
}
function showBootFailure(error: unknown): void {
  // backupUi is wired first in wireChatControllers; if boot dies earlier, still surface UI.
  if (backupUi) {
    backupUi.showBootFailure(error);
    return;
  }
  console.error("Ask: boot failed before backup UI wired", error);
  const log = document.querySelector<HTMLElement>("#chat-log");
  if (log) {
    log.textContent =
      "对话页加载失败。请完全退出 Agent Doctor 后重试；若仍白屏，可清除本机对话缓存。";
  }
}

function wireChatControllers(): void {
  backupUi = createBackupUiController({
    logEl,
    titleEl,
    getStore: () => store,
    setStore: (next) => {
      store = next;
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

  activity = createActivityController({
    logEl,
    isViewingRunningSession: () => isViewingRunningSession(),
    flushPendingTextSync: () => flushPendingTextSync(),
    sealAssistantBubble: () => sealAssistantBubble(),
    collapseResolvedPermissionsBeforeAssistant: (anchor) =>
      collapseResolvedPermissionsBeforeAssistant(anchor),
    getActivityEl: () => activityEl,
    setActivityEl: (el) => {
      activityEl = el;
    },
    getLifecycleActivityEl: () => lifecycleActivityEl,
    setLifecycleActivityEl: (el) => {
      lifecycleActivityEl = el;
    },
    getToolGroupEl: () => toolGroupEl,
    setToolGroupEl: (el) => {
      toolGroupEl = el;
    },
  });

  modelPicker = createModelPickerController({
    modelBtnEl,
    modelLabelEl,
    modelMenuEl,
    modelWrapEl,
    composerBoxEl,
    composerEl,
    isComposerLocked: () => isComposerLocked(),
    selectedRuntime: () => selectedRuntime(),
    getWiredProvider: () => wiredProvider,
    setWiredProvider: (provider) => {
      wiredProvider = provider;
    },
    getModelMenuOpen: () => modelMenuOpen,
    setModelMenuOpen: (open) => {
      modelMenuOpen = open;
    },
    setStatus: (text, tone) => setStatus(text, tone),
    updateContextMeter: () => updateContextMeter(),
  });

  decision = createDecisionController({
    decisionDockEl,
    decisionKickerEl,
    decisionTitleEl,
    decisionDetailEl,
    decisionActionsEl,
    logEl,
    promptEl,
    getBusy: () => busy,
    setStatus: (text, tone) => setStatus(text, tone),
    autoResizePrompt: () => autoResizePrompt(),
    sendAsk: () => sendAsk(),
  });

  attachments = createAttachmentsController({
    attachmentsEl,
    composerBoxEl,
    isComposerLocked: () => isComposerLocked(),
    getPendingAttachments: () => pendingAttachments,
    setPendingAttachments: (items) => {
      pendingAttachments = items;
    },
    setStatus: (text, tone) => setStatus(text, tone),
  });

  voiceInput = createVoiceInputController({
    voiceBtnEl: voiceEl,
    promptEl,
    isComposerLocked: () => isComposerLocked(),
    setStatus: (text, tone) => setStatus(text, tone),
    autoResizePrompt: () => autoResizePrompt(),
  });

  contextMeter = createContextMeterController({
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

  shellUi = createShellUiController({
    shellEl,
    cwdEl,
    workspaceSelectEl,
    workspaceActivateEl,
    workspaceHintEl,
    askResources,
    setStatus: (text, tone) => setStatus(text, tone),
    autoResizePrompt: () => autoResizePrompt(),
    setWorkspaceCwd: (cwd) => {
      workspaceCwd = cwd;
    },
    getWorkspaceDoc: () => workspaceDoc,
    setWorkspaceDoc: (doc) => {
      workspaceDoc = doc;
    },
    setDisplayedCwd: (cwd) => setDisplayedCwd(cwd),
  });

  bubbles = createBubblesController({
    logEl,
    titleEl,
    getBusy: () => busy,
    getStore: () => store,
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
    getPendingPermissionBatch: () => permissions.pendingPermissionBatch,
    getAssistantBubble: () => assistantBubble,
    setAssistantBubble: (el) => {
      assistantBubble = el;
    },
    getAssistantMessageId: () => assistantMessageId,
    setAssistantMessageId: (id) => {
      assistantMessageId = id;
    },
    getAssistantRaw: () => assistantRaw,
    setAssistantRaw: (raw) => {
      assistantRaw = raw;
    },
    getPendingText: () => pendingText,
    setPendingText: (value) => {
      pendingText = value;
    },
    getTurnHadAssistantText: () => turnHadAssistantText,
    setTurnHadAssistantText: (value) => {
      turnHadAssistantText = value;
    },
    getActivityEl: () => activityEl,
    setActivityEl: (el) => {
      activityEl = el;
    },
    getToolGroupEl: () => toolGroupEl,
    setToolGroupEl: (el) => {
      toolGroupEl = el;
    },
    getLifecycleActivityEl: () => lifecycleActivityEl,
    setLifecycleActivityEl: (el) => {
      lifecycleActivityEl = el;
    },
  });

  permissions = createPermissionsController({
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
    getActivityEl: () => activityEl,
    setActivityEl: (el) => {
      activityEl = el;
    },
    getToolGroupEl: () => toolGroupEl,
    setToolGroupEl: (el) => {
      toolGroupEl = el;
    },
    getAssistantBubble: () => assistantBubble,
    setAssistantBubble: (el) => {
      assistantBubble = el;
    },
    getAssistantMessageId: () => assistantMessageId,
    setAssistantMessageId: (id) => {
      assistantMessageId = id;
    },
    getAssistantRaw: () => assistantRaw,
    setAssistantRaw: (raw) => {
      assistantRaw = raw;
    },
  });

  sessions = createSessionsController({
    sessionListEl,
    titleEl,
    promptEl,
    logEl,
    getStore: () => store,
    setStore: (next) => {
      store = next;
    },
    getBusy: () => busy,
    getRunningChatSessionId: () => runningChatSessionId,
    getPendingPermissionBatch: () => permissions.pendingPermissionBatch,
    getUnseenCompletedSessionIds: () => unseenCompletedSessionIds,
    getCurrentRuntime: () => currentRuntime,
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
    getAssistantBubble: () => assistantBubble,
    setAssistantBubble: (el) => {
      assistantBubble = el;
    },
    getAssistantMessageId: () => assistantMessageId,
    setAssistantMessageId: (id) => {
      assistantMessageId = id;
    },
    getAssistantRaw: () => assistantRaw,
    setAssistantRaw: (raw) => {
      assistantRaw = raw;
    },
    getPendingText: () => pendingText,
    setPendingText: (value) => {
      pendingText = value;
    },
    getTurnHadAssistantText: () => turnHadAssistantText,
    setTurnHadAssistantText: (value) => {
      turnHadAssistantText = value;
    },
    getActivityEl: () => activityEl,
    setActivityEl: (el) => {
      activityEl = el;
    },
    getLifecycleActivityEl: () => lifecycleActivityEl,
    setLifecycleActivityEl: (el) => {
      lifecycleActivityEl = el;
    },
    getToolGroupEl: () => toolGroupEl,
    setToolGroupEl: (el) => {
      toolGroupEl = el;
    },
    setPendingAttachments: (items) => {
      pendingAttachments = items;
    },
    touchSession: (session) => touchSession(session),
    isComposerLocked: () => isComposerLocked(),
  });

  stream = createStreamController({
    getStore: () => store,
    getRunningChatSessionId: () => runningChatSessionId,
    setRunningBackendSessionId: (id) => {
      runningBackendSessionId = id;
    },
    getAssistantBubble: () => assistantBubble,
    setAssistantBubble: (el) => {
      assistantBubble = el;
    },
    getAssistantMessageId: () => assistantMessageId,
    setAssistantMessageId: (id) => {
      assistantMessageId = id;
    },
    getAssistantRaw: () => assistantRaw,
    setAssistantRaw: (raw) => {
      assistantRaw = raw;
    },
    getPendingText: () => pendingText,
    setPendingText: (value) => {
      pendingText = value;
    },
    getTurnHadAssistantText: () => turnHadAssistantText,
    setTurnHadAssistantText: (value) => {
      turnHadAssistantText = value;
    },
    getUnseenCompletedSessionIds: () => unseenCompletedSessionIds,
    isEventForCurrentRun: (sessionId) => isEventForCurrentRun(sessionId),
    setDisplayedCwd: (cwd) => setDisplayedCwd(cwd),
    pushActivity: (phase, message) => pushActivity(phase, message),
    flushSessionListRender: () => flushSessionListRender(),
    noteVerifyBrowserSignal: (text, source) => noteVerifyBrowserSignal(text, source),
    queueAssistantText: (text) => queueAssistantText(text),
    appendStderrLine: (line) => appendStderrLine(line),
    pushPermissionCard: (payload) => pushPermissionCard(payload),
    markPermissionResolved: (requestId, allowed) => markPermissionResolved(requestId, allowed),
    isViewingRunningSession: () => isViewingRunningSession(),
    flushPendingTextSync: () => flushPendingTextSync(),
    appendAssistantChunk: (chunk) => appendAssistantChunk(chunk),
    clearEphemeralActivity: () => clearEphemeralActivity(),
    sealAssistantBubble: () => sealAssistantBubble(),
    expireLivePermissionCards: () => expireLivePermissionCards(),
    hideDecisionDock: () => hideDecisionDock(),
    appendBubble: (kind, text, opts) => appendBubble(kind, text, opts),
    reportVerifyMcpIfNeeded: () => reportVerifyMcpIfNeeded(),
    applyVerifyMcpFooter: () => applyVerifyMcpFooter(),
    flushStorePersist: () => flushStorePersist(),
    setBusy: (next, chatSessionId) => setBusy(next, chatSessionId),
    settleRunRouting: () => settleRunRouting(),
    setStatus: (text, tone) => setStatus(text, tone),
    showQuickReplies: (sourceText) => showQuickReplies(sourceText),
    renderSessionList: () => renderSessionList(),
  });

  send = createSendController({
    promptEl,
    elevatedEl,
    askResources,
    mentionMenu,
    getStore: () => store,
    getBusy: () => busy,
    getBusyGen: () => busyGen,
    getRunningChatSessionId: () => runningChatSessionId,
    getPendingAttachments: () => pendingAttachments,
    setPendingAttachments: (items) => {
      pendingAttachments = items;
    },
    getWorkspaceCwd: () => workspaceCwd,
    getVerifyMcpTurn: () => verifyMcpTurn,
    setVerifyMcpTurn: (v) => {
      verifyMcpTurn = v;
    },
    getVerifySawBrowserNavigate: () => verifySawBrowserNavigate,
    setVerifySawBrowserNavigate: (v) => {
      verifySawBrowserNavigate = v;
    },
    getVerifyMcpReported: () => verifyMcpReported,
    setVerifyMcpReported: (v) => {
      verifyMcpReported = v;
    },
    getVerifyTurnText: () => verifyTurnText,
    setVerifyTurnText: (v) => {
      verifyTurnText = v;
    },
    getAssistantBubble: () => assistantBubble,
    setAssistantBubble: (el) => {
      assistantBubble = el;
    },
    getAssistantMessageId: () => assistantMessageId,
    setAssistantMessageId: (id) => {
      assistantMessageId = id;
    },
    getAssistantRaw: () => assistantRaw,
    setAssistantRaw: (raw) => {
      assistantRaw = raw;
    },
    getPendingText: () => pendingText,
    setPendingText: (value) => {
      pendingText = value;
    },
    getTurnHadAssistantText: () => turnHadAssistantText,
    setTurnHadAssistantText: (value) => {
      turnHadAssistantText = value;
    },
    setStatus: (text, tone) => setStatus(text, tone),
    selectedRuntime: () => selectedRuntime(),
    activeSession: () => activeSession(),
    ensureListener: () => ensureListener(),
    clearQuickReplies: () => clearQuickReplies(),
    setBusy: (next, chatSessionId) => setBusy(next, chatSessionId),
    pushActivity: (phase, message) => pushActivity(phase, message),
    persistMessage: (role, content, opts) => persistMessage(role, content, opts),
    appendBubble: (kind, text, opts) => appendBubble(kind, text, opts),
    autoResizePrompt: () => autoResizePrompt(),
    renderPendingAttachments: () => renderPendingAttachments(),
    buildPromptWithHistory: (text, attachments, chatSessionId, readings) =>
      buildPromptWithHistory(text, attachments, chatSessionId, readings),
    setDisplayedCwd: (cwd) => setDisplayedCwd(cwd),
    sessionById: (id) => sessionById(id),
    runTargetSession: () => runTargetSession(),
    touchSession: (session) => touchSession(session),
    saveStore: () => saveStore(),
    applyVerifyMcpFooter: () => applyVerifyMcpFooter(),
    applyVerifyEvidenceFromAssistant: () => applyVerifyEvidenceFromAssistant(),
    reportVerifyMcpIfNeeded: () => reportVerifyMcpIfNeeded(),
    expireLivePermissionCards: () => expireLivePermissionCards(),
    settleRunRouting: () => settleRunRouting(),
    renderSessionList: () => renderSessionList(),
    readImageTextEnabled: () => readImageTextEnabled(),
  });
}

function boot(): void {
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
    applyVerifyDraftIfAny();
  })();

  actionEl.addEventListener("click", () => {
    if (isViewingRunningSession()) void cancelAsk();
    else void sendAsk();
  });
  attachEl.addEventListener("click", () => void pickAttachments());
  clearEl.addEventListener("click", clearActiveSession);
  restoreBackupEl?.addEventListener("click", () => {
    restoreChatFromBackup();
  });
  newSessionEl.addEventListener("click", startNewSession);
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
    askResources.setResourcesQuery(resourcesSearchEl.value);
  });
  openResourcesEl.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    void openMainResources();
  });
  workspaceActivateEl.addEventListener("click", () => void activateSelectedWorkspace());
  workspaceSelectEl.addEventListener("change", () => {
    const name = workspaceSelectEl.value.trim();
    if (!name || !workspaceDoc) {
      syncWorkspaceActivateButton();
      return;
    }
    const path = workspaceDoc.workspaces[name]?.path;
    if (path) {
      cwdEl.textContent = path;
      cwdEl.title = path;
    }
    syncWorkspaceActivateButton();
  });
  workspaceHintEl.addEventListener("dblclick", () => void openMainWorkspace());
  elevatedEl.addEventListener("change", () => {
    if (elevatedEl.checked && !window.confirm(t("chat.elevatedConfirm"))) {
      elevatedEl.checked = false;
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
    if (isComposerLocked()) return;
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
    if (modelMenuOpen) {
      closeModelMenu();
      return;
    }
    openModelMenu();
  });

  window.addEventListener("resize", () => {
    if (modelMenuOpen) positionModelMenu();
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

  void ensureListener();
  void refreshWiredProvider();
  autoResizePrompt();
  promptEl.focus();
}

try {
  boot();
} catch (error) {
  showBootFailure(error);
}
