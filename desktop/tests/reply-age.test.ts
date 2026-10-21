/**
 * Run from desktop/: node --import tsx tests/reply-age.test.ts
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

const { setLocale } = await import("../src/i18n");
const { replyAgeLabel, replyExactLabel } = await import("../src/chat/reply-age");

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
  console.log(`ok - ${msg}`);
}

const now = Date.parse("2026-10-17T03:10:00Z");
const at = (ms: number) => now - ms;

await setLocale("zh");
assert(replyAgeLabel(at(20_000), now) === "刚刚", "chinese just now");
assert(replyAgeLabel(at(8 * 60_000), now) === "8分钟前", "chinese minutes");
assert(replyAgeLabel(at(8 * 3_600_000), now) === "8小时前", "chinese hours");
assert(replyAgeLabel(at(2 * 86_400_000), now) === "2天前", "chinese days");

await setLocale("en");
assert(replyAgeLabel(at(20_000), now) === "just now", "english just now");
assert(replyAgeLabel(at(8 * 60_000), now) === "8m ago", "english minutes");
assert(replyAgeLabel(at(8 * 3_600_000), now) === "8h ago", "english hours");
assert(replyAgeLabel(at(2 * 86_400_000), now) === "2d ago", "english days");

await setLocale("zh");
assert(replyAgeLabel(at(8 * 3_600_000), now) === "8小时前", "switching back to chinese");

const old = Date.parse("2026-09-01T01:05:00Z");
await setLocale("en");
const englishDate = replyAgeLabel(old, now);
assert(englishDate.includes("Sep") && englishDate.includes("1"), `english date, got ${englishDate}`);
await setLocale("zh");
const chineseDate = replyAgeLabel(old, now);
assert(chineseDate.includes("9") && chineseDate.includes("1"), `chinese date, got ${chineseDate}`);
assert(!chineseDate.includes("Sep"), "chinese date does not keep the english month");

const stamp = Date.parse("2026-10-17T03:04:00Z");
await setLocale("en");
const exactEn = replyExactLabel(stamp);
await setLocale("zh");
const exactZh = replyExactLabel(stamp);
assert(exactEn.length > 0 && exactZh.length > 0 && exactEn !== exactZh, "exact time follows the language");
assert(replyAgeLabel(0, now) === "", "missing time stays blank");
