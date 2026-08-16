import { getLocale, t, type MessageKey } from "./i18n";
import { renderMarkdown } from "./markdown";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

type AskRuntime = "claude-code" | "codex" | "hermes" | "openclaw";
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

interface SkillAgentUsage {
  runtime: string;
  scope: string;
  path: string;
  mounted: boolean;
}

interface SkillInventoryItem {
  skill_id: string;
  name: string;
  version: string;
  description: string | null;
  agents: SkillAgentUsage[];
}

interface SkillsInventoryReport {
  skills: SkillInventoryItem[];
}

interface McpInventoryItem {
  name: string;
  scope: string;
  healthy: boolean;
  issue: string | null;
  is_browser: boolean;
  runtime_hint: string;
}

interface McpInventoryReport {
  workspace_name: string | null;
  workspace_path: string | null;
  servers: McpInventoryItem[];
}

type MentionKind = "skill" | "mcp";

interface MentionRef {
  kind: MentionKind;
  id: string;
  label: string;
}

const STORAGE_KEY = "agent-doctor.chat.sessions.v2";
const LEGACY_STORAGE_KEY = "agent-doctor.chat.sessions.v1";
const MAX_SESSIONS = 40;
const MAX_CONTEXT_MESSAGES = 12;
const MAX_ATTACHMENTS = 8;
const MENTION_TOKEN_RE = /@(?:skill|mcp):([^\s@]+)/gi;

const IMAGE_EXTS = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "heic", "avif"];

const elevatedEl = document.querySelector<HTMLInputElement>("#chat-elevated")!;
const elevatedLabelEl = document.querySelector<HTMLElement>("#chat-elevated-label")!;
const promptEl = document.querySelector<HTMLTextAreaElement>("#chat-prompt")!;
const actionEl = document.querySelector<HTMLButtonElement>("#chat-action")!;
const attachEl = document.querySelector<HTMLButtonElement>("#chat-attach")!;
const attachmentsEl = document.querySelector<HTMLElement>("#chat-attachments")!;
const composerBoxEl = document.querySelector<HTMLElement>(".chat-composer-box")!;
const mentionsEl = document.querySelector<HTMLElement>("#chat-mentions")!;
const mentionMenuEl = document.querySelector<HTMLElement>("#chat-mention-menu")!;
const clearEl = document.querySelector<HTMLButtonElement>("#chat-clear")!;
const newSessionEl = document.querySelector<HTMLButtonElement>("#chat-new")!;
const terminalEl = document.querySelector<HTMLButtonElement>("#chat-terminal")!;
const sessionListEl = document.querySelector<HTMLElement>("#chat-sessions")!;
const logEl = document.querySelector<HTMLElement>("#chat-log")!;
const statusEl = document.querySelector<HTMLElement>("#chat-status")!;
const cwdEl = document.querySelector<HTMLElement>("#chat-cwd")!;
const workspaceSelectEl = document.querySelector<HTMLSelectElement>("#chat-workspace-select")!;
const workspaceActivateEl = document.querySelector<HTMLButtonElement>("#chat-workspace-activate")!;
const workspaceHintEl = document.querySelector<HTMLElement>("#chat-workspace-hint")!;
const titleEl = document.querySelector<HTMLElement>("#chat-title")!;
const shellEl = document.querySelector<HTMLElement>("#chat-shell")!;
const resourcesPanelEl = document.querySelector<HTMLElement>("#chat-resources-panel")!;
const resourcesToggleEl = document.querySelector<HTMLButtonElement>("#chat-resources-toggle")!;
const resourcesLabelEl = document.querySelector<HTMLElement>("#chat-resources-label")!;
const resourcesRefreshEl = document.querySelector<HTMLButtonElement>("#chat-resources-refresh")!;
const openResourcesEl = document.querySelector<HTMLButtonElement>("#chat-open-resources")!;
const skillsListEl = document.querySelector<HTMLElement>("#chat-skills-list")!;
const skillsEmptyEl = document.querySelector<HTMLElement>("#chat-skills-empty")!;
const mcpListEl = document.querySelector<HTMLElement>("#chat-mcp-list")!;
const mcpEmptyEl = document.querySelector<HTMLElement>("#chat-mcp-empty")!;

/** Locked by main-page Ask entry (`?runtime=` / ask-window-focus). Not switched in-chat. */
let currentRuntime: AskRuntime = "claude-code";

let store: SessionStore = loadStore();
let busy = false;
let busyGen = 0;
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
let mountedSkills: SkillInventoryItem[] = [];
let enabledMcps: McpInventoryItem[] = [];
let workspaceCwd: string | null = null;
let workspaceDoc: {
  active: string | null;
  workspaces: Record<string, { path: string }>;
} | null = null;
let selectedMentions: MentionRef[] = [];
let mentionMenuIndex = 0;
let mentionQuery: { kind: MentionKind | "any"; q: string; start: number; end: number } | null =
  null;

function uid(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function autoResizePrompt(): void {
  promptEl.style.height = "auto";
  const styles = window.getComputedStyle(promptEl);
  const maxHeight = Number.parseFloat(styles.maxHeight);
  const next = Number.isFinite(maxHeight)
    ? Math.min(promptEl.scrollHeight, maxHeight)
    : promptEl.scrollHeight;
  promptEl.style.height = `${next}px`;
}

function selectedRuntime(): AskRuntime {
  return currentRuntime;
}

function runtimeDisplayName(runtime: AskRuntime): string {
  if (runtime === "codex") return "Codex";
  if (runtime === "hermes") return "Hermes";
  if (runtime === "openclaw") return "OpenClaw";
  return "Claude Code";
}

function isAskRuntime(value: string | null | undefined): value is AskRuntime {
  return (
    value === "claude-code" ||
    value === "codex" ||
    value === "hermes" ||
    value === "openclaw"
  );
}

function setCurrentRuntime(runtime: AskRuntime, opts?: { syncSession?: boolean }): void {
  currentRuntime = runtime;
  updateElevatedLabel();
  if (opts?.syncSession) {
    const session = activeSession();
    if (session.messages.length === 0) {
      session.runtime = runtime;
      saveStore();
    }
  }
  void loadAskResources();
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
  attachEl.title = t("chat.attach");
  attachEl.setAttribute("aria-label", t("chat.attach"));
  document.documentElement.lang = getLocale() === "zh" ? "zh-CN" : "en";
  updateElevatedLabel();
  syncActionButton();
  renderSessionList();
  titleEl.textContent = sessionTitle(activeSession());
  updateResourcesSummary();
}

function updateElevatedLabel(): void {
  const runtime = selectedRuntime();
  if (runtime === "codex") {
    elevatedLabelEl.textContent = t("chat.elevatedCodex");
  } else if (runtime === "hermes") {
    elevatedLabelEl.textContent = t("chat.elevatedHermes");
  } else if (runtime === "openclaw") {
    elevatedLabelEl.textContent = t("chat.elevatedOpenclaw");
  } else {
    elevatedLabelEl.textContent = t("chat.elevatedClaude");
  }
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
  if (next) busyGen += 1;
  busy = next;
  promptEl.disabled = next;
  elevatedEl.disabled = next;
  newSessionEl.disabled = next;
  attachEl.disabled = next;
  sessionListEl.classList.toggle("is-busy", next);
  syncActionButton();
  if (!next) {
    settleActivity();
    finishToolGroup(true);
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
  return phase === "writing" || phase === "streaming" || phase === "info";
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
}

/** Render progress / tool calls inline in the chat stream (not a side panel). */
function pushActivity(phase: string, message: string): void {
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

function pushPermissionCard(payload: {
  session_id: string;
  request_id: string;
  tool_name: string;
  detail: string;
}): void {
  flushPendingTextSync();
  sealAssistantBubble();
  settleActivity();
  finishToolGroup(true);
  dismissLifecycleActivity();

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
  if (activityEl?.dataset.kind === "tool") settleActivity();
  finishToolGroup(true);
  dismissLifecycleActivity();
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

function addAttachmentPaths(paths: string[]): void {
  if (busy || paths.length === 0) return;
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
  if (busy) return;
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
  composerBoxEl.classList.toggle("is-drop-target", active && !busy);
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
  if (/^hermes (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  if (/^openclaw (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
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
  updateResourcesSummary();
}

function setResourcesOpen(open: boolean): void {
  shellEl.classList.toggle("is-resources-open", open);
  resourcesToggleEl.classList.toggle("is-open", open);
  resourcesToggleEl.setAttribute("aria-expanded", open ? "true" : "false");
  resourcesPanelEl.setAttribute("aria-hidden", open ? "false" : "true");
}

function toggleResourcesPanel(): void {
  setResourcesOpen(!shellEl.classList.contains("is-resources-open"));
}

function updateResourcesSummary(): void {
  const cwd = displayCwd();
  resourcesLabelEl.textContent = t("chat.resourcesSummary", {
    cwd: shortCwdLabel(cwd),
    skills: String(mountedSkills.length),
    mcp: String(enabledMcps.length),
  });
  resourcesLabelEl.title = cwd;
}

function mcpMatchesRuntime(server: McpInventoryItem, runtime: AskRuntime): boolean {
  const hint = server.runtime_hint.trim();
  return hint === runtime || hint === "shared" || hint === "";
}

function skillMountedForRuntime(skill: SkillInventoryItem, runtime: AskRuntime): boolean {
  return skill.agents.some((agent) => agent.runtime === runtime && agent.mounted);
}

function mentionKey(m: MentionRef): string {
  return `${m.kind}:${m.id}`;
}

function hasMention(kind: MentionKind, id: string): boolean {
  return selectedMentions.some((m) => m.kind === kind && m.id === id);
}

function upsertMention(mention: MentionRef): void {
  if (hasMention(mention.kind, mention.id)) return;
  selectedMentions.push(mention);
  renderMentions();
  renderResourceChips();
}

function removeMention(kind: MentionKind, id: string): void {
  selectedMentions = selectedMentions.filter((m) => !(m.kind === kind && m.id === id));
  renderMentions();
  renderResourceChips();
}

function toggleMention(mention: MentionRef): void {
  if (hasMention(mention.kind, mention.id)) removeMention(mention.kind, mention.id);
  else upsertMention(mention);
}

function clearMentions(): void {
  selectedMentions = [];
  renderMentions();
  renderResourceChips();
}

function renderMentions(): void {
  mentionsEl.replaceChildren();
  mentionsEl.hidden = selectedMentions.length === 0;
  for (const mention of selectedMentions) {
    const chip = document.createElement("span");
    chip.className = "chat-mention-chip";
    const label = document.createElement("span");
    label.textContent =
      mention.kind === "skill"
        ? t("chat.mentionSkill", { name: mention.label })
        : t("chat.mentionMcp", { name: mention.label });
    const remove = document.createElement("button");
    remove.type = "button";
    remove.setAttribute("aria-label", t("chat.mentionRemove"));
    remove.textContent = "×";
    remove.addEventListener("click", () => removeMention(mention.kind, mention.id));
    chip.append(label, remove);
    mentionsEl.appendChild(chip);
  }
}

function renderResourceChips(): void {
  skillsListEl.replaceChildren();
  mcpListEl.replaceChildren();
  skillsEmptyEl.hidden = mountedSkills.length > 0;
  mcpEmptyEl.hidden = enabledMcps.length > 0;

  for (const skill of mountedSkills) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "chat-res-chip";
    if (hasMention("skill", skill.skill_id)) btn.classList.add("is-active");
    btn.textContent = skill.name || skill.skill_id;
    btn.title = skill.description || skill.skill_id;
    btn.addEventListener("click", () =>
      toggleMention({
        kind: "skill",
        id: skill.skill_id,
        label: skill.name || skill.skill_id,
      }),
    );
    skillsListEl.appendChild(btn);
  }

  for (const server of enabledMcps) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "chat-res-chip";
    if (!server.healthy) btn.classList.add("is-warn");
    if (hasMention("mcp", server.name)) btn.classList.add("is-active");
    btn.textContent = server.is_browser ? `${server.name} · browser` : server.name;
    btn.title = server.issue || `${server.scope} · ${server.runtime_hint}`;
    btn.addEventListener("click", () =>
      toggleMention({
        kind: "mcp",
        id: server.name,
        label: server.name,
      }),
    );
    mcpListEl.appendChild(btn);
  }

  updateResourcesSummary();
}

async function loadAskResources(): Promise<void> {
  const runtime = selectedRuntime();
  try {
    const [skillsReport, mcpReport] = await Promise.all([
      invoke<SkillsInventoryReport>("list_skills_inventory_command", { remoteStats: false }),
      invoke<McpInventoryReport>("list_mcp_inventory_command"),
    ]);
    workspaceCwd = mcpReport.workspace_path;
    try {
      const doc = await invoke<{
        active: string | null;
        workspaces: Record<string, { path: string }>;
      }>("list_workspaces_command");
      workspaceDoc = doc;
      renderWorkspaceSwitcher(doc);
      if (doc.active && doc.workspaces[doc.active]?.path) {
        workspaceCwd = doc.workspaces[doc.active].path;
        cwdEl.dataset.workspace = doc.active;
        setDisplayedCwd(workspaceCwd);
        cwdEl.title = `${t("ask.workspaceActive", { name: doc.active })} · ${workspaceCwd}`;
        workspaceHintEl.textContent = t("ask.workspaceHint");
      } else {
        cwdEl.dataset.workspace = "";
        if (mcpReport.workspace_path) {
          setDisplayedCwd(mcpReport.workspace_path);
        } else {
          setDisplayedCwd("—");
        }
        cwdEl.title = t("ask.workspaceNone");
        workspaceHintEl.textContent = t("ask.workspaceNone");
      }
    } catch {
      workspaceDoc = null;
      if (mcpReport.workspace_path && (!cwdEl.dataset.cwd || cwdEl.dataset.cwd === "—")) {
        setDisplayedCwd(mcpReport.workspace_path);
      }
    }
    mountedSkills = (skillsReport.skills ?? []).filter((skill) =>
      skillMountedForRuntime(skill, runtime),
    );
    enabledMcps = (mcpReport.servers ?? []).filter((server) => mcpMatchesRuntime(server, runtime));
    // Drop mentions that no longer exist for this runtime.
    selectedMentions = selectedMentions.filter((m) => {
      if (m.kind === "skill") return mountedSkills.some((s) => s.skill_id === m.id);
      return enabledMcps.some((s) => s.name === m.id);
    });
    renderMentions();
    renderResourceChips();
  } catch (error) {
    setStatus(t("chat.resourcesLoadFailed", { error: String(error) }), "warn");
  }
}

function renderWorkspaceSwitcher(doc: {
  active: string | null;
  workspaces: Record<string, { path: string }>;
}): void {
  const names = Object.keys(doc.workspaces).sort();
  workspaceSelectEl.innerHTML = "";
  if (names.length === 0) {
    const opt = document.createElement("option");
    opt.value = "";
    opt.textContent = t("ask.workspaceEmpty");
    workspaceSelectEl.appendChild(opt);
    workspaceSelectEl.disabled = true;
    workspaceActivateEl.disabled = true;
    return;
  }

  workspaceSelectEl.disabled = false;
  workspaceActivateEl.disabled = false;
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
  workspaceActivateEl.disabled = Boolean(doc.active && workspaceSelectEl.value === doc.active);
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
  } finally {
    workspaceActivateEl.disabled = false;
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
    await invoke("focus_main_tab_command", { tab: "resources" });
  } catch (error) {
    setStatus(String(error), "error");
  }
}

function parseMentionsFromText(text: string): MentionRef[] {
  const found: MentionRef[] = [];
  const seen = new Set<string>();
  for (const match of text.matchAll(MENTION_TOKEN_RE)) {
    const raw = match[0];
    const id = (match[1] || "").trim();
    if (!id) continue;
    const kind: MentionKind = raw.toLowerCase().startsWith("@mcp:") ? "mcp" : "skill";
    const key = `${kind}:${id}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const label =
      kind === "skill"
        ? mountedSkills.find((s) => s.skill_id === id || s.name === id)?.name || id
        : enabledMcps.find((s) => s.name === id)?.name || id;
    found.push({ kind, id, label });
  }
  return found;
}

function stripMentionTokens(text: string): string {
  return text
    .replace(MENTION_TOKEN_RE, " ")
    .replace(/[ \t]{2,}/g, " ")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function mergeMentionsForSend(userText: string): MentionRef[] {
  const fromText = parseMentionsFromText(userText);
  const map = new Map<string, MentionRef>();
  for (const m of [...selectedMentions, ...fromText]) {
    map.set(mentionKey(m), m);
  }
  return [...map.values()];
}

function buildMentionConstraint(mentions: MentionRef[]): string {
  if (mentions.length === 0) return "";
  const list = mentions
    .map((m) => {
      if (m.kind === "skill") return `- Skill: ${m.label} (id: ${m.id})`;
      if (m.id.toLowerCase() === "browser" || m.label.toLowerCase().includes("browser")) {
        return `- MCP server: ${m.label} — use tools browser_navigate / browser_click / browser_screenshot (never shell open/curl)`;
      }
      return `- MCP server: ${m.label}`;
    })
    .join("\n");
  return t("chat.mentionHint", { list });
}

function promptRequestsBrowserMcp(text: string): boolean {
  const lower = text.toLowerCase();
  if (
    text.includes("浏览器") ||
    lower.includes("browser mcp") ||
    lower.includes("@mcp:browser") ||
    lower.includes("browser_navigate") ||
    lower.includes("open browser") ||
    lower.includes("launch browser") ||
    lower.includes("navigate to")
  ) {
    return true;
  }
  if (
    (lower.includes("open ") || lower.includes("visit ") || lower.includes("go to ")) &&
    (lower.includes("http://") ||
      lower.includes("https://") ||
      lower.includes(".com") ||
      lower.includes(".cn") ||
      lower.includes("baidu") ||
      lower.includes("google"))
  ) {
    return true;
  }
  return false;
}

function ensureBrowserMention(mentions: MentionRef[], userText: string): MentionRef[] {
  if (!promptRequestsBrowserMcp(userText)) return mentions;
  if (mentions.some((m) => m.kind === "mcp" && m.id.toLowerCase().includes("browser"))) {
    return mentions;
  }
  const browser = enabledMcps.find(
    (s) => s.is_browser || s.name.toLowerCase() === "browser" || s.name.toLowerCase().includes("browser"),
  );
  if (!browser) return mentions;
  return [
    ...mentions,
    { kind: "mcp", id: browser.name, label: browser.name },
  ];
}

function mentionCandidates(): MentionRef[] {
  const skills = mountedSkills.map((s) => ({
    kind: "skill" as const,
    id: s.skill_id,
    label: s.name || s.skill_id,
  }));
  const mcps = enabledMcps.map((s) => ({
    kind: "mcp" as const,
    id: s.name,
    label: s.name,
  }));
  return [...skills, ...mcps];
}

function detectMentionQuery(): typeof mentionQuery {
  const value = promptEl.value;
  const caret = promptEl.selectionStart ?? value.length;
  const before = value.slice(0, caret);
  const match = before.match(/(?:^|\s)(@(?:skill:|mcp:)?([^\s@]*))$/i);
  if (!match || match.index == null) return null;
  const token = match[1];
  const q = match[2] || "";
  const start = match.index + (match[0].startsWith("@") ? 0 : 1);
  const end = caret;
  let kind: MentionKind | "any" = "any";
  const lower = token.toLowerCase();
  if (lower.startsWith("@skill:")) kind = "skill";
  else if (lower.startsWith("@mcp:")) kind = "mcp";
  return { kind, q, start, end };
}

function filteredMentionOptions(): MentionRef[] {
  if (!mentionQuery) return [];
  const q = mentionQuery.q.toLowerCase();
  return mentionCandidates().filter((item) => {
    if (mentionQuery!.kind !== "any" && item.kind !== mentionQuery!.kind) return false;
    if (!q) return true;
    return item.id.toLowerCase().includes(q) || item.label.toLowerCase().includes(q);
  });
}

function hideMentionMenu(): void {
  mentionQuery = null;
  mentionMenuEl.hidden = true;
  mentionMenuEl.replaceChildren();
}

function renderMentionMenu(): void {
  mentionQuery = detectMentionQuery();
  const options = filteredMentionOptions();
  if (!mentionQuery || options.length === 0) {
    hideMentionMenu();
    return;
  }
  mentionMenuIndex = Math.max(0, Math.min(mentionMenuIndex, options.length - 1));
  mentionMenuEl.replaceChildren();
  options.forEach((option, index) => {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "chat-mention-option";
    if (index === mentionMenuIndex) btn.classList.add("is-active");
    btn.setAttribute("role", "option");
    const title = document.createElement("strong");
    title.textContent =
      option.kind === "skill"
        ? t("chat.mentionSkill", { name: option.label })
        : t("chat.mentionMcp", { name: option.label });
    const sub = document.createElement("span");
    sub.textContent = `@${option.kind}:${option.id}`;
    btn.append(title, sub);
    btn.addEventListener("mousedown", (event) => {
      event.preventDefault();
      applyMentionOption(option);
    });
    mentionMenuEl.appendChild(btn);
  });
  mentionMenuEl.hidden = false;
}

function applyMentionOption(option: MentionRef): void {
  if (!mentionQuery) return;
  const value = promptEl.value;
  const token = `@${option.kind}:${option.id} `;
  promptEl.value = `${value.slice(0, mentionQuery.start)}${token}${value.slice(mentionQuery.end)}`;
  const next = mentionQuery.start + token.length;
  promptEl.setSelectionRange(next, next);
  upsertMention(option);
  hideMentionMenu();
  autoResizePrompt();
  promptEl.focus();
}

function buildPromptWithHistory(userText: string, attachments: ChatAttachment[]): string {
  const session = activeSession();
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

async function ensureListener(): Promise<void> {
  if (unlisten) return;
  unlisten = await listen<PromptSessionEvent>("prompt-session-event", (event) => {
    const payload = event.payload;
    switch (payload.type) {
      case "started":
        setDisplayedCwd(payload.cwd);
        assistantBubble = null;
        assistantMessageId = null;
        assistantRaw = "";
        pendingText = "";
        turnHadAssistantText = false;
        pushActivity("think", t("chat.waitingModel"));
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
        pushActivity("error", payload.line);
        settleActivity();
        break;
      case "permission_request":
        pushPermissionCard(payload);
        noteVerifyBrowserSignal(payload.tool_name, "tool");
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
        noteVerifyBrowserSignal(assistantRaw, "assistant");
        clearEphemeralActivity();
        sealAssistantBubble();
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
        reportVerifyMcpIfNeeded();
        applyVerifyMcpFooter();
        setBusy(false);
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
  const gen = busyGen;
  try {
    const stopped = await invoke<boolean>("cancel_prompt_session_command");
    setStatus(t("chat.cancelling"), "warn");
    pushActivity("think", t("chat.cancelling"));
    if (!stopped && busy && busyGen === gen) {
      setBusy(false);
      setStatus(t("chat.forceStopped"), "warn");
      return;
    }
    window.setTimeout(() => {
      if (busy && busyGen === gen) {
        setBusy(false);
        setStatus(t("chat.forceStopped"), "warn");
      }
    }, 2500);
  } catch (error) {
    setBusy(false);
    setStatus(t("chat.cancelFailed", { error: String(error) }), "error");
  }
}

async function sendAsk(opts?: { verifyMcp?: boolean }): Promise<void> {
  if (busy) return;
  const text = promptEl.value.trim();
  const attachments = [...pendingAttachments];
  if (!text && attachments.length === 0 && selectedMentions.length === 0) {
    setStatus(t("chat.emptyPrompt"), "warn");
    return;
  }

  const runtime = selectedRuntime();
  const elevated = elevatedEl.checked;
  if (elevated && !window.confirm(t("chat.elevatedConfirm"))) return;

  verifyMcpTurn = Boolean(opts?.verifyMcp);
  verifySawBrowserNavigate = false;
  verifyMcpReported = false;
  verifyTurnText = "";
  if (verifyMcpTurn) {
    pushActivity("info", t("chat.verifyMcpWatching"));
  }

  const mentions = ensureBrowserMention(mergeMentionsForSend(text), text);
  const cleaned = stripMentionTokens(text);
  const userText = cleaned || text || t("chat.attachOnlyPrompt");
  const constraint = buildMentionConstraint(mentions);
  const promptUserText = constraint ? `${constraint}\n\n${userText}` : userText;
  const selectedMcps = mentions.filter((m) => m.kind === "mcp").map((m) => m.id);

  await ensureListener();
  persistMessage("user", userText, { attachments });
  appendBubble("user", userText, { persist: false, attachments });
  promptEl.value = "";
  hideMentionMenu();
  clearMentions();
  autoResizePrompt();
  pendingAttachments = [];
  renderPendingAttachments();
  assistantBubble = null;
  assistantMessageId = null;
  assistantRaw = "";
  pendingText = "";
  turnHadAssistantText = false;
  setBusy(true);
  setStatus(t("chat.running", { runtime }), "muted");
  pushActivity("think", t("chat.waitingModel"));

  const prompt = buildPromptWithHistory(promptUserText, attachments);
  const resumeThreadId = activeSession().runtimeThreadId?.trim() || null;

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
      const session = activeSession();
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
      appendBubble("meta", t("chat.forceStopped"), { persist: false });
    } else {
      setStatus(t("chat.failed", { error: message }), "error");
      appendBubble("meta", message, { persist: false });
    }
  } finally {
    applyVerifyEvidenceFromAssistant();
    if (verifySawBrowserNavigate) {
      reportVerifyMcpIfNeeded();
      applyVerifyMcpFooter();
    }
    setBusy(false);
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

function boot(): void {
  readInitialRuntime();
  applyI18n();
  renderActiveMessages();
  void setupFileDrop();
  void (async () => {
    await loadAskResources();
    applyVerifyDraftIfAny();
  })();

  actionEl.addEventListener("click", () => {
    if (busy) void cancelAsk();
    else void sendAsk();
  });
  attachEl.addEventListener("click", () => void pickAttachments());
  clearEl.addEventListener("click", clearActiveSession);
  newSessionEl.addEventListener("click", startNewSession);
  terminalEl.addEventListener("click", () => void openTerminal());
  resourcesToggleEl.addEventListener("click", () => {
    toggleResourcesPanel();
  });
  resourcesRefreshEl.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    void loadAskResources();
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
      return;
    }
    const path = workspaceDoc.workspaces[name]?.path;
    if (path) {
      cwdEl.textContent = path;
      cwdEl.title = path;
    }
    workspaceActivateEl.disabled = workspaceDoc.active === name;
  });
  workspaceHintEl.addEventListener("dblclick", () => void openMainWorkspace());
  elevatedEl.addEventListener("change", () => {
    if (elevatedEl.checked && !window.confirm(t("chat.elevatedConfirm"))) {
      elevatedEl.checked = false;
    }
  });
  promptEl.addEventListener("input", () => {
    autoResizePrompt();
    renderMentionMenu();
  });
  promptEl.addEventListener("keydown", (event) => {
    if (!mentionMenuEl.hidden) {
      const options = filteredMentionOptions();
      if (event.key === "ArrowDown" && options.length > 0) {
        event.preventDefault();
        mentionMenuIndex = (mentionMenuIndex + 1) % options.length;
        renderMentionMenu();
        return;
      }
      if (event.key === "ArrowUp" && options.length > 0) {
        event.preventDefault();
        mentionMenuIndex = (mentionMenuIndex - 1 + options.length) % options.length;
        renderMentionMenu();
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        hideMentionMenu();
        return;
      }
      if (event.key === "Enter" && !event.shiftKey && options[mentionMenuIndex]) {
        event.preventDefault();
        applyMentionOption(options[mentionMenuIndex]);
        return;
      }
      if (event.key === "Tab" && options[mentionMenuIndex]) {
        event.preventDefault();
        applyMentionOption(options[mentionMenuIndex]);
        return;
      }
    }
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
  document.addEventListener("click", (event) => {
    if (!(event.target instanceof Node)) return;
    if (mentionMenuEl.contains(event.target) || promptEl.contains(event.target)) return;
    hideMentionMenu();
  });

  void listen<{ runtime?: string }>("ask-window-focus", (event) => {
    const runtime = event.payload?.runtime;
    if (isAskRuntime(runtime)) {
      ensureRuntimeSession(runtime);
    }
    void (async () => {
      await loadAskResources();
      applyVerifyDraftIfAny();
    })();
    promptEl.focus();
  });

  void ensureListener();
  autoResizePrompt();
  promptEl.focus();
}

boot();
