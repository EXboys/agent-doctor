import assert from "node:assert/strict";

import { summarizeLongTasks, type LongTaskSample } from "../src/chat/perf-probe";

const sample = (
  at: number,
  gapMs: number,
  streaming = false,
  rows = 12,
  label = "idle",
): LongTaskSample => ({ at, gapMs, streaming, rows, label });

// A clean minute reports nothing — the probe must stay silent when idle.
assert.equal(summarizeLongTasks([], 1_000), "");
assert.equal(summarizeLongTasks([sample(900, 210)], 61_000), "");

// Samples inside the window summarize into one line with the worst frame.
const line = summarizeLongTasks(
  [
    sample(60_000, 80, true, 120, "history-pump"),
    sample(61_000, 55),
    sample(1_000, 300, false, 80, "store-save"),
  ],
  61_500,
);
assert.match(line, /2 long frame\(s\) >50ms in 60s/);
assert.match(line, /worst 80ms/);
assert.match(line, /1 while streaming/);
assert.match(line, /rows=120/);
assert.match(line, /by history-pump x1\/80ms, idle x1\/55ms/);

// A sample outside the window does not count, even when it is the worst.
const bounded = summarizeLongTasks(
  [sample(1_000, 500), sample(61_000, 60)],
  61_500,
);
assert.match(bounded, /worst 60ms/);

console.log("perf-probe summarize ok");
