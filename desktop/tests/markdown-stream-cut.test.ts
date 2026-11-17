import assert from "node:assert/strict";

import { markdownCutInfo, renderMarkdown, restoreCollapsedMarkdown, safeMarkdownCut } from "../src/markdown";

// No safe boundary yet: a single unfinished paragraph stays entirely in the tail.
assert.equal(safeMarkdownCut("just one line"), 0);
assert.equal(safeMarkdownCut("line one\nline two"), 0);

// A blank line ends the paragraph — everything up to it freezes.
assert.equal(safeMarkdownCut("para one\n\npara two"), "para one\n".length);
assert.equal(
  safeMarkdownCut("para one\n\npara two\n\npara three"),
  "para one\n\npara two\n".length,
);

// Trailing blanks freeze wholesale: the tail is empty.
assert.equal(safeMarkdownCut("para one\n\n\n\n"), "para one\n\n\n\n".length);

// An open fence blocks every boundary inside it; a closed one reopens them.
assert.equal(safeMarkdownCut("intro\n\n```js\nconst a = 1;\nmore code"), "intro\n".length);
assert.equal(
  safeMarkdownCut("intro\n\n```js\nconst a = 1;\n```\n\nafter code"),
  "intro\n\n```js\nconst a = 1;\n```\n".length,
);

// A list, quote or table keeps growing past the blank line — no cut there.
assert.equal(safeMarkdownCut("intro\n\n- item one\n- item two"), "intro\n".length);
assert.equal(safeMarkdownCut("intro\n\n1. first\n2. second"), "intro\n".length);
assert.equal(safeMarkdownCut("intro\n\n> quoted"), "intro\n".length);
assert.equal(safeMarkdownCut("intro\n\n| a | b |\n| - | - |"), "intro\n".length);

// But the paragraph after a list ends at its own boundary.
assert.equal(
  safeMarkdownCut("intro\n\n- item one\n\nafter list"),
  "intro\n\n- item one\n".length,
);

// An unclosed fence reports fenced=true: the streaming tail renders as
// plain code text instead of re-parsing the whole fence every tick.
{
  const open = "intro\n\n```bash\nnpm install\nnpm run build\nmore lines coming";
  const info = markdownCutInfo(open);
  assert.equal(info.cut, "intro\n".length);
  assert.equal(info.fenced, true);
}
{
  // The closer moves the cut past the whole fence and clears the flag.
  const closed = "intro\n\n```bash\nnpm install\n```\n\nafter the fence";
  const info = markdownCutInfo(closed);
  assert.equal(info.cut, "intro\n\n```bash\nnpm install\n```\n".length);
  assert.equal(info.fenced, false);
}

// Phone archive once stripped newlines; restore mid-line ## / 1. markers.
{
  const flat =
    "诊断完成。已跑通。 ## 当前状态 线路正常。 ## 值得优化的 4 点 1. 睡眠会拖住 cron 2. 没有幂等保护";
  const restored = restoreCollapsedMarkdown(flat);
  assert.match(restored, /\n\n## 当前状态/);
  assert.match(restored, /\n\n## 值得优化的 4 点/);
  assert.match(restored, /\n1\. 睡眠会拖住 cron/);
  assert.match(restored, /\n2\. 没有幂等保护/);
  const html = renderMarkdown(flat);
  assert.match(html, /<h2>/);
  assert.match(html, /<ol>/);
  assert.doesNotMatch(html, /## 当前状态/);
}
