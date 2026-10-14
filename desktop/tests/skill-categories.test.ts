/**
 * Run from desktop/: node --import tsx tests/skill-categories.test.ts
 */

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
}

const { classifySkillCategory } = await import("../src/skill-categories.ts");

assert(classifySkillCategory({ id: "browser-research" }) === "web", "classifies an English id");
assert(classifySkillCategory({ name: "代码审查" }) === "dev", "classifies Chinese without word boundaries");
assert(classifySkillCategory({ description: "分析 CSV 和数据库" }) === "data", "uses category priority");
assert(classifySkillCategory({ name: "随手工具" }) === "other", "keeps unknown skills in other");

console.log("skill-categories ok");
