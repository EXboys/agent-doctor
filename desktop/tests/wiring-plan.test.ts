/**
 * Run from desktop/: node --import tsx tests/wiring-plan.test.ts
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

import type {
  DoctorReport,
  PersonalProviderListItem,
  PersonalProvidersDocument,
  RuntimeDoctorResult,
  TeamupsAccountStatus,
} from "../src/types";

function assertEqual(actual: unknown, expected: unknown, msg: string) {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a !== e) throw new Error(`${msg}\n  expected ${e}\n  actual   ${a}`);
}

const OFFICIAL = "teamups-official";
const OFFICIAL_OPENAI = "https://agentdoctor.cn/api/v1/official";
const OFFICIAL_ANTHROPIC = "https://agentdoctor.cn/api/v1/official/anthropic";

function row(id: string, name: string, active = false): PersonalProviderListItem {
  return { id, name, url: "https://example.test/v1", model: "", protocol: "openai", api_key_hint: "sk-…", active };
}

function doc(...providers: PersonalProviderListItem[]): PersonalProvidersDocument {
  return {
    active_id: providers.find((p) => p.active)?.id ?? null,
    providers,
    store_path: "/tmp/providers.json",
  };
}

type Official = NonNullable<TeamupsAccountStatus["official"]>;

function account(opts: {
  signedIn: boolean;
  official?: Partial<Official> | null;
  giftLeft?: number;
}): TeamupsAccountStatus {
  return {
    base_url: "https://agentdoctor.cn",
    signed_in: opts.signedIn,
    pack_count: 0,
    packs: [],
    official:
      opts.official === null
        ? null
        : {
            active: true,
            via: "none",
            trial_calls_left: 0,
            member_until: null,
            token_cap: 0,
            tokens_used: 0,
            tokens_remaining: 0,
            period: null,
            ...opts.official,
          },
    agent:
      opts.giftLeft == null
        ? null
        : { enabled: true, calls_left: opts.giftLeft, monthly_calls: 30, reset_date: null },
  };
}

function runtime(id: string, installed: boolean, gatewayUrl: string | null): RuntimeDoctorResult {
  return {
    id,
    display_name: id,
    installed,
    version: null,
    binary_path: null,
    config_paths: [],
    profile: { gateway_url: gatewayUrl, key_source: null },
  };
}

function report(...runtimes: RuntimeDoctorResult[]): DoctorReport {
  return { profile_env_path: null, profile_env_exists: false, active_preset: null, runtimes };
}

async function main() {
  const { planWiring, officialFallback, wiredToOfficialLine, officialLineDrained } = await import(
    "../src/wiring-plan"
  );
  const { pickDrainedOfficialRuntime } = await import("../src/first-run");

  // Neither link has quota.
  assertEqual(
    planWiring(doc(), account({ signedIn: false, official: null })),
    { kind: "getQuota", officialPath: "login" },
    "fresh install, signed out: offer sign-in and own key",
  );
  assertEqual(
    planWiring(doc(row(OFFICIAL, "官方", true)), account({ signedIn: true })),
    { kind: "getQuota", officialPath: "buy" },
    "signed in with an empty official line: offer purchase and own key",
  );
  assertEqual(
    planWiring(doc(row(OFFICIAL, "官方", true)), account({ signedIn: false })),
    { kind: "getQuota", officialPath: "login" },
    "stale official row after sign-out is not a usable link",
  );
  assertEqual(
    planWiring(doc(row(OFFICIAL, "官方", true)), account({ signedIn: true, giftLeft: 20 })),
    { kind: "getQuota", officialPath: "buy" },
    "the built-in agent's monthly gift does not power CLIs",
  );
  assertEqual(
    planWiring(
      doc(row(OFFICIAL, "官方", true)),
      account({ signedIn: true, official: { active: false, via: "member", tokens_remaining: 5000 } }),
    ),
    { kind: "getQuota", officialPath: "buy" },
    "an inactive official line does not count even with tokens left",
  );

  // Either link with quota is enough; the official fallback stays while that line cannot serve.
  assertEqual(
    planWiring(doc(row("own", "My GLM", true)), account({ signedIn: false })),
    { kind: "apply", providerId: "own", providerName: "My GLM", officialPath: "login" },
    "own key active: write it, keep sign-in as the fallback",
  );
  assertEqual(
    planWiring(doc(row(OFFICIAL, "官方", true), row("own", "My GLM")), account({ signedIn: true })),
    { kind: "apply", providerId: "own", providerName: "My GLM", officialPath: "buy" },
    "official drained but own key saved: write the own key, keep purchase as the fallback",
  );
  assertEqual(
    planWiring(
      doc(row("own", "My GLM", true)),
      account({ signedIn: true, official: { via: "member", tokens_remaining: 800 } }),
    ),
    { kind: "apply", providerId: "own", providerName: "My GLM", officialPath: null },
    "own key active and official funded: no fallback needed",
  );
  assertEqual(
    planWiring(
      doc(row(OFFICIAL, "官方", true)),
      account({ signedIn: true, official: { via: "member", token_cap: 100000, tokens_remaining: 4200 } }),
    ),
    { kind: "apply", providerId: OFFICIAL, providerName: "官方", officialPath: null },
    "purchased official tokens: write 官方",
  );
  assertEqual(
    planWiring(
      doc(row(OFFICIAL, "官方", true)),
      account({ signedIn: true, official: { via: "trial", trial_calls_left: 3 } }),
    ),
    { kind: "apply", providerId: OFFICIAL, providerName: "官方", officialPath: null },
    "legacy trial calls count as official quota",
  );
  assertEqual(
    planWiring(
      doc(row(OFFICIAL, "官方")),
      account({ signedIn: true, official: { via: "member", tokens_remaining: 800 } }),
    ),
    { kind: "apply", providerId: OFFICIAL, providerName: "官方", officialPath: null },
    "official has quota but is not selected: still one click",
  );

  // Unreadable account keeps the old behavior.
  assertEqual(
    planWiring(doc(row(OFFICIAL, "官方", true)), null),
    { kind: "apply", providerId: OFFICIAL, providerName: "官方", officialPath: null },
    "account unreadable: trust the selected provider as before",
  );
  assertEqual(planWiring(null, null), { kind: "manual" }, "nothing readable: fall back to the plain provider page");

  assertEqual(officialFallback({ kind: "getQuota", officialPath: "buy" }), "buy", "no link: the fallback is the plan");
  assertEqual(
    officialFallback({ kind: "apply", providerId: "own", providerName: "My GLM", officialPath: "login" }),
    "login",
    "beside a saved key: the fallback rides along",
  );
  assertEqual(officialFallback({ kind: "manual" }), null, "nothing readable: no fallback");
  assertEqual(officialFallback(null), null, "no plan yet: no fallback");

  // Which agents ride the official line.
  assertEqual(wiredToOfficialLine(OFFICIAL_OPENAI), true, "OpenAI-protocol official endpoint");
  assertEqual(wiredToOfficialLine(OFFICIAL_ANTHROPIC), true, "Claude Code's official endpoint");
  assertEqual(wiredToOfficialLine("https://api.deepseek.com/v1"), false, "own provider is not the official line");
  assertEqual(wiredToOfficialLine(null), false, "no base URL on record");

  const empty = account({ signedIn: true });
  const funded = account({ signedIn: true, official: { via: "member", tokens_remaining: 900 } });
  assertEqual(officialLineDrained(OFFICIAL_ANTHROPIC, empty), true, "on the official line with nothing left");
  assertEqual(officialLineDrained(OFFICIAL_ANTHROPIC, funded), false, "on the official line with tokens left");
  assertEqual(officialLineDrained(OFFICIAL_ANTHROPIC, null), false, "unknown account is not treated as empty");
  assertEqual(officialLineDrained("https://api.deepseek.com/v1", empty), false, "own key is unaffected by the official balance");

  // Main window: a clean scan can still leave the top agent on an empty official line.
  assertEqual(
    pickDrainedOfficialRuntime(
      report(runtime("codex", true, OFFICIAL_OPENAI), runtime("claude-code", true, OFFICIAL_ANTHROPIC)),
      empty,
    )?.id,
    "claude-code",
    "picks the first agent in first-run order",
  );
  assertEqual(
    pickDrainedOfficialRuntime(
      report(runtime("claude-code", false, OFFICIAL_ANTHROPIC), runtime("codex", true, "https://api.deepseek.com/v1")),
      empty,
    ),
    null,
    "skips agents that are not installed or not on the official line",
  );
  assertEqual(
    pickDrainedOfficialRuntime(report(runtime("claude-code", true, OFFICIAL_ANTHROPIC)), funded),
    null,
    "no prompt while the official line has tokens",
  );

  console.log("wiring-plan: ok");
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
