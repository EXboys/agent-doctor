import {
  AskMentionMenuController,
  AskResourcesController,
  type AskRuntime,
  type WorkspaceDoc,
} from "./ask-resources";
import { getLocale, t, type MessageKey } from "./i18n";
import { withErrorDetail } from "./friendly-error";
import { isPersonalEdition } from "./edition";
import { modelsForProviderUrl, providerChipForUrl } from "./provider-models";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { currentMonitor, getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import type {
  PersonalProviderListItem,
  PersonalProviderStatus,
  PersonalProvidersDocument,
} from "./types";
import {
  ASK_VERIFY_DRAFT_KEY,
  COMPACT_KEEP_TURNS,
  CONTEXT_RING_LENGTH,
  LEGACY_STORAGE_KEY,
  MAX_ATTACHMENTS,
  MAX_SESSIONS,
  STORAGE_BACKUP_KEY,
  STORAGE_KEY,
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
import { isAskRuntime, runtimeDisplayName, runtimeFromLocation } from "./chat/runtime";
import {
  backupStoreRaw,
  createEmptySession,
  hasRestorableChatBackup,
  loadStore as loadStoreFromDisk,
  parseStoreRaw,
  persistStore,
  sessionTitle as sessionTitleBase,
  uid,
} from "./chat/store";
import {
  activityKind,
  cleanToolLabel,
  isQuietPhase,
  isQuietStderr,
  looksLikeChoiceQuestion,
  shortCwdLabel,
  toolSignature,
} from "./chat/format";
import {
  buildPromptWithHistory as buildPromptWithHistoryBase,
  contextUsagePercent as contextUsagePercentBase,
} from "./chat/context";
import {
  looksLikeBrowserMcpVerifyEvidence,
  looksLikeBrowserToolCall,
  withTimeoutChat,
} from "./chat/verify";
import {
  fileNameFromPath,
  isImagePath,
} from "./chat/copy-ui";
import { createPermissionsController, type PermissionsApi } from "./chat/permissions";
import { createSessionsController, type SessionsApi } from "./chat/sessions";
import { createBubblesController, type BubblesApi } from "./chat/bubbles";
import { createStreamController, type StreamApi } from "./chat/stream";
import { createSendController, type SendApi } from "./chat/send";

const elevatedEl = document.querySelector<HTMLInputElement>("#chat-elevated")!;
const elevatedLabelEl = document.querySelector<HTMLElement>("#chat-elevated-label")!;
const elevatedWrapEl = elevatedEl.closest("label") as HTMLLabelElement;
const modelBtnEl = document.querySelector<HTMLButtonElement>("#chat-model-btn")!;
const modelLabelEl = document.querySelector<HTMLElement>("#chat-model-label")!;
const modelMenuEl = document.querySelector<HTMLElement>("#chat-model-menu")!;
const modelWrapEl = modelBtnEl.closest(".chat-model-wrap") as HTMLElement;
const promptEl = document.querySelector<HTMLTextAreaElement>("#chat-prompt")!;
const actionEl = document.querySelector<HTMLButtonElement>("#chat-action")!;
const attachEl = document.querySelector<HTMLButtonElement>("#chat-attach")!;
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
): string {
  const session = sessionById(sessionId) ?? (busy ? runTargetSession() : activeSession());
  return buildPromptWithHistoryBase(userText, attachments, session);
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
  renderModelPickerLabel();
  updateContextMeter();
}

function closeModelMenu(): void {
  modelMenuOpen = false;
  modelMenuEl.hidden = true;
  modelBtnEl.classList.remove("is-open");
  modelBtnEl.setAttribute("aria-expanded", "false");
  modelWrapEl?.classList.remove("is-open");
  composerBoxEl.classList.remove("is-model-open");
  composerEl.classList.remove("is-model-open");
  modelMenuEl.style.left = "";
  modelMenuEl.style.right = "";
  modelMenuEl.style.top = "";
  modelMenuEl.style.bottom = "";
  modelMenuEl.style.width = "";
  modelMenuEl.style.minWidth = "";
  modelMenuEl.style.position = "";
  modelMenuEl.style.zIndex = "";
}

function positionModelMenu(): void {
  const rect = modelBtnEl.getBoundingClientRect();
  const gap = 8;
  const minWidth = Math.max(rect.width, 200);
  const maxWidth = Math.min(280, window.innerWidth - 24);
  const width = Math.min(Math.max(minWidth, rect.width), maxWidth);
  let left = rect.left;
  if (left + width > window.innerWidth - 12) {
    left = Math.max(12, window.innerWidth - 12 - width);
  }
  // Anchor just above the model button (original interaction).
  modelMenuEl.style.position = "fixed";
  modelMenuEl.style.left = `${Math.round(left)}px`;
  modelMenuEl.style.right = "auto";
  modelMenuEl.style.width = `${Math.round(width)}px`;
  modelMenuEl.style.minWidth = `${Math.round(width)}px`;
  modelMenuEl.style.bottom = `${Math.round(window.innerHeight - rect.top + gap)}px`;
  modelMenuEl.style.top = "auto";
  modelMenuEl.style.zIndex = "120";
}

function renderModelPickerLabel(): void {
  const runtimeName = runtimeDisplayName(currentRuntime);
  if (wiredProvider) {
    const chip = providerChipForUrl(wiredProvider.url, wiredProvider.name);
    const model = wiredProvider.model.trim() || "—";
    modelLabelEl.textContent = `${chip} · ${model}`;
    modelBtnEl.disabled = isComposerLocked();
    modelBtnEl.title = t("chat.modelPickHint");
    modelBtnEl.setAttribute("aria-label", modelLabelEl.textContent);
    return;
  }
  // Personal: keep clickable so the menu can say “go wire a provider”.
  // Team / locked: show runtime name only — model is chosen on the Agents page.
  if (isPersonalEdition()) {
    modelLabelEl.textContent = t("chat.modelPickLabel");
    modelBtnEl.disabled = isComposerLocked();
    modelBtnEl.title = t("chat.modelNeedProvider");
  } else {
    modelLabelEl.textContent = runtimeName;
    modelBtnEl.disabled = true;
    modelBtnEl.title = `${runtimeName} — ${t("chat.runtimeLockedHint")}`;
  }
  modelBtnEl.setAttribute("aria-label", modelLabelEl.textContent);
}

function renderModelMenu(): void {
  modelMenuEl.replaceChildren();
  if (!wiredProvider) {
    const hint = document.createElement("div");
    hint.className = "chat-model-menu-hint";
    hint.textContent = t("chat.modelNeedProvider");
    modelMenuEl.appendChild(hint);
    return;
  }
  const models = modelsForProviderUrl(wiredProvider.url, wiredProvider.model);
  if (models.length === 0) {
    const hint = document.createElement("div");
    hint.className = "chat-model-menu-hint";
    hint.textContent = wiredProvider.model || t("chat.modelNeedProvider");
    modelMenuEl.appendChild(hint);
    return;
  }
  for (const model of models) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `chat-model-option${model === wiredProvider.model ? " is-active" : ""}`;
    btn.role = "option";
    btn.textContent = model;
    btn.addEventListener("click", () => {
      void switchWiredModel(model);
    });
    modelMenuEl.appendChild(btn);
  }
}

function openModelMenu(): void {
  if (modelBtnEl.disabled || isComposerLocked()) return;
  renderModelMenu();
  modelMenuOpen = true;
  modelMenuEl.hidden = false;
  modelBtnEl.classList.add("is-open");
  modelBtnEl.setAttribute("aria-expanded", "true");
  modelWrapEl?.classList.add("is-open");
  composerBoxEl.classList.add("is-model-open");
  composerEl.classList.add("is-model-open");
  positionModelMenu();
}

async function refreshWiredProvider(): Promise<void> {
  if (!isPersonalEdition()) {
    wiredProvider = null;
    renderModelPickerLabel();
    return;
  }
  try {
    const [status, doc] = await Promise.all([
      invoke<PersonalProviderStatus>("get_personal_provider_status_command"),
      invoke<PersonalProvidersDocument>("list_personal_providers_command"),
    ]);
    const active =
      doc.providers.find((p) => p.active) ||
      (status.active_id
        ? doc.providers.find((p) => p.id === status.active_id)
        : undefined) ||
      null;
    wiredProvider = active
      ? {
          ...active,
          model: active.model || status.model || "",
          name: active.name || status.active_name || active.name,
        }
      : status.configured && status.active_id
        ? {
            id: status.active_id,
            name: status.active_name || "Provider",
            url: status.gateway_url || "",
            model: status.model || "",
            protocol: status.protocol || "openai",
            api_key_hint: status.api_key_hint || "",
            active: true,
          }
        : null;
  } catch (error) {
    console.warn("Ask: failed to load personal provider", error);
    wiredProvider = null;
  }
  renderModelPickerLabel();
  if (modelMenuOpen) renderModelMenu();
}

async function switchWiredModel(model: string): Promise<void> {
  if (!wiredProvider || isComposerLocked()) return;
  const next = model.trim();
  if (!next || next === wiredProvider.model) {
    closeModelMenu();
    return;
  }
  closeModelMenu();
  // Ask reads the active provider model from store at send time — skip full
  // activate/mode-switch so the picker stays snappy.
  const previous = wiredProvider.model;
  wiredProvider = { ...wiredProvider, model: next };
  renderModelPickerLabel();
  setStatus(t("chat.modelSwitching"), "muted");
  try {
    await invoke<PersonalProvidersDocument>("upsert_personal_provider_command", {
      id: wiredProvider.id,
      name: wiredProvider.name,
      url: wiredProvider.url,
      key: "",
      model: next,
      protocol: wiredProvider.protocol || "openai",
      activate: false,
    });
    setStatus(t("chat.modelSwitched", { model: next }), "ok");
  } catch (error) {
    wiredProvider = { ...wiredProvider, model: previous };
    renderModelPickerLabel();
    setStatus(withErrorDetail(t("chat.modelSwitchFailed"), error), "error");
  } finally {
    modelBtnEl.disabled = isComposerLocked() || !wiredProvider;
  }
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


/** Remove the transient lifecycle row once a more meaningful event replaces it. */
function dismissLifecycleActivity(): void {
  if (!lifecycleActivityEl) return;
  if (activityEl === lifecycleActivityEl) activityEl = null;
  lifecycleActivityEl.remove();
  lifecycleActivityEl = null;
}


function updateToolGroupSummary(group: HTMLDetailsElement, live: boolean): void {
  const count = group.querySelectorAll(".chat-activity.kind-tool").length;
  const label = group.querySelector<HTMLElement>(".chat-tool-group-label");
  if (!label) return;
  if (getLocale() === "zh") {
    label.textContent = live ? `正在调用工具 · ${count}` : `已调用 ${count} 个工具`;
  } else {
    label.textContent = live ? `Using tools · ${count}` : `${count} tool${count === 1 ? "" : "s"} used`;
  }
  group.classList.toggle("is-live", live);
}

function ensureToolGroup(): HTMLDetailsElement {
  if (toolGroupEl?.isConnected) return toolGroupEl;
  // Reuse a trailing unfinished tool chip instead of stacking "正在调用工具 · 1" rows.
  const last = logEl.lastElementChild as HTMLElement | null;
  if (
    last instanceof HTMLDetailsElement &&
    last.classList.contains("chat-tool-group") &&
    !last.classList.contains("chat-permission-group") &&
    !last.classList.contains("chat-turn-tools")
  ) {
    toolGroupEl = last;
    toolGroupEl.open = true;
    toolGroupEl.classList.add("is-live");
    updateToolGroupSummary(toolGroupEl, true);
    return toolGroupEl;
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
  logEl.appendChild(group);
  toolGroupEl = group;
  updateToolGroupSummary(group, true);
  return group;
}

function finishToolGroup(collapse = true): void {
  if (!toolGroupEl) return;
  if (activityEl && toolGroupEl.contains(activityEl)) settleActivity();
  updateToolGroupSummary(toolGroupEl, false);
  if (collapse) toolGroupEl.open = false;
  toolGroupEl = null;
}

/** Drop ephemeral progress rows so they don't litter the transcript. */
function clearEphemeralActivity(): void {
  settleActivity();
  finishToolGroup(true);
  for (const row of logEl.querySelectorAll<HTMLElement>(".chat-activity")) {
    const kind = row.dataset.kind ?? "";
    if (kind === "tool" || kind === "error") continue;
    row.remove();
  }
  lifecycleActivityEl = null;
  const assistants = logEl.querySelectorAll<HTMLElement>(".chat-bubble.assistant");
  const lastAssistant = assistants[assistants.length - 1];
  if (lastAssistant) collapseResolvedPermissionsBeforeAssistant(lastAssistant);
}


function appendStderrLine(line: string): void {
  if (!isViewingRunningSession()) return;
  const text = line.trim();
  if (!text || isQuietStderr(text)) return;
  const last = logEl.lastElementChild as HTMLElement | null;
  if (last?.dataset.kind === "error" && last.dataset.stderr === "1") {
    const label = last.querySelector<HTMLElement>(".chat-activity-text");
    if (label) {
      label.textContent = `${label.textContent}\n${text}`;
      logEl.scrollTop = logEl.scrollHeight;
      return;
    }
  }
  pushActivity("error", text);
  settleActivity();
  const row = logEl.lastElementChild as HTMLElement | null;
  if (row?.dataset.kind === "error") {
    row.dataset.stderr = "1";
  }
}

/** Render progress / tool calls inline in the chat stream (not a side panel). */
function pushActivity(phase: string, message: string): void {
  if (!isViewingRunningSession()) return;
  const text = message.trim() || phase;
  if (!text) return;

  // The permission card that follows carries this state and its resolution.
  if (phase === "permission") return;

  // Quiet lifecycle chatter — skip.
  if (isQuietPhase(phase) || phase === "writing") return;

  const kind = activityKind(phase);

  if (kind === "tool") {
    dismissLifecycleActivity();
    flushPendingTextSync();
    sealAssistantBubble();

    const group = ensureToolGroup();
    const list = group.querySelector<HTMLElement>(".chat-tool-list")!;
    const signature = toolSignature(text);
    const last = list.querySelector<HTMLElement>(".chat-activity.kind-tool:last-child");
    if (last?.dataset.signature === signature) {
      last.classList.add("is-live");
      last.classList.remove("is-done");
      activityEl = last;
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
    activityEl = row;
    updateToolGroupSummary(group, true);
    logEl.scrollTop = logEl.scrollHeight;
    return;
  }

  // Waiting/requesting/thinking are one evolving state, not transcript entries.
  if (kind !== "error") {
    if (lifecycleActivityEl?.isConnected) {
      lifecycleActivityEl.dataset.phase = phase;
      lifecycleActivityEl.className = `chat-activity is-live kind-${kind}`;
      const label = lifecycleActivityEl.querySelector<HTMLElement>(".chat-activity-text");
      if (label) label.textContent = text;
      activityEl = lifecycleActivityEl;
      logEl.scrollTop = logEl.scrollHeight;
      return;
    }
  } else {
    dismissLifecycleActivity();
  }

  const softPhase = kind === "write";
  const shouldAppend =
    !activityEl ||
    (!softPhase &&
      (activityEl.dataset.kind === "tool" || activityEl.dataset.phase !== phase));

  if (shouldAppend) {
    flushPendingTextSync();
    sealAssistantBubble();
    settleActivity();
    const row = document.createElement("div");
    row.className = `chat-activity is-live kind-${kind}`;
    row.dataset.phase = phase;
    row.dataset.kind = kind;
    row.innerHTML = `<span class="chat-spinner" aria-hidden="true"></span><span class="chat-activity-text"></span>`;
    const label = row.querySelector<HTMLElement>(".chat-activity-text")!;
    label.textContent = text;
    logEl.appendChild(row);
    activityEl = row;
    if (kind !== "error") lifecycleActivityEl = row;
  } else if (activityEl) {
    activityEl.dataset.phase = phase;
    activityEl.dataset.kind = kind;
    activityEl.className = `chat-activity is-live kind-${kind}`;
    const label = activityEl.querySelector<HTMLElement>(".chat-activity-text");
    if (label) label.textContent = text;
  }
  logEl.scrollTop = logEl.scrollHeight;
}

function settleActivity(): void {
  if (!activityEl) return;
  activityEl.classList.remove("is-live");
  activityEl.classList.add("is-done");
  const spinner = activityEl.querySelector(".chat-spinner");
  spinner?.remove();
  activityEl = null;
}


/** Apply queued assistant text immediately (before inserting later events). */
function flushPendingTextSync(): void {
  bubbles.flushPendingTextSync();
}

function hideDecisionDock(): void {
  decisionDockEl.hidden = true;
  decisionKickerEl.textContent = "";
  decisionTitleEl.textContent = "";
  decisionDetailEl.textContent = "";
  decisionDetailEl.hidden = true;
  decisionActionsEl.replaceChildren();
}

function showDecisionDock(opts: {
  kicker: string;
  title: string;
  detail?: string;
  onDismiss?: () => void;
  actions: Array<{ label: string; kind: "allow" | "deny" | "yes" | "no"; onClick: () => void }>;
}): void {
  decisionKickerEl.textContent = opts.kicker;
  decisionTitleEl.textContent = opts.title;
  const detail = opts.detail?.trim() ?? "";
  decisionDetailEl.textContent = detail;
  decisionDetailEl.hidden = !detail;
  decisionActionsEl.replaceChildren();
  for (const action of opts.actions) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `chat-decision-btn is-${action.kind}`;
    btn.textContent = action.label;
    btn.addEventListener("click", () => action.onClick());
    decisionActionsEl.appendChild(btn);
  }
  if (opts.onDismiss) {
    const close = document.createElement("button");
    close.type = "button";
    close.className = "chat-decision-dismiss";
    close.textContent = "×";
    close.setAttribute("aria-label", t("chat.dismissChoice"));
    close.title = t("chat.dismissChoice");
    const dismiss = opts.onDismiss;
    close.addEventListener("click", () => dismiss());
    decisionActionsEl.appendChild(close);
  }
  decisionDockEl.hidden = false;
}

function clearQuickReplies(): void {
  logEl.querySelectorAll(".chat-quick-replies").forEach((el) => el.remove());
  if (!decisionDockEl.querySelector(".chat-decision-btn.is-allow, .chat-decision-btn.is-deny")) {
    // Only clear dock when it is showing quick replies, not a permission prompt.
    if (decisionDockEl.querySelector(".chat-decision-btn.is-yes, .chat-decision-btn.is-no")) {
      hideDecisionDock();
    }
  }
}

/** Only yes/no confirmations — not open greetings like「有什么可以帮你的吗？」. */

function dismissChoice(): void {
  hideDecisionDock();
  setStatus("", "muted");
}

function showQuickReplies(sourceText: string): void {
  clearQuickReplies();
  if (!looksLikeChoiceQuestion(sourceText)) return;

  showDecisionDock({
    kicker: t("chat.needYourChoiceShort"),
    title: t("chat.decisionQuestionTitle"),
    onDismiss: dismissChoice,
    actions: [
      {
        label: t("chat.quickYes"),
        kind: "yes",
        onClick: () => {
          if (busy) return;
          hideDecisionDock();
          promptEl.value = t("chat.quickYesSend");
          autoResizePrompt();
          void sendAsk();
        },
      },
      {
        label: t("chat.quickNo"),
        kind: "no",
        onClick: () => {
          if (busy) return;
          dismissChoice();
        },
      },
    ],
  });
  setStatus(t("chat.needYourChoiceShort"), "warn");
}

function sealAssistantBubble(): void {
  bubbles.sealAssistantBubble();
}

function appendAssistantChunk(chunk: string): void {
  bubbles.appendAssistantChunk(chunk);
}


function renderPendingAttachments(): void {
  attachmentsEl.replaceChildren();
  attachmentsEl.hidden = pendingAttachments.length === 0;
  for (const item of pendingAttachments) {
    const chip = document.createElement("div");
    chip.className = "chat-attach-chip";

    if (item.kind === "image") {
      const img = document.createElement("img");
      img.className = "chat-attach-thumb";
      img.alt = item.name;
      try {
        img.src = convertFileSrc(item.path);
        chip.appendChild(img);
      } catch {
        const icon = document.createElement("div");
        icon.className = "chat-attach-icon";
        icon.textContent = "IMG";
        chip.appendChild(icon);
      }
    } else {
      const icon = document.createElement("div");
      icon.className = "chat-attach-icon";
      icon.textContent = "FILE";
      chip.appendChild(icon);
    }

    const name = document.createElement("span");
    name.className = "chat-attach-name";
    name.textContent = item.name;
    name.title = item.path;

    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "chat-attach-remove";
    remove.setAttribute("aria-label", t("chat.attachRemove"));
    remove.textContent = "×";
    remove.addEventListener("click", () => {
      pendingAttachments = pendingAttachments.filter((a) => a.id !== item.id);
      renderPendingAttachments();
    });

    chip.append(name, remove);
    attachmentsEl.appendChild(chip);
  }
}

function addAttachmentPaths(paths: string[]): void {
  if (isComposerLocked() || paths.length === 0) return;
  let added = 0;
  for (const path of paths) {
    const trimmed = path.trim();
    if (!trimmed) continue;
    if (pendingAttachments.some((a) => a.path === trimmed)) continue;
    if (pendingAttachments.length >= MAX_ATTACHMENTS) {
      setStatus(t("chat.attachLimit", { n: String(MAX_ATTACHMENTS) }), "warn");
      break;
    }
    const name = fileNameFromPath(trimmed);
    pendingAttachments.push({
      id: uid(),
      path: trimmed,
      name,
      kind: isImagePath(trimmed) ? "image" : "file",
    });
    added += 1;
  }
  if (added > 0) renderPendingAttachments();
}

async function pickAttachments(): Promise<void> {
  if (isComposerLocked()) return;
  try {
    const selected = await open({
      multiple: true,
      title: t("chat.attachPick"),
    });
    if (!selected) return;
    const paths = (Array.isArray(selected) ? selected : [selected]).filter(Boolean);
    addAttachmentPaths(paths);
  } catch (error) {
    setStatus(withErrorDetail(t("chat.attachFailed"), error), "error");
  }
}

function setComposerDropTarget(active: boolean): void {
  composerBoxEl.classList.toggle("is-drop-target", active && !isComposerLocked());
}

async function setupFileDrop(): Promise<void> {
  try {
    await getCurrentWebview().onDragDropEvent((event) => {
      const type = event.payload.type;
      if (type === "enter" || type === "over") {
        setComposerDropTarget(true);
        return;
      }
      if (type === "leave") {
        setComposerDropTarget(false);
        return;
      }
      if (type === "drop") {
        setComposerDropTarget(false);
        addAttachmentPaths(event.payload.paths ?? []);
      }
    });
  } catch {
    /* browser preview without Tauri drag-drop */
  }
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
  askResources.updateResourcesSummary();
}

/** Matches `.chat-shell.is-resources-open` grid first column. */
const RESOURCES_PANEL_WIDTH_PX = 320;
const ASK_WINDOW_MIN_WIDTH_PX = 720;
/** Remember width before opening Skills/MCP so close restores, not blindly -320. */
let askWidthBeforeResources: number | null = null;

async function adaptAskWindowForResources(open: boolean): Promise<void> {
  try {
    const win = getCurrentWindow();
    const size = await win.innerSize();
    const factor = await win.scaleFactor();
    const logicalW = size.width / factor;
    const logicalH = size.height / factor;

    let nextW: number;
    if (open) {
      askWidthBeforeResources = logicalW;
      nextW = logicalW + RESOURCES_PANEL_WIDTH_PX;
      const monitor = await currentMonitor();
      if (monitor) {
        const maxW = monitor.size.width / monitor.scaleFactor - 24;
        nextW = Math.min(nextW, maxW);
      }
    } else {
      nextW = askWidthBeforeResources ?? logicalW - RESOURCES_PANEL_WIDTH_PX;
      askWidthBeforeResources = null;
    }
    nextW = Math.max(ASK_WINDOW_MIN_WIDTH_PX, nextW);
    if (Math.abs(nextW - logicalW) < 1) return;
    await win.setSize(new LogicalSize(nextW, logicalH));
  } catch {
    // Browser / non-Tauri preview — CSS adaptation still applies.
  }
}

function toggleResourcesPanel(): void {
  const willOpen = !shellEl.classList.contains("is-resources-open");
  askResources.toggleResourcesPanel();
  void adaptAskWindowForResources(willOpen).finally(() => {
    // After layout width settles, re-measure the prompt (placeholder may wrap).
    requestAnimationFrame(() => autoResizePrompt());
  });
}

async function loadAskResources(): Promise<void> {
  const result = await askResources.loadAskResources({
    setStatus,
    renderWorkspaceSwitcher,
    cwdEl,
    workspaceHintEl,
    setDisplayedCwd,
  });
  workspaceCwd = result.workspaceCwd;
  workspaceDoc = result.workspaceDoc;
}

function syncWorkspaceActivateButton(doc: WorkspaceDoc | null = workspaceDoc): void {
  const selected = workspaceSelectEl.value.trim();
  const isCurrent = Boolean(doc?.active && selected && doc.active === selected);
  workspaceActivateEl.disabled = !selected || isCurrent || !doc || Object.keys(doc.workspaces).length === 0;
  workspaceActivateEl.classList.toggle("is-current", isCurrent);
  workspaceActivateEl.textContent = isCurrent ? t("ask.workspaceCurrent") : t("ask.workspaceActivate");
}

function renderWorkspaceSwitcher(doc: WorkspaceDoc): void {
  const names = Object.keys(doc.workspaces).sort();
  workspaceSelectEl.innerHTML = "";
  if (names.length === 0) {
    const opt = document.createElement("option");
    opt.value = "";
    opt.textContent = t("ask.workspaceEmpty");
    workspaceSelectEl.appendChild(opt);
    workspaceSelectEl.disabled = true;
    syncWorkspaceActivateButton(doc);
    return;
  }

  workspaceSelectEl.disabled = false;
  for (const name of names) {
    const opt = document.createElement("option");
    opt.value = name;
    const path = doc.workspaces[name]?.path ?? "";
    opt.textContent = path ? `${name} · ${shortCwdLabel(path)}` : name;
    if (name === doc.active) {
      opt.selected = true;
    }
    workspaceSelectEl.appendChild(opt);
  }
  if (!doc.active && names[0]) {
    workspaceSelectEl.value = names[0];
  }
  syncWorkspaceActivateButton(doc);
}

async function activateSelectedWorkspace(): Promise<void> {
  const name = workspaceSelectEl.value.trim();
  if (!name) {
    setStatus(t("ask.workspaceEmpty"), "warn");
    return;
  }
  workspaceActivateEl.disabled = true;
  setStatus(t("ask.workspaceActivating", { name }), "muted");
  try {
    await invoke("use_workspace_command", { name });
    setStatus(t("ask.workspaceActivated", { name }), "ok");
    await loadAskResources();
  } catch (error) {
    setStatus(String(error), "error");
    syncWorkspaceActivateButton();
  }
}

async function openMainWorkspace(): Promise<void> {
  try {
    await invoke("focus_main_tab_command", { tab: "workspace" });
  } catch (error) {
    setStatus(String(error), "error");
  }
}

async function openMainResources(): Promise<void> {
  try {
    await invoke("open_resources_window_command", { section: "catalog" });
  } catch (error) {
    setStatus(String(error), "error");
  }
}


/** Rough token estimate — CJK denser than ASCII. */


function closeContextPopover(): void {
  if (!contextPopoverEl || !contextMeterEl) return;
  contextPopoverEl.hidden = true;
  contextMeterEl.classList.remove("is-open");
  contextMeterEl.setAttribute("aria-expanded", "false");
  contextMeterEl.closest(".chat-context-wrap")?.classList.remove("is-open");
  composerBoxEl.classList.remove("is-context-open");
  composerEl.classList.remove("is-context-open");
  contextPopoverEl.style.left = "";
  contextPopoverEl.style.right = "";
  contextPopoverEl.style.top = "";
  contextPopoverEl.style.bottom = "";
  contextPopoverEl.style.width = "";
}

function positionContextPopover(): void {
  if (!contextPopoverEl || !contextMeterEl || contextPopoverEl.hidden) return;
  const rect = contextMeterEl.getBoundingClientRect();
  const gap = 8;
  const width = Math.min(248, window.innerWidth - 24);
  let left = rect.right - width;
  if (left < 12) left = 12;
  if (left + width > window.innerWidth - 12) {
    left = Math.max(12, window.innerWidth - 12 - width);
  }
  contextPopoverEl.style.position = "fixed";
  contextPopoverEl.style.left = `${Math.round(left)}px`;
  contextPopoverEl.style.right = "auto";
  contextPopoverEl.style.width = `${Math.round(width)}px`;
  contextPopoverEl.style.bottom = `${Math.round(window.innerHeight - rect.top + gap)}px`;
  contextPopoverEl.style.top = "auto";
  contextPopoverEl.style.zIndex = "120";
}

function openContextPopover(): void {
  if (!contextPopoverEl || !contextMeterEl) return;
  closeModelMenu();
  contextPopoverEl.hidden = false;
  contextMeterEl.classList.add("is-open");
  contextMeterEl.setAttribute("aria-expanded", "true");
  contextMeterEl.closest(".chat-context-wrap")?.classList.add("is-open");
  composerBoxEl.classList.add("is-context-open");
  composerEl.classList.add("is-context-open");
  positionContextPopover();
}

function toggleContextPopover(): void {
  if (!contextPopoverEl || contextMeterEl?.hidden) return;
  if (contextPopoverEl.hidden) openContextPopover();
  else closeContextPopover();
}

function updateContextMeter(): void {
  if (!contextMeterEl || !contextRingFillEl || !contextLabelEl) return;
  const session = activeSession();
  const turns = session.messages.filter((m) => m.role === "user" || m.role === "assistant");
  if (turns.length === 0 && !promptEl.value.trim()) {
    closeContextPopover();
    contextMeterEl.hidden = true;
    return;
  }
  contextMeterEl.hidden = false;
  const pct = contextUsagePercent(session, promptEl.value);
  const offset = CONTEXT_RING_LENGTH * (1 - pct / 100);
  contextRingFillEl.style.strokeDasharray = String(CONTEXT_RING_LENGTH);
  contextRingFillEl.style.strokeDashoffset = String(offset);
  contextLabelEl.textContent = t("chat.contextUsed", { pct: String(pct) });
  contextMeterEl.classList.toggle("is-warn", pct >= 70 && pct < 90);
  contextMeterEl.classList.toggle("is-full", pct >= 90);
  contextMeterEl.title = pct >= 70 ? t("chat.contextNearFull") : t("chat.contextMeterTitle");
  if (contextPopoverTitleEl) {
    contextPopoverTitleEl.textContent = t("chat.contextPopoverTitle", { pct: String(pct) });
  }
  if (contextPopoverBodyEl) {
    contextPopoverBodyEl.textContent =
      pct >= 70
        ? t("chat.contextNearFull")
        : t("chat.contextPopoverBody", { pct: String(pct) });
  }
  if (contextCompactEl) {
    const canCompact = turns.length > COMPACT_KEEP_TURNS && !isComposerLocked();
    contextCompactEl.disabled = !canCompact;
    contextCompactEl.hidden = false;
  }
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
  const raw = localStorage.getItem(STORAGE_BACKUP_KEY);
  if (!raw) {
    setStatus(t("chat.restoreBackupNone"), "warn");
    return false;
  }
  const loaded = parseStoreRaw(raw);
  if (!loaded) {
    setStatus(t("chat.restoreBackupNone"), "warn");
    return false;
  }
  store = loaded;
  flushStorePersist();
  renderActiveMessages();
  renderSessionList();
  titleEl.textContent = sessionTitle(activeSession());
  setStatus(t("chat.restoreBackupOk"), "ok");
  syncRestoreBackupButton();
  return true;
}

function offerBackupRestoreIfNeeded(): void {
  if (!hasRestorableChatBackup()) return;
  const session = activeSession();
  const looksEmpty =
    session.messages.length === 0 &&
    store.sessions.length <= 1 &&
    !session.title.trim();
  if (!looksEmpty) return;
  appendBubble("meta", t("chat.restoreBackupHint"), { persist: false });
  const row = document.createElement("div");
  row.className = "chat-restore-backup-row";
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "btn-secondary btn-compact";
  btn.textContent = t("chat.restoreBackup");
  btn.addEventListener("click", () => {
    restoreChatFromBackup();
    row.remove();
  });
  row.appendChild(btn);
  logEl.appendChild(row);
  logEl.scrollTop = logEl.scrollHeight;
}

function showBootFailure(error: unknown): void {
  console.error("Ask: boot failed", error);
  const shell = document.querySelector<HTMLElement>("#chat-shell");
  if (shell) shell.style.display = "grid";
  const log = document.querySelector<HTMLElement>("#chat-log");
  if (log) {
    log.replaceChildren();
    const box = document.createElement("div");
    box.className = "chat-bubble meta";
    box.style.margin = "12px";
    box.style.lineHeight = "1.5";
    box.textContent =
      "对话页加载失败。多半是本地聊天记录损坏或过大。你可以点下方按钮清空缓存后重试；若仍白屏，请完全退出 Agent Doctor 再打开。";
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "btn-primary";
    btn.style.marginTop = "10px";
    btn.textContent = "清空对话缓存并重试";
    btn.addEventListener("click", () => {
      const current = localStorage.getItem(STORAGE_KEY);
      if (current) backupStoreRaw(current);
      localStorage.removeItem(STORAGE_KEY);
      localStorage.removeItem(LEGACY_STORAGE_KEY);
      window.location.reload();
    });
    const restore = document.createElement("button");
    restore.type = "button";
    restore.className = "btn-secondary";
    restore.style.marginTop = "8px";
    restore.style.marginLeft = "8px";
    restore.textContent = t("chat.restoreBackup");
    restore.addEventListener("click", () => {
      if (restoreChatFromBackup()) {
        box.textContent = t("chat.restoreBackupOk");
        btn.remove();
        restore.remove();
      }
    });
    log.append(box, btn, restore);
  }
  const list = document.querySelector<HTMLElement>("#chat-sessions");
  if (list && list.childElementCount === 0) {
    const hint = document.createElement("p");
    hint.style.margin = "8px 10px";
    hint.style.fontSize = "0.72rem";
    hint.style.color = "var(--muted)";
    hint.textContent = "会话列表暂不可用";
    list.appendChild(hint);
  }
}


function wireChatControllers(): void {
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
    buildPromptWithHistory: (text, attachments, chatSessionId) =>
      buildPromptWithHistory(text, attachments, chatSessionId),
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
  promptEl.addEventListener("input", () => {
    autoResizePrompt();
    mentionMenu.renderMentionMenu();
    updateContextMeter();
  });
  contextMeterEl?.addEventListener("click", (event) => {
    event.stopPropagation();
    toggleContextPopover();
  });
  contextCompactEl?.addEventListener("click", (event) => {
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
    if (wrap && !wrap.contains(event.target)) {
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
