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

const { buildIslandRows, emptyIslandTrack, islandLinger, islandSnapshot, reduceIslandTrack } =
  await import("../src/island/track");

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

const baseNow = 1_800_000_000_000;
const emptyNewest = Array.from({ length: 5 }, (_, idx) => ({
  id: `empty-${idx}`,
  runtime: "claude-code",
  title: "新对话",
  updatedAt: baseNow - idx * 1000,
  messages: [],
  project: "  agent-doctor  ",
}));
const realSessions = Array.from({ length: 24 }, (_, idx) => ({
  id: `real-${idx}`,
  runtime: idx === 0 ? "pi" : idx === 1 ? "doctor" : "claude-code",
  title: `会话 ${idx}`,
  // Include sessions older than 7 days to ensure recent 20 are not dropped by a 7-day cutoff.
  updatedAt: baseNow - (idx + 1) * 24 * 60 * 60 * 1000,
  messages: [
    { role: "user", content: `问题 ${idx}` },
    { role: "assistant", content: `回答 ${idx}` },
  ],
  project: idx % 2 === 0 ? "  agent-doctor  " : "   ",
}));

const rows = buildIslandRows([...emptyNewest, ...realSessions], "empty-0", { sent: "", spoken: "" });
assert(rows.length === 20, `expected 20 non-empty rows, got ${rows.length}`);
assert(rows[0]?.id === "real-0", `expected newest non-empty first, got ${rows[0]?.id}`);
assert(rows[19]?.id === "real-19", `expected 20th non-empty last, got ${rows[19]?.id}`);
assert(rows[0]?.badge === "Pi", `expected Pi badge, got ${rows[0]?.badge}`);
assert(rows[1]?.badge === "Doctor", `expected Doctor badge, got ${rows[1]?.badge}`);
assert(rows[0]?.project === "agent-doctor", `expected trimmed project, got ${rows[0]?.project}`);
assert(rows[1]?.project === undefined, `expected blank project omitted, got ${rows[1]?.project}`);

// Background running session receives live spoken text & floats to top without clobbering activeId.
const bgRows = buildIslandRows(
  [
    {
      id: "active-chat",
      runtime: "claude-code",
      title: "前台会话",
      updatedAt: baseNow,
      messages: [{ role: "user", content: "你好" }],
      project: "前端项目",
    },
    {
      id: "bg-chat",
      runtime: "codex",
      title: "后台会话",
      updatedAt: baseNow - 60_000,
      messages: [{ role: "user", content: "排查构建" }],
      project: "后端服务",
      working: true,
      liveSpoken: "正在检查依赖配置。",
    },
  ],
  "active-chat",
  { sent: "", spoken: "" },
  { workingId: "bg-chat" },
);
assert(bgRows[0]?.id === "bg-chat" && bgRows[0]?.working, "working background session floats first");
assert(bgRows[0]?.spoken === "正在检查依赖配置。", `expected liveSpoken, got ${bgRows[0]?.spoken}`);
assert(bgRows[0]?.project === "后端服务", `expected project 后端服务, got ${bgRows[0]?.project}`);
assert(bgRows[1]?.id === "active-chat" && bgRows[1]?.current, "active chat keeps current flag");

console.log("island-status ok");
