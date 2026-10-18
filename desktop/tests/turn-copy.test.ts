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
  "other",
  "assistant",
  "other",
  "assistant",
]);
assert(oneTurn.join(",") === "false,false,false,false,false,true", "tools stay in the same turn");

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
