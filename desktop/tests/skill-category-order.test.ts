/**
 * Run from desktop/: node --import tsx tests/skill-category-order.test.ts
 */

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
}

const values = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
  value: {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  },
});

const {
  loadSkillCategoryOverrides,
  saveSkillCategoryOverride,
} = await import("../src/chat/skill-category-order.ts");
const {
  loadPinnedSkillIds,
  savePinnedSkillIds,
} = await import("../src/chat/skill-order.ts");

saveSkillCategoryOverride("browser-research", "docs");
assert(
  loadSkillCategoryOverrides()["browser-research"] === "docs",
  "keeps a manual category override",
);

values.set("agent-doctor.chat.skillCategories.v1", '{"ok":"dev","bad":"missing"}');
const cleaned = loadSkillCategoryOverrides();
assert(cleaned.ok === "dev" && !("bad" in cleaned), "drops unknown categories");

savePinnedSkillIds(["http-request", "llm-wiki"]);
assert(
  loadPinnedSkillIds().join(",") === "http-request,llm-wiki",
  "persists the pinned category",
);

console.log("skill-category-order ok");
