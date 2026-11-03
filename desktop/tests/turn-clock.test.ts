/**
 * The model-request clock starts over for each user message, and does not
 * inherit the time spent starting the runtime (src/chat/activity.ts).
 *
 * Run from desktop/:
 *   node --import tsx tests/turn-clock.test.ts
 */

export {};

const storage = new Map<string, string>([["agent-doctor-locale", "zh"]]);
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

const {
  clearThinkingStarted,
  elapsedLabel,
  markModelRequest,
  markThinkingStartedAt,
  markTurnStarted,
  noteTurnUserAt,
  thinkingClockActive,
  thinkingElapsedLabel,
} = await import("../src/chat/activity");

let failures = 0;
function check(name: string, ok: boolean, detail?: unknown): void {
  if (ok) console.log(`ok - ${name}`);
  else {
    failures += 1;
    console.log(`not ok - ${name}`, detail ?? "");
  }
}

const realNow = Date.now;
let now = 1_700_000_000_000;
Date.now = () => now;

markTurnStarted();
now += 90_000;
check("a turn that already ran shows that time", elapsedLabel() === "1 分 30 秒", elapsedLabel());

markTurnStarted();
check("a new user message starts the clock over", elapsedLabel() === "", elapsedLabel());

now += 180_000;
check("startup time is on the clock before the model is asked", elapsedLabel() === "3 分 0 秒", elapsedLabel());
markModelRequest();
check("the first model request starts from zero", elapsedLabel() === "", elapsedLabel());

now += 2_000;
check("the request then counts its own seconds", elapsedLabel() === "2 秒", elapsedLabel());
now += 60_000;
markModelRequest();
check("a later request in the same turn keeps counting", elapsedLabel() === "1 分 2 秒", elapsedLabel());

now += 80 * 60_000;
check("the turn clock keeps the earlier work", elapsedLabel() === "81 分 2 秒", elapsedLabel());
markThinkingStartedAt(now);
check("a new thinking stretch does not inherit that time", thinkingElapsedLabel() === "", thinkingElapsedLabel());
now += 5_000;
check("thinking counts only its own seconds", thinkingElapsedLabel() === "5 秒", thinkingElapsedLabel());
check("the turn clock still includes the earlier work", elapsedLabel() === "81 分 7 秒", elapsedLabel());
clearThinkingStarted();
check("thinking leaves the turn clock on the bar", thinkingClockActive() === false && elapsedLabel() === "81 分 7 秒", elapsedLabel());

markTurnStarted();
markThinkingStartedAt(now);
now += 230 * 60 * 60_000;
check("a clock left after the session stopped reads as 230 hours", elapsedLabel() === "13800 分 0 秒", elapsedLabel());
noteTurnUserAt(now);
check(
  "the first message after it stopped starts from that message",
  elapsedLabel() === "" && thinkingClockActive() === false,
  elapsedLabel(),
);
now += 3_000;
check("that message then counts its own seconds", elapsedLabel() === "3 秒", elapsedLabel());

Date.now = realNow;

if (failures > 0) {
  console.log(`${failures} failed`);
  process.exit(1);
}
