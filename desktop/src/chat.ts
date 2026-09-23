import {
  AskMentionMenuController,
  AskResourcesController,
  buildMentionConstraint,
  ensureBrowserMention,
  mergeMentionsForSend,
  stripMentionTokens,
  type AskRuntime,
  type WorkspaceDoc,
} from "./ask-resources";
import { getLocale, t, type MessageKey } from "./i18n";
import { isPersonalEdition } from "./edition";
import { modelsForProviderUrl, providerChipForUrl } from "./provider-models";
import { renderMarkdown } from "./markdown";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { currentMonitor, getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  PersonalProviderListItem,
  PersonalProviderStatus,
  PersonalProvidersDocument,
} from "./types";
type PromptSessionStatus = "succeeded" | "failed" | "cancelled" | "timed_out";
type ChatRole = "user" | "assistant" | "meta" | "permission";
type AttachKind = "file" | "image";

interface PromptSessionReport {
  session_id: string;
  runtime: string;
  cwd: string;
  status: PromptSessionStatus;
  exit_code: number | null;
  summary: string;
  duration_ms: number;
  runtime_thread_id?: string | null;
}

type PromptSessionEvent =
  | { type: "started"; session_id: string; runtime: string; cwd: string; command: string }
  | { type: "status"; session_id: string; phase: string; message: string }
  | { type: "delta"; session_id: string; text: string }
  | { type: "stdout_line"; session_id: string; line: string }
  | { type: "stderr_line"; session_id: string; line: string }
  | {
      type: "permission_request";
      session_id: string;
      request_id: string;
      tool_name: string;
      detail: string;
      input_json: string;
    }
  | {
      type: "permission_resolved";
      session_id: string;
      request_id: string;
      allowed: boolean;
    }
  | {
      type: "completed";
      session_id: string;
      status: PromptSessionStatus;
      exit_code: number | null;
      summary: string;
    };

interface ChatAttachment {
  id: string;
  path: string;
  name: string;
  kind: AttachKind;
}

interface PermissionMeta {
  requestId: string;
  toolName: string;
  detail: string;
  /** Backend prompt-session id used for resolve_permission_session_command */
  backendSessionId?: string;
  /** null = pending/expired without decision */
  allowed: boolean | null;
}

interface ChatMessage {
  id: string;
  role: ChatRole;
  content: string;
  at: number;
  attachments?: ChatAttachment[];
  permission?: PermissionMeta;
}

interface ChatSession {
  id: string;
  title: string;
  runtime: AskRuntime;
  createdAt: number;
  updatedAt: number;
  messages: ChatMessage[];
  /** Codex thread id / Claude session id for native resume */
  runtimeThreadId?: string | null;
}

interface SessionStore {
  activeId: string;
  sessions: ChatSession[];
}

const STORAGE_KEY = "agent-doctor.chat.sessions.v2";
const STORAGE_BACKUP_KEY = "agent-doctor.chat.sessions.v2.backup";
const LEGACY_STORAGE_KEY = "agent-doctor.chat.sessions.v1";
const MAX_MESSAGES_PER_SESSION = 120;
const MAX_SESSIONS = 40;
const MAX_CONTEXT_MESSAGES = 12;
/** Keep this many user/assistant turns after one-click compact. */
const COMPACT_KEEP_TURNS = 4;
const MAX_ATTACHMENTS = 8;
const IMAGE_EXTS = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "heic", "avif"];

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
const CONTEXT_RING_LENGTH = 2 * Math.PI * 12;
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

const CHAT_STORE_MAX_BYTES = 2_500_000;
const CHAT_THEME_KEY = "ad.ask.theme";
type ChatTheme = "light" | "dark";

function ensureChatThemeButton(): HTMLButtonElement {
  const existing = document.querySelector<HTMLButtonElement>("#chat-theme");
  if (existing) {
    return existing;
  }
  const top = document.querySelector<HTMLElement>(".chat-top");
  let actions = document.querySelector<HTMLElement>(".chat-top-actions");
  if (!actions) {
    actions = document.createElement("div");
    actions.className = "chat-top-actions";
    top?.appendChild(actions);
  }
  const btn = document.createElement("button");
  btn.id = "chat-theme";
  btn.type = "button";
  btn.className = "chat-theme";
  btn.innerHTML = `
    <svg class="chat-theme-icon chat-theme-icon-moon" width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path d="M21 14.3A8.4 8.4 0 0 1 9.7 3 7.2 7.2 0 1 0 21 14.3Z" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
    </svg>
    <svg class="chat-theme-icon chat-theme-icon-sun" width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="12" cy="12" r="4.2" stroke="currentColor" stroke-width="1.8"/>
      <path d="M12 2.5v2.2M12 19.3v2.2M2.5 12h2.2M19.3 12h2.2M5.05 5.05l1.56 1.56M17.39 17.39l1.56 1.56M5.05 18.95l1.56-1.56M17.39 6.61l1.56-1.56" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>
    </svg>
  `;
  actions.appendChild(btn);
  return btn;
}

function readStoredChatTheme(): ChatTheme | null {
  try {
    const saved = window.localStorage.getItem(CHAT_THEME_KEY);
    if (saved === "light" || saved === "dark") {
      return saved;
    }
  } catch {
    /* ignore */
  }
  return null;
}

function systemChatTheme(): ChatTheme {
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function currentChatTheme(): ChatTheme {
  return document.documentElement.dataset.theme === "dark" ? "dark" : "light";
}

function applyChatTheme(theme: ChatTheme, persist = true): void {
  document.documentElement.dataset.theme = theme;
  const nextLabel = theme === "dark" ? t("chat.themeLight") : t("chat.themeDark");
  themeEl.setAttribute("aria-label", nextLabel);
  themeEl.title = nextLabel;
  if (persist) {
    try {
      window.localStorage.setItem(CHAT_THEME_KEY, theme);
    } catch {
      /* ignore */
    }
  }
}

function normalizeChatMessage(message: ChatMessage): ChatMessage {
  return {
    ...message,
    content: typeof message.content === "string" ? message.content : String(message.content ?? ""),
    at: typeof message.at === "number" && Number.isFinite(message.at) ? message.at : Date.now(),
  };
}

let store: SessionStore = (() => {
  try {
    return loadStore();
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
let unlisten: UnlistenFn | null = null;
let assistantBubble: HTMLElement | null = null;
let assistantMessageId: string | null = null;
let assistantRaw = "";
let activityEl: HTMLElement | null = null;
let lifecycleActivityEl: HTMLElement | null = null;
let toolGroupEl: HTMLDetailsElement | null = null;
let pendingText = "";
let flushRaf = 0;
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
function expireLivePermissionCards(): void {
  if (permissionPaintTimer) {
    window.clearTimeout(permissionPaintTimer);
    permissionPaintTimer = 0;
  }
  pendingPermissionBatch = [];
  for (const card of logEl.querySelectorAll<HTMLElement>(".chat-permission.is-pending")) {
    card.classList.remove("is-pending");
    card.classList.add("is-expired");
    card.dataset.resolved = "1";
    const actions = card.querySelector(".chat-permission-actions");
    if (actions) {
      actions.replaceChildren();
      const badge = document.createElement("span");
      badge.className = "chat-permission-result";
      badge.textContent = t("chat.permissionExpired");
      actions.appendChild(badge);
    }
  }
}

function uid(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

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

function runtimeDisplayName(runtime: AskRuntime): string {
  if (runtime === "codex") return "Codex";
  if (runtime === "hermes") return "Hermes";
  if (runtime === "openclaw") return "OpenClaw";
  if (runtime === "deepseek-harness") return "DeepSeek Harness";
  return "Claude Code";
}

function isAskRuntime(value: string | null | undefined): value is AskRuntime {
  return (
    value === "claude-code" ||
    value === "codex" ||
    value === "hermes" ||
    value === "openclaw" ||
    value === "deepseek-harness"
  );
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
    setStatus(t("chat.modelSwitchFailed", { error: String(error) }), "error");
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

function normalizeSessionMessages(messages: ChatMessage[]): ChatMessage[] {
  return coalesceAssistantFragments(
    messages.map((message) => {
      const normalized = normalizeChatMessage(message);
      if (normalized.permission?.detail && normalized.permission.detail.length > 12_000) {
        return {
          ...normalized,
          permission: {
            ...normalized.permission,
            detail: `${normalized.permission.detail.slice(0, 12_000)}\n…`,
          },
        };
      }
      return normalized;
    }),
  );
}

function finalizeSessionStore(parsed: SessionStore): SessionStore {
  const sessions = parsed.sessions.map((session) => ({
    ...session,
    messages: normalizeSessionMessages((session.messages ?? []) as ChatMessage[]),
  }));
  const activeId =
    parsed.activeId && sessions.some((s) => s.id === parsed.activeId)
      ? parsed.activeId
      : sessions[0]!.id;
  return { activeId, sessions };
}

function trimStoreForSize(data: SessionStore): SessionStore {
  const sessions = data.sessions.slice(0, MAX_SESSIONS).map((session) => ({
    ...session,
    messages: session.messages.slice(-MAX_MESSAGES_PER_SESSION),
  }));
  const activeId = sessions.some((s) => s.id === data.activeId)
    ? data.activeId
    : (sessions[0]?.id ?? data.activeId);
  return { activeId, sessions };
}

function backupStoreRaw(raw: string): void {
  const trimmed = raw.trim();
  if (!trimmed) return;
  try {
    localStorage.setItem(STORAGE_BACKUP_KEY, trimmed);
  } catch {
    /* quota — keep trying on next save */
  }
}

function parseStoreRaw(raw: string): SessionStore | null {
  try {
    const parsed = JSON.parse(raw) as SessionStore;
    if (!parsed?.sessions?.length) return null;
    return finalizeSessionStore(parsed);
  } catch {
    return null;
  }
}

function loadStoreFromLocalKeys(): SessionStore | null {
  const keys = [STORAGE_KEY, STORAGE_BACKUP_KEY, LEGACY_STORAGE_KEY];
  for (const key of keys) {
    const raw = localStorage.getItem(key);
    if (!raw) continue;
    backupStoreRaw(raw);
    let loaded = parseStoreRaw(raw);
    if (!loaded) continue;
    let json = JSON.stringify(loaded);
    while (json.length > CHAT_STORE_MAX_BYTES && loaded.sessions.some((s) => s.messages.length > 8)) {
      loaded = trimStoreForSize(loaded);
      loaded = finalizeSessionStore(loaded);
      json = JSON.stringify(loaded);
    }
    if (key !== STORAGE_KEY) {
      console.warn(`Ask: restored chat store from ${key}`);
    }
    return loaded;
  }
  return null;
}

function loadStore(): SessionStore {
  const loaded = loadStoreFromLocalKeys();
  if (loaded) return loaded;
  const session = createEmptySession(currentRuntime);
  return { activeId: session.id, sessions: [session] };
}

/** Repair historical “one token = one message” fragmentation from early Codex streaming. */
function coalesceAssistantFragments(messages: ChatMessage[]): ChatMessage[] {
  const out: ChatMessage[] = [];
  for (const message of messages) {
    const prev = out[out.length - 1];
    const gap = prev ? message.at - prev.at : Number.POSITIVE_INFINITY;
    const content = typeof message.content === "string" ? message.content : "";
    const canMerge =
      message.role === "assistant" &&
      prev?.role === "assistant" &&
      !message.permission &&
      !prev.permission &&
      gap >= 0 &&
      gap < 250 &&
      content.length <= 16;
    if (canMerge && prev) {
      prev.content += message.content;
      prev.at = message.at;
      continue;
    }
    out.push({ ...message, attachments: message.attachments ? [...message.attachments] : undefined });
  }
  return out;
}

function saveStore(): void {
  const previous = localStorage.getItem(STORAGE_KEY);
  if (previous) backupStoreRaw(previous);
  let payload = JSON.stringify(store);
  if (payload.length > CHAT_STORE_MAX_BYTES) {
    store = trimStoreForSize(store);
    store = finalizeSessionStore(store);
    payload = JSON.stringify(store);
  }
  try {
    localStorage.setItem(STORAGE_KEY, payload);
  } catch (error) {
    console.warn("Ask: saveStore failed, trimming history", error);
    store = trimStoreForSize(store);
    store = finalizeSessionStore(store);
    localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
  }
}

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

function createEmptySession(runtime: AskRuntime): ChatSession {
  const now = Date.now();
  return {
    id: uid(),
    title: "",
    runtime,
    createdAt: now,
    updatedAt: now,
    messages: [],
    runtimeThreadId: null,
  };
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

function sessionTitle(session: ChatSession): string {
  if (session.title.trim()) return session.title.trim();
  const firstUser = session.messages.find((m) => m.role === "user");
  if (firstUser?.content.trim()) {
    const line = firstUser.content.trim().split(/\n/)[0];
    return line.length > 28 ? `${line.slice(0, 28)}…` : line;
  }
  return t("chat.untitled");
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

function activityKind(phase: string): "tool" | "think" | "write" | "info" | "error" {
  if (phase === "tool" || phase === "command" || phase === "permission") return "tool";
  if (phase === "thinking" || phase === "reasoning") return "think";
  if (phase === "writing" || phase === "streaming") return "write";
  if (phase === "error") return "error";
  return "info";
}

/** Lifecycle chatter that belongs in the header live pill, not the transcript. */
function isQuietPhase(phase: string): boolean {
  return phase === "writing" || phase === "streaming" || phase === "info" || phase === "done";
}

/** Remove the transient lifecycle row once a more meaningful event replaces it. */
function dismissLifecycleActivity(): void {
  if (!lifecycleActivityEl) return;
  if (activityEl === lifecycleActivityEl) activityEl = null;
  lifecycleActivityEl.remove();
  lifecycleActivityEl = null;
}

function cleanToolLabel(text: string): string {
  const cleaned = text
    .replace(/^(?:调用工具|call(?:ing)? tool)\s*/i, "")
    .replace(/[….\s]+$/g, "")
    .trim();
  const aliases = cleaned.split("__").filter(Boolean);
  return aliases.length > 1 ? aliases[aliases.length - 1] : cleaned || text;
}

function toolSignature(text: string): string {
  return cleanToolLabel(text).toLocaleLowerCase();
}

/** Prefer a short human summary; keep the full command for the expandable row. */
function formatPermissionDetail(raw: string): { summary: string; full: string } {
  const full = raw.trim();
  if (!full) {
    return { summary: "", full: "" };
  }
  const unfenced = full
    .replace(/^```(?:json)?\s*/i, "")
    .replace(/\s*```$/i, "")
    .trim();
  if (unfenced.startsWith("{") && unfenced.endsWith("}")) {
    try {
      const value = JSON.parse(unfenced) as Record<string, unknown>;
      const description =
        typeof value.description === "string" ? value.description.trim() : "";
      const command = typeof value.command === "string" ? value.command.trim() : "";
      if (description || command) {
        return {
          summary:
            description ||
            (command.replace(/\s+/g, " ").length > 96
              ? `${command.replace(/\s+/g, " ").slice(0, 96)}…`
              : command.replace(/\s+/g, " ")),
          full: command || unfenced,
        };
      }
    } catch {
      // fall through
    }
  }
  const oneLine = full.replace(/\s+/g, " ");
  return {
    summary: oneLine.length > 96 ? `${oneLine.slice(0, 96)}…` : oneLine,
    full,
  };
}

function looksLikeToolPayloadJson(text: string): boolean {
  const trimmed = text.trim();
  // Strip accidental markdown fences around a tool payload.
  const unfenced = trimmed
    .replace(/^```(?:json)?\s*/i, "")
    .replace(/\s*```$/i, "")
    .trim();
  if (!unfenced.startsWith("{") || !unfenced.endsWith("}")) {
    return false;
  }
  try {
    const value = JSON.parse(unfenced) as Record<string, unknown>;
    if (!value || typeof value !== "object" || Array.isArray(value)) {
      return false;
    }
    const hasToolShape =
      typeof value.command === "string" ||
      typeof value.description === "string" ||
      typeof value.tool === "string" ||
      typeof value.name === "string";
    return hasToolShape && !("role" in value) && !("content" in value);
  } catch {
    return false;
  }
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

function isQuietStderr(line: string): boolean {
  const text = line.trim();
  const lower = text.toLowerCase();
  return (
    /^session_id:/i.test(text) ||
    /^resume this session/i.test(text) ||
    /resumed session/i.test(text) ||
    lower.includes("unrecognized_model") ||
    lower.includes("unrecognized model") ||
    lower.startsWith("[secrets]") ||
    lower.includes("secrets.resolve unavailable") ||
    lower.includes("resolved command secrets locally") ||
    lower.includes("openclaw gateway run") ||
    lower.includes("openclaw gateway status") ||
    lower.startsWith("gateway target:") ||
    lower.startsWith("source: local loopback") ||
    lower.startsWith("bind: loopback") ||
    (lower.startsWith("config:") && lower.includes("openclaw.json"))
  );
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

type PendingPermission = {
  sessionId: string;
  requestId: string;
  toolName: string;
  detail: string;
  messageId: string;
};

/** Parallel tool asks in one turn — show one batch card instead of N confirm stacks. */
let pendingPermissionBatch: PendingPermission[] = [];
let permissionPaintTimer = 0;

function pushPermissionCard(payload: {
  session_id: string;
  request_id: string;
  tool_name: string;
  detail: string;
}): void {
  flushPendingTextSync();
  sealAssistantBubble();
  if (isViewingRunningSession()) {
    settleActivity();
    dismissLifecycleActivity();
    scrubToolFragmentsBeforePermission(payload.detail);
  }
  if (isViewingRunningSession()) {
    setStatus(t("chat.needYourChoice"), "warn");
  } else {
    setStatus(t("chat.waitingPermissionElsewhere"), "warn");
  }

  if (pendingPermissionBatch.some((item) => item.requestId === payload.request_id)) {
    return;
  }

  const persisted = persistMessage("permission", payload.detail.trim() || payload.tool_name, {
    permission: {
      requestId: payload.request_id,
      toolName: payload.tool_name,
      detail: payload.detail.trim() || payload.tool_name,
      backendSessionId: payload.session_id,
      allowed: null,
    },
  });

  pendingPermissionBatch.push({
    sessionId: payload.session_id,
    requestId: payload.request_id,
    toolName: payload.tool_name,
    detail: payload.detail.trim() || payload.tool_name,
    messageId: persisted.id,
  });

  if (isViewingRunningSession()) {
    hideDecisionDock();
    schedulePaintLivePermissionBatch();
  }
  flushSessionListRender();
}

function schedulePaintLivePermissionBatch(): void {
  // Brief coalesce so parallel Bash asks arrive as one card, not single→batch flash.
  if (permissionPaintTimer) window.clearTimeout(permissionPaintTimer);
  permissionPaintTimer = window.setTimeout(() => {
    permissionPaintTimer = 0;
    paintLivePermissionBatch();
  }, 50);
}

function clearLivePermissionBatchCard(): void {
  logEl.querySelectorAll<HTMLElement>(".chat-permission.is-batch.is-pending").forEach((el) => {
    el.remove();
  });
  for (const item of pendingPermissionBatch) {
    logEl
      .querySelectorAll<HTMLElement>(
        `.chat-permission.is-pending[data-request-id="${CSS.escape(item.requestId)}"]`,
      )
      .forEach((el) => {
        if (!el.classList.contains("is-batch")) el.remove();
      });
  }
}

function paintLivePermissionBatch(): void {
  clearLivePermissionBatchCard();
  if (pendingPermissionBatch.length === 0) {
    return;
  }

  // One pending ask: keep the compact single-tool card.
  if (pendingPermissionBatch.length === 1) {
    const only = pendingPermissionBatch[0];
    const message = runTargetSession().messages.find((m) => m.id === only.messageId);
    if (message) {
      const card = renderPermissionCard(message, true);
      logEl.appendChild(card);
      logEl.scrollTop = logEl.scrollHeight;
    }
    return;
  }

  const card = document.createElement("div");
  card.className = "chat-permission is-pending is-batch";
  card.dataset.batch = "1";

  const head = document.createElement("div");
  head.className = "chat-permission-row";
  const title = document.createElement("span");
  title.className = "chat-permission-summary chat-permission-batch-title";
  title.textContent = t("chat.permissionBatchTitle", {
    count: String(pendingPermissionBatch.length),
  });
  const actions = document.createElement("div");
  actions.className = "chat-permission-actions";
  const allowBtn = document.createElement("button");
  allowBtn.type = "button";
  allowBtn.className = "chat-permission-allow";
  allowBtn.textContent = t("chat.permissionAllowAll");
  const denyBtn = document.createElement("button");
  denyBtn.type = "button";
  denyBtn.className = "chat-permission-deny";
  denyBtn.textContent = t("chat.permissionDenyAll");
  const setBusyLocal = (busyLocal: boolean) => {
    allowBtn.disabled = busyLocal;
    denyBtn.disabled = busyLocal;
  };
  const resolveAll = async (allow: boolean) => {
    if (card.dataset.resolved === "1") return;
    setBusyLocal(true);
    const items = [...pendingPermissionBatch];
    try {
      for (const item of items) {
        await invoke<boolean>("resolve_permission_session_command", {
          sessionId: item.sessionId,
          requestId: item.requestId,
          allow,
        });
      }
    } catch (error) {
      setBusyLocal(false);
      const raw = String(error);
      if (/no active ask session/i.test(raw)) {
        setStatus(t("chat.permissionSessionGone"), "warn");
        expireLivePermissionCards();
      } else {
        setStatus(t("chat.permissionFailed", { error: raw }), "error");
      }
    }
  };
  allowBtn.addEventListener("click", () => void resolveAll(true));
  denyBtn.addEventListener("click", () => void resolveAll(false));
  actions.append(allowBtn, denyBtn);
  head.append(title, actions);
  card.appendChild(head);

  const list = document.createElement("ul");
  list.className = "chat-permission-batch-list";
  for (const item of pendingPermissionBatch) {
    const formatted = formatPermissionDetail(item.detail);
    const li = document.createElement("li");
    li.className = "chat-permission-batch-item";
    li.dataset.requestId = item.requestId;
    const tool = document.createElement("span");
    tool.className = "chat-permission-tool";
    tool.textContent = item.toolName;
    const summary = document.createElement("span");
    summary.className = "chat-permission-summary";
    summary.textContent = formatted.summary || item.toolName;
    summary.title = formatted.full || formatted.summary;
    li.append(tool, summary);
    if (formatted.full && formatted.full !== formatted.summary) {
      const more = document.createElement("details");
      more.className = "chat-permission-more";
      more.open = true;
      const moreSummary = document.createElement("summary");
      moreSummary.textContent = t("chat.permissionExpand");
      const detail = document.createElement("pre");
      detail.className = "chat-permission-detail";
      detail.textContent = formatted.full;
      more.append(moreSummary, detail);
      li.appendChild(more);
    }
    list.appendChild(li);
  }
  card.appendChild(list);
  logEl.appendChild(card);
  logEl.scrollTop = logEl.scrollHeight;
}

/** Drop the fragmented tool chip / JSON bubble that preceded this permission. */
function scrubToolFragmentsBeforePermission(detail: string): void {
  const formatted = formatPermissionDetail(detail.trim());
  const needles = [formatted.full, formatted.summary, detail.trim()]
    .map((s) => s.replace(/\s+/g, " ").trim())
    .filter((s) => s.length > 8);

  // Only strip ephemeral live tool chips — never the resolved permission history
  // group (`chat-turn-tools` / `chat-permission-group`), or a pending ask vanishes
  // when the next permission arrives.
  while (true) {
    const last = logEl.lastElementChild as HTMLElement | null;
    if (!last?.classList.contains("chat-tool-group")) break;
    if (
      last.classList.contains("chat-turn-tools") ||
      last.classList.contains("chat-permission-group") ||
      !last.classList.contains("is-live")
    ) {
      break;
    }
    last.remove();
  }
  toolGroupEl = null;
  activityEl = null;

  // Remove trailing assistant bubbles that are only the tool JSON / command dump.
  while (true) {
    const last = logEl.lastElementChild as HTMLElement | null;
    const bubble =
      last?.classList.contains("chat-msg-assistant")
        ? last.querySelector<HTMLElement>(":scope > .chat-bubble.assistant")
        : last?.classList.contains("assistant")
          ? last
          : null;
    if (!bubble) break;
    const text = bubblePlainText(bubble).replace(/\s+/g, " ").trim();
    if (!text) {
      removeAssistantBubbleElement(bubble);
      continue;
    }
    const isToolDump =
      looksLikeToolPayloadJson(text) ||
      needles.some((needle) => text === needle || text.includes(needle) || needle.includes(text));
    if (!isToolDump) break;
    removeAssistantBubbleElement(bubble);
  }
}

function removeAssistantBubbleElement(el: HTMLElement): void {
  const bubble = el.classList.contains("chat-bubble")
    ? el
    : el.querySelector<HTMLElement>(":scope > .chat-bubble.assistant") ?? el;
  const messageId = bubble.dataset.messageId;
  assistantMsgWrap(bubble).remove();
  if (!messageId) return;
  const session = runTargetSession();
  session.messages = session.messages.filter((m) => m.id !== messageId);
  saveStore();
  if (assistantMessageId === messageId) {
    assistantBubble = null;
    assistantMessageId = null;
    assistantRaw = "";
  }
}

function renderPermissionCard(message: ChatMessage, interactive: boolean): HTMLElement {
  const meta = message.permission;
  const card = document.createElement("div");
  const allowed = meta?.allowed;
  card.className =
    allowed === true
      ? "chat-permission is-allowed"
      : allowed === false
        ? "chat-permission is-denied"
        : interactive
          ? "chat-permission is-pending"
          : "chat-permission is-expired";
  card.dataset.requestId = meta?.requestId ?? "";
  card.dataset.messageId = message.id;
  if (allowed != null) card.dataset.resolved = "1";

  const toolName = meta?.toolName ?? "tool";
  const formatted = formatPermissionDetail(meta?.detail || message.content);

  const row = document.createElement("div");
  row.className = "chat-permission-row";

  const tool = document.createElement("span");
  tool.className = "chat-permission-tool";
  tool.textContent = toolName;

  const summary = document.createElement("span");
  summary.className = "chat-permission-summary";
  summary.textContent = formatted.summary || t("chat.permissionTitle", { tool: toolName });
  summary.title = formatted.full || formatted.summary;

  const actions = document.createElement("div");
  actions.className = "chat-permission-actions";

  if (interactive && allowed == null && meta) {
    const allowBtn = document.createElement("button");
    allowBtn.type = "button";
    allowBtn.className = "chat-permission-allow";
    allowBtn.textContent = t("chat.permissionAllow");
    const denyBtn = document.createElement("button");
    denyBtn.type = "button";
    denyBtn.className = "chat-permission-deny";
    denyBtn.textContent = t("chat.permissionDeny");
    const setLocalBusy = (busyLocal: boolean) => {
      allowBtn.disabled = busyLocal;
      denyBtn.disabled = busyLocal;
    };
    const resolve = async (allow: boolean) => {
      if (card.dataset.resolved === "1") return;
      setLocalBusy(true);
      try {
        await invoke<boolean>("resolve_permission_session_command", {
          sessionId: meta.backendSessionId ?? "",
          requestId: meta.requestId,
          allow,
        });
      } catch (error) {
        setLocalBusy(false);
        const raw = String(error);
        if (/no active ask session/i.test(raw)) {
          setStatus(t("chat.permissionSessionGone"), "warn");
          expireLivePermissionCards();
        } else {
          setStatus(t("chat.permissionFailed", { error: raw }), "error");
        }
      }
    };
    allowBtn.addEventListener("click", () => void resolve(true));
    denyBtn.addEventListener("click", () => void resolve(false));
    actions.append(allowBtn, denyBtn);
  } else {
    const badge = document.createElement("span");
    badge.className = "chat-permission-result";
    badge.textContent =
      allowed === true
        ? t("chat.permissionAllowed")
        : allowed === false
          ? t("chat.permissionDenied")
          : interactive
            ? t("chat.permissionExpired")
            : t("chat.permissionWaiting");
    actions.appendChild(badge);
  }

  row.append(tool, summary, actions);
  card.appendChild(row);

  if (formatted.full) {
    const showOpen =
      allowed == null &&
      (formatted.full !== formatted.summary || /[\n|&;]/.test(formatted.full));
    const more = document.createElement("details");
    more.className = "chat-permission-more";
    more.open = Boolean(showOpen && interactive);
    const moreSummary = document.createElement("summary");
    moreSummary.textContent = t("chat.permissionExpand");
    const detail = document.createElement("pre");
    detail.className = "chat-permission-detail";
    detail.textContent = formatted.full;
    more.append(moreSummary, detail);
    card.appendChild(more);
  }

  return card;
}

function permissionGroupSummaryLabel(count: number, toolNames: string[]): string {
  const allBash = toolNames.length > 0 && toolNames.every((name) => /^bash$/i.test(name));
  if (allBash) {
    return t("chat.permissionGroupBash", { count: String(count) });
  }
  return t("chat.permissionGroupTools", { count: String(count) });
}

function isCollapsiblePermissionEl(el: HTMLElement): boolean {
  if (!el.classList.contains("chat-permission")) return false;
  // Live asks and unanswered/expired cards must stay visible — folding them into
  // "$ N tools" makes the Allow/Deny UI look like it flashed away.
  if (el.classList.contains("is-pending")) return false;
  if (el.classList.contains("is-expired")) return false;
  if (el.classList.contains("is-batch")) return false;
  if (el.closest(".chat-permission-group, .chat-turn-tools")) return false;
  return el.classList.contains("is-allowed") || el.classList.contains("is-denied");
}

function isTurnToolEphemeral(el: HTMLElement): boolean {
  if (el.classList.contains("is-pending")) return false;
  if (el.classList.contains("chat-turn-tools")) return true;
  if (el.classList.contains("chat-permission-group")) return true;
  if (el.classList.contains("chat-tool-group")) return true;
  if (el.classList.contains("chat-permission")) return isCollapsiblePermissionEl(el);
  if (el.classList.contains("chat-activity") && el.dataset.kind === "tool") return true;
  return false;
}

function collectToolNamesFromNode(node: HTMLElement): string[] {
  const names: string[] = [];
  node.querySelectorAll<HTMLElement>(".chat-permission-tool").forEach((el) => {
    const name = el.textContent?.trim();
    if (name) names.push(name);
  });
  if (names.length > 0) return names;
  node.querySelectorAll<HTMLElement>(".chat-tool-cmd, .chat-activity-text").forEach((el) => {
    const label = cleanToolLabel(el.textContent ?? "");
    if (label) names.push(label.split(/\s+/)[0] || label);
  });
  return names;
}

function flattenTurnToolNode(node: HTMLElement, stack: HTMLElement): void {
  if (node.classList.contains("chat-turn-tools") || node.classList.contains("chat-permission-group")) {
    const inner =
      node.querySelector<HTMLElement>(".chat-permission-stack, .chat-tool-list") ?? null;
    if (inner) {
      while (inner.firstElementChild) {
        stack.appendChild(inner.firstElementChild);
      }
    }
    node.remove();
    return;
  }
  if (node.classList.contains("chat-tool-group")) {
    const list = node.querySelector<HTMLElement>(".chat-tool-list");
    if (list) {
      while (list.firstElementChild) {
        const row = list.firstElementChild as HTMLElement;
        row.classList.remove("is-live");
        row.classList.add("is-done");
        stack.appendChild(row);
      }
    }
    node.remove();
    return;
  }
  stack.appendChild(node);
}

function wrapTurnToolsInGroup(nodes: HTMLElement[]): HTMLDetailsElement {
  const toolNames = nodes.flatMap((node) => collectToolNamesFromNode(node));
  const count = Math.max(
    toolNames.length,
    nodes.reduce((sum, node) => {
      if (node.classList.contains("chat-permission")) return sum + 1;
      const nested =
        node.querySelectorAll(".chat-permission, .chat-activity.kind-tool").length ||
        (node.classList.contains("chat-activity") ? 1 : 0);
      return sum + nested;
    }, 0),
  );
  const group = document.createElement("details");
  group.className = "chat-tool-group chat-turn-tools chat-permission-group";
  group.open = false;

  const summary = document.createElement("summary");
  summary.className = "chat-tool-group-summary";
  const icon = document.createElement("span");
  icon.className = "chat-tool-group-icon";
  icon.setAttribute("aria-hidden", "true");
  icon.textContent = "$";
  const label = document.createElement("span");
  label.className = "chat-tool-group-label";
  label.textContent = permissionGroupSummaryLabel(Math.max(count, 1), toolNames);
  const chevron = document.createElement("span");
  chevron.className = "chat-tool-group-chevron";
  chevron.setAttribute("aria-hidden", "true");
  summary.append(icon, label, chevron);

  const stack = document.createElement("div");
  stack.className = "chat-permission-stack";
  const anchor = nodes[0];
  const parent = anchor.parentElement;
  if (parent) parent.insertBefore(group, anchor);
  for (const node of nodes) {
    flattenTurnToolNode(node, stack);
  }
  group.append(summary, stack);
  return group;
}

/** Fold consecutive tool chips + Bash permission cards into one Cursor-style row. */
function collapseResolvedPermissionsBeforeAssistant(anchor: HTMLElement): void {
  finishToolGroup(true);
  const block = assistantMsgWrap(anchor);
  const prev = block.previousElementSibling as HTMLElement | null;
  if (prev?.classList.contains("chat-turn-tools")) {
    const existing = prev as HTMLDetailsElement;
    existing.open = false;
    existing.classList.remove("is-live");
    return;
  }
  const nodes: HTMLElement[] = [];
  let sibling = prev;
  while (sibling && isTurnToolEphemeral(sibling)) {
    nodes.unshift(sibling);
    sibling = sibling.previousElementSibling as HTMLElement | null;
  }
  if (nodes.length === 0) return;
  if (nodes.length === 1 && nodes[0].classList.contains("chat-turn-tools")) {
    const only = nodes[0] as HTMLDetailsElement;
    only.open = false;
    only.classList.remove("is-live");
    return;
  }
  // One leftover live chip is still worth collapsing so it doesn't stay blue forever.
  wrapTurnToolsInGroup(nodes);
}

function renderPermissionGroup(messages: ChatMessage[], interactive: boolean): HTMLDetailsElement {
  const cards = messages.map((message) => renderPermissionCard(message, interactive));
  return wrapTurnToolsInGroup(cards);
}

function markPermissionResolved(requestId: string, allowed: boolean): void {
  const session = runTargetSession();
  const message = session.messages.find(
    (m) => m.role === "permission" && m.permission?.requestId === requestId,
  );
  if (message?.permission) {
    message.permission.allowed = allowed;
    touchSession(session);
    saveStore();
  }

  const wasInBatch = pendingPermissionBatch.some((item) => item.requestId === requestId);
  pendingPermissionBatch = pendingPermissionBatch.filter((item) => item.requestId !== requestId);
  const stillPending = pendingPermissionBatch.length > 0;

  // Always update the card in the open log if present (even if busy just cleared).
  const card = logEl.querySelector<HTMLElement>(
    `.chat-permission.is-pending[data-request-id="${CSS.escape(requestId)}"]:not(.is-batch)`,
  );
  if (card) {
    card.dataset.resolved = "1";
    card.classList.remove("is-pending", "is-expired");
    card.classList.add(allowed ? "is-allowed" : "is-denied");
    const actions = card.querySelector(".chat-permission-actions");
    if (actions) {
      actions.replaceChildren();
      const badge = document.createElement("span");
      badge.className = "chat-permission-result";
      badge.textContent = allowed ? t("chat.permissionAllowed") : t("chat.permissionDenied");
      actions.appendChild(badge);
    }
  }

  const batchCard = logEl.querySelector<HTMLElement>(".chat-permission.is-batch.is-pending");
  if (batchCard && wasInBatch) {
    if (stillPending) {
      // Batch shrank — repaint remaining asks without tearing down resolved cards.
      schedulePaintLivePermissionBatch();
    } else {
      if (permissionPaintTimer) {
        window.clearTimeout(permissionPaintTimer);
        permissionPaintTimer = 0;
      }
      batchCard.dataset.resolved = "1";
      batchCard.classList.remove("is-pending", "is-expired");
      batchCard.classList.add(allowed ? "is-allowed" : "is-denied");
      const actions = batchCard.querySelector(".chat-permission-actions");
      if (actions) {
        actions.replaceChildren();
        const badge = document.createElement("span");
        badge.className = "chat-permission-result";
        badge.textContent = allowed ? t("chat.permissionAllowed") : t("chat.permissionDenied");
        actions.appendChild(badge);
      }
    }
  }

  hideDecisionDock();
  if (stillPending) {
    setStatus(t("chat.needYourChoice"), "warn");
  } else {
    setStatus(allowed ? t("chat.permissionAllowed") : t("chat.permissionDenied"), "ok");
  }
  flushSessionListRender();
}

/** Apply queued assistant text immediately (before inserting later events). */
function flushPendingTextSync(): void {
  if (flushRaf) {
    window.cancelAnimationFrame(flushRaf);
    flushRaf = 0;
  }
  if (!pendingText) return;
  const chunk = pendingText;
  pendingText = "";
  appendAssistantChunk(chunk);
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
function looksLikeChoiceQuestion(text: string): boolean {
  const trimmed = text.trim();
  if (!trimmed) return false;
  const plain = trimmed.replace(/\s+/g, " ");
  const lower = plain.toLowerCase();
  // Open help offers should stay as chat, not a confirm dock.
  if (
    /有什么(可以|需要|能)?(帮|帮忙|做)/.test(plain) ||
    /需要我(做|帮忙|帮你)/.test(plain) ||
    /随时(找我|叫我|告诉我)/.test(plain) ||
    /how can i help|anything (i can|you need)|what can i (do|help)/i.test(lower)
  ) {
    return false;
  }
  const tail = plain.slice(-120);
  return (
    /(要不要|要我|是否|继续吗|可以吗|好吗|行吗|确认一下|选一个|选哪|选哪个)/.test(tail) ||
    /(shall i|should i|want me to|would you like me to|continue\?|proceed\?)/i.test(tail)
  );
}

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

/** Close the current streaming assistant bubble so later events render after it. */
function sealAssistantBubble(): void {
  if (!assistantMessageId && !assistantBubble) {
    assistantRaw = "";
    return;
  }
  if (assistantBubble?.isConnected) {
    assistantBubble.classList.remove("is-streaming");
  }
  if (!assistantRaw.trim()) {
    // Drop empty placeholder bubbles so tools aren't preceded by a blank card.
    const emptyId = assistantMessageId;
    if (assistantBubble?.isConnected) {
      assistantMsgWrap(assistantBubble).remove();
    }
    if (emptyId) {
      const session = runTargetSession();
      session.messages = session.messages.filter((m) => m.id !== emptyId);
      saveStore();
    }
  } else {
    if (assistantMessageId) {
      updateAssistantMessage(assistantMessageId, assistantRaw);
      flushStorePersist();
    }
    if (assistantBubble?.isConnected) {
      syncAssistantCopyButton(assistantBubble);
    }
  }
  assistantBubble = null;
  assistantMessageId = null;
  assistantRaw = "";
}

function ensureAssistantMessage(): string {
  if (assistantMessageId) return assistantMessageId;
  const message = persistMessage("assistant", "");
  assistantMessageId = message.id;
  assistantRaw = "";
  return assistantMessageId;
}

function ensureAssistantBubble(): HTMLElement {
  ensureAssistantMessage();
  if (assistantBubble?.isConnected) {
    assistantBubble.classList.add("is-streaming");
    return assistantBubble;
  }
  const existing = assistantMessageId
    ? logEl.querySelector<HTMLElement>(
        `.chat-bubble.assistant[data-message-id="${CSS.escape(assistantMessageId)}"]`,
      )
    : null;
  if (existing) {
    assistantBubble = existing;
    assistantBubble.classList.add("is-streaming");
    return assistantBubble;
  }
  assistantBubble = appendBubble("assistant", assistantRaw, {
    id: assistantMessageId ?? undefined,
    persist: false,
  });
  assistantBubble.classList.add("is-streaming");
  return assistantBubble;
}

function appendAssistantChunk(chunk: string): void {
  if (!chunk) return;
  if (looksLikeToolPayloadJson(chunk)) {
    if (isViewingRunningSession()) {
      const formatted = formatPermissionDetail(chunk.trim());
      pushActivity("tool", formatted.summary || cleanToolLabel(chunk));
    }
    return;
  }
  turnHadAssistantText = true;
  ensureAssistantMessage();
  assistantRaw += chunk;
  if (assistantMessageId) {
    updateAssistantMessage(assistantMessageId, assistantRaw, { persist: false });
  }
  scheduleStorePersist();
  if (!isViewingRunningSession()) return;
  if (activityEl?.dataset.kind === "tool") settleActivity();
  finishToolGroup(true);
  dismissLifecycleActivity();
  const bubble = ensureAssistantBubble();
  if (!bubble.isConnected) return;
  collapseResolvedPermissionsBeforeAssistant(bubble);
  setAssistantMarkdown(bubble, assistantRaw);
  logEl.scrollTop = logEl.scrollHeight;
}

function renderSessionList(): void {
  sessionListEl.replaceChildren();
  for (const session of store.sessions) {
    const isRunning = busy && session.id === runningChatSessionId;
    const awaitingConfirm = isRunning && pendingPermissionBatch.length > 0;
    const doneUnseen = !isRunning && unseenCompletedSessionIds.has(session.id);
    const row = document.createElement("div");
    row.className = `chat-session${session.id === store.activeId ? " is-active" : ""}${
      awaitingConfirm ? " is-awaiting-confirm" : isRunning ? " is-running" : ""
    }${doneUnseen ? " is-done-unseen" : ""}`;
    row.dataset.sessionId = session.id;

    const main = document.createElement("button");
    main.type = "button";
    main.className = "chat-session-main";

    const title = document.createElement("span");
    title.className = "chat-session-title";
    title.textContent = sessionTitle(session);

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
    sessionListEl.appendChild(row);
  }
}

function formatTime(ts: number): string {
  try {
    return new Date(ts).toLocaleString(getLocale() === "zh" ? "zh-CN" : "en", {
      month: "numeric",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return "";
  }
}

/** Flush live DOM pointers when leaving a running chat; keep run memory for background events. */
function detachLiveDom(): void {
  flushPendingTextSync();
  if (assistantMessageId && assistantRaw) {
    updateAssistantMessage(assistantMessageId, assistantRaw, { persist: false });
    scheduleStorePersist();
  }
  assistantBubble = null;
  activityEl = null;
  lifecycleActivityEl = null;
  toolGroupEl = null;
  hideDecisionDock();
}

/** Re-bind streaming bubble / pending permissions after switching back to a running chat. */
function reattachLiveUi(): void {
  if (!isViewingRunningSession()) return;
  if (assistantMessageId) {
    const bubble = logEl.querySelector<HTMLElement>(
      `.chat-bubble.assistant[data-message-id="${CSS.escape(assistantMessageId)}"]`,
    );
    if (bubble) {
      assistantBubble = bubble;
      bubble.classList.add("is-streaming");
      if (assistantRaw.trim()) {
        setAssistantMarkdown(bubble, assistantRaw);
      }
      syncAssistantCopyButton(bubble);
    } else if (assistantRaw.trim() || turnHadAssistantText) {
      assistantBubble = appendBubble("assistant", assistantRaw, {
        id: assistantMessageId,
        persist: false,
      });
      assistantBubble.classList.add("is-streaming");
    }
  }
  for (const item of pendingPermissionBatch) {
    logEl
      .querySelectorAll<HTMLElement>(
        `.chat-permission[data-request-id="${CSS.escape(item.requestId)}"]`,
      )
      .forEach((el) => el.remove());
  }
  if (pendingPermissionBatch.length > 0) {
    setStatus(t("chat.needYourChoice"), "warn");
    schedulePaintLivePermissionBatch();
  } else {
    setStatus(
      t("chat.running", { runtime: runtimeDisplayName(activeSession().runtime) }),
      "muted",
    );
    pushActivity("think", t("chat.typing"));
  }
  logEl.scrollTop = logEl.scrollHeight;
}

function switchSession(id: string): void {
  if (id === store.activeId) return;
  const session = store.sessions.find((s) => s.id === id);
  if (!session) return;

  const leavingRunning = Boolean(busy && store.activeId === runningChatSessionId);
  const enteringRunning = Boolean(busy && id === runningChatSessionId);

  if (leavingRunning) {
    detachLiveDom();
  } else if (!busy) {
    assistantBubble = null;
    assistantMessageId = null;
    assistantRaw = "";
    pendingText = "";
    turnHadAssistantText = false;
    activityEl = null;
    lifecycleActivityEl = null;
    toolGroupEl = null;
  } else {
    // Leaving a non-running chat while another run continues — clear local view only.
    assistantBubble = null;
    activityEl = null;
    lifecycleActivityEl = null;
    toolGroupEl = null;
  }

  store.activeId = id;
  saveStore();
  setCurrentRuntime(session.runtime);
  if (unseenCompletedSessionIds.delete(id)) {
    // Opened after finishing elsewhere — clear 【完成】 badge.
  }
  renderActiveMessages();
  if (enteringRunning) {
    reattachLiveUi();
  } else if (busy && runningChatSessionId) {
    setStatus(t("chat.otherSessionRunningHint"), "muted");
  } else {
    setStatus("");
  }
  syncComposerUi();
  renderSessionList();
  titleEl.textContent = sessionTitle(session);
  updateContextMeter();
  promptEl.focus();
}

function deleteSession(id: string): void {
  if (busy && id === runningChatSessionId) {
    setStatus(t("chat.cannotDeleteRunning"), "warn");
    return;
  }
  if (!window.confirm(t("chat.deleteSessionConfirm"))) return;
  unseenCompletedSessionIds.delete(id);
  const remaining = store.sessions.filter((s) => s.id !== id);
  if (remaining.length === 0) {
    const session = createEmptySession(currentRuntime);
    store = { activeId: session.id, sessions: [session] };
  } else {
    store.sessions = remaining;
    if (store.activeId === id) {
      store.activeId = remaining[0].id;
    }
  }
  saveStore();
  const active = activeSession();
  setCurrentRuntime(active.runtime);
  if (!busy) {
    assistantBubble = null;
    assistantMessageId = null;
    assistantRaw = "";
    pendingText = "";
    turnHadAssistantText = false;
  } else {
    assistantBubble = null;
  }
  pendingAttachments = [];
  activityEl = null;
  lifecycleActivityEl = null;
  toolGroupEl = null;
  renderPendingAttachments();
  renderActiveMessages();
  if (isViewingRunningSession()) {
    reattachLiveUi();
  }
  syncComposerUi();
  renderSessionList();
  titleEl.textContent = sessionTitle(active);
  setStatus("");
  promptEl.focus();
}

function ensureRuntimeSession(runtime: AskRuntime): void {
  setCurrentRuntime(runtime);
  const active = activeSession();
  if (active.runtime === runtime) {
    renderSessionList();
    return;
  }
  // Prefer the most recently updated session for this agent.
  const existing = store.sessions
    .filter((s) => s.runtime === runtime)
    .sort((a, b) => b.updatedAt - a.updatedAt)[0];
  if (existing) {
    switchSession(existing.id);
    return;
  }
  startNewSession();
}

function startNewSession(): void {
  if (busy && store.activeId === runningChatSessionId) {
    detachLiveDom();
  } else if (!busy) {
    assistantBubble = null;
    assistantMessageId = null;
    assistantRaw = "";
    pendingText = "";
    turnHadAssistantText = false;
  } else {
    assistantBubble = null;
  }
  const session = createEmptySession(selectedRuntime());
  store.sessions.unshift(session);
  store.activeId = session.id;
  store.sessions = store.sessions.slice(0, MAX_SESSIONS);
  saveStore();
  pendingAttachments = [];
  activityEl = null;
  lifecycleActivityEl = null;
  toolGroupEl = null;
  renderPendingAttachments();
  renderActiveMessages();
  syncComposerUi();
  renderSessionList();
  titleEl.textContent = sessionTitle(session);
  if (busy && runningChatSessionId) {
    setStatus(t("chat.otherSessionRunningHint"), "muted");
  } else {
    setStatus(t("chat.newSessionReady"), "ok");
  }
  updateContextMeter();
  promptEl.focus();
}

function clearActiveSession(): void {
  if (isViewingRunningSession()) {
    setStatus(t("chat.cannotClearRunning"), "warn");
    return;
  }
  if (busy && store.activeId === runningChatSessionId) return;
  const session = activeSession();
  session.messages = [];
  session.title = "";
  session.runtimeThreadId = null;
  session.updatedAt = Date.now();
  saveStore();
  if (!busy) {
    assistantBubble = null;
    assistantMessageId = null;
    assistantRaw = "";
    turnHadAssistantText = false;
  } else {
    assistantBubble = null;
  }
  activityEl = null;
  pendingAttachments = [];
  renderPendingAttachments();
  renderActiveMessages();
  renderSessionList();
  titleEl.textContent = sessionTitle(session);
  setStatus("");
}

function fileNameFromPath(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}

function isImagePath(path: string): boolean {
  const ext = fileNameFromPath(path).split(".").pop()?.toLowerCase() ?? "";
  return IMAGE_EXTS.includes(ext);
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
    setStatus(t("chat.attachFailed", { error: String(error) }), "error");
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

function attachmentSummary(attachments: ChatAttachment[] | undefined): string {
  if (!attachments?.length) return "";
  return attachments.map((a) => `- ${a.path} (${a.kind})`).join("\n");
}

function renderAttachmentStrip(attachments: ChatAttachment[] | undefined): HTMLElement | null {
  if (!attachments?.length) return null;
  const wrap = document.createElement("div");
  wrap.className = "chat-bubble-attachments";
  for (const item of attachments) {
    const chip = document.createElement("div");
    chip.className = "chat-bubble-attach";
    chip.title = item.path;
    if (item.kind === "image") {
      const img = document.createElement("img");
      img.alt = item.name;
      try {
        img.src = convertFileSrc(item.path);
        chip.appendChild(img);
      } catch {
        /* ignore preview */
      }
    }
    const span = document.createElement("span");
    span.textContent = item.name;
    chip.appendChild(span);
    wrap.appendChild(chip);
  }
  return wrap;
}

function persistMessage(
  role: ChatRole,
  content: string,
  opts?: { id?: string; attachments?: ChatAttachment[]; permission?: PermissionMeta },
): ChatMessage {
  const session = busy ? runTargetSession() : activeSession();
  const message: ChatMessage = {
    id: opts?.id ?? uid(),
    role,
    content,
    at: Date.now(),
    attachments: opts?.attachments?.length ? opts.attachments : undefined,
    permission: opts?.permission,
  };
  session.messages.push(message);
  if (role === "user" && !session.title.trim()) {
    const seed = content.trim() || opts?.attachments?.[0]?.name || "";
    session.title = seed.split(/\n/)[0].slice(0, 48);
  }
  if (!busy || session.id === store.activeId) {
    session.runtime = selectedRuntime();
  }
  touchSession(session);
  if (busy) {
    scheduleStorePersist();
    scheduleSessionListRender();
  } else {
    saveStore();
    renderSessionList();
  }
  if (session.id === store.activeId) {
    titleEl.textContent = sessionTitle(session);
    updateContextMeter();
  }
  return message;
}

function updateAssistantMessage(id: string, content: string, opts?: { persist?: boolean }): void {
  const session = busy ? runTargetSession() : activeSession();
  const message = session.messages.find((m) => m.id === id);
  if (!message) return;
  message.content = content;
  if (opts?.persist === false) return;
  message.at = Date.now();
  touchSession(session);
  if (busy) {
    scheduleStorePersist();
    scheduleSessionListRender();
  } else {
    saveStore();
  }
}

function msgWrap(el: HTMLElement, role: "assistant" | "user"): HTMLElement {
  return el.closest(`.chat-msg-${role}`) ?? el;
}

function assistantMsgWrap(bubble: HTMLElement): HTMLElement {
  return msgWrap(bubble, "assistant");
}

function bubblePlainText(bubble: HTMLElement): string {
  return (bubble.innerText ?? "").trim();
}

function assistantMarkdownSource(bubble: HTMLElement): string {
  const id = bubble.dataset.messageId;
  if (id) {
    const session = busy ? runTargetSession() : activeSession();
    const message = session.messages.find((m) => m.id === id);
    if (message?.content?.trim()) return message.content;
  }
  if (bubble === assistantBubble && assistantRaw.trim()) return assistantRaw;
  return bubblePlainText(bubble);
}

const SHORT_MSG_COPY_CHARS = 140;
const SHORT_MSG_COPY_LINES = 2;

function isShortCopyLayout(bubble: HTMLElement, source: string): boolean {
  if (bubble.querySelector("pre, .chat-code-block, table, .chat-bubble-attachments")) return false;
  const text = source.trim();
  if (!text) return false;
  const lines = text.split(/\n/).filter((line) => line.trim().length > 0);
  return text.length <= SHORT_MSG_COPY_CHARS && lines.length <= SHORT_MSG_COPY_LINES;
}

function copyIconSvg(kind: "copy" | "check"): string {
  if (kind === "check") {
    return `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M20 6L9 17l-5-5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
  }
  return `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true"><rect x="9" y="9" width="13" height="13" rx="2" stroke="currentColor" stroke-width="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
}

type CopyIdleKind = "text" | "code";

function copyIdleLabel(kind: CopyIdleKind): string {
  return kind === "code" ? t("chat.copyCode") : t("chat.copy");
}

function setCopyButtonState(
  btn: HTMLButtonElement,
  state: "idle" | "copied" | "failed",
  idleKind: CopyIdleKind = "text",
): void {
  if (state === "copied") {
    btn.innerHTML = copyIconSvg("check");
    btn.classList.add("is-copied");
    btn.title = t("chat.copied");
    btn.setAttribute("aria-label", t("chat.copied"));
    return;
  }
  btn.innerHTML = copyIconSvg("copy");
  btn.classList.remove("is-copied");
  const label = state === "failed" ? t("chat.copyFailed") : copyIdleLabel(idleKind);
  btn.title = label;
  btn.setAttribute("aria-label", label);
}

async function copyTextToClipboard(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.style.position = "fixed";
  area.style.left = "-9999px";
  document.body.appendChild(area);
  area.select();
  document.execCommand("copy");
  area.remove();
}

async function runCopyButton(
  btn: HTMLButtonElement,
  getText: () => string,
  idleKind: CopyIdleKind,
): Promise<void> {
  // Code blocks keep their leading indentation; only trailing newlines go.
  const raw = getText();
  const text = idleKind === "code" ? raw.replace(/\n+$/, "") : raw.trim();
  if (!text) return;
  try {
    await copyTextToClipboard(text);
    setCopyButtonState(btn, "copied", idleKind);
    window.setTimeout(() => {
      if (!btn.isConnected) return;
      setCopyButtonState(btn, "idle", idleKind);
    }, 1600);
  } catch {
    setCopyButtonState(btn, "failed", idleKind);
    window.setTimeout(() => {
      if (!btn.isConnected) return;
      setCopyButtonState(btn, "idle", idleKind);
    }, 1600);
  }
}

function createCopyActionButton(
  idleKind: CopyIdleKind,
  getText: () => string,
): HTMLButtonElement {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "chat-msg-copy";
  btn.dataset.copyKind = idleKind;
  btn.hidden = true;
  setCopyButtonState(btn, "idle", idleKind);
  btn.addEventListener("click", () => {
    void runCopyButton(btn, getText, idleKind);
  });
  return btn;
}

function syncMessageCopyActions(
  wrap: HTMLElement,
  bubble: HTMLElement,
  streaming: boolean,
  layoutSource: string,
): void {
  const hasText = layoutSource.trim().length > 0;
  const show = hasText && !streaming;
  const actions = wrap.querySelector<HTMLElement>(":scope > .chat-msg-actions");
  const btn = actions?.querySelector<HTMLButtonElement>(".chat-msg-copy");
  if (btn) {
    const kind: CopyIdleKind = btn.dataset.copyKind === "code" ? "code" : "text";
    btn.hidden = !show;
    if (show) setCopyButtonState(btn, "idle", kind);
  }
  if (actions) actions.hidden = !show;
  wrap.classList.toggle("is-compact", show && isShortCopyLayout(bubble, layoutSource));
}

function syncAssistantCopyButton(bubble: HTMLElement): void {
  const source = assistantMarkdownSource(bubble);
  syncMessageCopyActions(
    assistantMsgWrap(bubble),
    bubble,
    bubble.classList.contains("is-streaming"),
    source,
  );
}

function enhanceCodeBlocks(root: HTMLElement): void {
  for (const pre of Array.from(root.querySelectorAll("pre"))) {
    if (pre.parentElement?.classList.contains("chat-code-block")) continue;
    const wrap = document.createElement("div");
    wrap.className = "chat-code-block";
    pre.replaceWith(wrap);

    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "chat-code-copy";
    setCopyButtonState(btn, "idle", "code");
    btn.addEventListener("click", (event) => {
      event.stopPropagation();
      const code = pre.querySelector("code");
      const text = code?.textContent ?? pre.textContent ?? "";
      void runCopyButton(btn, () => text, "code");
    });

    wrap.append(btn, pre);
  }
}

function createAssistantBubbleEl(opts?: { id?: string }): { wrap: HTMLElement; bubble: HTMLElement } {
  const wrap = document.createElement("div");
  wrap.className = "chat-msg chat-msg-assistant";

  const bubble = document.createElement("div");
  bubble.className = "chat-bubble assistant chat-md";
  if (opts?.id) bubble.dataset.messageId = opts.id;

  const actions = document.createElement("div");
  actions.className = "chat-msg-actions";
  actions.hidden = true;
  // Default copy = markdown source (better for paste/edit).
  actions.appendChild(createCopyActionButton("text", () => assistantMarkdownSource(bubble)));
  wrap.append(bubble, actions);
  return { wrap, bubble };
}

function createUserBubbleEl(opts?: {
  id?: string;
  text?: string;
  attachments?: ChatAttachment[];
}): { wrap: HTMLElement; bubble: HTMLElement } {
  const wrap = document.createElement("div");
  wrap.className = "chat-msg chat-msg-user";

  const bubble = document.createElement("div");
  bubble.className = "chat-bubble user";
  if (opts?.id) bubble.dataset.messageId = opts.id;
  bubble.textContent = opts?.text ?? "";
  const strip = renderAttachmentStrip(opts?.attachments);
  if (strip) bubble.appendChild(strip);

  const actions = document.createElement("div");
  actions.className = "chat-msg-actions";
  const getText = () => {
    const id = bubble.dataset.messageId;
    if (id) {
      const message = activeSession().messages.find((m) => m.id === id);
      if (message) return message.content;
    }
    return opts?.text ?? bubblePlainText(bubble);
  };
  actions.appendChild(createCopyActionButton("text", getText));
  wrap.append(bubble, actions);
  syncMessageCopyActions(wrap, bubble, false, getText());
  return { wrap, bubble };
}

function setAssistantMarkdown(bubble: HTMLElement, markdown: string): void {
  bubble.innerHTML = markdown.trim() ? renderMarkdown(markdown) : "";
  if (markdown.trim()) enhanceCodeBlocks(bubble);
  syncAssistantCopyButton(bubble);
}

function appendBubble(
  kind: ChatRole,
  text: string,
  opts?: { id?: string; persist?: boolean; attachments?: ChatAttachment[] },
): HTMLElement {
  if (kind === "assistant") {
    const { wrap, bubble } = createAssistantBubbleEl({ id: opts?.id });
    setAssistantMarkdown(bubble, text);
    logEl.appendChild(wrap);
    logEl.scrollTop = logEl.scrollHeight;
    return bubble;
  }
  if (kind === "user") {
    const { wrap, bubble } = createUserBubbleEl({
      id: opts?.id,
      text,
      attachments: opts?.attachments,
    });
    logEl.appendChild(wrap);
    logEl.scrollTop = logEl.scrollHeight;
    return bubble;
  }
  const bubble = document.createElement("div");
  bubble.className = `chat-bubble ${kind}`;
  if (opts?.id) bubble.dataset.messageId = opts.id;
  bubble.textContent = text;
  const strip = renderAttachmentStrip(opts?.attachments);
  if (strip) bubble.appendChild(strip);
  logEl.appendChild(bubble);
  logEl.scrollTop = logEl.scrollHeight;
  return bubble;
}

function renderActiveMessages(): void {
  try {
    logEl.replaceChildren();
    assistantBubble = null;
    activityEl = null;
    toolGroupEl = null;
    lifecycleActivityEl = null;
    // Keep in-flight run memory so background events and switch-back still work.
    if (!busy) {
      assistantMessageId = null;
      assistantRaw = "";
      turnHadAssistantText = false;
    }
    const session = activeSession();
    if (session.messages.length === 0) {
      appendBubble("meta", t("chat.welcome"), { persist: false });
      return;
    }
    for (let i = 0; i < session.messages.length; ) {
      const message = session.messages[i];
      if (message.role === "permission") {
        const pendingLive =
          isViewingRunningSession() &&
          message.permission?.allowed == null &&
          pendingPermissionBatch.some((p) => p.requestId === message.permission?.requestId);
        if (pendingLive) {
          i += 1;
          continue;
        }
        // Unanswered asks stay as standalone cards — never fold into "$ N tools".
        if (message.permission?.allowed == null) {
          logEl.appendChild(renderPermissionCard(message, false));
          i += 1;
          continue;
        }
        const run: ChatMessage[] = [message];
        while (
          i + run.length < session.messages.length &&
          session.messages[i + run.length].role === "permission"
        ) {
          const next = session.messages[i + run.length];
          if (next.permission?.allowed == null) break;
          const nextPending =
            isViewingRunningSession() &&
            pendingPermissionBatch.some((p) => p.requestId === next.permission?.requestId);
          if (nextPending) break;
          run.push(next);
        }
        if (run.length >= 2) {
          try {
            logEl.appendChild(renderPermissionGroup(run, false));
          } catch {
            for (const item of run) {
              logEl.appendChild(renderPermissionCard(item, false));
            }
          }
        } else {
          logEl.appendChild(renderPermissionCard(message, false));
        }
        i += run.length;
        continue;
      }
      if (message.role === "assistant") {
        const { wrap, bubble } = createAssistantBubbleEl({ id: message.id });
        const liveContent =
          busy && message.id === assistantMessageId && assistantRaw
            ? assistantRaw
            : message.content;
        setAssistantMarkdown(bubble, liveContent);
        if (busy && message.id === assistantMessageId) {
          bubble.classList.add("is-streaming");
          assistantBubble = bubble;
        }
        logEl.appendChild(wrap);
      } else if (message.role === "user") {
        const { wrap } = createUserBubbleEl({
          id: message.id,
          text: message.content,
          attachments: message.attachments,
        });
        logEl.appendChild(wrap);
      } else {
        const bubble = document.createElement("div");
        bubble.className = `chat-bubble ${message.role}`;
        bubble.dataset.messageId = message.id;
        bubble.textContent = message.content;
        const strip = renderAttachmentStrip(message.attachments);
        if (strip) bubble.appendChild(strip);
        logEl.appendChild(bubble);
      }
      i += 1;
    }
    logEl.scrollTop = logEl.scrollHeight;
    updateContextMeter();
  } catch (error) {
    console.error("Ask: failed to render messages", error);
    logEl.replaceChildren();
    appendBubble("meta", t("chat.welcome"), { persist: false });
  }
}

function queueAssistantText(text: string): void {
  if (!text) return;
  pendingText += text;
  if (flushRaf) return;
  flushRaf = window.requestAnimationFrame(() => {
    flushRaf = 0;
    const chunk = pendingText;
    pendingText = "";
    if (!chunk) return;
    appendAssistantChunk(chunk);
  });
}

function preferPlainSummary(summary: string): string {
  const trimmed = summary.trim();
  if (!trimmed) return "";
  if (trimmed.startsWith("{")) {
    try {
      const parsed = JSON.parse(trimmed) as { result?: unknown };
      if (typeof parsed.result === "string" && parsed.result.trim()) {
        return parsed.result.trim();
      }
    } catch {
      /* not a single JSON object — try JSONL last result */
      for (const line of trimmed.split("\n").reverse()) {
        try {
          const parsed = JSON.parse(line) as { type?: string; result?: unknown; is_error?: boolean };
          if (
            parsed.type === "result" &&
            !parsed.is_error &&
            typeof parsed.result === "string" &&
            parsed.result.trim()
          ) {
            return parsed.result.trim();
          }
        } catch {
          /* continue */
        }
      }
    }
    return "";
  }
  if (/^claude-code (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  if (/^codex (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  if (/^hermes (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  if (/^openclaw (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  if (/^deepseek-harness (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  return trimmed;
}

function shortCwdLabel(cwd: string): string {
  const trimmed = cwd.trim();
  if (!trimmed || trimmed === "—") return "—";
  const parts = trimmed.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] || trimmed;
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

function buildPromptWithHistory(
  userText: string,
  attachments: ChatAttachment[],
  sessionId?: string,
): string {
  const session = sessionById(sessionId) ?? (busy ? runTargetSession() : activeSession());
  const responseStyle =
    "Response style: answer the user directly and concisely. Lead with the result. " +
    "Use short sections or bullets only when they improve clarity. Do not narrate hidden reasoning, " +
    "routine progress, tool-selection decisions, retries, or permission flow. Do not repeat the request.";
  // Native resume already carries thread history — only send this turn.
  if (session.runtimeThreadId?.trim()) {
    const parts: string[] = [responseStyle];
    if (attachments.length > 0) {
      parts.push(
        `Attached local files for this turn (read them with your tools if needed):\n${attachmentSummary(attachments)}`,
      );
    }
    parts.push(userText);
    return parts.join("\n\n");
  }

  const prior = session.messages
    .filter((m) => m.role === "user" || m.role === "assistant")
    .filter((m) => m.content.trim() || m.attachments?.length)
    .slice(0, -1)
    .slice(-MAX_CONTEXT_MESSAGES);

  const parts: string[] = [responseStyle];
  if (prior.length > 0) {
    const transcript = prior
      .map((m) => {
        const body = m.content.trim() || "(attachments only)";
        const files = attachmentSummary(m.attachments);
        return files
          ? `${m.role === "user" ? "User" : "Assistant"}: ${body}\nAttachments:\n${files}`
          : `${m.role === "user" ? "User" : "Assistant"}: ${body}`;
      })
      .join("\n\n");
    parts.push(`Conversation so far:\n\n${transcript}`);
  }

  if (attachments.length > 0) {
    parts.push(
      `Attached local files for this turn (read them with your tools if needed):\n${attachmentSummary(attachments)}`,
    );
  }

  parts.push(`User: ${userText}\n\nAssistant:`);
  return parts.join("\n\n");
}

/** Rough token estimate — CJK denser than ASCII. */
function estimateTokens(text: string): number {
  let cjk = 0;
  let other = 0;
  for (const ch of text) {
    if ((ch.codePointAt(0) ?? 0) > 0xff) cjk += 1;
    else other += 1;
  }
  return Math.max(1, Math.ceil(cjk / 1.5 + other / 4));
}

function contextLimitForModel(model: string | null | undefined): number {
  const m = (model ?? "").toLowerCase();
  if (!m) return 128_000;
  if (m.includes("haiku")) return 200_000;
  if (
    m.includes("gpt-5.6") ||
    m.includes("gpt-5.4") ||
    m.includes("gpt-4.1") ||
    m.includes("o3") ||
    m.includes("o4-mini")
  ) {
    return 128_000;
  }
  if (
    m.includes("deepseek") ||
    m.includes("qwen") ||
    m.includes("claude") ||
    m.includes("gemini") ||
    m.includes("kimi") ||
    m.includes("moonshot") ||
    m.includes("glm") ||
    m.includes("minimax")
  ) {
    return 1_000_000;
  }
  return 128_000;
}

function sessionContextText(session: ChatSession, draft = ""): string {
  const chunks: string[] = [];
  for (const message of session.messages) {
    if (message.role !== "user" && message.role !== "assistant" && message.role !== "meta") {
      continue;
    }
    if (message.content.trim()) chunks.push(message.content);
    if (message.attachments?.length) {
      chunks.push(message.attachments.map((a) => a.name).join("\n"));
    }
  }
  const trimmedDraft = draft.trim();
  if (trimmedDraft) chunks.push(trimmedDraft);
  return chunks.join("\n");
}

function contextUsagePercent(session: ChatSession, draft = ""): number {
  const used = estimateTokens(sessionContextText(session, draft));
  const limit = contextLimitForModel(wiredProvider?.model);
  return Math.min(100, Math.round((used / Math.max(limit, 1)) * 100));
}

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

function compactActiveSession(): void {
  if (isComposerLocked()) return;
  const session = activeSession();
  const turns = session.messages.filter((m) => m.role === "user" || m.role === "assistant");
  if (turns.length <= COMPACT_KEEP_TURNS) {
    setStatus(t("chat.contextCompactNeedMore"), "muted");
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
  touchSession(session);
  saveStore();
  closeContextPopover();
  renderActiveMessages();
  renderSessionList();
  updateContextMeter();
  setStatus(t("chat.contextCompactDone"), "ok");
}

async function ensureListener(): Promise<void> {
  if (unlisten) return;
  unlisten = await listen<PromptSessionEvent>("prompt-session-event", (event) => {
    const payload = event.payload;
    const eventSessionId = "session_id" in payload ? payload.session_id : undefined;
    if (payload.type !== "started" && !isEventForCurrentRun(eventSessionId)) {
      return;
    }
    switch (payload.type) {
      case "started":
        runningBackendSessionId = payload.session_id;
        setDisplayedCwd(payload.cwd);
        assistantBubble = null;
        assistantMessageId = null;
        assistantRaw = "";
        pendingText = "";
        turnHadAssistantText = false;
        pushActivity("think", t("chat.waitingModel"));
        flushSessionListRender();
        break;
      case "status":
        pushActivity(payload.phase, payload.message);
        noteVerifyBrowserSignal(payload.message, "status");
        break;
      case "delta":
        queueAssistantText(payload.text);
        noteVerifyBrowserSignal(payload.text, "assistant");
        break;
      case "stdout_line":
        queueAssistantText(`${payload.line}\n`);
        noteVerifyBrowserSignal(payload.line, "assistant");
        break;
      case "stderr_line":
        appendStderrLine(payload.line);
        break;
      case "permission_request":
        pushPermissionCard(payload);
        noteVerifyBrowserSignal(payload.tool_name, "tool");
        break;
      case "permission_resolved":
        markPermissionResolved(payload.request_id, payload.allowed);
        break;
      case "completed": {
        const completedSessionId = runningChatSessionId;
        const viewing = isViewingRunningSession() || store.activeId === completedSessionId;
        flushPendingTextSync();
        // Fallback only when this turn never streamed assistant text.
        if (!turnHadAssistantText && !assistantRaw.trim() && payload.summary?.trim()) {
          const fallback = preferPlainSummary(payload.summary);
          if (fallback) appendAssistantChunk(fallback);
        }
        noteVerifyBrowserSignal(assistantRaw, "assistant");
        const hadAssistantText = turnHadAssistantText;
        const finalAssistantText = assistantRaw;
        if (viewing) {
          clearEphemeralActivity();
        }
        sealAssistantBubble();
        expireLivePermissionCards();
        if (viewing) {
          hideDecisionDock();
          if (!hadAssistantText) {
            appendBubble("meta", t("chat.emptyReply"), { persist: false });
          }
        }
        reportVerifyMcpIfNeeded();
        applyVerifyMcpFooter();
        flushStorePersist();
        setBusy(false);
        settleRunRouting();
        if (!viewing && completedSessionId) {
          unseenCompletedSessionIds.add(completedSessionId);
        }
        if (payload.status === "cancelled") {
          setStatus(t("chat.forceStopped"), "warn");
        } else if (payload.status !== "succeeded") {
          if (viewing) {
            appendBubble(
              "meta",
              t("chat.completed", {
                status: payload.status,
                code: payload.exit_code == null ? "—" : String(payload.exit_code),
              }),
              { persist: false },
            );
          }
          setStatus(
            t("chat.completed", {
              status: payload.status,
              code: payload.exit_code == null ? "—" : String(payload.exit_code),
            }),
            "error",
          );
        } else if (finalAssistantText.trim() && viewing) {
          showQuickReplies(finalAssistantText);
        } else if (!viewing) {
          setStatus(t("chat.doneElsewhere"), "ok");
        }
        renderSessionList();
        break;
      }
    }
  });
}

function runtimeFromLocation(): string | null {
  const injected = (window as Window & { __AD_ASK_RUNTIME__?: unknown }).__AD_ASK_RUNTIME__;
  if (typeof injected === "string" && injected.trim()) {
    return injected.trim();
  }
  const query = new URLSearchParams(window.location.search).get("runtime");
  if (query) {
    return query;
  }
  const hash = window.location.hash.replace(/^#/, "");
  if (!hash) {
    return null;
  }
  if (hash.startsWith("runtime=")) {
    return decodeURIComponent(hash.slice("runtime=".length).split("&")[0] ?? "");
  }
  return new URLSearchParams(hash).get("runtime");
}

function readInitialRuntime(): void {
  const runtime = runtimeFromLocation();
  if (isAskRuntime(runtime)) {
    ensureRuntimeSession(runtime);
  } else {
    setCurrentRuntime(activeSession().runtime);
  }
}

const ASK_VERIFY_DRAFT_KEY = "agent-doctor.ask.verifyDraft";

/** When true, this Ask turn is a browser MCP pathway verify. */
let verifyMcpTurn = false;
let verifySawBrowserNavigate = false;
let verifyMcpReported = false;
let verifyTurnText = "";

function looksLikeBrowserToolCall(message: string): boolean {
  const lower = message.toLowerCase();
  return (
    lower.includes("browser_navigate") ||
    lower.includes("browser_snapshot") ||
    lower.includes("browser_get_text") ||
    lower.includes("browser__browser_") ||
    /mcp__browser__/.test(lower) ||
    /browser\.(navigate|snapshot|click)/.test(lower)
  );
}

/** OpenClaw often replies with the page title and never streams the tool name. */
function looksLikeBrowserMcpVerifyEvidence(message: string): boolean {
  const lower = message.toLowerCase();
  if (
    lower.includes("browser mcp ready") ||
    lower.includes("browser mcp skipped") ||
    lower.includes("browser mcp wire") ||
    lower.includes("watching for browser")
  ) {
    return false;
  }
  const hasUrl = lower.includes("example.com");
  const hasTitle = lower.includes("example domain");
  const claimedUse =
    /已用\s*browser\s*mcp/.test(message) ||
    /\bused\s+browser\s+mcp\b/.test(lower) ||
    /with\s+browser\s+mcp/.test(lower);
  return (hasUrl && hasTitle) || (claimedUse && (hasUrl || hasTitle));
}

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

function withTimeoutChat<T>(promise: Promise<T>, ms: number, timeoutError: Error): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => reject(timeoutError), ms);
    promise.then(
      (value) => {
        window.clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        window.clearTimeout(timer);
        reject(error);
      },
    );
  });
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
    setStatus(t("chat.terminalFailed", { error: String(error) }), "error");
  }
}

async function cancelAsk(): Promise<void> {
  const gen = busyGen;
  clearQuickReplies();
  try {
    const stopped = await invoke<boolean>("cancel_prompt_session_command");
    setStatus(t("chat.cancelling"), "warn");
    pushActivity("think", t("chat.cancelling"));
    if (!stopped && busy && busyGen === gen) {
      setBusy(false);
      expireLivePermissionCards();
      settleRunRouting();
      setStatus(t("chat.forceStopped"), "warn");
      return;
    }
    window.setTimeout(() => {
      if (busy && busyGen === gen) {
        setBusy(false);
        expireLivePermissionCards();
        settleRunRouting();
        setStatus(t("chat.forceStopped"), "warn");
      }
    }, 2500);
  } catch (error) {
    setBusy(false);
    expireLivePermissionCards();
    settleRunRouting();
    setStatus(t("chat.cancelFailed", { error: String(error) }), "error");
  }
}

async function sendAsk(opts?: { verifyMcp?: boolean }): Promise<void> {
  if (busy) {
    if (runningChatSessionId && runningChatSessionId !== store.activeId) {
      setStatus(t("chat.otherSessionRunning"), "warn");
    }
    return;
  }
  const text = promptEl.value.trim();
  const attachments = [...pendingAttachments];
  if (!text && attachments.length === 0 && askResources.selectedMentions.length === 0) {
    setStatus(t("chat.emptyPrompt"), "warn");
    return;
  }

  const runtime = selectedRuntime();
  const elevated = selectedRuntime() !== "deepseek-harness" && elevatedEl.checked;
  if (elevated && !window.confirm(t("chat.elevatedConfirm"))) return;

  const chatSessionId = store.activeId;
  const resumeThreadId = activeSession().runtimeThreadId?.trim() || null;

  verifyMcpTurn = Boolean(opts?.verifyMcp);
  verifySawBrowserNavigate = false;
  verifyMcpReported = false;
  verifyTurnText = "";

  const mentions = ensureBrowserMention(
    mergeMentionsForSend(
      text,
      askResources.selectedMentions,
      askResources.mountedSkills,
      askResources.enabledMcps,
    ),
    text,
    askResources.enabledMcps,
  );
  const cleaned = stripMentionTokens(text);
  const userText = cleaned || text || t("chat.attachOnlyPrompt");
  const constraint = buildMentionConstraint(mentions);
  const promptUserText = constraint ? `${constraint}\n\n${userText}` : userText;
  const selectedMcps = mentions.filter((m) => m.kind === "mcp").map((m) => m.id);

  await ensureListener();
  clearQuickReplies();
  assistantBubble = null;
  assistantMessageId = null;
  assistantRaw = "";
  pendingText = "";
  turnHadAssistantText = false;
  setBusy(true, chatSessionId);
  if (verifyMcpTurn) {
    pushActivity("info", t("chat.verifyMcpWatching"));
  }
  const userMessage = persistMessage("user", userText, { attachments });
  if (store.activeId === chatSessionId) {
    appendBubble("user", userText, { id: userMessage.id, persist: false, attachments });
  }
  promptEl.value = "";
  mentionMenu.hideMentionMenu();
  askResources.clearMentions();
  autoResizePrompt();
  pendingAttachments = [];
  renderPendingAttachments();
  setStatus(t("chat.running", { runtime }), "muted");
  pushActivity("think", t("chat.waitingModel"));

  const prompt = buildPromptWithHistory(promptUserText, attachments, chatSessionId);

  try {
    const report = await invoke<PromptSessionReport>("start_prompt_session_command", {
      runtime,
      prompt,
      cwd: workspaceCwd?.trim() || null,
      timeoutSec: 600,
      dangerouslySkipPermissions:
        (runtime === "claude-code" || runtime === "hermes") && elevated,
      fullAuto: (runtime === "codex" || runtime === "openclaw") && elevated,
      resumeThreadId,
      selectedMcps,
    });
    setDisplayedCwd(report.cwd);
    if (report.runtime_thread_id?.trim()) {
      const session = sessionById(chatSessionId) ?? runTargetSession();
      session.runtimeThreadId = report.runtime_thread_id.trim();
      touchSession(session);
      saveStore();
    }
    const tone =
      report.status === "succeeded" ? "ok" : report.status === "cancelled" ? "warn" : "error";
    if (verifyMcpTurn) {
      applyVerifyMcpFooter();
    } else {
      setStatus(t("chat.done", { status: report.status, ms: String(report.duration_ms) }), tone);
    }
  } catch (error) {
    const message = String(error);
    if (/already running/i.test(message)) {
      try {
        await invoke<boolean>("cancel_prompt_session_command");
      } catch {
        /* ignore */
      }
      setStatus(t("chat.forceStopped"), "warn");
      if (store.activeId === chatSessionId) {
        appendBubble("meta", t("chat.forceStopped"), { persist: false });
      }
    } else {
      setStatus(t("chat.failed", { error: message }), "error");
      if (store.activeId === chatSessionId) {
        appendBubble("meta", message, { persist: false });
      }
    }
  } finally {
    applyVerifyEvidenceFromAssistant();
    if (verifySawBrowserNavigate) {
      reportVerifyMcpIfNeeded();
      applyVerifyMcpFooter();
    }
    // Yield so any trailing `completed` / delta events from this invoke are
    // handled before we tear down busy UI (avoids dropping the rest of the turn).
    await Promise.resolve();
    setBusy(false);
    expireLivePermissionCards();
    settleRunRouting();
    renderSessionList();
    const wasVerify = verifyMcpTurn;
    if (wasVerify && !verifyMcpReported) {
      window.setTimeout(() => {
        applyVerifyEvidenceFromAssistant();
        reportVerifyMcpIfNeeded();
        applyVerifyMcpFooter();
        verifyMcpTurn = false;
      }, 80);
    } else {
      verifyMcpTurn = false;
    }
  }
}

function hasRestorableChatBackup(): boolean {
  return Boolean(localStorage.getItem(STORAGE_BACKUP_KEY)?.trim());
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

function boot(): void {
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
