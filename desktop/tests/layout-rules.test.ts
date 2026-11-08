/**
 * Run from desktop/: node --import tsx tests/layout-rules.test.ts
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { chatSideDefault, dockedHomeHeight, fittedHomeHeight, layout } from "../src/layout-rules.ts";

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

// A home window that stands alone ends with its content.
assert.equal(fittedHomeHeight(520, screen), 520);
assert.equal(fittedHomeHeight(200, screen), 480);
assert.equal(fittedHomeHeight(2000, screen), screen - layout.homeEdgeMargin * 2);
assert.equal(fittedHomeHeight(520, 100), 4);

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

// The Windows home fills the tall window: the agent card grows into the
// leftover height, and nothing shrinks below its content.
const styles = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "../src/styles.css"), "utf8");

const winAgents = block(styles, 'html[data-os="windows"] .agents-main');
assert.match(winAgents, /flex:\s*1 0 auto/);
assert.match(winAgents, /min-height:\s*0/);

const winPanels = block(styles, 'html[data-os="windows"] .runtime-panels');
assert.match(winPanels, /display:\s*flex/);

const winCard = block(styles, 'html[data-os="windows"] .runtime-panels > article.runtime:last-of-type');
assert.match(winCard, /flex:\s*1 0 auto/);
assert.match(winCard, /min-height:\s*0/);

const winScrollbar = block(styles, 'html[data-os="windows"] .main-panels::-webkit-scrollbar');
assert.equal(px(winScrollbar, "width"), 8);

console.log("layout-rules.test.ts ok");
