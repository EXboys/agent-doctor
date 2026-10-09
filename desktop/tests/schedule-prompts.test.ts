/**
 * Schedule scene prompts and which chat they target.
 *
 * Run from desktop/:
 *   node_modules/.bin/tsx tests/schedule-prompts.test.ts
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
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  writable: true,
  value: { language: "zh-CN" },
});

const { schedulePrompt } = await import("../src/chat/schedule-prompts.ts");
const { latestSessionInSelectedProject } = await import("../src/chat/schedule-page.ts");
import type { ChatSession } from "../src/chat/types.ts";

let failures = 0;

function check(name: string, fn: () => void): void {
  try {
    fn();
    console.log(`  ok   ${name}`);
  } catch (error) {
    failures += 1;
    console.log(`  FAIL ${name}\n         ${(error as Error).message}`);
  }
}

function ok(condition: boolean, what: string): void {
  if (!condition) throw new Error(what);
}

function session(partial: Pick<ChatSession, "id" | "runtime" | "updatedAt" | "workspaceName">): ChatSession {
  return {
    title: "",
    createdAt: partial.updatedAt,
    messages: [],
    runtimeThreadId: null,
    ...partial,
  };
}

check("hermes morning prompt names /cron and keeps the time editable", () => {
  const text = schedulePrompt("morning", "hermes");
  ok(text.includes("每个工作日早上 9 点"), "when");
  ok(text.includes("/cron"), "hermes shortcut");
  ok(!text.includes("系统定时") || text.includes("不要另写"), "tells the agent not to invent a system timer");
});

check("claude prompt names /schedule and /loop", () => {
  const text = schedulePrompt("once", "claude-code");
  ok(text.includes("/schedule"), "durable shortcut");
  ok(text.includes("/loop"), "session shortcut");
});

check("openclaw prompt names its own scheduler", () => {
  const text = schedulePrompt("weekly", "openclaw");
  ok(text.includes("automations"), "openclaw scheduler");
  ok(!text.includes("/cron add"), "does not borrow hermes syntax");
});

check("codex prompt does not invent a schedule command", () => {
  const text = schedulePrompt("repeat", "codex");
  ok(text.includes("如果你自己能定时跑"), "honest fallback");
  ok(!text.includes("/cron"), "no hermes slash");
  ok(!text.includes("/schedule"), "no claude slash");
  ok(!text.includes("/loop"), "no loop slash");
});

check("latest chat is the newest one in the selected project", () => {
  const active = session({ id: "old", runtime: "codex", updatedAt: 10, workspaceName: "app" });
  const newer = session({ id: "new", runtime: "hermes", updatedAt: 50, workspaceName: "app" });
  const other = session({ id: "other", runtime: "openclaw", updatedAt: 90, workspaceName: "site" });
  const picked = latestSessionInSelectedProject(active, [other, active, newer], null);
  ok(picked.id === "new", `picked ${picked.id}`);
});

if (failures > 0) {
  console.error(`${failures} failed`);
  process.exit(1);
}
console.log("schedule prompts ok");
