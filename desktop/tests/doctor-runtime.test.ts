/**
 * The built-in doctor agent is a chat runtime with no binary: it must pass the
 * ask-runtime type guard, carry a display name, and always appear in the
 * runtime catalog (appended even when the Rust catalog fetch fails, because
 * a machine with no agents installed is exactly where it matters most).
 * Run from desktop/: node --import tsx tests/doctor-runtime.test.ts
 */

import assert from "node:assert/strict";

const data = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
  value: {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => {
      data.set(key, value);
    },
    removeItem: (key: string) => {
      data.delete(key);
    },
    clear: () => data.clear(),
  },
});

const { isAskRuntime, runtimeDisplayName } = await import("../src/chat/runtime");
const catalog = await import("../src/runtime-catalog");

assert.equal(isAskRuntime("doctor"), true, "doctor is an ask runtime");
assert.equal(isAskRuntime("claude-code"), true, "external runtimes still pass");
assert.equal(isAskRuntime("not-a-runtime"), false);

const name = runtimeDisplayName("doctor");
assert.ok(name.includes("Agent Doctor"), `display name mentions Agent Doctor: ${name}`);
assert.equal(runtimeDisplayName("codex"), "Codex", "existing names unchanged");

// No Tauri runtime here: the fetch fails, the catalog falls back to empty,
// and the doctor entry is still appended.
await catalog.loadRuntimeCatalog();
const entries = catalog.runtimeCatalog().filter((entry) => entry.id === "doctor");
assert.equal(entries.length, 1, "doctor appears exactly once in the catalog");
assert.equal(entries[0].ask, true, "doctor is ask-capable");
assert.equal(entries[0].opens_app, false, "doctor is not a desktop-app runtime");
assert.equal(entries[0].browser_mcp, false, "doctor has no browser mcp");
assert.equal(entries[0].skill_mount, false, "doctor has no skill mount");

console.log("doctor runtime ok");
