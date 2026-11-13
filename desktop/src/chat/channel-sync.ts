import { perfMark } from "./perf-probe";
import { placeChannelSession } from "./channel-route";
import { MAX_SESSIONS, type ChatMessage, type ChatSession, type SessionStore } from "./types";
import { isChatRunning } from "./live-runs";

/**
 * The site archives every phone conversation and replays the ones that moved
 * after hello. This file pours a replayed conversation back into the desktop
 * chat it names, so a reinstalled computer finds its phone sessions again.
 *
 * Merge rules mirror the phone's conversations.ts: rows join by identity (a
 * row key is one message id), sentences join by text, so clock drift never
 * blocks a catch-up and rows this chat already holds are never doubled.
 */

const CHANNEL_PREFIX = "phone:";
/** The site keeps at most this many rows per conversation. */
const MAX_SYNC_ROWS = 100;
/** A turn that is still going leaves this marker in the archive, never on disk. */
const WORKING_TEXT = "电脑正在做…";
const TEXT_CAP_ME = 4_000;
const TEXT_CAP_PC = 8_000;
/** Replayed rows carry no timestamps; restored turns sit one second apart. */
const ROW_SPACING_MS = 1_000;

type SyncedMeRow = { key: string; kind: "me"; text: string };
type SyncedPcRow = { key: string; kind: "pc"; jobId: string; text: string };
type SyncedAskRow = {
  key: string;
  kind: "ask";
  jobId: string;
  requestId: string;
  text: string;
  answered?: string;
};
type SyncedRow = SyncedMeRow | SyncedPcRow | SyncedAskRow;

type SyncedConversation = {
  chat: string;
  title: string;
  updatedAt: number;
  rows: SyncedRow[];
};

export function readSyncedConversation(value: unknown): SyncedConversation | null {
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  const chat = typeof record.chat === "string" ? record.chat.trim() : "";
  const updatedAt = typeof record.updatedAt === "number" ? record.updatedAt : 0;
  if (!chat || !Number.isFinite(updatedAt) || updatedAt <= 0) return null;
  const rows = readRows(record.rows);
  if (rows.length === 0) return null;
  const title = typeof record.title === "string" ? record.title.trim().slice(0, 80) : "";
  return { chat: chat.slice(0, 80), title, updatedAt, rows };
}

function readRows(value: unknown): SyncedRow[] {
  if (!Array.isArray(value)) return [];
  const rows: SyncedRow[] = [];
  for (const item of value.slice(-MAX_SYNC_ROWS)) {
    if (!item || typeof item !== "object") continue;
    const record = item as Record<string, unknown>;
    const key = typeof record.key === "string" ? record.key.trim() : "";
    const text = typeof record.text === "string" ? record.text.trim() : "";
    if (!key || !text) continue;
    if (record.kind === "me") {
      rows.push({ key, kind: "me", text: text.slice(0, TEXT_CAP_ME) });
      continue;
    }
    const jobId = typeof record.jobId === "string" ? record.jobId.trim() : "";
    if (!jobId) continue;
    if (record.kind === "pc") {
      rows.push({ key, kind: "pc", jobId, text: text.slice(0, TEXT_CAP_PC) });
      continue;
    }
    if (record.kind !== "ask") continue;
    const requestId = typeof record.requestId === "string" ? record.requestId.trim() : "";
    if (!requestId) continue;
    const answered =
      typeof record.answered === "string" ? record.answered.trim().slice(0, 80) : "";
    rows.push({
      key,
      kind: "ask",
      jobId,
      requestId,
      text: text.slice(0, TEXT_CAP_ME),
      ...(answered ? { answered } : {}),
    });
  }
  return rows;
}

function messageId(chat: string, key: string): string {
  return `sync:${chat}:${key}`;
}

function rowContent(row: SyncedRow): string {
  return row.kind === "ask" && row.answered ? `${row.text}\n\n已回答：${row.answered}` : row.text;
}

function rowRole(row: SyncedRow): ChatMessage["role"] {
  return row.kind === "me" ? "user" : "assistant";
}

/**
 * `phone:` conversation ids -> the archive stamp this chat already merged.
 * The hello sends this map; an entry left out is a chat the site may replay
 * whole, which the merge tolerates.
 */
export function channelSyncSince(sessions: ChatSession[]): Record<string, number> {
  const since: Record<string, number> = {};
  for (const session of sessions) {
    const key = session.channelKey?.trim() ?? "";
    if (!key.startsWith(CHANNEL_PREFIX)) continue;
    const at = session.channelSyncedAt ?? 0;
    if (!Number.isFinite(at) || at <= 0) continue;
    const id = key.slice(CHANNEL_PREFIX.length).trim();
    if (!id) continue;
    since[id.slice(0, 80)] = at;
  }
  return since;
}

type ChannelSyncDeps = {
  store: () => SessionStore | null;
  createSession: () => ChatSession;
  /** Persist, repaint and re-publish after a chat took rows in. */
  applied: (session: ChatSession) => void;
};

let syncDeps: ChannelSyncDeps | null = null;

export function bindChannelSync(deps: ChannelSyncDeps): void {
  syncDeps = deps;
}

/** One replayed conversation from the site. Rows join the chat they name. */
export function applyChannelSync(payload: unknown): void {
  perfMark("channel-sync");
  const deps = syncDeps;
  if (!deps) return;
  const conversation = readSyncedConversation(payload);
  const store = deps.store();
  if (!conversation || !store) return;
  const key = conversation.chat.startsWith(CHANNEL_PREFIX)
    ? conversation.chat
    : `${CHANNEL_PREFIX}${conversation.chat}`;
  const placed = placeChannelSession(store.sessions, key, deps.createSession);
  const session = placed.session;
  // A live turn owns its transcript: the chat is already rendering its own
  // rows, so archive copies must not pour in mid-run. But the cursor MUST
  // ack what the replay carried - with the cursor parked, the site replayed
  // the whole growing conversation on every hello (O(n²) traffic that froze
  // the window under memory pressure).
  if (isChatRunning(session.id)) {
    session.channelSyncedAt = Math.max(session.channelSyncedAt ?? 0, conversation.updatedAt);
    return;
  }
  const merged = mergeSyncedConversation(session, conversation);
  if (placed.created) {
    if (!merged) return;
    store.sessions = placed.sessions.slice(0, MAX_SESSIONS);
  }
  session.channelSyncedAt = Math.max(session.channelSyncedAt ?? 0, conversation.updatedAt);
  if (!merged) return;
  deps.applied(session);
}

/**
 * Pour the replayed rows into one chat, in place. Returns whether anything
 * changed; the archive cursor only moves when it did.
 */
function mergeSyncedConversation(session: ChatSession, conversation: SyncedConversation): boolean {
  const messages = session.messages ?? [];
  const empty = messages.length === 0;
  const byId = new Map<string, number>();
  messages.forEach((message, index) => byId.set(message.id, index));
  const texts = new Set<string>();
  const longTexts: { role: ChatMessage["role"]; content: string }[] = [];
  for (const message of messages) {
    if (message.role !== "user" && message.role !== "assistant") continue;
    const content = message.content.trim();
    texts.add(`${message.role}\n${content}`);
    if (content.length >= TEXT_CAP_ME) longTexts.push({ role: message.role, content });
  }
  const knownText = (role: ChatMessage["role"], content: string): boolean => {
    if (texts.has(`${role}\n${content}`)) return true;
    // The archive clips long lines; a clipped row of a line held whole is not new.
    return (
      content.length >= TEXT_CAP_ME &&
      longTexts.some((item) => item.role === role && item.content.startsWith(content))
    );
  };
  // Replayed rows have no timestamps. A fresh chat spreads its rows behind the
  // archive stamp; an existing chat appends after the words it already has.
  let base = empty
    ? conversation.updatedAt - conversation.rows.length * ROW_SPACING_MS
    : (messages[messages.length - 1]?.at ?? 0);
  const next = messages.slice();
  let appended = 0;
  let changed = false;
  for (const row of conversation.rows) {
    if (row.kind === "pc" && row.text === WORKING_TEXT) continue;
    const role = rowRole(row);
    const content = rowContent(row);
    const id = messageId(conversation.chat, row.key);
    // A live chat shows the question as a card it answered already; only a
    // restored chat takes the archived question. One it already holds may
    // still come again, carrying the answer.
    if (row.kind === "ask" && !empty && !byId.has(id)) continue;
    const at = byId.get(id);
    if (at !== undefined) {
      const current = next[at];
      if (current.role === role && current.content === content) continue;
      next[at] = { ...current, role, content };
      changed = true;
      continue;
    }
    if (knownText(role, content)) continue;
    byId.set(id, next.length);
    next.push({ id, role, content, at: base + (appended + 1) * ROW_SPACING_MS });
    appended += 1;
    changed = true;
  }
  if (!changed) return false;
  session.messages = next;
  session.channelSyncedAt = conversation.updatedAt;
  if (!session.title.trim() && conversation.title) session.title = conversation.title;
  return true;
}
