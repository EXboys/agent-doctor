import type { AskRuntime } from "../ask-resources";
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

export function createEmptySession(runtime: AskRuntime): ChatSession {
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

export function loadStore(fallbackRuntime: AskRuntime): SessionStore {
  const loaded = loadStoreFromLocalKeys();
  if (loaded) return loaded;
  const session = createEmptySession(fallbackRuntime);
  return { activeId: session.id, sessions: [session] };
}

export function persistStore(store: SessionStore): SessionStore {
  const previous = localStorage.getItem(STORAGE_KEY);
  if (previous) backupStoreRaw(previous);
  let next = store;
  let payload = JSON.stringify(next);
  if (payload.length > CHAT_STORE_MAX_BYTES) {
    next = finalizeSessionStore(trimStoreForSize(next));
    payload = JSON.stringify(next);
  }
  try {
    localStorage.setItem(STORAGE_KEY, payload);
  } catch (error) {
    console.warn("Ask: saveStore failed, trimming history", error);
    next = finalizeSessionStore(trimStoreForSize(next));
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  }
  return next;
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

export function sessionTitle(session: ChatSession, untitled: string): string {
  if (session.title.trim()) return session.title.trim();
  const firstUser = session.messages.find((m) => m.role === "user");
  if (firstUser?.content.trim()) {
    const line = firstUser.content.trim().split(/\n/)[0] ?? "";
    return line.length > 28 ? `${line.slice(0, 28)}…` : line;
  }
  return untitled;
}
