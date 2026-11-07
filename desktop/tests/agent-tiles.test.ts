/**
 * The home's agent tiles: the built-in doctor is an agent, so a machine with
 * a single CLI still gets the doctor tile. A fresh Windows box with only
 * Claude Code used to lose it, and the built-in agent became unreachable.
 * Run from desktop/: node --import tsx tests/agent-tiles.test.ts
 */

import assert from "node:assert/strict";

const { planAgentTiles, HOME_AGENT_VISIBLE, DOCTOR_RUNTIME_ID } = await import("../src/agent-tiles");

// Solo machine: one CLI installed.
const solo = planAgentTiles(["claude-code"], "claude-code", false);
assert.equal(solo.doctor, true, "a solo machine still shows the doctor tile");
assert.equal(solo.visibleCount, 1, "the one installed CLI is painted");
assert.equal(solo.overflow, false, "one CLI never overflows");

// Nothing installed: the empty state owns the layout, still no CLI tiles.
const empty = planAgentTiles([], "claude-code", false);
assert.equal(empty.doctor, true, "the doctor tile survives an empty machine");
assert.equal(empty.visibleCount, 0, "no CLI tiles without a CLI");
assert.equal(empty.overflow, false, "nothing to overflow");

// Two CLIs: both painted, no overflow.
const pair = planAgentTiles(["claude-code", "codex"], "claude-code", false);
assert.equal(pair.doctor, true, "the doctor tile comes first");
assert.equal(pair.visibleCount, 2, "both CLIs are painted");
assert.equal(pair.overflow, false, "two CLIs fit");

// A full home: the doctor takes the first slot, the rest overflow.
const many = ["a", "b", "c", "d", "e", "f", "g"];
const packed = planAgentTiles(many, "a", false);
assert.equal(packed.overflow, true, "more CLIs than tiles overflow");
assert.equal(packed.visibleCount, HOME_AGENT_VISIBLE - 1, "the doctor takes one tile");
assert.equal(packed.hiddenCount, many.length - (HOME_AGENT_VISIBLE - 1), "the rest hide behind more");
assert.equal(packed.expandForSelection, false, "a visible selection does not expand the list");

// Selecting an agent past the window expands the list to reveal it.
const hidden = planAgentTiles(many, "g", false);
assert.equal(hidden.expandForSelection, true, "a hidden selection expands the list");
assert.equal(hidden.visibleCount, many.length, "the expanded list shows every CLI");

// The user opened the overflow list themselves.
const opened = planAgentTiles(many, "a", true);
assert.equal(opened.visibleCount, many.length, "the opened list shows every CLI");
assert.equal(opened.hiddenCount, 0, "an opened list hides nothing");
assert.equal(opened.expandForSelection, false, "opening the list is not a selection change");

// The doctor is never a CLI tile, so its own selection keeps the list packed.
const doctorSelected = planAgentTiles(many, DOCTOR_RUNTIME_ID, false);
assert.equal(doctorSelected.expandForSelection, false, "the doctor is not a hidden CLI");
assert.equal(doctorSelected.visibleCount, HOME_AGENT_VISIBLE - 1, "the list stays packed");

console.log("agent tiles ok");
