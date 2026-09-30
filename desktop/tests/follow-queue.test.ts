/**
 * Standalone check for the chat follow-up queue (src/chat/follow-queue.ts).
 *
 * Run from desktop/:
 *   node_modules/.bin/tsx tests/follow-queue.test.ts
 *
 * Lives outside src/ on purpose: desktop/tsconfig.json includes only "src", so
 * this harness never enters `npm run build` (tsc) nor the vite bundle.
 */

// follow-queue imports ../i18n, which reads localStorage/navigator at import time.
// Both are read-only accessors on newer Node, so redefine rather than assign.
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

const {
  enqueueFollowUp,
  followUpsFor,
  removeFollowUp,
  takeFollowUps,
  mergedFollowUp,
  holdFollowUps,
  insertFollowUpsNow,
  consumeDrainIntent,
  setFollowQueueInsertHandler,
} = await import("../src/chat/follow-queue.ts");

let failures = 0;
let checks = 0;

function check(name: string, fn: () => void): void {
  checks += 1;
  try {
    fn();
    console.log(`  ok   ${name}`);
  } catch (error) {
    failures += 1;
    console.log(`  FAIL ${name}\n         ${(error as Error).message}`);
  }
}

function eq<T>(actual: T, expected: T, what: string): void {
  const a = JSON.stringify(actual);
  const b = JSON.stringify(expected);
  if (a !== b) throw new Error(`${what}: got ${a}, expected ${b}`);
}

function ok(condition: boolean, what: string): void {
  if (!condition) throw new Error(what);
}

const mention = (id: string) => ({ id, label: id, kind: "agent" }) as never;
const attachment = (name: string) =>
  ({ id: name, name, path: `/tmp/${name}` }) as never;

console.log("\nfollow-queue: queue storage");
// The module keeps a singleton Map, so every case uses its own session id.

check("enqueue keeps fields and assigns a unique id", () => {
  const a = enqueueFollowUp("s-fields", {
    sessionId: "s-fields",
    text: "first",
    attachments: [attachment("a.txt")],
    mentions: [mention("m1")],
  });
  const b = enqueueFollowUp("s-fields", {
    sessionId: "s-fields",
    text: "second",
    attachments: [],
    mentions: [],
  });
  eq(a.text, "first", "text");
  eq(a.sessionId, "s-fields", "sessionId");
  eq(a.attachments.length, 1, "attachment count");
  eq(a.mentions.length, 1, "mention count");
  ok(typeof a.id === "string" && a.id.length > 0, "id should be non-empty");
  ok(a.id !== b.id, "ids should be unique");
  takeFollowUps("s-fields");
});

check("queues are isolated per session", () => {
  enqueueFollowUp("s-x", { sessionId: "s-x", text: "x", attachments: [], mentions: [] });
  enqueueFollowUp("s-y", { sessionId: "s-y", text: "y1", attachments: [], mentions: [] });
  enqueueFollowUp("s-y", { sessionId: "s-y", text: "y2", attachments: [], mentions: [] });
  eq(followUpsFor("s-x").length, 1, "s-x length");
  eq(followUpsFor("s-y").length, 2, "s-y length");
  eq(followUpsFor("s-empty").length, 0, "unknown session length");
  takeFollowUps("s-x");
  takeFollowUps("s-y");
});

check("removeFollowUp drops one item and keeps the rest", () => {
  const keep = enqueueFollowUp("s-remove", { sessionId: "s-remove", text: "keep", attachments: [], mentions: [] });
  const drop = enqueueFollowUp("s-remove", { sessionId: "s-remove", text: "drop", attachments: [], mentions: [] });
  removeFollowUp("s-remove", drop.id);
  eq(followUpsFor("s-remove").map((item) => item.text), ["keep"], "remaining");
  removeFollowUp("s-remove", keep.id);
  eq(followUpsFor("s-remove").length, 0, "after removing last");
  eq(takeFollowUps("s-remove").length, 0, "take after emptying");
});

check("takeFollowUps drains: the second take is empty", () => {
  enqueueFollowUp("s-take", { sessionId: "s-take", text: "one", attachments: [], mentions: [] });
  eq(takeFollowUps("s-take").length, 1, "first take");
  eq(takeFollowUps("s-take").length, 0, "second take");
  eq(followUpsFor("s-take").length, 0, "after drain");
});

console.log("\nfollow-queue: drain intent (Stop must not send; 'insert now' must win)");

check("defaults to when-ready and resets after every read", () => {
  consumeDrainIntent(); // normalise in case an earlier case left it dirty
  eq(consumeDrainIntent(), "when-ready", "default");
  eq(consumeDrainIntent(), "when-ready", "reset after read");
});

check("holdFollowUps sets hold", () => {
  consumeDrainIntent();
  holdFollowUps();
  eq(consumeDrainIntent(), "hold", "hold");
  eq(consumeDrainIntent(), "when-ready", "reset");
});

check("insertFollowUpsNow sets now", () => {
  consumeDrainIntent();
  insertFollowUpsNow();
  eq(consumeDrainIntent(), "now", "now");
});

check("hold after now must not downgrade the intent", () => {
  consumeDrainIntent();
  insertFollowUpsNow();
  holdFollowUps();
  eq(consumeDrainIntent(), "now", "now should survive a late Press-Stop");
});

check("now after hold upgrades the intent", () => {
  consumeDrainIntent();
  holdFollowUps();
  insertFollowUpsNow();
  eq(consumeDrainIntent(), "now", "now");
});

check("setFollowQueueInsertHandler accepts a handler", () => {
  let called = 0;
  setFollowQueueInsertHandler(() => {
    called += 1;
  });
  eq(called, 0, "registering must not fire immediately");
});

console.log("\nfollow-queue: merging several notes into one follow-up");

check("no items merges to null", () => {
  eq(mergedFollowUp([]), null, "empty");
});

check("a single item passes through unchanged", () => {
  const only = enqueueFollowUp("s-merge1", { sessionId: "s-merge1", text: "only", attachments: [], mentions: [] });
  const items = [only];
  const merged = mergedFollowUp(items);
  ok(merged === only, "should be the same object");
  eq(items.length, 1, "input must not be mutated");
  takeFollowUps("s-merge1");
});

check("several items collapse with the last one as the task", () => {
  const first = enqueueFollowUp("s-mergeN", {
    sessionId: "s-mergeN",
    text: "EARLIER_ONE",
    attachments: [attachment("one.txt")],
    mentions: [mention("m-one")],
  });
  const middle = enqueueFollowUp("s-mergeN", {
    sessionId: "s-mergeN",
    text: "EARLIER_TWO",
    attachments: [attachment("two.txt")],
    mentions: [],
  });
  const last = enqueueFollowUp("s-mergeN", {
    sessionId: "s-mergeN",
    text: "LATEST_TASK",
    attachments: [attachment("three.txt")],
    mentions: [mention("m-last")],
  });
  const items = [first, middle, last];
  const merged = mergedFollowUp(items);
  ok(merged !== null, "merged should exist");
  const m = merged!;
  eq(m.id, last.id, "id should come from the last note");
  eq(m.sessionId, "s-mergeN", "sessionId");
  ok(m.text.includes("EARLIER_ONE"), "earlier note 1 should be carried over");
  ok(m.text.includes("EARLIER_TWO"), "earlier note 2 should be carried over");
  ok(m.text.includes("LATEST_TASK"), "latest note should be carried over");
  ok(
    m.text.indexOf("EARLIER_ONE") < m.text.indexOf("LATEST_TASK"),
    "earlier notes should precede the task",
  );
  eq(m.attachments.length, 3, "attachments should be concatenated");
  eq(m.mentions.length, 2, "mentions should be concatenated");
  eq(items.length, 3, "input array must not be drained");
  eq(followUpsFor("s-mergeN").length, 3, "merging must not consume the queue");
  takeFollowUps("s-mergeN");
});

console.log("");
if (failures > 0) {
  console.log(`${failures} of ${checks} checks FAILED`);
  process.exit(1);
}
console.log(`all ${checks} checks passed`);
