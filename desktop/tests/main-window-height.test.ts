/**
 * Run from desktop/: node --import tsx tests/main-window-height.test.ts
 */
import assert from "node:assert/strict";
import { dockedHomeHeight } from "../src/main-window-height.ts";

const screen = 1040;

// Halfway from the tallest tab toward the screen, under the Ask window.
assert.equal(dockedHomeHeight(520, screen, 980), 780);

// Ask is shorter than the midpoint, so the home stops at Ask.
assert.equal(dockedHomeHeight(520, screen, 600), 600);

// Tallest tab already fills the room under the screen margins: use that room.
assert.equal(dockedHomeHeight(1000, screen, 980), 1040 - 48 * 2);

// Shorter than Ask, but the screen margin still caps it.
assert.equal(dockedHomeHeight(900, screen, 1200), 1040 - 48 * 2);

console.log("main-window-height.test.ts ok");
