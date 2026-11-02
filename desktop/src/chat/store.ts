import type { AskRuntime } from "../ask-resources";
import { readAutoApproveDefault } from "./confirm-pref";
import { normalizePlan } from "../plan";
import type { ChatMessage, ChatSession, SessionStore } from "./types";
import { chatBodyStamp } from "./store-patch";
import {
  CHAT_BACKUP_MAX_BYTES,
  CHAT_STORE_MAX_BYTES,
  LEGACY_STORAGE_KEY,
  MAX_MESSAGES_PER_SESSION,
  MAX_SESSIONS,
  OPEN_CHAT_KEY,
  STORAGE_BACKUP_KEY,
  STORAGE_KEY,
  STORAGE_PARTIAL_KEY,
  storageBytes,
} from "./types";

export function uid(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

const COMPACT_SUMMARY_PREFIXES = ["此前对话摘要（共 ", "Earlier conversation summary ("];

/** The old compact action stored a transcript of dropped turns. That text is not a message. */
export function isContextCompactSummary(message: ChatMessage): boolean {
  if (message.role !== "meta" || message.log) return false;
  const text = message.content.trimStart();
  return COMPACT_SUMMARY_PREFIXES.some((prefix) => text.startsWith(prefix));
}

/** Drop saved compact summaries. Returns whether any session changed. */
export function stripContextCompactSummaries(store: SessionStore): boolean {
  let changed = false;
  for (const session of store.sessions) {
    const messages = session.messages.filter((message) => !isContextCompactSummary(message));
    if (messages.length === session.messages.length) continue;
    session.messages = messages;
    changed = true;
  }
  return changed;
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

/**
 * The browser copy of one chat. It is a contiguous tail of the log: the rows
 * before it stay safe when the copy is written back to the database, and a
 * tail cut is the only cut that keeps that property.
 *
 * When the tail holds no line of the conversation it reaches back for one, but
 * no further than one tail's length. The tail of a long turn is all tool
 * output, and a copy without the words in it reads like the chat is empty.
 */
export function trimSessionMessages(
  messages: readonly ChatMessage[],
  maxRows = MAX_MESSAGES_PER_SESSION,
): ChatMessage[] {
  if (messages.length <= maxRows) return [...messages];
  const start = messages.length - maxRows;
  let first = -1;
  for (let index = start; index < messages.length; index += 1) {
    if (isConversationMessage(messages[index])) {
      first = index;
      break;
    }
  }
  if (first === -1) {
    // Nothing was said inside the tail. Reach back for the last line of the
    // conversation, but never further than one tail's length.
    for (let index = start - 1; index >= Math.max(0, start - maxRows); index -= 1) {
      if (isConversationMessage(messages[index])) {
        first = index;
        break;
      }
    }
  }
  if (first === -1) return messages.slice(start);
  return messages.slice(first);
}

function isConversationMessage(message: ChatMessage | undefined): boolean {
  return message?.role === "user" || message?.role === "assistant";
}

export function trimStoreForSize(data: SessionStore): SessionStore {
  const sessions = data.sessions.slice(0, MAX_SESSIONS).map((session) => ({
    ...session,
    messages: trimSessionMessages(session.messages ?? []),
  }));
  const activeId = sessions.some((s) => s.id === data.activeId)
    ? data.activeId
    : (sessions[0]?.id ?? data.activeId);
  return { activeId, sessions };
}

export function backupStoreRaw(raw: string): void {
  const trimmed = raw.trim();
  if (!trimmed) return;
  const shrunk = shrinkRaw(trimmed, CHAT_BACKUP_MAX_BYTES);
  if (!shrunk) return;
  try {
    localStorage.setItem(STORAGE_BACKUP_KEY, shrunk);
  } catch {
    /* quota — keep trying on next save */
  }
}

/** Trim a raw store until it fits a byte budget. Returns null when it cannot be read. */
function shrinkRaw(raw: string, maxBytes: number): string | null {
  const parsed = parseStoreRaw(raw);
  if (!parsed) return null;
  return fitStore(parsed, maxBytes);
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
    const open = localStorage.getItem(OPEN_CHAT_KEY)?.trim();
    const activeId =
      open && parsed.sessions.some((session) => session.id === open) ? open : parsed.activeId;
    const active = parsed.sessions.find((session) => session.id === activeId);
    return typeof active?.runtime === "string" && active.runtime.trim()
      ? active.runtime.trim()
      : null;
  } catch {
    return null;
  }
}

export function loadStoreFromLocalKeys(): SessionStore | null {
  const keys = [STORAGE_KEY, STORAGE_BACKUP_KEY, LEGACY_STORAGE_KEY];
  readPartialCopies();
  for (const key of keys) {
    const raw = localStorage.getItem(key);
    if (!raw) continue;
    backupStoreRaw(raw);
    const loaded = parseStoreRaw(raw);
    if (!loaded) continue;
    // The stored blob can be bigger than the browser accepts for a rewrite.
    // Trim it down to something that fits. The old loop kept trimming until
    // the blob was small enough, but a store that is already at the per
    // session cap could not get any smaller — that loop never ended and the
    // page never finished booting.
    const fitted = parseStoreRaw(fitStore(loaded, CHAT_STORE_MAX_BYTES)) ?? loaded;
    if (fitted !== loaded) {
      // Heal the mirror: an oversized copy fills the storage quota and keeps
      // every later write failing.
      writeLocal(STORAGE_KEY, JSON.stringify(fitted));
    }
    if (key !== STORAGE_KEY) {
      console.warn(`Ask: restored chat store from ${key}`);
    }
    return applyOpenChatId(fitted);
  }
  return null;
}

/** The open chat lives in a tiny key so a switch does not depend on the transcript blob. */
export function applyOpenChatId(store: SessionStore): SessionStore {
  let open = "";
  try {
    open = localStorage.getItem(OPEN_CHAT_KEY)?.trim() ?? "";
  } catch {
    return store;
  }
  if (!open || open === store.activeId) return store;
  if (!store.sessions.some((session) => session.id === open)) return store;
  return { ...store, activeId: open };
}

function rememberOpenChat(id: string): void {
  try {
    localStorage.setItem(OPEN_CHAT_KEY, id);
  } catch {
    // The chat is already open in memory for this visit.
  }
}

/** Drop the "transcript unchanged" memory after a restore replaces the blob. */
export function forgetChatMirror(): void {
  lastBodyStamp = "";
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
    const stripped = stripContextCompactSummaries(cleaned);
    if (cleaned !== loaded || stripped) persistStore(cleaned);
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

let lastBodyStamp = "";

/** How much of the store survives, from the most to the least. */
type TrimLevel = { sessions: number; rows: number };

const TRIM_LEVELS: TrimLevel[] = [
  { sessions: MAX_SESSIONS, rows: MAX_MESSAGES_PER_SESSION },
  { sessions: MAX_SESSIONS, rows: 48 },
  { sessions: MAX_SESSIONS, rows: 24 },
  { sessions: 12, rows: MAX_MESSAGES_PER_SESSION },
  { sessions: 4, rows: 48 },
  { sessions: 1, rows: 24 },
];

function storeAtLevel(store: SessionStore, level: TrimLevel): SessionStore {
  const sessions = store.sessions.slice(0, level.sessions).map((session) => ({
    ...session,
    messages: trimSessionMessages(session.messages ?? [], level.rows),
  }));
  const activeId = sessions.some((s) => s.id === store.activeId)
    ? store.activeId
    : (sessions[0]?.id ?? store.activeId);
  return finalizeSessionStore({ activeId, sessions });
}

/** One way the store can be written, with the rows each chat kept. */
type StoreCopy = { json: string; rows: Map<string, number> };

/**
 * Every payload the store can be written as, largest first. The rows of a chat
 * are given up before the chats themselves, so the list of chats stays whole.
 * Every copy fits the budget it was built for — a copy that is too big does not
 * count as a copy, it just fills the storage the next write needs.
 */
function storeCandidates(store: SessionStore, maxBytes: number): StoreCopy[] {
  const out: StoreCopy[] = [];
  for (const level of TRIM_LEVELS) {
    if (!store.sessions.length) break;
    const trimmed = storeAtLevel(store, level);
    const json = JSON.stringify(trimmed);
    if (storageBytes(json) > maxBytes) continue;
    out.push({ json, rows: rowsBySession(trimmed) });
  }
  if (!out.length && store.sessions.length) {
    // Nothing fit. Keep the open chat alone and give up its rows until it does.
    const active = store.sessions.find((session) => session.id === store.activeId) ?? store.sessions[0]!;
    for (const rows of [8, 4, 2, 1]) {
      const trimmed = finalizeSessionStore({
        activeId: active.id,
        sessions: [{ ...active, messages: trimSessionMessages(active.messages ?? [], rows) }],
      });
      const json = JSON.stringify(trimmed);
      if (storageBytes(json) > maxBytes) continue;
      out.push({ json, rows: rowsBySession(trimmed) });
      break;
    }
  }
  if (!out.length) {
    // One row with its text cut down. This is small enough for any browser.
    const minimal = minimalStoreCopy(store);
    out.push({ json: JSON.stringify(minimal), rows: rowsBySession(minimal) });
  }
  return out;
}

/** The smallest copy worth writing: the open chat, its last row, short text. */
function minimalStoreCopy(store: SessionStore): SessionStore {
  const active = store.sessions.find((session) => session.id === store.activeId) ?? store.sessions[0];
  if (!active) return { activeId: store.activeId, sessions: [] };
  const messages = active.messages ?? [];
  const last = messages[messages.length - 1];
  const message = last
    ? {
        ...last,
        content: capCopyText(last.content ?? "", 2_000),
        step: last.step ? { ...last.step, output: capCopyText(last.step.output ?? "", 2_000) } : last.step,
        permission: last.permission
          ? { ...last.permission, detail: capCopyText(last.permission.detail ?? "", 2_000) }
          : last.permission,
      }
    : null;
  return finalizeSessionStore({
    activeId: active.id,
    sessions: [{ ...active, messages: message ? [message] : [] }],
  });
}

/** Text kept in a copy that has to fit a small budget. */
function capCopyText(text: string, max: number): string {
  return text.length <= max ? text : `${text.slice(0, max)}…`;
}

function rowsBySession(store: SessionStore): Map<string, number> {
  const rows = new Map<string, number>();
  for (const session of store.sessions) rows.set(session.id, (session.messages ?? []).length);
  return rows;
}

/** The largest copy of the store that fits a byte budget. */
function fitStore(store: SessionStore, maxBytes: number): string {
  return storeCandidates(store, maxBytes)[0]?.json ?? JSON.stringify(store);
}

/**
 * Write to the browser copy. A full storage quota must not throw: the caller
 * keeps the transcript in memory and the database has the real copy, so a
 * failed mirror is a warning, never a crash.
 */
function writeLocal(key: string, value: string): boolean {
  try {
    localStorage.setItem(key, value);
    return true;
  } catch (error) {
    console.warn("Ask: local chat mirror failed", error);
    return false;
  }
}

/** Chats the browser copy holds only in part. */
let partialCopies = new Set<string>();

/** The chats whose browser copy holds only part of the log. */
export function partialCopySessionIds(): ReadonlySet<string> {
  return partialCopies;
}

function rememberPartialCopies(store: SessionStore, rows: Map<string, number>): void {
  const partial = new Set<string>();
  for (const session of store.sessions) {
    const kept = rows.get(session.id);
    if (kept != null && kept < (session.messages ?? []).length) partial.add(session.id);
  }
  partialCopies = partial;
  try {
    if (partial.size) {
      localStorage.setItem(STORAGE_PARTIAL_KEY, JSON.stringify([...partial]));
    } else {
      localStorage.removeItem(STORAGE_PARTIAL_KEY);
    }
  } catch {
    /* the marker is a hint; the merge still has its own rules */
  }
}

function readPartialCopies(): void {
  try {
    const raw = localStorage.getItem(STORAGE_PARTIAL_KEY);
    const parsed = raw ? (JSON.parse(raw) as unknown) : null;
    partialCopies = new Set(Array.isArray(parsed) ? parsed.filter((id) => typeof id === "string") : []);
  } catch {
    partialCopies = new Set();
  }
}

export function persistStore(store: SessionStore): SessionStore {
  const stamp = chatBodyStamp(store);
  // Opening another chat does not change the transcript. Remember the id and skip the blob.
  if (lastBodyStamp && stamp === lastBodyStamp) {
    rememberOpenChat(store.activeId);
    return store;
  }
  const previous = localStorage.getItem(STORAGE_KEY);
  if (previous) backupStoreRaw(previous);
  const projected = chatStoreForDisk(store);
  for (const copy of storeCandidates(projected, CHAT_STORE_MAX_BYTES)) {
    if (!writeLocal(STORAGE_KEY, copy.json)) continue;
    lastBodyStamp = stamp;
    rememberOpenChat(store.activeId);
    rememberPartialCopies(projected, copy.rows);
    return store;
  }
  // Nothing fit. The mirror keeps what it had; the database still has the log.
  rememberOpenChat(store.activeId);
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
  rememberOpenChat(loaded.activeId);
  forgetChatMirror();
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
 *
 * `partialIds` names the chats whose browser copy holds only part of the log.
 * Those chats keep both sides: the copy was cut to fit the browser, and it
 * must not stand in for the log on disk.
 */
export function mergeChatStores(
  disk: SessionStore | null,
  local: SessionStore | null,
  droppedIds?: ReadonlySet<string>,
  partialIds?: ReadonlySet<string>,
): SessionStore | null {
  if (!disk && !local) return null;
  if (!disk) return local;
  if (!local) return disk;
  if (isFreshEmptyWindow(local, disk)) return disk;
  const dropped = droppedIds ?? droppedBeforeDurable;
  const partial = partialIds ?? partialCopies;
  const byId = new Map<string, ChatSession>();
  const absorb = (session: ChatSession) => {
    if (dropped.has(session.id) && !local.sessions.some((item) => item.id === session.id)) return;
    const prev = byId.get(session.id);
    byId.set(
      session.id,
      prev ? mergeChatSession(prev, session, partial.has(session.id)) : session,
    );
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

function mergeChatSession(left: ChatSession, right: ChatSession, partial = false): ChatSession {
  const newer = right.updatedAt >= left.updatedAt ? right : left;
  const older = newer === right ? left : right;
  return {
    ...older,
    ...newer,
    messages: mergeSessionMessages(
      older.messages ?? [],
      newer.messages ?? [],
      newer.updatedAt === older.updatedAt,
      partial,
    ),
  };
}

/**
 * The browser copy is capped before it is written to localStorage, so it can
 * be the tail of a longer on-disk log. A capped copy must not replace the
 * full history, or every reload cuts the beginning off the transcript. A
 * newer save that dropped rows on purpose still wins.
 */
function mergeSessionMessages(
  older: ChatMessage[],
  newer: ChatMessage[],
  sameClock: boolean,
  partial = false,
): ChatMessage[] {
  if (sameClock || partial || isCappedTail(newer, older)) return unionMessages(older, newer);
  return authoritativeMessages(older, newer);
}

/** True when `candidate` is the capped tail of a longer list. */
function isCappedTail(candidate: ChatMessage[], full: ChatMessage[]): boolean {
  if (candidate.length === 0 || candidate.length >= full.length) return false;
  // A log that fits in one copy was not cut. A shorter copy of it means the
  // rows were dropped on purpose, and that must not come back.
  if (full.length <= MAX_MESSAGES_PER_SESSION) return false;
  const offset = full.length - candidate.length;
  for (let index = 0; index < candidate.length; index += 1) {
    if (full[offset + index]?.id !== candidate[index]?.id) return false;
  }
  return true;
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
