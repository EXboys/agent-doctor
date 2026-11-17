/**
 * The phone's agent / model picker rules.
 * Run from desktop/: node --import tsx tests/phone-agents.test.ts
 */

import {
  agentRows,
  applyPick,
  modelRows,
  phoneInventoryProviders,
  pickLabel,
  pickOf,
} from "../src/chat/phone-agents";

function assert(cond: boolean, message: string): void {
  if (!cond) throw new Error(message);
}

const agents = agentRows([
  { id: "claude-code", label: "Claude Code" },
  { id: "hermes", label: "" },
  { id: "claude-code", label: "dup" },
  { id: "", label: "blank" },
]);
assert(agents.length === 2, `agents deduped: ${JSON.stringify(agents)}`);
assert(agents[1].name === "hermes", "an agent without a label falls back to its id");

const models = modelRows(
  [
    { id: "prov-a", model: "deepseek-flash", models: ["deepseek-v4-pro", "deepseek-flash"] },
    { id: "prov-b", model: "glm-5.3", models: [] },
  ],
  (id) => (id === "deepseek-flash" ? "DeepSeek V4.1-Flash" : ""),
  "hermes",
);
assert(
  models.map((row) => row.id).join(",") === "deepseek-flash,deepseek-v4-pro,glm-5.3",
  `models keep provider order and dedupe: ${models.map((row) => row.id).join(",")}`,
);
assert(models[0].label === "DeepSeek V4.1-Flash", "labels ride along");
assert(models[0].agent === "hermes", "the agent hint is carried");
assert(models[0].provider === "prov-a", "the provider rides along so the pin is exact");
assert(models[2].provider === "prov-b", "a second provider keeps its own id");
// The same model id on two providers is two rows, not one: the key differs.
const twice = modelRows(
  [
    { id: "prov-a", models: ["same"] },
    { id: "prov-b", models: ["same"] },
  ],
  (id) => id,
);
assert(twice.length === 2, `same model on two providers stays two rows: ${twice.length}`);
const capped = modelRows([{ models: Array.from({ length: 60 }, (_, i) => `m${i}`) }], (id) => id);
assert(capped.length === 40, `model rows are capped: ${capped.length}`);

// Phone inventory gating: ONLY signed-in accounts appear; signed-out accounts are omitted.
const invSignedOut = phoneInventoryProviders({
  installedIds: new Set(["doctor", "claude-code", "codex", "qoder", "cursor"]),
  personalProviders: [
    { id: "teamups-official", name: "官方", url: "https://agentdoctor.cn/api/v1/official", model: "deepseek-flash" },
    { id: "pp_ds", name: "DeepSeek", url: "https://api.deepseek.com/v1", model: "deepseek-flash" },
  ],
  nativeRows: [
    { runtime_id: "claude-code", logged_in: true, models: ["claude-sonnet-5"] },
    { runtime_id: "codex", logged_in: false, models: ["gpt-5.6-sol"] },
    { runtime_id: "qoder", logged_in: true, models: [] },
    { runtime_id: "cursor", logged_in: false, models: [] },
  ],
  official: {
    signedIn: false,
    giftLeft: 0,
    paidOrTrialReady: false,
    model: "deepseek-flash",
    officialLabel: "官方模型",
  },
  officialProviderId: "teamups-official",
  officialGiftModel: "official-gift",
  modelsForUrl: (_url, m) => (m ? [m] : []),
  chipForUrl: (_url, name) => name,
  accountLabel: (rt) => `${rt} 账号`,
});
const signedOutRows = modelRows(invSignedOut, (id) => id, "", 60);
assert(
  !signedOutRows.some((r) => r.provider === "teamups-official"),
  "signed-out official account is excluded from phone inventory",
);
assert(
  !signedOutRows.some((r) => r.provider === "native:codex" || r.provider === "native:cursor"),
  "un-logged-in native accounts are excluded from phone inventory",
);
assert(
  signedOutRows.some((r) => r.provider === "native:claude-code" && r.account === true && r.agent === "claude-code"),
  "logged-in Claude account is included for claude-code",
);
assert(
  signedOutRows.some((r) => r.provider === "native:qoder" && r.id === "auto" && r.label === "qoder 账号"),
  "logged-in Qoder account without model list exposes auto option labeled with account name",
);
assert(
  signedOutRows.some((r) => r.provider === "pp_ds" && r.agent === ""),
  "configured API-key provider stays shared across agents",
);

const session = {
  runtime: "claude-code",
  model: "old",
  providerId: "prov-1",
  runtimeThreadId: "t-1",
  updatedAt: 1,
};
assert(pickOf(session).agent === "claude-code", "pick reads the chat's agent");
assert(pickOf(session).provider === "prov-1", "pick reads the pinned provider");
const changed = applyPick(session, {
  agent: "hermes",
  model: "deepseek-flash",
  provider: "prov-2",
});
assert(changed, "a real change reports changed");
assert(session.runtime === "hermes", "agent applied");
assert(session.runtimeThreadId === null, "a new agent drops the old thread");
assert(session.model === "deepseek-flash", "model applied");
assert(session.providerId === "prov-2", "the model's own provider is pinned");
assert(session.updatedAt > 1, "the chat moves up the list");

// Switching between two providers that share the same model id updates providerId and resets thread.
const sameModelDifferentProvider = {
  runtime: "qoder",
  model: "deepseek-flash",
  providerId: "prov-1" as string | null,
  runtimeThreadId: "t-keep" as string | null,
};
assert(
  applyPick(sameModelDifferentProvider, {
    agent: "qoder",
    model: "deepseek-flash",
    provider: "prov-2",
  }),
  "switching provider on the same model id counts as a change",
);
assert(sameModelDifferentProvider.providerId === "prov-2", "new provider id is pinned");
assert(sameModelDifferentProvider.runtimeThreadId === null, "switching provider drops the old thread");

const stale = { runtime: "hermes", model: "old", providerId: "prov-1" } as {
  runtime: string;
  model: string;
  providerId: string | null;
};
assert(
  applyPick(stale, { agent: "", model: "new", provider: "" }),
  "a model without a provider is still applied",
);
assert(
  stale.providerId === null,
  "the old provider pin is released when the model arrives without one",
);

const running = { runtime: "claude-code", model: "old", runtimeThreadId: "t-2" };
assert(
  applyPick(running, { agent: "hermes", model: "new", provider: "" }, { running: true }),
  "the model is still applied",
);
assert(running.runtime === "claude-code", "a running turn keeps its agent");
assert(running.runtimeThreadId === "t-2", "a running turn keeps its thread");
assert(running.model === "new", "the next turn uses the new model");

const same = { runtime: "hermes", model: "deepseek-flash", providerId: "prov-2" };
assert(
  !applyPick(same, { agent: "hermes", model: "deepseek-flash", provider: "prov-2" }),
  "no change, no write",
);
assert(!applyPick(same, { agent: "", model: "", provider: "" }), "an empty pick never clears a chat");

assert(
  pickLabel({ agent: "hermes", model: "deepseek-flash", provider: "" }, () => "Hermes", "默认") ===
    "Hermes · deepseek-flash",
  "one line label",
);
assert(
  pickLabel({ agent: "", model: "", provider: "" }, () => "Hermes", "跟着电脑") === "跟着电脑",
  "empty pick falls back",
);

console.log("phone-agents: all assertions passed");
