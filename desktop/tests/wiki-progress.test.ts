/**
 * Wiki generation progress from pages written.
 *
 * Run from desktop/:
 *   node_modules/.bin/tsx tests/wiki-progress.test.ts
 */

const { wikiRunProgress } = await import("../src/chat/wiki-progress.ts");

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

check("before the first page there is no remaining time", () => {
  const progress = wikiRunProgress({ elapsedMs: 20_000, done: 0, total: 4 });
  ok(progress.phase === "reading", progress.phase);
  ok(progress.remainingMs === null, "remaining");
  ok(progress.percent > 0 && progress.percent <= 12, `percent ${progress.percent}`);
});

check("written pages estimate the time left", () => {
  const progress = wikiRunProgress({ elapsedMs: 60_000, done: 1, total: 4 });
  ok(progress.phase === "writing", progress.phase);
  ok(progress.remainingMs === 180_000, `remaining ${progress.remainingMs}`);
  ok(progress.percent === 25, `percent ${progress.percent}`);
});

check("a full pass stays short of done until the run ends", () => {
  const progress = wikiRunProgress({ elapsedMs: 120_000, done: 4, total: 4 });
  ok(progress.phase === "finishing", progress.phase);
  ok(progress.percent === 92, `percent ${progress.percent}`);
  ok(progress.remainingMs === null, "remaining");
});

if (failures > 0) {
  console.error(`${failures} failed`);
  process.exit(1);
}
console.log("wiki progress ok");
