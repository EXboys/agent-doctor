/**
 * Run from desktop/: node_modules/.bin/tsx tests/active-session-runtime.test.ts
 */

const storage = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  writable: true,
  value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => {
      storage.set(key, value);
    },
    removeItem: (key: string) => {
      storage.delete(key);
    },
  },
});

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
}

async function main() {
  const { STORAGE_KEY } = await import("../src/chat/types");
  const { readActiveSessionRuntime } = await import("../src/chat/store");

  assert(readActiveSessionRuntime() === null, "missing store");

  localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify({
      activeId: "b",
      sessions: [
        { id: "a", runtime: "claude-code" },
        { id: "b", runtime: "openclaw" },
      ],
    }),
  );
  assert(readActiveSessionRuntime() === "openclaw", "active session runtime");

  localStorage.setItem(STORAGE_KEY, "{");
  assert(readActiveSessionRuntime() === null, "broken json");

  localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify({
      activeId: "gone",
      sessions: [{ id: "a", runtime: "codex" }],
    }),
  );
  assert(readActiveSessionRuntime() === null, "active id missing");

  console.log("active-session-runtime ok");
}

void main();
