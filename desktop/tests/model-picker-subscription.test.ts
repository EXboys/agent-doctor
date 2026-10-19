/**
 * Run from desktop/: node_modules/.bin/tsx tests/model-picker-subscription.test.ts
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
  const { subscriptionModelForSession } = await import("../src/chat/model-picker");
  const account = ["claude-sonnet-5", "claude-haiku-4-5"];

  const deepseek = subscriptionModelForSession(
    { model: "deepseek-flash", providerId: "provider-1" } as never,
    account,
  );
  assert(deepseek.model === "deepseek-flash", "deepseek model stays");
  assert(deepseek.replace === false, "a pinned provider is not replaced");

  const byName = subscriptionModelForSession({ model: "glm-5.3" } as never, account);
  assert(byName.replace === false && byName.model === "glm-5.3", "vendor model stays");

  const login = subscriptionModelForSession({ model: "gpt-5.4" } as never, account);
  assert(login.replace === true && login.model === "claude-sonnet-5", "other models join the account");

  const already = subscriptionModelForSession({ model: "claude-haiku-4-5" } as never, account);
  assert(already.replace === false && already.model === "claude-haiku-4-5", "account model stays");

  const waiting = subscriptionModelForSession(
    { model: "deepseek-flash", providerId: "provider-1" } as never,
    [],
  );
  assert(waiting.replace === false && waiting.model === "deepseek-flash", "empty catalog does not clear it");

  console.log("model-picker-subscription.test.ts ok");
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
