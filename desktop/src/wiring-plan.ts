import { OFFICIAL_PROVIDER_ID, officialLineUsable } from "./official-quota";
import type { PersonalProvidersDocument, TeamupsAccountStatus } from "./types";

/** Every official-line endpoint shares this path, whichever protocol the agent speaks. */
const OFFICIAL_LINE_PATH = "/api/v1/official";

export function wiredToOfficialLine(gatewayUrl: string | null | undefined): boolean {
  return (gatewayUrl ?? "").includes(OFFICIAL_LINE_PATH);
}

/**
 * An agent on the official line has nothing to spend once that line is empty,
 * even though its config looks complete. Unknown account state is not empty.
 */
export function officialLineDrained(
  gatewayUrl: string | null | undefined,
  account: TeamupsAccountStatus | null,
): boolean {
  return wiredToOfficialLine(gatewayUrl) && account != null && !officialLineUsable(account);
}

export type OfficialPath = "login" | "buy";

/**
 * `officialPath` is the official fallback still worth offering: set whenever
 * the official line cannot serve yet, even next to a saved key of the user's
 * own, whose balance this app cannot see.
 */
export type WiringPlan =
  | { kind: "apply"; providerId: string; providerName: string; officialPath: OfficialPath | null }
  | { kind: "getQuota"; officialPath: OfficialPath }
  | { kind: "manual" };

export function officialFallback(plan: WiringPlan | null): OfficialPath | null {
  return plan && plan.kind !== "manual" ? plan.officialPath : null;
}

/**
 * Any link with quota is enough: a saved provider of the user's own, or the
 * official account's purchased line. With neither, offer both ways to get
 * quota. Either input is null when it could not be read.
 */
export function planWiring(
  doc: PersonalProvidersDocument | null,
  account: TeamupsAccountStatus | null,
): WiringPlan {
  // Agents ride the official line; the built-in agent's monthly gift does not count.
  const officialReady = account ? officialLineUsable(account) : null;
  const officialPath: OfficialPath | null =
    account && !officialReady ? (account.signed_in ? "buy" : "login") : null;
  const rows = doc?.providers ?? [];
  const isOfficial = (id: string) => id === OFFICIAL_PROVIDER_ID;
  const pick =
    rows.find((row) => row.active && (!isOfficial(row.id) || officialReady !== false)) ??
    rows.find((row) => !isOfficial(row.id)) ??
    (officialReady ? rows.find((row) => isOfficial(row.id)) : undefined);
  if (pick) {
    return { kind: "apply", providerId: pick.id, providerName: pick.name, officialPath };
  }
  if (officialPath) {
    return { kind: "getQuota", officialPath };
  }
  return { kind: "manual" };
}
