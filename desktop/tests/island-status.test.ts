/**
 * Run from desktop/: node --import tsx tests/island-status.test.ts
 */
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

const { emptyIslandTrack, islandLinger, islandSnapshot, reduceIslandTrack } = await import("../src/island/track");

const labels = {
  idle: "就绪",
  browser: "正在看网页",
  working: "正在做",
  needsConfirm: "要你确认",
  needsReply: "要你回一句",
  needsAnswer: "要你回答",
};

function assert(cond: unknown, message: string): void {
  if (!cond) throw new Error(message);
}

const started = reduceIslandTrack(emptyIslandTrack(), {
  type: "started",
  session_id: "s",
  runtime: "claude-code",
  cwd: "/tmp",
  command: "hi",
});
const working = islandSnapshot(started, labels, false);
assert(working.title === "正在做", `working title ${working.title}`);
assert(working.tone === "working", `working tone ${working.tone}`);

const waiting = reduceIslandTrack(started, {
  type: "permission_request",
  session_id: "s",
  request_id: "r",
  tool_name: "Edit",
  detail: "改文件",
  input_json: "{}",
  input_mode: "choice",
});
const confirm = islandSnapshot(waiting, labels, false);
assert(confirm.title === "要你确认", `waiting title ${confirm.title}`);
assert(confirm.tone === "waiting", `waiting tone ${confirm.tone}`);

const failed = reduceIslandTrack(started, {
  type: "completed",
  session_id: "s",
  status: "failed",
  exit_code: 1,
  summary: "",
});
const failLine = islandLinger(failed, "做完了", "失败了", "停了");
assert(failLine.title === "失败了" && failLine.tone === "failed", JSON.stringify(failLine));

const stopped = reduceIslandTrack(started, {
  type: "completed",
  session_id: "s",
  status: "cancelled",
  exit_code: null,
  summary: "",
});
const stopLine = islandLinger(stopped, "做完了", "失败了", "停了");
assert(stopLine.title === "停了" && stopLine.tone === "stopped", JSON.stringify(stopLine));

const done = reduceIslandTrack(started, {
  type: "completed",
  session_id: "s",
  status: "succeeded",
  exit_code: 0,
  summary: "好了",
});
const doneLine = islandLinger(done, "做完了", "失败了", "停了");
assert(doneLine.title === "做完了" && doneLine.tone === "done", JSON.stringify(doneLine));

console.log("island-status ok");
