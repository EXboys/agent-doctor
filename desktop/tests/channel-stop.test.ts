/**
 * The phone's stop button broke three ways: the answer went back to whichever
 * chat was on screen (b722473), a stop never reached the Feishu job it named
 * (7382c44), and a stop for a job that had already finished buried the real
 * answer under a "stopped." This pins the decision table behind all three.
 * Run from desktop/: node --import tsx tests/channel-stop.test.ts
 */

import assert from "node:assert/strict";

const { decidePhoneStop } = await import("../src/chat/channel-stop");
const { createChannelLedger } = await import("../src/chat/channel-ledger");

function memoryStore() {
  const data = new Map<string, string>();
  return {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => {
      data.set(key, value);
    },
  };
}

// A stop that already went through is never acted on twice: the phone repeats
// it while the screen stays quiet.
assert.deepEqual(
  decidePhoneStop({
    asked: true,
    seen: { kind: "new" },
    running: true,
    active: true,
  }),
  { action: "ignore" },
  "a repeated stop must be ignored",
);

// The chat that owns the job still runs — cancel it, whatever is on screen.
assert.deepEqual(
  decidePhoneStop({
    asked: false,
    seen: { kind: "new" },
    running: true,
    active: false,
  }),
  { action: "cancel" },
  "a stop for a running chat must cancel it",
);
assert.deepEqual(
  decidePhoneStop({
    asked: false,
    seen: { kind: "new" },
    running: false,
    active: true,
  }),
  { action: "cancel" },
  "the chat on screen counts as stoppable even when its run bookkeeping is gone",
);

// The turn already finished but the finishing reply was lost on the way to
// the site: answer from the ledger instead of claiming a stop. The reply and
// its ok flag travel back to the chat that asked.
assert.deepEqual(
  decidePhoneStop({
    asked: false,
    seen: { kind: "done", reply: "做完了。", ok: true },
    running: false,
    active: false,
  }),
  { action: "replay", reply: "做完了。", ok: true },
  "a finished job must replay its real answer, not report a stop",
);
assert.deepEqual(
  decidePhoneStop({
    asked: false,
    seen: { kind: "done", reply: "", ok: false },
    running: false,
    active: false,
  }),
  { action: "replay", reply: "", ok: false },
  "a failed ledger entry replays its failure; the caller fills an empty reply",
);

// Nothing runs here and nothing is recorded. The job may belong to another
// computer or still be alive, so the decision must not claim it was stopped
// and must not close it on the site (the caller sends "missed" with done=false).
for (const seen of [{ kind: "new" }, { kind: "here" }, { kind: "cut" }] as const) {
  assert.deepEqual(
    decidePhoneStop({ asked: false, seen, running: false, active: false }),
    { action: "missed" },
    `a stop for a job with ledger "${seen.kind}" must answer "not found", never "stopped"`,
  );
}

// The ledger the decision reads from really hands back what finish() stored,
// so a stop arriving after a reconnect still finds the answer.
const store = memoryStore();
const ledger = createChannelLedger(store);
ledger.finish("job-9", "答案在这。", true);
const seen = ledger.seen("job-9");
assert.deepEqual(
  decidePhoneStop({ asked: false, seen, running: false, active: false }),
  { action: "replay", reply: "答案在这。", ok: true },
  "a stop after the app restarted replays what the ledger kept",
);

console.log("channel stop tests passed");
