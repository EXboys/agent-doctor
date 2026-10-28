import type { ChatMessage, ChatSession, SessionStore } from "./types";

/** What one save sends. Untouched sessions are absent. `fromSeq` is the first changed row. */
export type ChatWritePatch = {
  v: 2;
  activeId: string;
  order: string[];
  deletedIds: string[];
  sessions: Array<Omit<ChatSession, "messages"> & { fromSeq: number; messages: ChatMessage[] }>;
};

type SessionCursor = {
  header: string;
  stamps: string[];
};

/** Cursor for the last successful write. The next save diffs against this, not against the whole file. */
export type ChatWriteMemory = {
  activeId: string;
  order: string[];
  sessions: Map<string, SessionCursor>;
};

export function memoryFromStore(store: SessionStore): ChatWriteMemory {
  const sessions = new Map<string, SessionCursor>();
  for (const session of store.sessions) sessions.set(session.id, sessionCursor(session));
  return {
    activeId: store.activeId,
    order: store.sessions.map((session) => session.id),
    sessions,
  };
}

export function diffChatWrite(
  store: SessionStore,
  prev: ChatWriteMemory | null,
): { patch: ChatWritePatch | null; next: ChatWriteMemory } {
  const next = memoryFromStore(store);
  if (prev && sameMemory(prev, next)) return { patch: null, next };
  const deletedIds = prev
    ? [...prev.sessions.keys()].filter((id) => !next.sessions.has(id))
    : [];
  const sessions: ChatWritePatch["sessions"] = [];
  for (const session of store.sessions) {
    const cursor = next.sessions.get(session.id);
    const old = prev?.sessions.get(session.id);
    if (!cursor) continue;
    if (old && old.header === cursor.header && sameStamps(old.stamps, cursor.stamps)) continue;
    const fromSeq = old ? commonPrefix(old.stamps, cursor.stamps) : 0;
    const { messages, ...header } = session;
    sessions.push({
      ...header,
      fromSeq,
      messages: (messages ?? []).slice(fromSeq),
    });
  }
  return {
    patch: { v: 2, activeId: store.activeId, order: next.order, deletedIds, sessions },
    next,
  };
}

function sessionCursor(session: ChatSession): SessionCursor {
  const { messages, ...header } = session;
  return {
    header: JSON.stringify(header),
    stamps: (messages ?? []).map(stampMessage),
  };
}

type StampCache = {
  content: string;
  step: string;
  detail: string;
  at: number;
  role: string;
  log: boolean;
  stamp: string;
};

const stampCache = new WeakMap<ChatMessage, StampCache>();

function stampMessage(message: ChatMessage): string {
  const step = message.step?.output ?? "";
  const detail = message.permission?.detail ?? "";
  const log = Boolean(message.log);
  const hit = stampCache.get(message);
  if (
    hit &&
    hit.content === message.content &&
    hit.step === step &&
    hit.detail === detail &&
    hit.at === message.at &&
    hit.role === message.role &&
    hit.log === log
  ) {
    return hit.stamp;
  }
  const stamp = [
    message.id,
    message.at,
    message.role,
    log ? 1 : 0,
    textHash(message.content),
    textHash(step),
    textHash(detail),
  ].join("\t");
  stampCache.set(message, {
    content: message.content,
    step,
    detail,
    at: message.at,
    role: message.role,
    log,
    stamp,
  });
  return stamp;
}

function mixStamp(hash: number, text: string): number {
  for (let i = 0; i < text.length; i += 1) hash = Math.imul(hash, 16777619) ^ text.charCodeAt(i);
  return hash;
}

/**
 * Fingerprint of everything except which chat is open.
 * Switching chats keeps the same stamp, so the save can skip rewriting the transcript.
 */
export function chatBodyStamp(store: SessionStore): string {
  let hash = 2166136261;
  store.sessions.forEach((session, index) => {
    hash = mixStamp(hash, String(index));
    const cursor = sessionCursor(session);
    hash = mixStamp(hash, cursor.header);
    for (const stamp of cursor.stamps) hash = mixStamp(hash, stamp);
  });
  return (hash >>> 0).toString(16);
}

function textHash(text: string): number {
  let hash = 5381;
  for (let i = 0; i < text.length; i += 1) hash = ((hash << 5) + hash + text.charCodeAt(i)) | 0;
  return hash;
}

function commonPrefix(left: string[], right: string[]): number {
  const limit = Math.min(left.length, right.length);
  let index = 0;
  while (index < limit && left[index] === right[index]) index += 1;
  return index;
}

function sameStamps(left: string[], right: string[]): boolean {
  if (left.length !== right.length) return false;
  for (let i = 0; i < left.length; i += 1) if (left[i] !== right[i]) return false;
  return true;
}

function sameMemory(left: ChatWriteMemory, right: ChatWriteMemory): boolean {
  if (left.activeId !== right.activeId || left.order.length !== right.order.length) return false;
  for (let i = 0; i < left.order.length; i += 1) if (left.order[i] !== right.order[i]) return false;
  if (left.sessions.size !== right.sessions.size) return false;
  for (const [id, cursor] of left.sessions) {
    const other = right.sessions.get(id);
    if (!other || other.header !== cursor.header || !sameStamps(other.stamps, cursor.stamps)) return false;
  }
  return true;
}
