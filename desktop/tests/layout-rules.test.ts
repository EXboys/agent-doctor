/**
 * Run from desktop/: node --import tsx tests/layout-rules.test.ts
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { chatSideDefault, heroRoom, heroShouldShow, heroSpare, layout } from "../src/layout-rules.ts";

assert.equal(layout.homeEdgeMargin, 48);
assert.equal(layout.chatSide, 250);
assert.equal(layout.chatSideWindows, 272);

const screen = 1040;

assert.equal(chatSideDefault("windows"), layout.chatSideWindows);
assert.equal(chatSideDefault("macos"), layout.chatSide);
assert.equal(chatSideDefault(undefined), layout.chatSide);

// The big card appears when there is room, and stays inside the band.
assert.equal(heroShouldShow(heroRoom.show, false), true);
assert.equal(heroShouldShow(heroRoom.show - 1, false), false);
// Once painted it survives a shrinking window until the page is really tight.
assert.equal(heroShouldShow(heroRoom.show - 1, true), true);
assert.equal(heroShouldShow(heroRoom.hide + 1, true), true);
assert.equal(heroShouldShow(heroRoom.hide, true), false);
// No window height can flip the card twice: re-deciding keeps the answer.
for (let spare = 0; spare <= 400; spare += 4) {
  for (const shown of [true, false]) {
    const once = heroShouldShow(spare, shown);
    assert.equal(heroShouldShow(spare, once), once, `flip at spare=${spare}`);
  }
}
assert.ok(heroRoom.hide < heroRoom.show);

// The big block is taken back out before the room is judged, so a card that
// grows into the panel cannot shrink its own spare height.
assert.equal(heroSpare({ panel: 400, card: 300, hero: 0, gap: 0 }), 100);
assert.equal(heroSpare({ panel: 400, card: 300, hero: 80, gap: 12 }), 192);
// A card taller than the panel leaves no room, never a negative one to show.
assert.equal(heroSpare({ panel: 200, card: 300, hero: 0, gap: 0 }), -100);

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
