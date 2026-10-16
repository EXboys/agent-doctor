/**
 * Run from desktop/: node --import tsx tests/hand-missing.test.ts
 */
const storage = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  writable: true,
  value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
    removeItem: (key: string) => storage.delete(key),
  },
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  writable: true,
  value: { language: "zh" },
});

const { handMissingNotice } = await import("../src/chat/format");

function assert(cond: boolean, msg: string) {
  if (!cond) {
    console.error("FAIL:", msg);
    process.exit(1);
  }
}

const old = handMissingNotice(
  "computer:cli\ncomputer MCP skipped: the installed Agent Doctor CLI does not include computer tools yet",
);
assert(old.sentence.includes("检查更新"), "missing computer part points at Check for updates");
assert(!/MCP|CLI/.test(old.sentence), "the sentence has no MCP or CLI words");
assert(old.detail.startsWith("computer MCP skipped"), "the original error stays for the log");

const chrome = handMissingNotice("browser:chrome\nbrowser MCP skipped: Chrome not found (x)");
assert(chrome.sentence.includes("Chrome"), "no Chrome says to install Chrome");
assert(chrome.sentence.includes("网页"), "browser sentence talks about web pages");

const other = handMissingNotice("computer:other\ncomputer MCP wire failed for codex: denied");
assert(other.sentence.includes("这台电脑"), "other computer failure names this computer");
assert(other.detail.includes("denied"), "other failure keeps the original");

const bare = handMissingNotice("browser:other");
assert(bare.detail === "", "no detail when only the head line arrives");

console.log("hand-missing ok");
