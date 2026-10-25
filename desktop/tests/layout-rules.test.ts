/**
 * Run from desktop/: node --import tsx tests/layout-rules.test.ts
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { chatSideDefault, dockedHomeHeight, layout } from "../src/layout-rules.ts";

assert.equal(layout.homeEdgeMargin, 48);
assert.equal(layout.chatSide, 250);
assert.equal(layout.chatSideWindows, 272);

const screen = 1040;

assert.equal(dockedHomeHeight(520, screen, 980), 780);
assert.equal(dockedHomeHeight(520, screen, 600), 600);
assert.equal(dockedHomeHeight(1000, screen, 980), screen - layout.homeEdgeMargin * 2);
assert.equal(dockedHomeHeight(900, screen, 1200), screen - layout.homeEdgeMargin * 2);

assert.equal(chatSideDefault("windows"), layout.chatSideWindows);
assert.equal(chatSideDefault("macos"), layout.chatSide);
assert.equal(chatSideDefault(undefined), layout.chatSide);

const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "../src/chat.css"), "utf8");

function block(source: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = source.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  assert.ok(match, `missing ${selector}`);
  return match[1] ?? "";
}

function px(body: string, name: string): number {
  const match = body.match(new RegExp(`(?:^|\\n)\\s*${name}:\\s*(\\d+)px`));
  assert.ok(match, `missing ${name}`);
  return Number(match[1]);
}

const root = block(css, ":root");
assert.equal(px(root, "--chat-side-w"), layout.chatSide);
assert.equal(px(root, "--chat-split-chat-min"), layout.chatSplit);
assert.equal(px(root, "--chat-split-chat-w"), layout.chatSplit);
assert.equal(px(root, "--chat-splitter-w"), layout.splitter);

const windows = block(css, 'html[data-os="windows"]');
assert.equal(px(windows, "--chat-side-w"), layout.chatSideWindows);

console.log("layout-rules.test.ts ok");
