/**
 * Run from desktop/: node --import tsx tests/chat-store-merge.test.ts
 */
import type { ChatMessage, ChatSession, SessionStore } from "../src/chat/types";

const storage = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  writable: true,
  value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
    removeItem: (key: string) => storage.delete(key),
  },
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  writable: true,
  value: { language: "zh" },
});
Object.defineProperty(globalThis, "document", {
  configurable: true,
  writable: true,
  value: { documentElement: { lang: "zh" } },
});

const { mergeChatStores, stripContextCompactSummaries } = await import("../src/chat/store");

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
  console.log(`ok - ${msg}`);
}

const message = (id: string, role: ChatMessage["role"], content: string, at: number): ChatMessage => ({
  id,
  role,
  content,
  at,
});

const session = (id: string, updatedAt: number, messages: ChatMessage[], title = ""): ChatSession => ({
  id,
  title,
  runtime: "claude-code",
  createdAt: 1,
  updatedAt,
  messages,
});

const store = (activeId: string, sessions: ChatSession[]): SessionStore => ({ activeId, sessions });

const disk = store("a", [
  session("a", 10, [
    message("u1", "user", "旧的一句", 1),
    message("t1", "tool", "x".repeat(100), 2),
    message("r1", "assistant", "旧回复", 3),
  ]),
]);
const local = store("a", [
  session("a", 10, [
    message("u1", "user", "旧的一句", 1),
    message("t1", "tool", "x".repeat(20), 2),
    message("r1", "assistant", "旧回复", 3),
    message("u2", "user", "旧客户端新写的", 4),
  ], "本地标题"),
]);

const merged = mergeChatStores(disk, local);
assert(merged?.sessions.length === 1, "one session after union");
const messages = merged?.sessions[0].messages ?? [];
assert(messages.length === 4, "message that only the old client has is kept");
assert(messages[1].content.length === 100, "shorter trimmed tool text does not replace the disk copy");
assert(messages[3].content === "旧客户端新写的", "old client append stays");
assert(merged?.sessions[0].title === "本地标题", "newer header fields survive a tie by taking the local session");

const newerDisk = store("b", [
  session("a", 5, [message("u1", "user", "只在盘上", 1)]),
  session("b", 50, [message("u9", "user", "当前", 9)], "盘上的"),
]);
const olderLocal = store("a", [session("a", 30, [message("u1", "user", "只在盘上", 1), message("u3", "user", "本地续写", 8)])]);
const picked = mergeChatStores(newerDisk, olderLocal);
assert(picked?.activeId === "b", "the newer open chat stays open");
const sessionA = picked?.sessions.find((item) => item.id === "a");
assert(sessionA?.messages.length === 2, "disk-only and local-only messages combine");
assert(sessionA?.messages[1].content === "本地续写", "local continuation is ordered after the earlier line");

const dropped = mergeChatStores(newerDisk, store("b", [session("b", 50, [])]), new Set(["a"]));
assert(!dropped?.sessions.some((item) => item.id === "a"), "a session deleted before disk loaded is not restored");

const compacted = mergeChatStores(
  store("a", [session("a", 10, [message("u1", "user", "旧的一句", 1), message("r1", "assistant", "旧回复", 3)])]),
  store("a", [session("a", 20, [message("r1", "assistant", "只留这句", 3)])]),
);
assert(compacted?.sessions[0].messages.length === 1, "a newer save does not bring back messages it dropped");
assert(compacted?.sessions[0].messages[0].content === "只留这句", "the newer text replaces the older one");

const capped = "x".repeat(16_000) + "\n…";
const full = "x".repeat(16_000) + "yyyy";
const cappedMerge = mergeChatStores(
  store("a", [session("a", 10, [message("t1", "tool", full, 2)])]),
  store("a", [session("a", 10, [message("t1", "tool", capped, 2)])]),
);
assert(cappedMerge?.sessions[0].messages[0].content === capped, "a capped tool copy stays capped");

const v1 = store("legacy", [
  session("legacy", 3, [message("m", "user", "v1 里的话", 3)]),
]);
const fromLegacy = mergeChatStores(null, v1);
assert(fromLegacy?.sessions[0].messages[0].content === "v1 里的话", "a browser copy with no disk row is kept");

const placeholder = store("new", [session("new", Date.now(), [])]);
const restored = mergeChatStores(disk, placeholder);
assert(restored?.sessions.length === 1 && restored.sessions[0].id === "a", "a fresh empty window does not hide the saved chats");

const withSummary = store("a", [
  session("a", 10, [
    message("s", "meta", "此前对话摘要（共 89 条）：\n- 用户: 你好", 1),
    message("u", "user", "你好", 2),
    message("a1", "assistant", "你好，有什么需要操作的尽管说。", 3),
    message("n", "meta", "这一步没完成。", 4),
  ]),
]);
assert(stripContextCompactSummaries(withSummary), "a saved compact summary is removed");
assert(withSummary.sessions[0].messages.length === 3, "the summary line is the only one removed");
assert(withSummary.sessions[0].messages[0].content === "你好", "the kept turns stay in their original order");
assert(withSummary.sessions[0].messages[2].role === "meta", "a normal notice stays");
assert(!stripContextCompactSummaries(withSummary), "a second pass finds nothing to remove");

const englishSummary = store("a", [
  session("a", 10, [message("s", "meta", "Earlier conversation summary (4 messages):\n- User: hi", 1)]),
]);
assert(stripContextCompactSummaries(englishSummary), "an English compact summary is removed");
assert(englishSummary.sessions[0].messages.length === 0, "the English summary does not stay as a message");
