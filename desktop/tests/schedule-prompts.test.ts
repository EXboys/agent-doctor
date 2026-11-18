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
const { buildScheduleClientOptions, latestSessionInSelectedProject, normalizeScheduleHhmm } =
  await import("../src/chat/schedule-page.ts");
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

check("client select is never empty even before ownClientId resolves", () => {
  const res = buildScheduleClientOptions({
    ownClientId: "",
    ownComputerName: "",
    computers: [],
    editingSchedule: null,
    pickedClientId: null,
  });
  ok(res.options.length === 2, `expected 2 options, got ${res.options.length}`);
  ok(res.selected === "__self__", `selected ${res.selected}`);
  ok(res.options[0]?.label === "这台电脑", `label ${res.options[0]?.label}`);
});

check("client select binds to this named computer even when remote list is empty", () => {
  const res = buildScheduleClientOptions({
    ownClientId: "ad-mac-1",
    ownComputerName: "AirLu的MacBook Pro",
    computers: [],
    editingSchedule: null,
    pickedClientId: null,
  });
  ok(res.selected === "ad-mac-1", `selected ${res.selected}`);
  ok(res.options[0]?.label === "AirLu的MacBook Pro（这台电脑）", `label ${res.options[0]?.label}`);
});

check("client select preserves explicit follow-default choice across refreshes", () => {
  const res = buildScheduleClientOptions({
    ownClientId: "ad-mac-1",
    ownComputerName: "MacBook",
    computers: [{ id: "ad-desk", name: "办公室主机", os: "windows", default: true }],
    editingSchedule: null,
    pickedClientId: "",
  });
  ok(res.selected === "", `selected ${res.selected}`);
  ok(
    res.options.some((opt) => opt.value === "" && opt.label.includes("办公室主机")),
    "names default computer when different from self",
  );
});

check("client select retains offline bound computer when editing", () => {
  const res = buildScheduleClientOptions({
    ownClientId: "ad-mac-1",
    ownComputerName: "MacBook",
    computers: [],
    editingSchedule: { client_id: "ad-home", client_name: "家里台式机" },
    pickedClientId: null,
  });
  ok(res.selected === "ad-home", `selected ${res.selected}`);
  ok(
    res.options.some((opt) => opt.value === "ad-home" && opt.label.includes("家里台式机")),
    "keeps offline computer option",
  );
});

check("time picker normalizes HH:MM, single-digit hour, full-width colon, and 4-digit input", () => {
  ok(normalizeScheduleHhmm("8:5") === "08:05", "single digits");
  ok(normalizeScheduleHhmm("09：30") === "09:30", "full-width colon");
  ok(normalizeScheduleHhmm("1845") === "18:45", "compact 4 digits");
  ok(normalizeScheduleHhmm("23:59") === "23:59", "max valid");
  ok(normalizeScheduleHhmm("24:00") === null, "rejects hour 24");
  ok(normalizeScheduleHhmm("12:60") === null, "rejects minute 60");
});

if (failures > 0) {
  console.error(`${failures} failed`);
  process.exit(1);
}
console.log("schedule prompts ok");
