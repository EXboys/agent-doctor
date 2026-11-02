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

const { HISTORY_FIRST_SCREEN, historyRenderCut, messageContinuesRun } = await import(
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
