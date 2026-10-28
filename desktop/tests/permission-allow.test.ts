/**
 * Choice allows: one retry after a finished failure, and no second write
 * when the first write has not returned.
 *
 * Run from desktop/:
 *   node --import tsx tests/permission-allow.test.ts
 */
import assert from "node:assert/strict";
import {
  consumeAllowStop,
  isAllowAckTimeout,
  isAskSessionGone,
  permissionHoldKind,
  requestAllowStop,
  resolveChoiceAllow,
  shareAllow,
} from "../src/chat/permission-allow.ts";

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

const sent: string[] = [];

await resolveChoiceAllow(async () => {
  sent.push("ok");
});
assert.deepEqual(sent, ["ok"]);

sent.length = 0;
await resolveChoiceAllow(async () => {
  sent.push("try");
  if (sent.length === 1) throw new Error("write failed");
});
assert.deepEqual(sent, ["try", "try"]);

let goneCalls = 0;
await assert.rejects(
  () =>
    resolveChoiceAllow(async () => {
      goneCalls += 1;
      throw new Error("no active ask session");
    }),
  (error: unknown) => isAskSessionGone(error),
);
assert.equal(goneCalls, 1);

let hungCalls = 0;
await assert.rejects(
  () =>
    resolveChoiceAllow(
      () => {
        hungCalls += 1;
        return new Promise(() => {});
      },
      30,
    ),
  (error: unknown) => isAllowAckTimeout(error),
);
await delay(40);
assert.equal(hungCalls, 1);

let shared = 0;
await resolveChoiceAllow(() =>
  shareAllow("same", true, async () => {
    shared += 1;
    if (shared === 1) throw new Error("write failed");
  }),
);
assert.equal(shared, 2);

let hungShared = 0;
const hang = () =>
  shareAllow("hung", true, () => {
    hungShared += 1;
    return new Promise(() => {});
  });
await assert.rejects(() => resolveChoiceAllow(hang, 20), (error: unknown) => isAllowAckTimeout(error));
await assert.rejects(() => resolveChoiceAllow(hang, 20), (error: unknown) => isAllowAckTimeout(error));
assert.equal(hungShared, 1);

requestAllowStop("chat-1");
assert.equal(consumeAllowStop("chat-2"), false);
assert.equal(consumeAllowStop("chat-1"), true);
assert.equal(consumeAllowStop("chat-1"), false);

assert.equal(permissionHoldKind("等待确认：Write"), "allow");
assert.equal(permissionHoldKind("需要你回答"), "answer");
assert.equal(permissionHoldKind("需要你输入"), "answer");

console.log("ok - permission allow");
