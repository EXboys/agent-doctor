import assert from "node:assert/strict";

import { safeMarkdownCut } from "../src/markdown";

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
