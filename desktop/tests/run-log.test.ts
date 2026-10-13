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
const { isLogMessage, isRuntimeLog } = await import("../src/chat/format");

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

console.log("run log ok");
