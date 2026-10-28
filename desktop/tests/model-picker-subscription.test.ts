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

  const { releaseStalePins } = await import("../src/chat/model-picker");
  const sessions = [
    { providerId: "minimax", model: "MiniMax-M3", providerTag: "minimax|minimax.io" },
    { providerId: "deepseek", model: "deepseek-reasoner", providerTag: "deepseek|api.deepseek.com" },
    { providerId: null, model: "", providerTag: null },
  ];
  assert(releaseStalePins(sessions, "deepseek") === true, "a switch with stale pins reports a change");
  assert(sessions[0]?.providerId === null && sessions[0]?.model === "" && sessions[0]?.providerTag === null, "the minimax pin is released");
  assert(sessions[1]?.providerId === "deepseek" && sessions[1]?.model === "deepseek-reasoner", "a pin on the active provider stays");
  assert(releaseStalePins(sessions, "deepseek") === false, "a second pass finds nothing to release");
  assert(releaseStalePins([{ providerId: "minimax", model: "MiniMax-M3" }], null) === false, "no active provider releases nothing");

  console.log("model-picker-subscription.test.ts ok");
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
