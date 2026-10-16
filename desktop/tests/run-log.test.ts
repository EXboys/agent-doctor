/**
 * Run from desktop/: node --import tsx tests/run-log.test.ts
 */
import type { ChatMessage } from "../src/chat/types";

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

const { appendRunLogMessage } = await import("../src/chat/tool-records");
const { coveredRunLogPointers, isLogMessage, isRuntimeLog, isTransportDump, markTurnLogFailed } = await import("../src/chat/format");

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
  console.log(`ok - ${msg}`);
}

let next = 0;
const newId = () => `m${(next += 1)}`;
const user = (content: string): ChatMessage => ({ id: newId(), role: "user", content, at: 0 });

const messages: ChatMessage[] = [user("你好啊")];
appendRunLogMessage(messages, "[state-migrations] Plugin \"deepseek\" pending", newId);
appendRunLogMessage(messages, "[skills] Skipping escaped skill path outside its configured root", newId);
appendRunLogMessage(messages, "warn: something odd happened", newId);
assert(messages.length === 2, "every line of one turn lands in one saved log");
assert(messages[1].log === true && messages[1].role === "meta", "the saved log is marked as a log");
assert(messages[1].content.split("\n").length === 3, "the original lines are all kept");

messages.push(user("再试一次"));
appendRunLogMessage(messages, "[skills] again", newId);
assert(messages.length === 4, "the next turn starts its own log");

assert(isLogMessage(messages[1]), "a marked message is a log");
assert(
  isLogMessage({ role: "meta", content: "[skills] Skipping escaped skill path" }),
  "an older save is recognized by its shape",
);
assert(
  !isLogMessage({ role: "meta", content: "这台电脑上的 Hermes 太旧，这里还用不了。" }),
  "a sentence for the person is not a log",
);
assert(!isRuntimeLog("[1] 第一步"), "a numbered list item is not a log");
assert(isRuntimeLog("\u001b[31mfailed\u001b[0m"), "color codes mark a log");
assert(
  isTransportDump('MCP tool: failed (handshaking failed: HTTP 401: {"error":{"code":"token_expired"}})'),
  "an HTTP body is a transport dump",
);
assert(!isTransportDump("HTTP 200 ok"), "a status code without a body is not a dump");

const saved = [
  user("你好"),
  { id: "log", role: "meta" as const, content: "Installing Python dependencies failed", at: 0, log: true },
  { id: "ptr", role: "meta" as const, content: "这次没有回话。运行记录收在上面，点开可以看原文。再发一次试试。", at: 1 },
];
const covered = coveredRunLogPointers(saved);
assert(covered.failedLogs.has(1), "a log followed by the old pointer owns the failure");
assert(covered.hidden.has(2), "the old sentence is not shown again");
const onlyLog = coveredRunLogPointers([
  user("你好"),
  { id: "log2", role: "meta" as const, content: "Installing Python dependencies failed", at: 0, log: true },
]);
assert(onlyLog.failedLogs.has(1), "a log with no reply is the failure line");
assert(markTurnLogFailed(saved), "marking finds this turn's log");
assert(saved[1].summary === "commandFailed", "the log row keeps the failure sentence");
assert(!markTurnLogFailed([user("只有一句")]), "a turn with no log is not marked");

console.log("run log ok");
