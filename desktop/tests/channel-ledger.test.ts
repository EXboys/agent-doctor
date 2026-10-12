/**
 * The site may send the same Feishu sentence again after a reconnect.
 * Run from desktop/: node --import tsx tests/channel-ledger.test.ts
 */

import { createChannelLedger } from "../src/chat/channel-ledger";

function assert(cond: boolean, message: string): void {
  if (!cond) throw new Error(message);
}

function memoryStore() {
  const data = new Map<string, string>();
  return {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => {
      data.set(key, value);
    },
  };
}

const store = memoryStore();
const first = createChannelLedger(store);
assert(first.seen("a").kind === "new", "an unknown sentence runs");
first.take("a");
assert(first.seen("a").kind === "here", "a queued sentence sent again is not run twice");
first.start("a");
assert(first.seen("a").kind === "here", "a running sentence sent again is not run twice");
first.start("b");
first.unstart("b");
first.take("c");
first.start("d");
first.finish("d", "答案", true);

const second = createChannelLedger(store);
assert(second.seen("a").kind === "cut", "a sentence running when the app closed is not rerun");
assert(second.seen("b").kind === "new", "a sentence that only waited in line runs after a restart");
assert(second.seen("c").kind === "new", "a sentence queued when the app closed runs after a restart");
const done = second.seen("d");
assert(done.kind === "done" && done.reply === "答案" && done.ok, "a finished sentence replays its answer");

let clock = 1_000;
const agingStore = memoryStore();
const aging = createChannelLedger(agingStore, () => clock);
aging.finish("old", "x", true);
clock += 4 * 24 * 60 * 60 * 1000;
aging.finish("new", "y", true);
const later = createChannelLedger(agingStore, () => clock);
assert(later.seen("old").kind === "new", "entries older than a few days are dropped");
assert(later.seen("new").kind === "done", "recent entries stay");

const broken = createChannelLedger(null);
assert(broken.seen("x").kind === "new", "no storage still runs the sentence");
broken.take("x");
assert(broken.seen("x").kind === "here", "no storage still dedupes within one run");

console.log("channel ledger ok");
