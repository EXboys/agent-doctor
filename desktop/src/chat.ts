import { getLocale, t, type MessageKey } from "./i18n";
import { renderMarkdown } from "./markdown";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

type AskRuntime = "claude-code" | "codex";
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
const LEGACY_STORAGE_KEY = "agent-doctor.chat.sessions.v1";
const MAX_SESSIONS = 40;
const MAX_CONTEXT_MESSAGES = 12;
const MAX_ATTACHMENTS = 8;

const IMAGE_EXTS = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "heic", "avif"];

const elevatedEl = document.querySelector<HTMLInputElement>("#chat-elevated")!;
const elevatedLabelEl = document.querySelector<HTMLElement>("#chat-elevated-label")!;
const runtimeLabelEl = document.querySelector<HTMLElement>("#chat-runtime-label")!;
const promptEl = document.querySelector<HTMLTextAreaElement>("#chat-prompt")!;
const actionEl = document.querySelector<HTMLButtonElement>("#chat-action")!;
const attachFileEl = document.querySelector<HTMLButtonElement>("#chat-attach-file")!;
const attachImageEl = document.querySelector<HTMLButtonElement>("#chat-attach-image")!;
const attachmentsEl = document.querySelector<HTMLElement>("#chat-attachments")!;
const clearEl = document.querySelector<HTMLButtonElement>("#chat-clear")!;
const newSessionEl = document.querySelector<HTMLButtonElement>("#chat-new")!;
const terminalEl = document.querySelector<HTMLButtonElement>("#chat-terminal")!;
const sessionListEl = document.querySelector<HTMLElement>("#chat-sessions")!;
const logEl = document.querySelector<HTMLElement>("#chat-log")!;
const statusEl = document.querySelector<HTMLElement>("#chat-status")!;
const cwdEl = document.querySelector<HTMLElement>("#chat-cwd")!;
const liveEl = document.querySelector<HTMLElement>("#chat-live")!;
const liveLabelEl = document.querySelector<HTMLElement>("#chat-live-label")!;
const liveElapsedEl = document.querySelector<HTMLElement>("#chat-live-elapsed")!;
const titleEl = document.querySelector<HTMLElement>("#chat-title")!;

/** Locked by main-page Ask entry (`?runtime=` / ask-window-focus). Not switched in-chat. */
let currentRuntime: AskRuntime = "claude-code";

let store: SessionStore = loadStore();
let busy = false;
let unlisten: UnlistenFn | null = null;
let assistantBubble: HTMLElement | null = null;
let assistantMessageId: string | null = null;
let assistantRaw = "";
let activityEl: HTMLElement | null = null;
let startedAt = 0;
let elapsedTimer: number | null = null;
let pendingText = "";
let flushRaf = 0;
let pendingAttachments: ChatAttachment[] = [];
/** True once any assistant text was rendered this turn (avoids result-fallback duplicates). */
let turnHadAssistantText = false;

function uid(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function selectedRuntime(): AskRuntime {
  return currentRuntime;
}

function runtimeDisplayName(runtime: AskRuntime): string {
  return runtime === "codex" ? "Codex" : "Claude Code";
}

function setCurrentRuntime(runtime: AskRuntime, opts?: { syncSession?: boolean }): void {
  currentRuntime = runtime;
  runtimeLabelEl.textContent = runtimeDisplayName(runtime);
  updateElevatedLabel();
  if (opts?.syncSession) {
    const session = activeSession();
    if (session.messages.length === 0) {
      session.runtime = runtime;
      saveStore();
    }
  }
}

function loadStore(): SessionStore {
  try {
    const raw = localStorage.getItem(STORAGE_KEY) ?? localStorage.getItem(LEGACY_STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as SessionStore;
      if (parsed?.sessions?.length && parsed.activeId) {
        parsed.sessions = parsed.sessions.map((session) => ({
          ...session,
          messages: coalesceAssistantFragments(session.messages ?? []),
        }));
        return parsed;
      }
    }
  } catch {
    /* ignore corrupt store */
  }
  const session = createEmptySession(currentRuntime);
  return { activeId: session.id, sessions: [session] };
}

/** Repair historical “one token = one message” fragmentation from early Codex streaming. */
function coalesceAssistantFragments(messages: ChatMessage[]): ChatMessage[] {
  const out: ChatMessage[] = [];
  for (const message of messages) {
    const prev = out[out.length - 1];
    const gap = prev ? message.at - prev.at : Number.POSITIVE_INFINITY;
    const canMerge =
      message.role === "assistant" &&
      prev?.role === "assistant" &&
      !message.permission &&
      !prev.permission &&
      gap >= 0 &&
      gap < 250 &&
      message.content.length <= 16;
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
  localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
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
    if (el === actionEl) return;
    const key = el.dataset.i18n as MessageKey | undefined;
    if (key) el.textContent = t(key);
  });
  promptEl.placeholder = t("chat.placeholder");
  document.documentElement.lang = getLocale() === "zh" ? "zh-CN" : "en";
  updateElevatedLabel();
  syncActionButton();
  renderSessionList();
  titleEl.textContent = sessionTitle(activeSession());
}

function updateElevatedLabel(): void {
  elevatedLabelEl.textContent =
    selectedRuntime() === "codex" ? t("chat.elevatedCodex") : t("chat.elevatedClaude");
}

function setStatus(text: string, tone: "ok" | "warn" | "error" | "muted" = "muted"): void {
  statusEl.textContent = text;
  statusEl.classList.remove("is-ok", "is-warn", "is-error");
  if (tone === "ok") statusEl.classList.add("is-ok");
  if (tone === "warn") statusEl.classList.add("is-warn");
  if (tone === "error") statusEl.classList.add("is-error");
}

function syncActionButton(): void {
  if (busy) {
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

function setBusy(next: boolean): void {
  busy = next;
  promptEl.disabled = next;
  elevatedEl.disabled = next;
  newSessionEl.disabled = next;
  attachFileEl.disabled = next;
  attachImageEl.disabled = next;
  liveEl.hidden = !next;
  sessionListEl.classList.toggle("is-busy", next);
  syncActionButton();
  if (next) {
    startedAt = performance.now();
    if (elapsedTimer != null) window.clearInterval(elapsedTimer);
    elapsedTimer = window.setInterval(() => {
      const sec = ((performance.now() - startedAt) / 1000).toFixed(1);
      liveElapsedEl.textContent = `${sec}s`;
    }, 120);
  } else {
    if (elapsedTimer != null) {
      window.clearInterval(elapsedTimer);
      elapsedTimer = null;
    }
    settleActivity();
    if (assistantBubble) assistantBubble.classList.remove("is-streaming");
  }
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
  return (
    phase === "starting" ||
    phase === "requesting" ||
    phase === "writing" ||
    phase === "streaming" ||
    phase === "info"
  );
}

function setLiveLabel(message: string): void {
  liveLabelEl.textContent = message.trim() || t("chat.live");
}

/** Drop ephemeral progress rows so they don't litter the transcript. */
function clearEphemeralActivity(): void {
  settleActivity();
  for (const row of logEl.querySelectorAll<HTMLElement>(".chat-activity")) {
    const phase = row.dataset.phase ?? "";
    const kind = row.dataset.kind ?? "";
    if (kind === "tool" || kind === "error" || kind === "think") continue;
    if (isQuietPhase(phase) || phase === "writing") row.remove();
  }
}

/** Render progress / tool calls inline in the chat stream (not a side panel). */
function pushActivity(phase: string, message: string): void {
  const text = message.trim() || phase;
  if (!text) return;

  // Quiet lifecycle → header only (avoids "正在请求 / 本轮完成 / 已开始" spam).
  if (isQuietPhase(phase) || phase === "writing") {
    setLiveLabel(text);
    return;
  }

  const kind = activityKind(phase);

  // Deduplicate identical consecutive tool chips (started+completed, or retries).
  if (kind === "tool") {
    const last = logEl.querySelector<HTMLElement>(".chat-activity.kind-tool:last-of-type");
    if (last?.dataset.signature === text) {
      last.classList.add("is-live");
      last.classList.remove("is-done");
      activityEl = last;
      setLiveLabel(t("chat.toolRunning", { cmd: text }));
      return;
    }
  }

  const softPhase = kind === "write";
  const shouldAppend =
    !activityEl ||
    (!softPhase &&
      (kind === "tool" ||
        activityEl.dataset.kind === "tool" ||
        activityEl.dataset.phase !== phase));

  if (shouldAppend) {
    flushPendingTextSync();
    sealAssistantBubble();
    settleActivity();
    const row = document.createElement("div");
    row.className = `chat-activity is-live kind-${kind}`;
    row.dataset.phase = phase;
    row.dataset.kind = kind;
    if (kind === "tool") row.dataset.signature = text;
    if (kind === "tool") {
      row.innerHTML = `<span class="chat-tool-badge" aria-hidden="true">$</span><code class="chat-activity-text chat-tool-cmd"></code>`;
      setLiveLabel(t("chat.toolRunning", { cmd: text }));
    } else {
      row.innerHTML = `<span class="chat-spinner" aria-hidden="true"></span><span class="chat-activity-text"></span>`;
    }
    const label = row.querySelector<HTMLElement>(".chat-activity-text")!;
    label.textContent = text;
    logEl.appendChild(row);
    activityEl = row;
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

function pushPermissionCard(payload: {
  session_id: string;
  request_id: string;
  tool_name: string;
  detail: string;
}): void {
  flushPendingTextSync();
  sealAssistantBubble();
  settleActivity();

  const persisted = persistMessage("permission", payload.detail.trim() || payload.tool_name, {
    permission: {
      requestId: payload.request_id,
      toolName: payload.tool_name,
      detail: payload.detail.trim() || payload.tool_name,
      backendSessionId: payload.session_id,
      allowed: null,
    },
  });

  const card = renderPermissionCard(persisted, true);
  logEl.appendChild(card);
  logEl.scrollTop = logEl.scrollHeight;
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

  const title = document.createElement("div");
  title.className = "chat-permission-title";
  title.textContent = t("chat.permissionTitle", { tool: meta?.toolName ?? "tool" });

  const detail = document.createElement("pre");
  detail.className = "chat-permission-detail";
  detail.textContent = meta?.detail || message.content;

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
        setStatus(t("chat.permissionFailed", { error: String(error) }), "error");
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
          : t("chat.permissionExpired");
    actions.appendChild(badge);
  }

  card.append(title, detail, actions);
  return card;
}

function markPermissionResolved(requestId: string, allowed: boolean): void {
  const session = activeSession();
  const message = session.messages.find(
    (m) => m.role === "permission" && m.permission?.requestId === requestId,
  );
  if (message?.permission) {
    message.permission.allowed = allowed;
    touchSession(session);
    saveStore();
  }

  const card = logEl.querySelector<HTMLElement>(
    `.chat-permission[data-request-id="${CSS.escape(requestId)}"]`,
  );
  if (!card) return;
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

/** Close the current streaming assistant bubble so later events render after it. */
function sealAssistantBubble(): void {
  if (!assistantBubble) {
    assistantMessageId = null;
    assistantRaw = "";
    return;
  }
  assistantBubble.classList.remove("is-streaming");
  if (!assistantRaw.trim()) {
    // Drop empty placeholder bubbles so tools aren't preceded by a blank card.
    const emptyId = assistantMessageId;
    assistantBubble.remove();
    if (emptyId) {
      const session = activeSession();
      session.messages = session.messages.filter((m) => m.id !== emptyId);
      saveStore();
    }
  } else if (assistantMessageId) {
    updateAssistantMessage(assistantMessageId, assistantRaw);
  }
  assistantBubble = null;
  assistantMessageId = null;
  assistantRaw = "";
}

function appendAssistantChunk(chunk: string): void {
  if (!chunk) return;
  turnHadAssistantText = true;
  const bubble = ensureAssistantBubble();
  assistantRaw += chunk;
  bubble.innerHTML = renderMarkdown(assistantRaw);
  if (assistantMessageId) updateAssistantMessage(assistantMessageId, assistantRaw);
  logEl.scrollTop = logEl.scrollHeight;
}

function renderSessionList(): void {
  sessionListEl.replaceChildren();
  for (const session of store.sessions) {
    const row = document.createElement("div");
    row.className = `chat-session${session.id === store.activeId ? " is-active" : ""}`;
    row.dataset.sessionId = session.id;

    const main = document.createElement("button");
    main.type = "button";
    main.className = "chat-session-main";

    const title = document.createElement("span");
    title.className = "chat-session-title";
    title.textContent = sessionTitle(session);

    const meta = document.createElement("span");
    meta.className = "chat-session-meta";
    meta.textContent = `${runtimeDisplayName(session.runtime)} · ${formatTime(session.updatedAt)}`;

    main.append(title, meta);
    main.addEventListener("click", () => {
      if (busy) return;
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
      if (busy) return;
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

function switchSession(id: string): void {
  if (id === store.activeId) return;
  const session = store.sessions.find((s) => s.id === id);
  if (!session) return;
  store.activeId = id;
  saveStore();
  setCurrentRuntime(session.runtime);
  renderActiveMessages();
  renderSessionList();
  titleEl.textContent = sessionTitle(session);
  setStatus("");
  promptEl.focus();
}

function deleteSession(id: string): void {
  if (busy) return;
  if (!window.confirm(t("chat.deleteSessionConfirm"))) return;
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
  assistantBubble = null;
  assistantMessageId = null;
  assistantRaw = "";
  pendingText = "";
  turnHadAssistantText = false;
  pendingAttachments = [];
  activityEl = null;
  renderPendingAttachments();
  renderActiveMessages();
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
  const existing = store.sessions.find((s) => s.runtime === runtime);
  if (existing) {
    switchSession(existing.id);
    return;
  }
  startNewSession();
}

function startNewSession(): void {
  if (busy) return;
  const session = createEmptySession(selectedRuntime());
  store.sessions.unshift(session);
  store.activeId = session.id;
  store.sessions = store.sessions.slice(0, MAX_SESSIONS);
  saveStore();
  assistantBubble = null;
  assistantMessageId = null;
  assistantRaw = "";
  pendingText = "";
  turnHadAssistantText = false;
  pendingAttachments = [];
  activityEl = null;
  renderPendingAttachments();
  renderActiveMessages();
  renderSessionList();
  titleEl.textContent = sessionTitle(session);
  setStatus(t("chat.newSessionReady"), "ok");
  promptEl.focus();
}

function clearActiveSession(): void {
  if (busy) return;
  const session = activeSession();
  session.messages = [];
  session.title = "";
  session.runtimeThreadId = null;
  session.updatedAt = Date.now();
  saveStore();
  assistantBubble = null;
  assistantMessageId = null;
  assistantRaw = "";
  activityEl = null;
  turnHadAssistantText = false;
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

async function pickAttachments(kind: AttachKind): Promise<void> {
  if (busy) return;
  try {
    const selected = await open({
      multiple: true,
      title: kind === "image" ? t("chat.attachPickImage") : t("chat.attachPickFile"),
      filters:
        kind === "image"
          ? [{ name: "Images", extensions: IMAGE_EXTS }]
          : undefined,
    });
    if (!selected) return;
    const paths = (Array.isArray(selected) ? selected : [selected]).filter(Boolean);
    for (const path of paths) {
      if (pendingAttachments.some((a) => a.path === path)) continue;
      if (pendingAttachments.length >= MAX_ATTACHMENTS) {
        setStatus(t("chat.attachLimit", { n: String(MAX_ATTACHMENTS) }), "warn");
        break;
      }
      const name = fileNameFromPath(path);
      pendingAttachments.push({
        id: uid(),
        path,
        name,
        kind: kind === "image" || isImagePath(path) ? "image" : "file",
      });
    }
    renderPendingAttachments();
  } catch (error) {
    setStatus(t("chat.attachFailed", { error: String(error) }), "error");
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
  const session = activeSession();
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
  session.runtime = selectedRuntime();
  touchSession(session);
  saveStore();
  renderSessionList();
  titleEl.textContent = sessionTitle(session);
  return message;
}

function updateAssistantMessage(id: string, content: string): void {
  const session = activeSession();
  const message = session.messages.find((m) => m.id === id);
  if (!message) return;
  message.content = content;
  message.at = Date.now();
  touchSession(session);
  saveStore();
}

function appendBubble(
  kind: ChatRole,
  text: string,
  opts?: { id?: string; persist?: boolean; attachments?: ChatAttachment[] },
): HTMLElement {
  const bubble = document.createElement("div");
  bubble.className = `chat-bubble ${kind}`;
  if (opts?.id) bubble.dataset.messageId = opts.id;
  if (kind === "assistant") {
    bubble.classList.add("chat-md");
    bubble.innerHTML = text.trim() ? renderMarkdown(text) : "";
  } else {
    bubble.textContent = text;
    const strip = renderAttachmentStrip(opts?.attachments);
    if (strip) bubble.appendChild(strip);
  }
  logEl.appendChild(bubble);
  logEl.scrollTop = logEl.scrollHeight;
  return bubble;
}

function renderActiveMessages(): void {
  logEl.replaceChildren();
  assistantBubble = null;
  assistantMessageId = null;
  assistantRaw = "";
  activityEl = null;
  const session = activeSession();
  if (session.messages.length === 0) {
    appendBubble("meta", t("chat.welcome"), { persist: false });
    return;
  }
  for (const message of session.messages) {
    if (message.role === "assistant") {
      const bubble = document.createElement("div");
      bubble.className = "chat-bubble assistant chat-md";
      bubble.dataset.messageId = message.id;
      bubble.innerHTML = renderMarkdown(message.content);
      logEl.appendChild(bubble);
    } else if (message.role === "permission") {
      logEl.appendChild(renderPermissionCard(message, false));
    } else {
      const bubble = document.createElement("div");
      bubble.className = `chat-bubble ${message.role}`;
      bubble.dataset.messageId = message.id;
      bubble.textContent = message.content;
      const strip = renderAttachmentStrip(message.attachments);
      if (strip) bubble.appendChild(strip);
      logEl.appendChild(bubble);
    }
  }
  logEl.scrollTop = logEl.scrollHeight;
}

function ensureAssistantBubble(): HTMLElement {
  if (!assistantBubble) {
    const message = persistMessage("assistant", "");
    assistantMessageId = message.id;
    assistantRaw = "";
    assistantBubble = appendBubble("assistant", "", { id: message.id, persist: false });
    assistantBubble.classList.add("is-streaming");
  }
  return assistantBubble;
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
  return trimmed;
}

function buildPromptWithHistory(userText: string, attachments: ChatAttachment[]): string {
  const session = activeSession();
  // Native resume already carries thread history — only send this turn.
  if (session.runtimeThreadId?.trim()) {
    const parts: string[] = [];
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

  const parts: string[] = [];
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

async function ensureListener(): Promise<void> {
  if (unlisten) return;
  unlisten = await listen<PromptSessionEvent>("prompt-session-event", (event) => {
    const payload = event.payload;
    switch (payload.type) {
      case "started":
        cwdEl.textContent = payload.cwd;
        assistantBubble = null;
        assistantMessageId = null;
        assistantRaw = "";
        pendingText = "";
        turnHadAssistantText = false;
        setLiveLabel(t("chat.phaseStarting"));
        break;
      case "status":
        pushActivity(payload.phase, payload.message);
        break;
      case "delta":
        setLiveLabel(t("chat.phaseStreaming"));
        queueAssistantText(payload.text);
        break;
      case "stdout_line":
        setLiveLabel(t("chat.phaseStreaming"));
        queueAssistantText(`${payload.line}\n`);
        break;
      case "stderr_line":
        pushActivity("error", payload.line);
        settleActivity();
        break;
      case "permission_request":
        pushPermissionCard(payload);
        break;
      case "permission_resolved":
        markPermissionResolved(payload.request_id, payload.allowed);
        break;
      case "completed":
        flushPendingTextSync();
        // Fallback only when this turn never streamed assistant text.
        if (!turnHadAssistantText && !assistantRaw.trim() && payload.summary?.trim()) {
          const fallback = preferPlainSummary(payload.summary);
          if (fallback) appendAssistantChunk(fallback);
        }
        clearEphemeralActivity();
        sealAssistantBubble();
        setLiveLabel(t("chat.live"));
        // Disable any unanswered permission cards if the session ended.
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
        if (!turnHadAssistantText) {
          appendBubble("meta", t("chat.emptyReply"), { persist: false });
        }
        // Success stays silent in the transcript; failures get one short note.
        if (payload.status !== "succeeded") {
          appendBubble(
            "meta",
            t("chat.completed", {
              status: payload.status,
              code: payload.exit_code == null ? "—" : String(payload.exit_code),
            }),
            { persist: false },
          );
        }
        renderSessionList();
        break;
    }
  });
}

function readInitialRuntime(): void {
  const params = new URLSearchParams(window.location.search);
  const runtime = params.get("runtime");
  if (runtime === "claude-code" || runtime === "codex") {
    ensureRuntimeSession(runtime);
  } else {
    setCurrentRuntime(activeSession().runtime);
  }
}

async function openTerminal(): Promise<void> {
  try {
    await invoke("open_session_command", {
      runtime: selectedRuntime(),
      cwd: null,
      prompt: null,
      terminal: true,
    });
    setStatus(t("chat.terminalOpened"), "ok");
  } catch (error) {
    setStatus(t("chat.terminalFailed", { error: String(error) }), "error");
  }
}

async function cancelAsk(): Promise<void> {
  try {
    await invoke<boolean>("cancel_prompt_session_command");
    setStatus(t("chat.cancelling"), "warn");
    pushActivity("info", t("chat.cancelling"));
  } catch (error) {
    setStatus(t("chat.cancelFailed", { error: String(error) }), "error");
  }
}

async function sendAsk(): Promise<void> {
  if (busy) return;
  const text = promptEl.value.trim();
  const attachments = [...pendingAttachments];
  if (!text && attachments.length === 0) {
    setStatus(t("chat.emptyPrompt"), "warn");
    return;
  }

  const runtime = selectedRuntime();
  const elevated = elevatedEl.checked;
  if (elevated && !window.confirm(t("chat.elevatedConfirm"))) return;

  const userText = text || t("chat.attachOnlyPrompt");
  await ensureListener();
  persistMessage("user", userText, { attachments });
  appendBubble("user", userText, { persist: false, attachments });
  promptEl.value = "";
  pendingAttachments = [];
  renderPendingAttachments();
  assistantBubble = null;
  assistantMessageId = null;
  assistantRaw = "";
  pendingText = "";
  turnHadAssistantText = false;
  setBusy(true);
  setLiveLabel(t("chat.phaseStarting"));
  setStatus(t("chat.running", { runtime }), "muted");

  const prompt = buildPromptWithHistory(userText, attachments);
  const resumeThreadId = activeSession().runtimeThreadId?.trim() || null;

  try {
    const report = await invoke<PromptSessionReport>("start_prompt_session_command", {
      runtime,
      prompt,
      cwd: null,
      timeoutSec: 600,
      dangerouslySkipPermissions: runtime === "claude-code" && elevated,
      fullAuto: runtime === "codex" && elevated,
      resumeThreadId,
    });
    cwdEl.textContent = report.cwd;
    if (report.runtime_thread_id?.trim()) {
      const session = activeSession();
      session.runtimeThreadId = report.runtime_thread_id.trim();
      touchSession(session);
      saveStore();
    }
    const tone =
      report.status === "succeeded" ? "ok" : report.status === "cancelled" ? "warn" : "error";
    setStatus(t("chat.done", { status: report.status, ms: String(report.duration_ms) }), tone);
  } catch (error) {
    setStatus(t("chat.failed", { error: String(error) }), "error");
    appendBubble("meta", String(error), { persist: false });
  } finally {
    setBusy(false);
    renderSessionList();
  }
}

function boot(): void {
  readInitialRuntime();
  applyI18n();
  renderActiveMessages();

  actionEl.addEventListener("click", () => {
    if (busy) void cancelAsk();
    else void sendAsk();
  });
  attachFileEl.addEventListener("click", () => void pickAttachments("file"));
  attachImageEl.addEventListener("click", () => void pickAttachments("image"));
  clearEl.addEventListener("click", clearActiveSession);
  newSessionEl.addEventListener("click", startNewSession);
  terminalEl.addEventListener("click", () => void openTerminal());
  elevatedEl.addEventListener("change", () => {
    if (elevatedEl.checked && !window.confirm(t("chat.elevatedConfirm"))) {
      elevatedEl.checked = false;
    }
  });
  promptEl.addEventListener("keydown", (event) => {
    if (event.key !== "Enter") return;
    // Shift+Enter keeps a newline for multi-line prompts.
    if (event.shiftKey) return;
    // IME candidate confirm (Chinese etc.): Enter commits the composition, not send.
    if (event.isComposing || event.keyCode === 229 || promptEl.dataset.composing === "1") return;
    event.preventDefault();
    if (busy) return;
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

  void listen<{ runtime?: string }>("ask-window-focus", (event) => {
    const runtime = event.payload?.runtime;
    if (runtime === "claude-code" || runtime === "codex") {
      ensureRuntimeSession(runtime);
    }
    promptEl.focus();
  });

  void ensureListener();
  promptEl.focus();
}

boot();
