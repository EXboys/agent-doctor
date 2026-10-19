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
  const { explainChatFailure, missingProviderFieldsMessage } = await import("../src/friendly-error");

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
  assert(mac!.actions.includes("privacy"), "macos denied offers system settings");

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
  assert(limited!.actions.includes("askAgain"), "rate limit offers ask again");
  assert(missingProviderFieldsMessage(["url"]) === "还缺地址。", "only a missing address is named");
  assert(
    !missingProviderFieldsMessage(["url"]).includes("密钥"),
    "address-only hint does not mention the key",
  );

  const unreachable = explainChatFailure("connect ECONNREFUSED 127.0.0.1:18789");
  assert(
    unreachable?.kind === "provider_url" || unreachable?.kind === "network",
    "connection refused is an address problem",
  );
  assert(!unreachable!.actions.includes("repair"), "unreachable address does not offer a settings apply");
  assert(unreachable!.actions.includes("provider"), "unreachable address offers the provider button");

  const expired = explainChatFailure(
    'HTTP 401: {"error":{"code":"token_expired","message":"Provided authentication token is expired. Please try signing in again."}}',
  );
  assert(expired?.kind === "session_expired", "an expired sign-in is not a pasted key");
  assert(expired!.actions.length === 1 && expired!.actions[0] === "relogin", "expired sign-in offers signing in again");
  assert(!expired!.message.includes("密钥") && !expired!.message.includes("401"), "expired sign-in stays one plain sentence");

  const refresh = explainChatFailure(
    "2026-10-16T13:21:46.927396Z ERROR auth::manager: Failed to refresh token: Your access token could not be refreshed.",
  );
  assert(refresh?.kind === "session_expired", "a failed token refresh is the same expired sign-in");

  const accountModel = explainChatFailure(
    [
      "2026-10-18T03:40:45.466165Z ERROR codex_api::endpoint::responses_websocket: failed to connect to websocket: IO error: Connection reset by peer (os error 54), url: wss://chatgpt.com/backend-api/codex/responses",
      "Reconnecting... 5/5",
      '{"detail":"The \'glm-5.3\' model is not supported when using Codex with a ChatGPT account."}',
    ].join("\n"),
  );
  assert(accountModel?.kind === "account_model", "an account that rejects the model is not a dead connection");
  assert(accountModel!.actions.length === 1 && accountModel!.actions[0] === "askAgain", "account model offers asking again");
  assert(
    !accountModel!.message.includes("glm") && !accountModel!.message.includes("websocket") && !accountModel!.next.includes("密钥"),
    "account model copy hides the log and does not send the user to a key",
  );
  const resets = explainChatFailure(
    "2026-10-18T03:40:45.466165Z ERROR codex_api::endpoint::responses_websocket: failed to connect to websocket: Connection reset by peer",
  );
  assert(resets?.kind === "network", "a connection reset without a model refusal stays a network failure");

  const unknown = explainChatFailure("unexpected status 500 Internal Server Error");
  assert(unknown?.kind === "unknown", "bare 500 stays unknown");
  assert(!unknown!.actions.includes("repair"), "unknown failure does not offer a settings apply");

  const { autoFixNoticeKey, failurePrimary, failsNeedAddress, settingsApplyKind } = await import(
    "../src/diagnose-flow"
  );
  assert(
    failurePrimary({ can_apply_repair: true, summary: { fail: 2 } }, false) === "apply-settings",
    "auto-fixable failures offer a settings apply",
  );
  assert(
    failurePrimary({ can_apply_repair: false, summary: { fail: 1 } }, false) === "check-address",
    "gateway-only failures send the user to check the address",
  );
  assert(
    failurePrimary({ can_apply_repair: true, summary: { fail: 0 } }, false) === "apply-settings",
    "a saved key with an auto-fixable warning still offers the settings button",
  );
  assert(
    failurePrimary({ can_apply_repair: true, summary: { fail: 0 } }, true) === null,
    "after the settings button was already used, it is not offered again",
  );
  assert(
    failurePrimary(
      {
        can_apply_repair: false,
        summary: { fail: 1 },
        checks: [{ id: "binary.version", status: "fail", message: "--version timed out after 1.5s" }],
      },
      false,
    ) === null,
    "a slow version check does not send the user to fill the key",
  );
  assert(
    !failsNeedAddress({
      summary: { fail: 1 },
      checks: [{ id: "binary.version", status: "fail", message: "timed out" }],
    }),
    "version timeout is not an address problem",
  );
  assert(
    autoFixNoticeKey({
      suggested_repairs: [{ id: "fix-hermes-browser-mcp", auto_fixable: true }],
    }) === "diagnose.flow.scoreNeedsBrowser",
    "only a browser-tool write names the browser tool",
  );

  const { plainRepairCheckCopy, relatedIssueLabel } = await import("../src/repair-plain");
  const slow = plainRepairCheckCopy({
    id: "binary.version",
    title: "Version command",
    status: "warn",
    message: "--version timed out after 1.5s",
  });
  assert(slow.title === "这一下没读到版本", `slow version title: ${slow.title}`);
  assert(slow.message.includes("已经装好了"), `slow version message: ${slow.message}`);
  assert(!slow.message.includes("强制重装"), "a slow version check does not ask for a reinstall");
  const unread = plainRepairCheckCopy({
    id: "binary.version",
    title: "Version command",
    status: "n/a",
    message: "version command ran but returned no output",
  });
  assert(unread.title === "这一下没读到版本", `unread version title: ${unread.title}`);
  const browserTag = relatedIssueLabel("mcp", "browser");
  assert(browserTag === "浏览器工具还没接上", `browser tag: ${browserTag}`);
  assert(!browserTag.includes("MCP"), "the browser tag does not say MCP");
  assert(
    settingsApplyKind({ summary: { fail: 1 }, last_execute: { executed: ["backup-runtime-configs"] } }) ===
      "nothing",
    "backup alone is not a settings change",
  );
  assert(
    settingsApplyKind({ summary: { fail: 1 }, last_execute: { executed: ["fix-codex-url"] } }) === "still-fail",
    "a write that leaves failures does not count as fixed",
  );
  assert(
    settingsApplyKind({ summary: { fail: 0 }, last_execute: { executed: ["fix-codex-url"] } }) === "changed",
    "a write that clears failures is a settings change",
  );

  const { explainProviderFailure, accountBlockMessage } = await import("../src/friendly-error");
  const provider = explainProviderFailure(balanceRaw, { statusCode: 429 });
  assert(provider.kind === "balance", "provider verify should say balance, not a bad key");
  const banner = accountBlockMessage("balance");
  assert(banner.includes("去服务商"), "diagnose banner names the next button");
  assert(!banner.includes("LLM HTTP"), "diagnose banner hides the raw dump");

  console.log("chat-failure.test.ts OK");
}

void main();
