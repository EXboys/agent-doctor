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
  const {
    accountAllowanceTitle,
    accountSupportsModelList,
    loggedAccountCatalogFor,
    nativeProviderId,
    localClusterModelIds,
    pinSessionAfterFailover,
    providersWithLocalCluster,
    releaseStalePins,
    runtimeAcceptsProvider,
    sessionKeepsSavedService,
    sessionUsesLoggedAccount,
    subscriptionModelForSession,
  } = await import("../src/chat/model-picker");
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

  // Logged-in account gating: ONLY logged-in accounts appear; signed-out accounts never appear.
  const nativeRows = [
    {
      runtime_id: "claude-code",
      using_subscription: false,
      logged_in: true,
      reachable: true,
      visible: true,
      provider_name: "DeepSeek",
      models: ["claude-sonnet-5"],
    },
    {
      runtime_id: "codex",
      using_subscription: true,
      logged_in: false,
      reachable: true,
      visible: true,
      provider_name: "DeepSeek",
      models: ["gpt-5.6-sol"],
    },
    {
      runtime_id: "qoder",
      using_subscription: false,
      logged_in: true,
      reachable: true,
      visible: true,
      provider_name: "DeepSeek",
      models: [],
    },
    {
      runtime_id: "cursor",
      using_subscription: false,
      logged_in: false,
      reachable: true,
      visible: true,
      provider_name: "",
      models: [],
    },
  ];
  const claudeCat = loggedAccountCatalogFor("claude-code", nativeRows);
  assert(claudeCat !== null && claudeCat.providerId === "native:claude-code", "logged-in Claude account is selectable even while a personal provider is active");
  assert(loggedAccountCatalogFor("codex", nativeRows) === null, "un-logged-in Codex account has no option even if mode said using_subscription");
  const qoderCat = loggedAccountCatalogFor("qoder", nativeRows);
  assert(qoderCat !== null && qoderCat.providerId === "native:qoder", "logged-in Qoder account is selectable");
  assert(loggedAccountCatalogFor("cursor", nativeRows) === null, "un-logged-in Cursor account has no option");
  const cursorQuota = loggedAccountCatalogFor("cursor", [
    {
      runtime_id: "cursor",
      using_subscription: true,
      logged_in: true,
      reachable: true,
      visible: true,
      provider_name: "",
      models: ["auto"],
      quota_cursor_percent: 8,
      quota_other_percent: 53,
      model_sections: ["auto", "cursor", "other"],
    },
  ]);
  assert(
    cursorQuota?.quotaCursorPercent === 8 && cursorQuota?.quotaOtherPercent === 53,
    "signed-in Cursor account carries the two plan-page bars",
  );
  assert(
    cursorQuota?.modelSections.join(",") === "auto,cursor,other",
    "Cursor menu keeps Cursor models separate from other models",
  );
  const codexQuota = loggedAccountCatalogFor("codex", [
    {
      runtime_id: "codex",
      using_subscription: true,
      logged_in: true,
      reachable: true,
      visible: true,
      provider_name: "",
      models: ["gpt-5.3-codex", "gpt-5.2"],
      model_labels: ["GPT-5.3 Codex", "GPT-5.2"],
      model_sections: ["codex-coding", "codex-general"],
      quota_hours: 5,
      quota_hours_percent: 12,
      quota_week_percent: 40,
    },
  ]);
  assert(
    accountAllowanceTitle(codexQuota!) === "ChatGPT 账号 · 5 小时还剩 88% · 本周还剩 60%",
    "Codex shows how much of the shared allowance is left on the account title",
  );
  const qoderQuota = loggedAccountCatalogFor("qoder", [
    {
      runtime_id: "qoder",
      using_subscription: true,
      logged_in: true,
      reachable: true,
      visible: true,
      provider_name: "",
      models: ["auto"],
      quota_credits_left: 297,
    },
  ]);
  assert(
    accountAllowanceTitle(qoderQuota!) === "Qoder 账号 · 积分还剩 297",
    "Qoder shows the credits still left on the account title",
  );
  const zcodeQuota = loggedAccountCatalogFor("zcode", [
    {
      runtime_id: "zcode",
      using_subscription: true,
      logged_in: true,
      reachable: true,
      visible: true,
      provider_name: "",
      models: ["GLM-5.3"],
      quota_hours: 5,
      quota_hours_percent: 1,
      quota_week_percent: 3,
    },
  ]);
  assert(
    accountAllowanceTitle(zcodeQuota!) === "ZCode 账号 · 5 小时还剩 99% · 本周还剩 97%",
    "ZCode shows the 5-hour window and the week on the account title",
  );

  // Desktop-signed-in IDE without CLI authorization yet exposes a sync-pending catalog (not yet cliReady).
  const desktopOnlyRows = [
    {
      runtime_id: "cursor",
      using_subscription: false,
      logged_in: false,
      desktop_logged_in: true,
      desktop_needs_sync: true,
      reachable: true,
      visible: true,
      provider_name: "",
      models: [],
    },
  ];
  const cursorSyncCat = loggedAccountCatalogFor("cursor", desktopOnlyRows);
  assert(
    cursorSyncCat !== null && !cursorSyncCat.cliReady && cursorSyncCat.desktopNeedsSync,
    "signed-in Cursor desktop IDE surfaces sync-ready account tab in Ask",
  );
  assert(
    !sessionUsesLoggedAccount(
      { model: "auto", providerId: "native:cursor" } as never,
      cursorSyncCat,
    ),
    "chat does not route to native CLI before one-click sync completes",
  );

  // Explicit native pin vs saved service pin.
  const pinnedAccount = { model: "claude-sonnet-5", providerId: nativeProviderId("claude-code") } as never;
  assert(!sessionKeepsSavedService(pinnedAccount), "native account pin is not treated as a saved API-key service");
  assert(sessionUsesLoggedAccount(pinnedAccount, claudeCat), "explicit native pin routes to the logged-in account");
  assert(!sessionUsesLoggedAccount({ model: "deepseek-flash", providerId: "pp_ds" } as never, claudeCat), "explicit saved provider pin stays on that provider");
  assert(!sessionUsesLoggedAccount({ model: "", providerId: null } as never, claudeCat), "unpinned chat follows global provider when usingByDefault is false");

  // Provider compatibility & official sign-in gate.
  assert(
    !runtimeAcceptsProvider(
      "claude-code",
      { id: "teamups-official", url: "https://agentdoctor.cn/api/v1/official" },
      { officialSignedIn: false },
    ),
    "signed-out official account is hidden from provider picker",
  );
  assert(
    runtimeAcceptsProvider(
      "claude-code",
      { id: "teamups-official", url: "https://agentdoctor.cn/api/v1/official" },
      { officialSignedIn: true },
    ),
    "signed-in official account is selectable",
  );
  assert(
    !runtimeAcceptsProvider(
      "cursor",
      { id: "pp_ds", url: "https://api.deepseek.com/v1" },
      { officialSignedIn: true },
    ),
    "Cursor does not accept OpenAI-compatible saved providers",
  );

  // WorkBuddy and ZCode full subscription parity with Qoder.
  assert(accountSupportsModelList("workbuddy"), "WorkBuddy supports subscription model lists");
  assert(accountSupportsModelList("zcode"), "ZCode supports subscription model lists");

  const wbAndZcodeRows = [
    {
      runtime_id: "workbuddy",
      using_subscription: true,
      logged_in: true,
      reachable: true,
      visible: true,
      provider_name: "DeepSeek",
      models: ["hy4-preview", "deepseek-v4.1-flash", "glm-5.3"],
    },
    {
      runtime_id: "zcode",
      using_subscription: true,
      logged_in: true,
      reachable: true,
      visible: true,
      provider_name: "",
      models: ["GLM-5.3", "GLM-5.3-Flash", "GLM-5.2"],
    },
  ];
  const wbCat = loggedAccountCatalogFor("workbuddy", wbAndZcodeRows);
  const zcodeCat = loggedAccountCatalogFor("zcode", wbAndZcodeRows);
  assert(
    wbCat !== null && wbCat.models.includes("glm-5.3"),
    "signed-in WorkBuddy catalog exposes account models",
  );
  assert(
    sessionUsesLoggedAccount({ model: "glm-5.3", providerId: null } as never, wbCat),
    "WorkBuddy subscription session keeps native glm-5.3 instead of mistaking it for a saved provider",
  );
  assert(
    !sessionUsesLoggedAccount({ model: "deepseek-chat", providerId: "pp_ds" } as never, wbCat),
    "WorkBuddy chat explicitly pinned to a saved provider stays on that provider",
  );
  assert(
    zcodeCat !== null && sessionUsesLoggedAccount({ model: "GLM-5.3", providerId: null } as never, zcodeCat),
    "ZCode subscription session routes GLM-5.3 to ZCode native account",
  );

  const sessions = [
    { providerId: "minimax", model: "MiniMax-M3", providerTag: "minimax|minimax.io" },
    { providerId: "deepseek", model: "deepseek-reasoner", providerTag: "deepseek|api.deepseek.com" },
    { providerId: "native:claude-code", model: "claude-sonnet-5", providerTag: "native:claude-code|native:claude-code" },
    { providerId: null, model: "", providerTag: null },
  ];
  assert(releaseStalePins(sessions, "deepseek") === true, "a switch with stale pins reports a change");
  assert(sessions[0]?.providerId === null && sessions[0]?.model === "" && sessions[0]?.providerTag === null, "the minimax pin is released");
  assert(sessions[1]?.providerId === "deepseek" && sessions[1]?.model === "deepseek-reasoner", "a pin on the active provider stays");
  assert(sessions[2]?.providerId === "native:claude-code", "a pin on a signed-in subscription account stays across personal provider switches");
  assert(releaseStalePins(sessions, "deepseek") === false, "a second pass finds nothing to release");
  assert(releaseStalePins([{ providerId: "minimax", model: "MiniMax-M3" }], null) === false, "no active provider releases nothing");

  const failingChat = {
    providerId: null as string | null,
    model: "",
    providerTag: "claude|api.anthropic.com" as string | null,
    runtimeThreadId: "thread-old" as string | null,
  };
  pinSessionAfterFailover(
    failingChat,
    [{ id: "deepseek", url: "https://api.deepseek.com/v1" }],
    "deepseek",
    "deepseek-chat",
  );
  assert(failingChat.providerId === "deepseek", "failover pins only this chat");
  assert(failingChat.model === "deepseek-chat", "failover pins the backup model");
  assert(failingChat.runtimeThreadId === null, "old provider's native thread is dropped");
  assert(failingChat.providerTag === "deepseek|api.deepseek.com", "tag tracks the backup host");

  const saved = [{ id: "teamups-official" }, { id: "native-looking" }, { id: "local-cluster" }];
  const withLocal = providersWithLocalCluster(saved, { id: "local-cluster", model: "qwen" });
  assert(
    withLocal.map((item) => item.id).join(",") === "teamups-official,native-looking,local-cluster",
    "a local model is added beside official and saved services",
  );
  assert(withLocal[2]?.model === "qwen", "the live local row replaces the saved one");
  const kept = providersWithLocalCluster(saved.slice(0, 2), null);
  assert(kept.length === 2 && kept[0]?.id === "teamups-official", "no local model leaves the old list alone");
  const llamaIds = localClusterModelIds({
    backend: "llama_cpp",
    active_models: ["qwen"],
    installed_models: ["extra"],
  } as never);
  assert(llamaIds.join(",") === "qwen,extra", "a running local model stays selectable");

  console.log("model-picker-subscription.test.ts ok");
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
