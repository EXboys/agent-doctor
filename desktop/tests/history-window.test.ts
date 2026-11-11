/**
 * Run from desktop/: node --import tsx tests/history-window.test.ts
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
  value: { language: "en" },
});

function assert(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
  console.log(`ok - ${msg}`);
}

const { HISTORY_FIRST_SCREEN, HISTORY_RUN_SLICE, historyRenderCut, messageContinuesRun, snapRunBoundary } = await import(
  "../src/chat/history-window.ts"
);

const row = (role: string, content = "hi") => ({ role, content });

assert(historyRenderCut(Array.from({ length: 10 }, () => row("user"))) === 0, "a short chat paints at once");

const long = Array.from({ length: 80 }, (_, index) => row(index % 2 ? "assistant" : "user"));
const cut = historyRenderCut(long);
assert(cut > 0 && cut < long.length, "a long chat paints the latest rows first");
assert(long.length - cut <= HISTORY_FIRST_SCREEN, "the first screen stays bounded");
assert(!messageContinuesRun(long, cut), "the cut is on a row boundary");

const mixed = Array.from({ length: 40 }, (_, index) => row(index >= 10 && index <= 30 ? "tool" : "user", "step"));
const mixedCut = historyRenderCut(mixed);
assert(mixedCut === 10, "a tool run stays on the first screen instead of being split");
assert(!messageContinuesRun(mixed, mixedCut), "the tool run starts a fresh block");

const heavy = [...Array.from({ length: 40 }, () => row("user", "a")), row("assistant", "x".repeat(30_000))];
assert(historyRenderCut(heavy) === 40, "one huge reply does not drag the older rows into the first paint");

// The newest rows of a long turn are all tool output. Those rows are painted
// first, so without help the user's own words only appear once the older rows
// are pumped in — and that pump does not run while the window is hidden.
const talkThenTools = [
  ...Array.from({ length: 10 }, () => row("user")),
  ...Array.from({ length: 100 }, () => row("tool")),
];
const talkCut = historyRenderCut(talkThenTools);
assert(talkCut <= 9, "the first screen reaches back to the conversation");
assert(
  talkThenTools.slice(talkCut).some((m) => m.role === "user"),
  "what the user said is on the first screen",
);

// A first screen that already holds the conversation is left alone.
const talkFirst = [...Array.from({ length: 10 }, () => row("user")), ...Array.from({ length: 30 }, () => row("assistant"))];
assert(historyRenderCut(talkFirst) === 16, "a first screen with talk is not stretched");

// A run of thousands of rows (one long agent turn) must not drag a slice
// boundary back to its start - a slice absorbs at most the run cap, and the
// rest of the run fills the following slices.
{
  const run = Array.from({ length: 2_000 }, (_, i) => ({
    role: "tool",
    content: `step ${i}`,
  }));
  assert(
    snapRunBoundary(run, run.length - 1, HISTORY_RUN_SLICE) === run.length - 1 - HISTORY_RUN_SLICE,
    "a mega-run slice absorbs at most the cap",
  );
  // A run shorter than the cap snaps all the way to its start.
  const short = [
    { role: "user", content: "问一句" },
    { role: "tool", content: "a" },
    { role: "tool", content: "b" },
  ];
  assert(
    snapRunBoundary(short, short.length - 1, HISTORY_RUN_SLICE) === 1,
    "a short run snaps to its first row",
  );
  // The first-screen cut of a mega-run transcript with no conversation rows
  // stays bounded: first screen + run slice + capped search + one more slice.
  const cut = historyRenderCut(run);
  assert(run.length - cut <= 600, `the mega-run cut stays bounded (cut=${cut})`);
}
