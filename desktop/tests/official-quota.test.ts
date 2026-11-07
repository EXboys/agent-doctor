/**
 * Run from desktop/: node --import tsx tests/official-quota.test.ts
 *
 * The official quota wording is one source of truth for the Ask model chip,
 * the doctor status menu, and the provider card. These cases pin the display
 * matrix: no purchase → only the free gift shows.
 */
import assert from "node:assert/strict";

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

async function main() {
  const { t } = await import("../src/i18n");
  const {
    builtinQuotaLabel,
    builtinQuotaState,
    formatOfficialTokens,
    formatResetDate,
    officialQuotaLines,
    officialQuotaState,
  } = await import("../src/official-quota");

  assert(formatOfficialTokens(86000) === "8.6万", "compact zh token format");
  assert(formatOfficialTokens(0) === "0", "zero tokens");
  assert(formatResetDate("2026-12-01") === "12月1日", "zh reset date");
  assert(formatResetDate("not-a-date") === "not-a-date", "raw fallback for odd dates");

  // Not signed in: nothing but the sign-in line, whichever surface.
  const out = officialQuotaState(null);
  assert(builtinQuotaState(out) === "signed_out", "null status is signed out");
  assert(builtinQuotaLabel(out) === t("official.quotaSignedOut"), "signed-out chip");
  assert.deepEqual(officialQuotaLines(out), [t("personal.officialNeedsLogin")]);

  // Signed in, monthly gift only — no purchase shows nothing extra.
  const freeOnly = officialQuotaState({
    signed_in: true,
    agent: { enabled: true, calls_left: 12, monthly_calls: 30, reset_date: "2026-12-01" },
  } as never);
  assert(builtinQuotaState(freeOnly) === "free", "gift-only state");
  assert(builtinQuotaLabel(freeOnly) === t("official.quotaFree", { left: "12", total: "30" }));
  assert.deepEqual(
    officialQuotaLines(freeOnly),
    [t("official.quotaFree", { left: "12", total: "30" })],
    "no purchase → only the gift line",
  );

  // Gift drained, purchased tokens left: the chip names the pool being spent.
  const paid = officialQuotaState({
    signed_in: true,
    agent: { enabled: true, calls_left: 0, monthly_calls: 30, reset_date: "2026-12-01" },
    official: {
      active: true,
      via: "member",
      trial_calls_left: 0,
      member_until: null,
      token_cap: 100000,
      tokens_used: 14000,
      tokens_remaining: 86000,
      period: null,
    },
  } as never);
  assert(builtinQuotaState(paid) === "paid", "drained gift falls to purchased tokens");
  assert(builtinQuotaLabel(paid) === t("official.quotaPaid", { tokens: "8.6万" }));
  assert.deepEqual(officialQuotaLines(paid), [
    t("official.quotaFreeReset", { date: "12月1日" }),
    t("official.quotaPaid", { tokens: "8.6万" }),
  ]);

  // Both drained: empty state (buy CTA lives in the menu, not here).
  const empty = officialQuotaState({
    signed_in: true,
    agent: { enabled: true, calls_left: 0, monthly_calls: 30, reset_date: null },
    official: {
      active: false,
      via: "member",
      trial_calls_left: 0,
      member_until: null,
      token_cap: 100000,
      tokens_used: 100000,
      tokens_remaining: 0,
      period: null,
    },
  } as never);
  assert(builtinQuotaState(empty) === "empty", "member with zero tokens is empty");
  assert(builtinQuotaLabel(empty) === t("official.quotaEmpty"));

  // Legacy trial calls surface on the official line, never as built-in quota.
  const trial = officialQuotaState({
    signed_in: true,
    agent: { enabled: true, calls_left: 0, monthly_calls: 30, reset_date: null },
    official: {
      active: true,
      via: "trial",
      trial_calls_left: 3,
      member_until: null,
      token_cap: 0,
      tokens_used: 0,
      tokens_remaining: 0,
      period: null,
    },
  } as never);
  assert(builtinQuotaState(trial) === "empty", "trial calls are not built-in quota");
  assert(
    officialQuotaLines(trial).includes(t("official.quotaTrial", { count: "3" })),
    "trial line shows on the card",
  );

  console.log("official-quota.test.ts ok");
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
