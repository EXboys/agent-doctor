/**
 * Run from desktop/: node --import tsx tests/skill-order.test.ts
 */

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
}

const {
  orderBySaved,
  pinSkillId,
  reorderSkillIds,
  togglePinnedSkillId,
} = await import("../src/chat/skill-order.ts");

const items = ["a", "b", "c", "d"].map((skill_id) => ({ skill_id }));

const pinned = orderBySaved(items, ["c", "a"]);
assert(
  pinned.map((item) => item.skill_id).join(",") === "c,a,b,d",
  "saved ids lead, the rest keep their order",
);

assert(pinSkillId(["c", "a"], "b").join(",") === "b,c,a", "pin moves an id to the front");
assert(pinSkillId(["b", "c"], "b").join(",") === "b,c", "pinning the first id does not duplicate it");
assert(togglePinnedSkillId([], "b").join(",") === "b", "pin list adds a skill");
assert(togglePinnedSkillId(["b", "c"], "b").join(",") === "c", "pin list removes a pinned skill");

const dragged = reorderSkillIds(["a", "b", "c", "d"], ["a", "c"], "c", "a", "before");
assert(dragged.join(",") === "c,b,a,d", "dragging a visible skill leaves hidden ids in place");

const fresh = reorderSkillIds([], ["a", "b", "c"], "c", "a", "before");
assert(fresh.join(",") === "c,a,b", "the first drag writes the visible order");

console.log("skill-order ok");
