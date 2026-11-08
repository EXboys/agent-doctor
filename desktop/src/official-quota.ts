/**
 * The official account's two quota pools, in one place:
 * - 赠送 (free): the built-in agent's monthly gift on the /api/v1/agent channel.
 * - 官方额度 (paid): tokens the user bought, shared by every official channel.
 *
 * Copy, the model-picker status chip, and the provider card all render from
 * these helpers so the two windows can never drift apart.
 */

import { getLocale, t } from "./i18n";
import type { TeamupsAccountStatus } from "./types";

export const OFFICIAL_PROVIDER_ID = "teamups-official";

/** Pin marker for the monthly-gift entry: rides the /api/v1/agent channel. */
export const OFFICIAL_GIFT_MODEL = "official-gift";

export type OfficialQuotaState = {
  signedIn: boolean;
  /** Monthly gift for the built-in agent. null when not reported or disabled. */
  free: { left: number; total: number; resetDate: string | null } | null;
  /** Purchased official tokens; present for members even when drained. */
  paid: { tokens: number } | null;
  /** Legacy trial calls on the official (external-CLI) line. */
  trial: { left: number } | null;
};

export function officialQuotaState(
  status: TeamupsAccountStatus | null | undefined,
): OfficialQuotaState {
  const official = status?.official ?? null;
  const agent = status?.agent ?? null;
  const signedIn = status?.signed_in === true;
  const tokens = Math.max(0, official?.tokens_remaining ?? 0);
  const hasPaidLine =
    signedIn &&
    official != null &&
    (official.via === "member" || (official.token_cap ?? 0) > 0 || tokens > 0);
  const trials = Math.max(0, official?.trial_calls_left ?? 0);
  return {
    signedIn,
    free:
      signedIn && agent?.enabled
        ? {
            left: Math.max(0, agent.calls_left ?? 0),
            total: Math.max(0, agent.monthly_calls ?? 0),
            resetDate: agent.reset_date ?? null,
          }
        : null,
    paid: hasPaidLine ? { tokens } : null,
    trial: signedIn && official?.via === "trial" && trials > 0 ? { left: trials } : null,
  };
}

/**
 * Whether the official line that external CLIs ride can take a request:
 * purchased tokens, or legacy trial calls. The monthly gift does not count —
 * it only serves the built-in agent.
 */
export function officialLineUsable(status: TeamupsAccountStatus | null | undefined): boolean {
  const official = status?.official;
  if (!status?.signed_in || !official?.active) return false;
  const tokens = Math.max(0, official.tokens_remaining ?? 0);
  const trials = Math.max(0, official.trial_calls_left ?? 0);
  return tokens > 0 || (official.via === "trial" && trials > 0);
}

/** Which pool the built-in agent's next message would spend. */
export type BuiltinQuotaState = "signed_out" | "free" | "paid" | "empty";

export function builtinQuotaState(quota: OfficialQuotaState): BuiltinQuotaState {
  if (!quota.signedIn) return "signed_out";
  if ((quota.free?.left ?? 0) > 0) return "free";
  if ((quota.paid?.tokens ?? 0) > 0) return "paid";
  return "empty";
}

/** Compact text after「官方模型 ·」on the model button. */
export function builtinQuotaLabel(quota: OfficialQuotaState): string {
  switch (builtinQuotaState(quota)) {
    case "signed_out":
      return t("official.quotaSignedOut");
    case "free":
      return t("official.quotaFree", {
        left: String(quota.free?.left ?? 0),
        total: String(quota.free?.total ?? 0),
      });
    case "paid":
      return t("official.quotaPaid", {
        tokens: formatOfficialTokens(quota.paid?.tokens ?? 0),
      });
    case "empty":
      return t("official.quotaEmpty");
  }
}

/**
 * The quota line for the provider card and menus. No purchase → only the
 * free gift shows; both drained → the buy-more fallback line.
 */
export function officialQuotaLines(quota: OfficialQuotaState): string[] {
  if (!quota.signedIn) return [t("personal.officialNeedsLogin")];
  const lines: string[] = [];
  if (quota.free) {
    lines.push(
      quota.free.left > 0
        ? t("official.quotaFree", {
            left: String(quota.free.left),
            total: String(quota.free.total),
          })
        : quota.free.resetDate
          ? t("official.quotaFreeReset", { date: formatResetDate(quota.free.resetDate) })
          : t("official.quotaFreeOut"),
    );
  }
  if (quota.paid) {
    lines.push(t("official.quotaPaid", { tokens: formatOfficialTokens(quota.paid.tokens) }));
  }
  if (quota.trial) {
    lines.push(t("official.quotaTrial", { count: String(quota.trial.left) }));
  }
  return lines.length > 0 ? lines : [t("personal.officialNeedsMembership")];
}

/** 12500 → 1.3万 (zh) / 12.5K (en), matching the provider card's old format. */
export function formatOfficialTokens(tokens: number): string {
  if (!Number.isFinite(tokens) || tokens <= 0) return "0";
  return new Intl.NumberFormat(getLocale() === "zh" ? "zh-CN" : "en", {
    notation: tokens >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(Math.floor(tokens));
}

/** "2026-12-01" → 12月1日 / Dec 1. Falls back to the raw string. */
export function formatResetDate(iso: string): string {
  const day = /^(\d{4})-(\d{2})-(\d{2})/.exec(iso.trim());
  if (!day) return iso.trim();
  const date = new Date(Number(day[1]), Number(day[2]) - 1, Number(day[3]));
  if (Number.isNaN(date.getTime())) return iso.trim();
  return new Intl.DateTimeFormat(getLocale() === "zh" ? "zh-CN" : "en", {
    month: "short",
    day: "numeric",
  }).format(date);
}
