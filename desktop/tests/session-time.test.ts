/**
 * Session sidebar times (src/chat/copy-ui.ts#formatTime): today shows the
 * clock, anything older counts the days since.
 *
 * Run from desktop/:
 *   node --import tsx tests/session-time.test.ts
 */

export {};

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

const { formatTime } = await import("../src/chat/copy-ui");

let failures = 0;
function check(name: string, ok: boolean, detail?: unknown): void {
  if (ok) {
    console.log(`ok - ${name}`);
  } else {
    failures += 1;
    console.log(`not ok - ${name}`, detail ?? "");
  }
}

const DAY = 86_400_000;
const now = new Date();
const at = (dayOffset: number, hour: number, minute: number) =>
  new Date(now.getFullYear(), now.getMonth(), now.getDate() + dayOffset, hour, minute).getTime();

// The clock keeps the runtime locale's own rendering (09:05 / 09:05 AM), so
// only the digits are asserted here.
check("today shows the clock", formatTime(at(0, 9, 5)).startsWith("09:05"), formatTime(at(0, 9, 5)));
check("yesterday counts one day", formatTime(at(-1, 23, 7)) === "1 天", formatTime(at(-1, 23, 7)));
check("three days back counts three", formatTime(at(-3, 8, 0)) === "3 天", formatTime(at(-3, 8, 0)));
check("late yesterday is still one day, not zero", formatTime(at(-1, 0, 1)) === "1 天", formatTime(at(-1, 0, 1)));
check("a moment before midnight flips the count", formatTime(at(0, 0, 0) - 1) === "1 天", formatTime(at(0, 0, 0) - 1));

if (failures > 0) {
  console.log(`${failures} failed`);
  process.exit(1);
}
