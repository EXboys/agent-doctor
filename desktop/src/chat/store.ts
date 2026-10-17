import type { AskRuntime } from "../ask-resources";
import { readAutoApproveDefault } from "./confirm-pref";
import { normalizePlan } from "../plan";
import type { ChatMessage, ChatSession, SessionStore } from "./types";
import {
  CHAT_STORE_MAX_BYTES,
  LEGACY_STORAGE_KEY,
  MAX_MESSAGES_PER_SESSION,
  MAX_SESSIONS,
  STORAGE_BACKUP_KEY,
  STORAGE_KEY,
} from "./types";

export function uid(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

export function normalizeChatMessage(message: ChatMessage): ChatMessage {
  return {
    ...message,
    content: typeof message.content === "string" ? message.content : String(message.content ?? ""),
    at: typeof message.at === "number" && Number.isFinite(message.at) ? message.at : Date.now(),
  };
}

/** Repair historical “one token = one message” fragmentation from early Codex streaming. */
export function coalesceAssistantFragments(messages: ChatMessage[]): ChatMessage[] {
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

export function normalizeSessionMessages(messages: ChatMessage[]): ChatMessage[] {
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

export function finalizeSessionStore(parsed: SessionStore): SessionStore {
  const sessions = parsed.sessions.map((session) => ({
    ...session,
    messages: normalizeSessionMessages((session.messages ?? []) as ChatMessage[]),
    plan: normalizePlan(session.plan),
  }));
  const activeId =
    parsed.activeId && sessions.some((s) => s.id === parsed.activeId)
      ? parsed.activeId
      : sessions[0]!.id;
  return { activeId, sessions };
}

export function trimStoreForSize(data: SessionStore): SessionStore {
  const sessions = data.sessions.slice(0, MAX_SESSIONS).map((session) => ({
    ...session,
    messages: session.messages.slice(-MAX_MESSAGES_PER_SESSION),
  }));
  const activeId = sessions.some((s) => s.id === data.activeId)
    ? data.activeId
    : (sessions[0]?.id ?? data.activeId);
  return { activeId, sessions };
}

export function backupStoreRaw(raw: string): void {
  const trimmed = raw.trim();
  if (!trimmed) return;
  try {
    localStorage.setItem(STORAGE_BACKUP_KEY, trimmed);
  } catch {
    /* quota — keep trying on next save */
  }
}

export function parseStoreRaw(raw: string): SessionStore | null {
  try {
    const parsed = JSON.parse(raw) as SessionStore;
    if (!parsed?.sessions?.length) return null;
    return finalizeSessionStore(parsed);
  } catch {
    return null;
  }
}

/** Agent of the conversation currently open in 问答. Read-only; does not rewrite the store. */
export function readActiveSessionRuntime(): string | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as {
      activeId?: unknown;
      sessions?: { id?: unknown; runtime?: unknown }[];
    };
    if (!Array.isArray(parsed.sessions)) return null;
    const active = parsed.sessions.find((session) => session.id === parsed.activeId);
    return typeof active?.runtime === "string" && active.runtime.trim()
      ? active.runtime.trim()
      : null;
  } catch {
    return null;
  }
}

export function loadStoreFromLocalKeys(): SessionStore | null {
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

export function createEmptySession(
  runtime: AskRuntime,
  workspaceName?: string | null,
): ChatSession {
  const now = Date.now();
  return {
    id: uid(),
    title: "",
    runtime,
    createdAt: now,
    updatedAt: now,
    messages: [],
    runtimeThreadId: null,
    workspaceName: workspaceName ?? null,
    autoApprove: readAutoApproveDefault(),
  };
}

function dropPreviewPrompts(store: SessionStore): SessionStore {
  let changed = false;
  const sessions = store.sessions.map((session) => {
    const messages = session.messages.filter((message) => {
      const requestId = message.permission?.requestId ?? "";
      if (!requestId.startsWith("preview-")) return true;
      changed = true;
      return false;
    });
    return messages.length === session.messages.length ? session : { ...session, messages };
  });
  return changed ? { ...store, sessions } : store;
}

export function loadStore(fallbackRuntime: AskRuntime): SessionStore {
  const loaded = loadStoreFromLocalKeys();
  if (loaded) {
    const cleaned = dropPreviewPrompts(loaded);
    if (cleaned !== loaded) persistStore(cleaned);
    return cleaned;
  }
  const session = createEmptySession(fallbackRuntime);
  return { activeId: session.id, sessions: [session] };
}

const TOOL_TEXT_CAP = 16_000;

function capDiskText(text: string): string {
  if (text.length <= TOOL_TEXT_CAP) return text;
  return `${text.slice(0, TOOL_TEXT_CAP)}\n…`;
}

/** What gets written down. User and assistant text stay whole; tool output is bounded. */
export function chatStoreForDisk(store: SessionStore): SessionStore {
  return {
    ...store,
    sessions: store.sessions.map((session) => ({
      ...session,
      messages: session.messages.map((message) => {
        const isLog = message.role === "meta" && message.log === true;
        let next = message;
        if (message.role === "tool" || isLog) {
          next = { ...next, content: capDiskText(next.content) };
        }
        if (next.step?.output && next.step.output.length > TOOL_TEXT_CAP) {
          next = { ...next, step: { ...next.step, output: capDiskText(next.step.output) } };
        }
        if (next.permission?.detail && next.permission.detail.length > TOOL_TEXT_CAP) {
          next = {
            ...next,
            permission: { ...next.permission, detail: capDiskText(next.permission.detail) },
          };
        }
        return next;
      }),
    })),
  };
}

export function persistStore(store: SessionStore): SessionStore {
  const previous = localStorage.getItem(STORAGE_KEY);
  if (previous) backupStoreRaw(previous);
  const projected = chatStoreForDisk(store);
  let payload = "";
  try {
    payload = JSON.stringify(projected);
  } catch (error) {
    console.warn("Ask: saveStore failed, trimming history", error);
  }
  const disk =
    !payload || payload.length > CHAT_STORE_MAX_BYTES
      ? finalizeSessionStore(trimStoreForSize(projected))
      : projected;
  try {
    localStorage.setItem(STORAGE_KEY, disk === projected ? payload : JSON.stringify(disk));
  } catch (error) {
    console.warn("Ask: saveStore failed, trimming history", error);
    localStorage.setItem(STORAGE_KEY, JSON.stringify(finalizeSessionStore(trimStoreForSize(projected))));
  }
  // The open chat keeps every step so scrolling up can load them. Disk may keep less.
  return store;
}

export function hasRestorableChatBackup(): boolean {
  return Boolean(localStorage.getItem(STORAGE_BACKUP_KEY)?.trim());
}

export function restoreChatStoreFromBackup(): SessionStore | null {
  const raw = localStorage.getItem(STORAGE_BACKUP_KEY);
  if (!raw?.trim()) return null;
  const loaded = parseStoreRaw(raw);
  if (!loaded) return null;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(loaded));
  return loaded;
}

const droppedBeforeDurable = new Set<string>();
let durableReady = false;

/** A delete that happens before the on-disk log has been read must not come back. */
export function noteSessionDropped(id: string): void {
  if (!durableReady) droppedBeforeDurable.add(id);
}

export function isChatStoreDurableReady(): boolean {
  return durableReady;
}

export function markChatStoreDurableReady(): void {
  durableReady = true;
  droppedBeforeDurable.clear();
}

/**
 * Union of the on-disk projection and a browser copy (v1, v2, or the backup key).
 * Same message id keeps the later text, or the longer text when the time matches,
 * so a trimmed browser copy does not erase a full disk copy.
 */
export function mergeChatStores(
  disk: SessionStore | null,
  local: SessionStore | null,
  droppedIds?: ReadonlySet<string>,
): SessionStore | null {
  if (!disk && !local) return null;
  if (!disk) return local;
  if (!local) return disk;
  if (isFreshEmptyWindow(local, disk)) return disk;
  const dropped = droppedIds ?? droppedBeforeDurable;
  const byId = new Map<string, ChatSession>();
  const absorb = (session: ChatSession) => {
    if (dropped.has(session.id) && !local.sessions.some((item) => item.id === session.id)) return;
    const prev = byId.get(session.id);
    byId.set(session.id, prev ? mergeChatSession(prev, session) : session);
  };
  for (const session of disk.sessions) absorb(session);
  for (const session of local.sessions) absorb(session);
  const sessions = [...byId.values()].sort((a, b) => b.updatedAt - a.updatedAt);
  if (!sessions.length) return null;
  const diskActive = disk.sessions.find((session) => session.id === disk.activeId);
  const localActive = local.sessions.find((session) => session.id === local.activeId);
  const preferLocal = (localActive?.updatedAt ?? 0) >= (diskActive?.updatedAt ?? 0);
  const activeId = preferLocal ? local.activeId : disk.activeId;
  return {
    activeId: sessions.some((session) => session.id === activeId) ? activeId : sessions[0].id,
    sessions,
  };
}

function isFreshEmptyWindow(local: SessionStore, disk: SessionStore): boolean {
  if (disk.sessions.length === 0 || local.sessions.length !== 1) return false;
  const session = local.sessions[0];
  if (disk.sessions.some((item) => item.id === session.id)) return false;
  return (
    session.messages.length === 0 &&
    !session.title.trim() &&
    !session.runtimeThreadId &&
    !session.channelKey
  );
}

function mergeChatSession(left: ChatSession, right: ChatSession): ChatSession {
  const newer = right.updatedAt >= left.updatedAt ? right : left;
  const older = newer === right ? left : right;
  return {
    ...older,
    ...newer,
    messages:
      newer.updatedAt === older.updatedAt
        ? unionMessages(older.messages ?? [], newer.messages ?? [])
        : authoritativeMessages(older.messages ?? [], newer.messages ?? []),
  };
}

/** Same clock: keep lines that exist on only one side. A newer save replaces the list. */
function unionMessages(older: ChatMessage[], newer: ChatMessage[]): ChatMessage[] {
  const newerById = new Map(newer.map((message) => [message.id, message]));
  const seen = new Set(older.map((message) => message.id));
  const merged = older.map((message) => {
    const alt = newerById.get(message.id);
    return alt ? preferChatMessage(message, alt) : message;
  });
  for (const message of newer) {
    if (seen.has(message.id)) continue;
    const index = merged.findIndex((item) => item.at > message.at);
    if (index === -1) merged.push(message);
    else merged.splice(index, 0, message);
  }
  return merged;
}

function authoritativeMessages(older: ChatMessage[], newer: ChatMessage[]): ChatMessage[] {
  const olderById = new Map(older.map((message) => [message.id, message]));
  return newer.map((message) => {
    const prev = olderById.get(message.id);
    return prev ? preferChatMessage(prev, message) : message;
  });
}

function preferChatMessage(older: ChatMessage, newer: ChatMessage): ChatMessage {
  if (newer.at > older.at) return newer;
  if (older.at > newer.at) return older;
  if (isCappedCopy(newer, older)) return newer;
  if (isCappedCopy(older, newer)) return older;
  return newer.content.length >= older.content.length ? newer : older;
}

/** Disk cap appends a newline and an ellipsis. That copy should stay capped. */
function isCappedCopy(shorter: ChatMessage, longer: ChatMessage): boolean {
  if (!shorter.content.endsWith("\n…") || shorter.content.length >= longer.content.length) return false;
  const prefix = shorter.content.slice(0, -2);
  return prefix.length >= TOOL_TEXT_CAP && longer.content.startsWith(prefix);
}

export function sessionTitle(session: ChatSession, untitled: string): string {
  if (session.title.trim()) return session.title.trim();
  const firstUser = session.messages.find((m) => m.role === "user");
  if (firstUser?.content.trim()) {
    const line = firstUser.content.trim().split(/\n/)[0] ?? "";
    return line.length > 28 ? `${line.slice(0, 28)}…` : line;
  }
  return untitled;
}
