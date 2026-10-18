/**
 * The file card belongs to the latest reply only.
 *
 * Run from desktop/: node --import tsx tests/reply-changes.test.ts
 */
const storage = new Map<string, string>([["agent-doctor.locale", "zh"]]);
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
  value: { language: "zh-CN" },
});

const { latestReplyFileChanges } = await import("../src/chat/reply-changes.ts");
type ChatMessage = import("../src/chat/types").ChatMessage;

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
  console.log(`ok - ${msg}`);
}

const older: ChatMessage = {
  id: "t0",
  role: "tool",
  content: "",
  at: 1,
  step: { id: "e0", kind: "edit", status: "done", path: "old.ts", additions: 3, deletions: 1 },
};
const firstUser: ChatMessage = { id: "u1", role: "user", content: "先改旧的", at: 2 };
const latest: ChatMessage = {
  id: "t1",
  role: "tool",
  content: "",
  at: 4,
  step: { id: "e1", kind: "edit", status: "done", path: "src/chat.css", additions: 40, deletions: 42 },
};
const reply: ChatMessage = { id: "a1", role: "assistant", content: "改好了", at: 5 };
const again: ChatMessage = { id: "u2", role: "user", content: "再来一条", at: 6 };

const shown = latestReplyFileChanges([older, firstUser, latest, reply]);
assert(shown.length === 1 && shown[0]?.name === "chat.css" && shown[0]?.additions === 40, "only the latest reply's file");

const hidden = latestReplyFileChanges([older, firstUser, latest, reply, again]);
assert(hidden.length === 0, "a new user message hides the card");

const readOnly: ChatMessage = {
  id: "r",
  role: "tool",
  content: "",
  at: 3,
  step: { id: "r1", kind: "read", status: "done", path: "notes.md" },
};
assert(latestReplyFileChanges([firstUser, readOnly, reply]).length === 0, "reading a file is not a change");

const fromText: ChatMessage = {
  id: "t2",
  role: "tool",
  content: "调用工具 write_file…\n/Users/airlu/test.txt\n+1 -0",
  at: 3,
};
const named = latestReplyFileChanges([firstUser, fromText, reply]);
assert(
  named.length === 1 && named[0]?.name === "test.txt" && named[0]?.additions === 1,
  "a write recorded only as the tool line still counts",
);

const shellWrite: ChatMessage = {
  id: "sh",
  role: "tool",
  content:
    "调用工具 终端…\nprintf '第一版\\n' > \"/Users/airlu/Desktop/filetest-测试.txt\" && cat \"/Users/airlu/Desktop/filetest-测试.txt\"",
  at: 3,
};
const patch: ChatMessage = {
  id: "p",
  role: "tool",
  content: "调用工具 patch…\n/Users/airlu/Desktop/filetest-测试.txt",
  at: 4,
};
const desktop = latestReplyFileChanges([
  firstUser,
  { id: "look", role: "tool", content: "调用工具 mcp__computer__screen_state…", at: 2 },
  shellWrite,
  patch,
  { id: "rd", role: "tool", content: "调用工具 读文件…\n/Users/airlu/Desktop/filetest-测试.txt", at: 5 },
  reply,
]);
assert(
  desktop.length === 1 && desktop[0]?.name === "filetest-测试.txt" && desktop[0]?.additions === 1,
  "a shell redirect and a patch of the same file count once",
);
