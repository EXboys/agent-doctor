/**
 * The site replays archived phone conversations after hello; the desktop chat
 * takes the rows in without doubling what it already holds.
 * Run from desktop/: node --import tsx tests/channel-sync.test.ts
 */

import { applyChannelSync, bindChannelSync, channelSyncSince } from "../src/chat/channel-sync";
import { beginLiveRun, endLiveRun } from "../src/chat/live-runs";
import type { ChatSession, SessionStore } from "../src/chat/types";

function assert(cond: boolean, message: string): void {
  if (!cond) throw new Error(message);
}

function makeSession(id: string): ChatSession {
  const now = Date.now();
  return {
    id,
    title: "",
    runtime: "doctor",
    createdAt: now,
    updatedAt: now,
    messages: [],
    runtimeThreadId: null,
  };
}

function makeStore(): SessionStore {
  const session = makeSession("open");
  return { activeId: "open", sessions: [session] };
}

const store = makeStore();
let applied: string[] = [];
bindChannelSync({
  store: () => store,
  createSession: () => makeSession(`synced-${store.sessions.length}`),
  applied: (session) => {
    applied.push(session.id);
  },
});

const conversation = (rows: unknown[]): {
  chat: string;
  title: string;
  updatedAt: number;
  rows: unknown[];
} => ({
  chat: "conv-1",
  title: "帮我看下项目",
  updatedAt: 1_000_000,
  rows,
});

applyChannelSync(
  conversation([
    { key: "m:1", kind: "me", text: "打开项目" },
    { key: "p:j1", kind: "pc", jobId: "j1", text: "做完了。" },
    { key: "a:r1", kind: "ask", jobId: "j1", requestId: "r1", text: "要继续吗？", questions: [] },
    { key: "p:j2", kind: "pc", jobId: "j2", text: "电脑正在做…" },
  ]),
);

const restored = store.sessions[0];
assert(restored.id === "synced-1", "a replayed conversation opens its own chat");
assert(restored.channelKey === "phone:conv-1", "the chat carries the phone conversation key");
assert(restored.title === "帮我看下项目", "the archive title names the chat");
assert(restored.messages.length === 3, "the working marker never lands, three rows do");
assert(restored.messages[0]?.role === "user" && restored.messages[0]?.content === "打开项目", "a sentence becomes a user line");
assert(restored.messages[1]?.role === "assistant" && restored.messages[1]?.content === "做完了。", "a reply becomes an assistant line");
assert(
  restored.messages[2]?.content === "要继续吗？",
  "an archived question becomes an assistant line",
);
assert(
  restored.messages.every((message, index) => index === 0 || message.at > restored.messages[index - 1]!.at),
  "restored rows sit in order",
);
assert(restored.channelSyncedAt === 1_000_000, "the cursor moves to the archive stamp");
assert(applied.join(",") === "synced-1", "a changed chat is reported back");

const countAfterFirst = restored.messages.length;
applyChannelSync(conversation([{ key: "m:1", kind: "me", text: "打开项目" }]));
assert(restored.messages.length === countAfterFirst, "the same rows never double");
assert(restored.channelSyncedAt === 1_000_000, "a no-op keeps the cursor");

applyChannelSync(
  conversation([
    {
      key: "a:r1",
      kind: "ask",
      jobId: "j1",
      requestId: "r1",
      text: "要继续吗？",
      answered: "1",
      questions: [],
    },
  ]),
);
assert(
  restored.messages[2]?.content === "要继续吗？\n\n已回答：1",
  "a replayed question carries the answer it got",
);

// A live chat already holds the words it sent; the replay must not double them.
store.sessions[0].messages.push(
  { id: "live-u", role: "user", content: "接着做", at: 2_000_000 },
  { id: "live-a", role: "assistant", content: "做完了。", at: 2_000_001 },
);
const heldCount = store.sessions[0].messages.length;
applyChannelSync(
  conversation([
    { key: "m:9", kind: "me", text: "接着做" },
    { key: "p:j9", kind: "pc", jobId: "j9", text: "做完了。" },
  ]),
);
assert(
  store.sessions[0].messages.every((message) => message.id !== "sync:conv-1:p:j9"),
  "a reply the chat already holds does not land again",
);
assert(
  store.sessions[0].messages.every((message) => message.id !== "sync:conv-1:m:9"),
  "a sentence the chat already holds does not land again",
);
assert(store.sessions[0].messages.length === heldCount, "a replay of known words adds nothing");

// The archive clips long lines. A clipped copy of a line held whole is not new.
const long = "长".repeat(9_000);
store.sessions[0].messages.push({ id: "live-long", role: "assistant", content: long, at: 2_000_002 });
applyChannelSync(
  conversation([{ key: "p:jl", kind: "pc", jobId: "jl", text: long.slice(0, 8_000) }]),
);
assert(
  store.sessions[0].messages.every((message) => message.id !== "sync:conv-1:p:jl"),
  "a clipped row of a held line does not land",
);

// A running turn owns its messages; the cursor stays so the site replays later.
const liveStore = makeStore();
const liveSession = liveStore.sessions[0];
liveSession.channelKey = "phone:conv-live";
beginLiveRun(liveSession.id);
let liveApplied = 0;
bindChannelSync({
  store: () => liveStore,
  createSession: () => makeSession("other"),
  applied: () => {
    liveApplied += 1;
  },
});
applyChannelSync({
  chat: "conv-live",
  title: "还在吗",
  updatedAt: 1_000_000,
  rows: [{ key: "m:2", kind: "me", text: "还在吗" }],
});
endLiveRun(liveSession.id);
// A running chat acks the replay stamp without taking the archived rows in
// (its own live rows own the transcript) - the ack is what stops the site
// from replaying the whole conversation on every hello.
assert(liveApplied === 0, "a running chat takes no archived rows in");
assert(
  liveSession.channelSyncedAt === 1_000_000,
  "the acked stamp stops the site from replaying the same rows",
);

const since = channelSyncSince(store.sessions);
assert(since["conv-1"] === 1_000_000, "the cursor map names the phone conversation");
const plain = [{ ...makeSession("x"), channelKey: "phone:plain" }];
assert(Object.keys(channelSyncSince(plain)).length === 0, "a chat that never synced asks for everything");

console.log("channel-sync: ok");
