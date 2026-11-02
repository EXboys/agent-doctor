/**
 * Run from desktop/: node --import tsx tests/chat-store-size.test.ts
 *
 * The browser copy has to stay inside the storage the webview gives it, and it
 * has to keep the conversation in it. Both were broken: the copy grew past
 * the cap, the loader then trimmed it in a loop that could never finish, and
 * the page never booted.
 */
const storage = new Map<string, string>();
let failWrites = false;
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  writable: true,
  value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => {
      if (failWrites) throw new Error("QuotaExceededError");
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

const { CHAT_BACKUP_MAX_BYTES, CHAT_STORE_MAX_BYTES, STORAGE_BACKUP_KEY, STORAGE_KEY, storageBytes } =
  await import("../src/chat/types");
const {
  loadStoreFromLocalKeys,
  mergeChatStores,
  persistStore,
  trimStoreForSize,
  trimSessionMessages,
} = await import("../src/chat/store");

type Msg = { id: string; role: string; content: string; at: number };

const tool = (n: number, size = 16_000): Msg[] =>
  Array.from({ length: n }, (_, i) => ({ id: `t${i}`, role: "tool", content: "x".repeat(size), at: i }));

const user = (id: string, at: number): Msg => ({ id, role: "user", content: `ask ${id}`, at });

const session = (id: string, messages: Msg[], updatedAt = 1) => ({
  id,
  title: `chat ${id}`,
  runtime: "claude-code" as const,
  createdAt: 1,
  updatedAt,
  messages,
  runtimeThreadId: null,
  workspaceName: null,
  autoApprove: false,
});

// --- the copy keeps the conversation and stays a tail --------------------
{
  const messages = [user("u1", 0), ...tool(200)];
  const trimmed = trimSessionMessages(messages as never);
  assert(
    trimmed.some((m) => m.role === "user"),
    "the copy keeps what the user said",
  );
  assert(
    trimmed[trimmed.length - 1]!.id === messages[messages.length - 1]!.id,
    "the copy ends with the newest row",
  );
  assert(
    trimmed.length <= messages.length && trimmed.length >= 1,
    "the copy is a tail of the log",
  );
  const tailStart = messages.length - trimmed.length;
  assert(
    trimmed.every((m, index) => m.id === messages[tailStart + index]!.id),
    "the copy is contiguous",
  );
}

{
  // A chat that is mostly tool output still shows the question.
  const messages = [user("u1", 0), ...tool(200)];
  const trimmed = trimSessionMessages(messages as never);
  assert(
    trimmed.some((m) => m.role === "user"),
    "a long run of tool output does not hide the question",
  );
  assert(trimmed.length <= 240, "the copy stays bounded");
}

{
  // A run longer than the copy can reach is cut at the tail. The rows before
  // it stay on disk and come back from there.
  const messages = [user("u1", 0), ...tool(400)];
  const trimmed = trimSessionMessages(messages as never);
  assert(trimmed.length <= 120, "a very long run is cut to the tail");
  assert(
    trimmed[trimmed.length - 1]!.id === messages[messages.length - 1]!.id,
    "the cut copy still ends with the newest row",
  );
}

{
  const store = { activeId: "a", sessions: [session("a", [user("u1", 0), ...tool(200)] as never)] };
  const trimmed = trimStoreForSize(store as never);
  assert(
    trimmed.sessions[0]!.messages.some((m) => m.role === "user"),
    "a trimmed store still holds the conversation",
  );
}

// --- loading a copy that is over the cap must not spin --------------------
{
  const sessions = Array.from({ length: 34 }, (_, s) =>
    session(String(s), s === 0 ? (tool(120) as never) : (tool(120, 200) as never)),
  );
  const blob = JSON.stringify({ activeId: "0", sessions });
  assert(storageBytes(blob) > CHAT_STORE_MAX_BYTES, "the test copy is over the cap");
  storage.set(STORAGE_KEY, blob);

  const started = Date.now();
  const loaded = loadStoreFromLocalKeys();
  const elapsed = Date.now() - started;
  assert(elapsed < 2000, `loading an oversized copy finishes (${elapsed}ms)`);
  assert(loaded != null && loaded.sessions.length === 34, "every chat survives the trim");
  assert(
    storageBytes(storage.get(STORAGE_KEY) ?? "") <= CHAT_STORE_MAX_BYTES,
    "the copy is healed to a size that fits",
  );
}

// --- a partial copy must not replace the log on disk -----------------------
{
  const full = Array.from({ length: 300 }, (_, index) => ({
    id: `m${index}`,
    role: index % 30 === 0 ? "user" : "tool",
    content: `row ${index}`,
    at: index,
  }));
  const diskStore = {
    activeId: "a",
    sessions: [session("a", full as never, 100)],
  };
  const copy = trimSessionMessages(full as never);
  const localStore = {
    activeId: "a",
    sessions: [session("a", copy as never, 200)],
  };
  // A cut copy that is newer than the log: the log must win the rows.
  const merged = mergeChatStores(diskStore as never, localStore as never);
  assert(merged?.sessions[0]!.messages.length === 300, "a cut copy does not shorten the log");

  // The same copy, marked as cut, keeps the rows of the log.
  const marked = mergeChatStores(
    diskStore as never,
    localStore as never,
    undefined,
    new Set(["a"]),
  );
  assert(marked?.sessions[0]!.messages.length === 300, "a marked cut copy keeps the log whole");
}

// --- a full storage must not break the save -------------------------------
{
  failWrites = true;
  let threw = false;
  try {
    persistStore({ activeId: "a", sessions: [session("a", [user("u1", 0)] as never)] });
  } catch {
    threw = true;
  }
  failWrites = false;
  assert(!threw, "a full storage does not throw out of the save");
}

// --- a row bigger than the backup budget still leaves a usable backup -------
{
  storage.clear();
  // A reply this big does not fit any of the normal copies.
  const giant = [{ id: "g1", role: "assistant", content: "x".repeat(900_000), at: 0 }];
  const store = { activeId: "a", sessions: [session("a", giant as never)] };
  persistStore(store as never);
  const written = storage.get(STORAGE_KEY) ?? "";
  assert(written.length > 0, "a huge reply still reaches the copy");
  // The second save backs up the copy the first one left.
  persistStore({
    activeId: "a",
    sessions: [session("a", giant as never, 2)],
  } as never);
  const backup = storage.get(STORAGE_BACKUP_KEY) ?? "";
  assert(backup.length > 0, "a huge reply still leaves a backup");
  assert(
    storageBytes(backup) <= CHAT_BACKUP_MAX_BYTES,
    `the backup of a huge reply stays inside its budget (${storageBytes(backup)} bytes)`,
  );
  const parsed = JSON.parse(backup) as { sessions?: unknown[] };
  assert(Array.isArray(parsed.sessions) && parsed.sessions.length > 0, "the backup still reads back");
}

// --- the backup stays small enough to leave room --------------------------
{
  storage.clear();
  const sessions = Array.from({ length: 34 }, (_, s) =>
    session(String(s), s === 0 ? (tool(120) as never) : (tool(120, 200) as never)),
  );
  persistStore({ activeId: "0", sessions } as never);
  // A second save backs up the copy the first one left.
  persistStore({
    activeId: "0",
    sessions: sessions.map((s) => ({ ...s, updatedAt: s.updatedAt + 1 })),
  } as never);
  const backup = storage.get(STORAGE_BACKUP_KEY) ?? "";
  assert(backup.length > 0, "a backup is kept");
  assert(
    storageBytes(backup) <= CHAT_BACKUP_MAX_BYTES,
    `the backup stays inside its own budget (${storageBytes(backup)} bytes)`,
  );
  assert(
    storageBytes(storage.get(STORAGE_KEY) ?? "") <= CHAT_STORE_MAX_BYTES,
    "the copy stays inside the cap",
  );
  const main = storage.get(STORAGE_KEY) ?? "";
  const backupCopy = storage.get(STORAGE_BACKUP_KEY) ?? "";
  assert(
    storageBytes(main) + storageBytes(backupCopy) < 5_242_880,
    "the copy and its backup fit in one origin's storage",
  );
}
