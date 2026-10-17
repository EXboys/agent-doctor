/**
 * Run from desktop/: node --import tsx tests/chat-store-patch.test.ts
 */
import type { ChatMessage, ChatSession, SessionStore } from "../src/chat/types";
import { diffChatWrite, memoryFromStore } from "../src/chat/store-patch";

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

const session = (id: string, updatedAt: number, messages: ChatMessage[], title = id): ChatSession => ({
  id,
  title,
  runtime: "claude-code",
  createdAt: 1,
  updatedAt,
  messages,
});

const store = (activeId: string, sessions: ChatSession[]): SessionStore => ({ activeId, sessions });

const base = store("a", [
  session("a", 1, [message("m1", "user", "你好", 1)]),
  session("b", 2, [message("b1", "user", "别动", 1)], "旁边"),
]);
const remembered = memoryFromStore(base);

const asked = store("a", [
  session("a", 3, [message("m1", "user", "你好", 1), message("m2", "assistant", "在", 2)]),
  session("b", 2, [message("b1", "user", "别动", 1)], "旁边"),
]);
const patch = diffChatWrite(asked, remembered).patch;
assert(patch?.sessions.length === 1, "only the changed chat is in the patch");
assert(patch?.sessions[0].id === "a", "the changed chat is the one just used");
assert(patch?.sessions[0].fromSeq === 1, "the new line starts after the saved prefix");
assert(patch?.sessions[0].messages.length === 1, "only the new line is sent");
assert(patch?.sessions[0].messages[0].content === "在", "the new line is the reply");
assert(!patch?.sessions.some((item) => item.id === "b"), "the other chat is not rewritten");

const again = diffChatWrite(asked, diffChatWrite(asked, remembered).next).patch;
assert(again === null, "an unchanged chat sends nothing");

const growing = store("a", [
  session("a", 4, [message("m1", "user", "你好", 1), message("m2", "assistant", "在的", 2)]),
  session("b", 2, [message("b1", "user", "别动", 1)], "旁边"),
]);
const tail = diffChatWrite(growing, memoryFromStore(asked)).patch;
assert(tail?.sessions[0].fromSeq === 1 && tail.sessions[0].messages[0].content === "在的", "a growing reply replaces only that row");

const removed = store("b", [session("b", 2, [message("b1", "user", "别动", 1)], "旁边")]);
const deletion = diffChatWrite(removed, remembered).patch;
assert(deletion?.deletedIds.includes("a") && deletion.sessions.length === 0, "a removed chat is a delete, not a rewrite");
