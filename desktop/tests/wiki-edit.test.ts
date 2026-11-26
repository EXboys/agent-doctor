/**
 * Run from desktop/: node --import tsx tests/wiki-edit.test.ts
 */
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { renderMarkdown } from "../src/markdown.ts";
import {
  htmlToMarkdown,
  joinWikiPage,
  nextNotePath,
  plainLead,
  splitWikiPage,
  upsertIndexLink,
} from "../src/chat/wiki-edit.ts";

function fromHtml(html: string): string {
  const { document } = parseHTML(`<!doctype html><div id="root"></div>`);
  const root = document.getElementById("root");
  assert.ok(root);
  root.innerHTML = html;
  return htmlToMarkdown(root);
}

function normalize(markdown: string): string {
  return fromHtml(renderMarkdown(markdown, { wikiLinks: true }));
}

const page = [
  "本页是目录。每个页面出现一次。",
  "",
  "## 来源",
  "",
  "- 一段话，含 `code` 和 **粗体**，以及 [规格](guides/spec.md)。",
  "- 外链看 [文档](https://example.com/doc)。",
  "",
  "> 这里有冲突",
  "",
  "```ts",
  "const n = 1",
  "```",
  "",
  "| 名称 | 说明 |",
  "| --- | --- |",
  "| A | 一 |",
].join("\n");

const once = normalize(page);
const twice = normalize(once);
assert.equal(twice, once, "editing round-trip stays stable");
assert.match(once, /## 来源/);
assert.match(once, /`code`/);
assert.match(once, /\*\*粗体\*\*/);
assert.match(once, /\[规格\]\(guides\/spec\.md\)/);
assert.match(once, /\[文档\]\(https:\/\/example.com\/doc\)/);
assert.match(once, /> 这里有冲突/);
assert.match(once, /```ts\nconst n = 1\n```/);
assert.match(once, /\| A \| 一 \|/);

assert.equal(
  fromHtml("<div>第一段</div><div><br></div><div>第二段</div>"),
  "第一段\n\n第二段",
);

assert.deepEqual(splitWikiPage("# 知识库索引\n\n正文\n"), { title: "知识库索引", body: "正文" });
assert.equal(joinWikiPage("知识库索引", "正文"), "# 知识库索引\n\n正文\n");
assert.equal(joinWikiPage("  只有标题  ", "  \n"), "# 只有标题\n");

assert.equal(nextNotePath([]), "notes/note.md");
assert.equal(nextNotePath(["notes/note.md", "notes/note-2.md"]), "notes/note-3.md");

assert.equal(plainLead("## 跳过\n\n- 手写的第一句，很长。"), "手写的第一句，很长。");

const index = "# 知识库索引\n\n- [旧标题](notes/note.md) — 原摘要\n";
assert.equal(
  upsertIndexLink(index, "notes/note.md", "新标题", "", true),
  "# 知识库索引\n\n- [新标题](notes/note.md) — 原摘要\n",
);
assert.equal(upsertIndexLink(index, "notes/note.md", "新标题", "", false), index);
assert.equal(
  upsertIndexLink(index, "notes/note-2.md", "另一页", "手写的一句", true),
  "# 知识库索引\n\n- [旧标题](notes/note.md) — 原摘要\n- [另一页](notes/note-2.md) — 手写的一句\n",
);
assert.equal(upsertIndexLink(index, "index.md", "目录"), index);
assert.equal(
  upsertIndexLink("# 目录\n", "notes/note.md", "价格 $100", "", true),
  "# 目录\n- [价格 $100](notes/note.md)\n",
);

console.log("wiki-edit ok");
