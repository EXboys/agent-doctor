/**
 * 把电脑本地会话增量上传到官网，手机登录同一账号才能看到这些对话。
 * 只传没绑定渠道的纯本地会话（和手机列表快照同一口径）；phone: 会话有
 * 自己的存档链路（reply 帧），飞书会话的内容本来就走飞书。
 * 内容上传是隐私敏感操作：设置里的「同步会话到手机」开关（默认关）管着这里，
 * 关掉时 Rust 侧还会请求服务端清空已上传的副本。
 */
import type { ChatMessage, ChatSession } from "./types";
import { teamupsChannelTranscript } from "../ipc";

export function deskChatKey(sessionId: string): string {
  return `desk:${sessionId}`;
}

type UploadRow = { key: string; kind: "me"; text: string } | { key: string; kind: "pc"; jobId: string; text: string };

const LIST_KEY = "agent-doctor.desk-upload.v1";
const MAX_SESSIONS = 30;
const ROWS_KEEP = 200;
const CHUNK = 50;
const ME_TEXT_MAX = 4000;
const PC_TEXT_MAX = 8000;
const BACKOFF_MS = 5 * 60_000;

let enabled = false;
let connected = false;
let storeTimer = 0;
let flushTimer = 0;
let flushing = false;
/** sessionId -> 已上传的行 key。持久化，重启后不重发整段历史。 */
const uploaded = new Map<string, Set<string>>();
/** sessionId -> 上次看到的会话指纹，没变就不重新映射行。 */
const fingerprints = new Map<string, string>();
/** sessionId -> 上次发送失败后的退期。 */
const retryAt = new Map<string, number>();

function loadUploaded(): void {
  try {
    const raw = window.localStorage.getItem(LIST_KEY);
    if (!raw) return;
    const parsed = JSON.parse(raw) as Record<string, string[]>;
    for (const [id, keys] of Object.entries(parsed)) {
      if (Array.isArray(keys) && keys.length) uploaded.set(id, new Set(keys.slice(-400)));
    }
  } catch {
    /* 损了的记录当没有：顶多整段重传一遍，服务端按 key 幂等覆盖。 */
  }
}

function saveUploaded(): void {
  const value: Record<string, string[]> = {};
  for (const [id, keys] of uploaded) value[id] = [...keys].slice(-400);
  try {
    window.localStorage.setItem(LIST_KEY, JSON.stringify(value));
  } catch {
    /* 空间满了就下轮再存，不影响上传。 */
  }
}

function scheduleSave(): void {
  window.clearTimeout(storeTimer);
  storeTimer = window.setTimeout(saveUploaded, 1500);
}

export function mapTranscriptRows(messages: ChatMessage[]): UploadRow[] {
  const rows: UploadRow[] = [];
  for (const message of messages) {
    const content = typeof message.content === "string" ? message.content.trim() : "";
    if (!content) continue;
    if (message.role === "user") {
      rows.push({ key: `m:${message.id}`, kind: "me", text: content.slice(0, ME_TEXT_MAX) });
    } else if (message.role === "assistant") {
      rows.push({
        key: `p:${message.id}`,
        kind: "pc",
        jobId: message.id,
        text: content.slice(0, PC_TEXT_MAX),
      });
    }
  }
  return rows.slice(-ROWS_KEEP);
}

function fingerprint(session: ChatSession): string {
  const messages = session.messages || [];
  const last = messages[messages.length - 1];
  return `${session.updatedAt || 0}|${messages.length}|${last ? `${last.id}:${(last.content || "").length}` : ""}`;
}

async function flushSession(session: ChatSession): Promise<boolean> {
  const rows = mapTranscriptRows(session.messages || []);
  if (rows.length === 0) return true;
  const seen = uploaded.get(session.id) ?? new Set<string>();
  const pending = rows.filter((row) => !seen.has(row.key));
  if (pending.length === 0) return true;
  for (let at = 0; at < pending.length; at += CHUNK) {
    try {
      await teamupsChannelTranscript(
        deskChatKey(session.id),
        session.title || "",
        session.updatedAt || Date.now(),
        pending.slice(at, at + CHUNK),
      );
    } catch {
      // 线没连上或发送失败：这一段留到下个触发点再试，先退避。
      retryAt.set(session.id, Date.now() + BACKOFF_MS);
      return false;
    }
    for (const row of pending.slice(at, at + CHUNK)) seen.add(row.key);
    uploaded.set(session.id, seen);
    scheduleSave();
  }
  retryAt.delete(session.id);
  return true;
}

async function flush(sessions: ChatSession[]): Promise<void> {
  if (!enabled || flushing) return;
  // 只传没绑定渠道的纯本地会话，和手机列表的快照口径一致：
  // phone: 会话已有自己的存档链路，飞书会话的内容本来就走飞书。
  const queue = sessions
    .filter((session) => !(session.channelKey ?? "").trim())
    .sort((a, b) => (b.updatedAt || 0) - (a.updatedAt || 0))
    .slice(0, MAX_SESSIONS)
    .filter((session) => (retryAt.get(session.id) || 0) <= Date.now());
  if (queue.length === 0) return;
  flushing = true;
  try {
    for (const session of queue) {
      if (!(await flushSession(session))) break;
    }
  } finally {
    flushing = false;
  }
}

/** 手机快照的发布点顺带调这里：已上传的行直接跳过，日常只发增量。 */
export function syncDeskTranscripts(sessions: ChatSession[] | undefined): void {
  if (!enabled || !sessions?.length) return;
  let changed = false;
  for (const session of sessions) {
    if ((session.channelKey ?? "").trim()) continue;
    const mark = fingerprint(session);
    if (fingerprints.get(session.id) === mark) continue;
    fingerprints.set(session.id, mark);
    changed = true;
  }
  if (!changed) return;
  window.clearTimeout(flushTimer);
  flushTimer = window.setTimeout(() => void flush(sessions), 600);
}

export function setDeskTranscriptEnabled(on: boolean): void {
  if (enabled === on) return;
  enabled = on;
  if (!on) {
    window.clearTimeout(flushTimer);
    return;
  }
  if (uploaded.size === 0) loadUploaded();
}

export function setDeskTranscriptConnected(on: boolean): void {
  if (connected === on) return;
  connected = on;
  if (on) {
    // 重连后服务端可能有缺口（掉线期间的改动），清指纹让下轮全量对一遍。
    fingerprints.clear();
  }
}
