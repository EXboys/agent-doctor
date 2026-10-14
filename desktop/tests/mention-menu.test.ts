/**
 * Run from desktop/: node --import tsx tests/mention-menu.test.ts
 */

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
}

globalThis.localStorage = {
  getItem: () => "zh",
  setItem: () => undefined,
  removeItem: () => undefined,
  clear: () => undefined,
  key: () => null,
  length: 0,
};

const { buildMentionConstraint, knowledgeMentionsFromPages, parseMentionsFromText } = await import(
  "../src/ask-resources.ts"
);

const pages = knowledgeMentionsFromPages([
  {
    hint: "全局",
    root: "/Users/me/.agent-doctor/knowledge",
    pages: [
      { path: "index.md", title: "目录" },
      { path: "log.md", title: "记录" },
      { path: "guides/deploy.md", title: "部署流程" },
    ],
  },
  {
    hint: "这个项目",
    root: "/work/app/.agent-doctor/knowledge",
    pages: [{ path: "billing.md", title: "报价规则" }],
  },
]);
assert(pages.length === 2, "skips index and log");
assert(pages[0]?.kind === "knowledge" && pages[0].label === "部署流程", "uses the page title");
assert(pages[0].id.endsWith("/wiki/guides/deploy.md"), "points at the page file");
assert(pages[0].hint === "全局" && pages[1]?.hint === "这个项目", "keeps the scope");

const constraint = buildMentionConstraint([
  { kind: "skill", id: "deploy", label: "部署" },
  pages[0],
]);
assert(constraint.includes("部署"), "keeps the skill line");
assert(constraint.includes("部署流程"), "names the knowledge page");
assert(constraint.includes(pages[0].id), "gives the assistant the page path");
assert(constraint.includes("不要把整个知识库翻一遍"), "tells the assistant not to browse the library");

const parsed = parseMentionsFromText("@knowledge:guides/deploy.md", [], []);
assert(parsed.length === 1 && parsed[0]?.kind === "knowledge", "reads a knowledge token");

console.log("mention-menu ok");
