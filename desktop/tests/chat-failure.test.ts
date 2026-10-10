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

  const denied =
    "Error: error loading default config after config error: Operation not permitted (os error 1)";
  const mac = explainChatFailure(denied);
  assert(mac?.kind === "macos_denied", "codex eperm should map to macos_denied");
  assert(mac!.actions.length === 0, "macos denied has no in-app fix button");

  const noise = explainChatFailure("session_id: abc");
  assert(noise === null, "benign stderr should not classify");

  const planRaw =
    'LLM HTTP 429 Too Many Requests url: https://open.bigmodel.cn/api/paas/v4/chat/completions {"error":{"code":"1113","message":"余额不足或无可用资源包,请充值。"}}';
  const plan = explainChatFailure(planRaw);
  assert(plan?.kind === "glm_coding_plan", "coding-plan key on payg url should not look like a real empty balance");
  assert(plan!.next.includes("编程套餐"), "coding-plan failure tells the user which key source to pick");

  const qwenPlan = explainChatFailure(
    "HTTP 401 invalid api-key url: https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions",
  );
  assert(qwenPlan?.kind === "glm_coding_plan", "qwen coding key on payg should offer the plan switch");

  const balanceRaw =
    'LLM HTTP 429 Too Many Requests: {"error":{"code":"1113","message":"余额不足或无可用资源包,请充值。"}}';
  const balance = explainChatFailure(balanceRaw);
  assert(balance?.kind === "account_balance", "zhipu 1113 should be a balance block");
  assert(balance!.actions.length === 1 && balance!.actions[0] === "provider", "balance offers provider only");
  assert(!balance!.message.includes("429"), "balance copy hides the raw status");
  assert(balance!.message.includes("不是本软件"), "balance copy says the app is fine");

  const limited = explainChatFailure("LLM HTTP 429 Too Many Requests");
  assert(limited?.kind === "rate_limit", "bare 429 should be rate limit");
  assert(limited!.actions.length === 0, "rate limit has no repair button");

  const { explainProviderFailure, accountBlockMessage } = await import("../src/friendly-error");
  const provider = explainProviderFailure(balanceRaw, { statusCode: 429 });
  assert(provider.kind === "balance", "provider verify should say balance, not a bad key");
  const banner = accountBlockMessage("balance");
  assert(banner.includes("去服务商"), "diagnose banner names the next button");
  assert(!banner.includes("LLM HTTP"), "diagnose banner hides the raw dump");

  console.log("chat-failure.test.ts OK");
}

void main();
