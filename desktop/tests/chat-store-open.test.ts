/**
 * Run from desktop/: node --import tsx tests/chat-store-open.test.ts
 *
 * Switching the open chat must not rewrite the transcript blob.
 */
const storage = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  writable: true,
  value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => {
      storage.set(key, value);
    },
    removeItem: (key: string) => {
      storage.delete(key);
    },
  },
});

function assert(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
  console.log(`ok - ${msg}`);
}

const { OPEN_CHAT_KEY, STORAGE_KEY } = await import("../src/chat/types");
const { applyOpenChatId, parseStoreRaw, persistStore } = await import("../src/chat/store");

const session = {
  id: "a",
  title: "A",
  runtime: "claude-code" as const,
  createdAt: 1,
  updatedAt: 1,
  messages: [{ id: "m", role: "user" as const, content: "hello ".repeat(400), at: 1 }],
};
const other = {
  ...session,
  id: "b",
  title: "B",
  messages: [{ id: "m2", role: "user" as const, content: "there", at: 2 }],
};

persistStore({ activeId: "a", sessions: [session, other] });
const blob = localStorage.getItem(STORAGE_KEY);
assert(Boolean(blob && blob.length > 100), "the transcript is written once");
assert(localStorage.getItem(OPEN_CHAT_KEY) === "a", "the open chat is recorded");

persistStore({ activeId: "b", sessions: [session, other] });
assert(localStorage.getItem(STORAGE_KEY) === blob, "switching chats does not rewrite the transcript");
assert(localStorage.getItem(OPEN_CHAT_KEY) === "b", "the open chat key follows the switch");

const loaded = parseStoreRaw(blob ?? "");
assert(loaded != null && applyOpenChatId(loaded).activeId === "b", "loading honors the open chat key");

session.messages[0]!.content += "!";
persistStore({ activeId: "b", sessions: [session, other] });
assert(localStorage.getItem(STORAGE_KEY) !== blob, "editing a line rewrites the transcript");
