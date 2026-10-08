/**
 * Run from desktop/: node_modules/.bin/tsx tests/chat-failure.test.ts
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
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  writable: true,
  value: { language: "zh" },
});

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
}

async function main() {
  const { explainChatFailure } = await import("../src/friendly-error");

  const glm404 =
    "unexpected status 404 Not Found: Unknown error, url: https://open.bigmodel.cn/api/paas/v4/responses";
  const glm = explainChatFailure(glm404);
  assert(glm?.kind === "glm_codex_url", "glm 404 should map to glm_codex_url");
  assert(glm!.actions.includes("repair"), "glm should offer repair");

  const anthropic =
    "Unable to connect to Anthropic services Failed to connect to api.anthropic.com: Status 403";
  const geo = explainChatFailure(anthropic);
  assert(geo?.kind === "geo_blocked", "anthropic 403 should map to geo_blocked");

  const noise = explainChatFailure("session_id: abc");
  assert(noise === null, "benign stderr should not classify");

  console.log("chat-failure.test.ts OK");
}

void main();
