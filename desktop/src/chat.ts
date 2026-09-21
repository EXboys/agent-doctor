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
import { renderMarkdown } from "./markdown";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
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
const elevatedWrapEl = elevatedEl.closest("label") as HTMLLabelElement;
const runtimeLabelEl = document.querySelector<HTMLElement>("#chat-runtime-label")!;
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
const closeEl = document.querySelector<HTMLButtonElement>("#chat-close")!;
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
let workspaceCwd: string | null = null;
let workspaceDoc: WorkspaceDoc | null = null;

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
  const name = runtimeDisplayName(currentRuntime);
  runtimeLabelEl.textContent = name;
  runtimeLabelEl.title = `${name} — ${t("chat.runtimeLockedHint")}`;
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
  if (resourcesSearchEl) {
    resourcesSearchEl.placeholder = t("chat.resourcesSearch");
  }
  document.documentElement.lang = getLocale() === "zh" ? "zh-CN" : "en";
  updateElevatedLabel();
  updateRuntimeLabel();
  syncActionButton();
  renderSessionList();
  titleEl.textContent = sessionTitle(activeSession());
  updateResourcesSummary();
  askResources.renderResourceChips();
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
  elevatedEl.disabled = busy;
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
  elevatedEl.disabled = next || selectedRuntime() === "deepseek-harness";
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

function isQuietStderr(line: string): boolean {
  const text = line.trim();
  const lower = text.toLowerCase();
  return (
    /^session_id:/i.test(text) ||
    /^resume this session/i.test(text) ||
    /resumed session/i.test(text) ||
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
  settleActivity();
  dismissLifecycleActivity();
  scrubToolFragmentsBeforePermission(payload.detail);
  setStatus(t("chat.needYourChoice"), "warn");

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

  hideDecisionDock();
  schedulePaintLivePermissionBatch();
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
    const message = activeSession().messages.find((m) => m.id === only.messageId);
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
      setStatus(t("chat.permissionFailed", { error: String(error) }), "error");
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

  // Remove trailing tool groups created from "调用工具 Bash…" status events.
  while (true) {
    const last = logEl.lastElementChild as HTMLElement | null;
    if (!last?.classList.contains("chat-tool-group")) break;
    last.remove();
  }
  toolGroupEl = null;
  activityEl = null;

  // Remove trailing assistant bubbles that are only the tool JSON / command dump.
  while (true) {
    const last = logEl.lastElementChild as HTMLElement | null;
    if (!last?.classList.contains("assistant")) break;
    const text = (last.textContent ?? "").replace(/\s+/g, " ").trim();
    if (!text) {
      removeAssistantBubbleElement(last);
      continue;
    }
    const isToolDump =
      looksLikeToolPayloadJson(text) ||
      needles.some((needle) => text === needle || text.includes(needle) || needle.includes(text));
    if (!isToolDump) break;
    removeAssistantBubbleElement(last);
  }
}

function removeAssistantBubbleElement(el: HTMLElement): void {
  const messageId = el.dataset.messageId;
  el.remove();
  if (!messageId) return;
  const session = activeSession();
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

  const wasInBatch = pendingPermissionBatch.some((item) => item.requestId === requestId);
  pendingPermissionBatch = pendingPermissionBatch.filter((item) => item.requestId !== requestId);

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
  if (batchCard && wasInBatch && pendingPermissionBatch.length === 0) {
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

  hideDecisionDock();
  setStatus(allowed ? t("chat.permissionAllowed") : t("chat.permissionDenied"), "ok");
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
  if (looksLikeToolPayloadJson(chunk)) {
    const formatted = formatPermissionDetail(chunk.trim());
    pushActivity("tool", formatted.summary || cleanToolLabel(chunk));
    return;
  }
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
  // Prefer the most recently updated session for this agent.
  const existing = store.sessions
    .filter((s) => s.runtime === runtime)
    .sort((a, b) => b.updatedAt - a.updatedAt)[0];
  if (existing) {
    if (busy) {
      void cancelAsk().finally(() => {
        if (selectedRuntime() !== runtime) return;
        if (!busy) switchSession(existing.id);
      });
      return;
    }
    switchSession(existing.id);
    return;
  }
  if (busy) {
    // Finish/cancel the in-flight turn before creating a session for the new agent.
    void cancelAsk().finally(() => {
      if (selectedRuntime() !== runtime) return;
      if (activeSession().runtime === runtime) return;
      if (!busy) startNewSession();
    });
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
);

function updateResourcesSummary(): void {
  askResources.updateResourcesSummary();
}

function toggleResourcesPanel(): void {
  askResources.toggleResourcesPanel();
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
        appendStderrLine(payload.line);
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
        const finalAssistantText = assistantRaw;
        sealAssistantBubble();
        // Disable any unanswered permission cards if the session ended.
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
        hideDecisionDock();
        if (!turnHadAssistantText) {
          appendBubble("meta", t("chat.emptyReply"), { persist: false });
        }
        reportVerifyMcpIfNeeded();
        applyVerifyMcpFooter();
        setBusy(false);
        if (payload.status === "cancelled") {
          setStatus(t("chat.forceStopped"), "warn");
        } else if (payload.status !== "succeeded") {
          appendBubble(
            "meta",
            t("chat.completed", {
              status: payload.status,
              code: payload.exit_code == null ? "—" : String(payload.exit_code),
            }),
            { persist: false },
          );
        } else if (finalAssistantText.trim()) {
          showQuickReplies(finalAssistantText);
        }
        renderSessionList();
        break;
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
  if (!text && attachments.length === 0 && askResources.selectedMentions.length === 0) {
    setStatus(t("chat.emptyPrompt"), "warn");
    return;
  }

  const runtime = selectedRuntime();
  const elevated = selectedRuntime() !== "deepseek-harness" && elevatedEl.checked;
  if (elevated && !window.confirm(t("chat.elevatedConfirm"))) return;

  verifyMcpTurn = Boolean(opts?.verifyMcp);
  verifySawBrowserNavigate = false;
  verifyMcpReported = false;
  verifyTurnText = "";
  if (verifyMcpTurn) {
    pushActivity("info", t("chat.verifyMcpWatching"));
  }

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
  persistMessage("user", userText, { attachments });
  appendBubble("user", userText, { persist: false, attachments });
  promptEl.value = "";
  mentionMenu.hideMentionMenu();
  askResources.clearMentions();
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
  (window as Window & { __AD_ASK_APPLY_RUNTIME__?: (runtime: string) => void }).__AD_ASK_APPLY_RUNTIME__ =
    (runtime) => {
      if (isAskRuntime(runtime)) {
        ensureRuntimeSession(runtime);
      }
    };
  readInitialRuntime();
  updateRuntimeLabel();
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
  closeEl.addEventListener("click", () => {
    void invoke("close_ask_window_command", { destroy: false });
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
  });
  promptEl.addEventListener("keydown", (event) => {
    if (!mentionMenuEl.hidden) {
      const options = mentionMenu.filteredMentionOptions();
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
      if (event.key === "Tab" && options[mentionMenu.mentionMenuIndex]) {
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
    mentionMenu.hideMentionMenu();
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
