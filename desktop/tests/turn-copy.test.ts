/**
 * One copy button per assistant turn.
 *
 * Run from desktop/: node --import tsx tests/turn-copy.test.ts
 */
const { assistantTurnCopyOwners, joinAssistantTurnCopy } = await import("../src/chat/turn-copy.ts");

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
  console.log(`ok - ${msg}`);
}

const oneTurn = assistantTurnCopyOwners([
  "user",
  "assistant",
  "process",
  "assistant",
  "process",
  "assistant",
]);
assert(oneTurn.join(",") === "false,false,false,false,false,true", "tools stay in the same turn");

const stillWorking = assistantTurnCopyOwners([
  "user",
  "process",
  "assistant",
  "process",
]);
assert(stillWorking.join(",") === "false,false,false,false", "a process row after the sentence hides the rating");

const answerThenCard = assistantTurnCopyOwners([
  "user",
  "process",
  "assistant",
  "other",
]);
assert(answerThenCard.join(",") === "false,false,true,false", "a change card after the reply keeps the rating");

const twoTurns = assistantTurnCopyOwners([
  "assistant",
  "assistant",
  "user",
  "assistant",
  "other",
]);
assert(twoTurns.join(",") === "false,true,false,true,false", "a user message starts the next turn");

assert(joinAssistantTurnCopy(["  打开微信。  ", "", "  找到群了。\n"]) === "打开微信。\n\n找到群了。", "fragments join with a blank line");
assert(joinAssistantTurnCopy(["只有一句"]) === "只有一句", "a single fragment stays as it is");
