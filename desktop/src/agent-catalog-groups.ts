/**
 * Catalog grouping, ordering & install shape per agent.
 *
 * Groups in the Resources → Agent catalog:
 * 1. `builtin`: built into this app, ready without installing (`doctor`).
 * 2. `npm`: direct command-line installs (Codex, Claude, OpenClaw, Pi).
 * 3. `domestic`: dual-client suites with a desktop window and a command line
 *    (Cursor, WorkBuddy, Qoder, ZCode).
 * 4. `experimental`: preview / unstable helpers — Hermes (installer needs
 *    overseas / GitHub network) and DeepSeek Harness (experimental preview).
 */

export type AgentCatalogGroupId = "builtin" | "npm" | "domestic" | "experimental";

export type AgentCatalogTabId = "all" | AgentCatalogGroupId;

export const AGENT_CATALOG_GROUP_ORDER: readonly AgentCatalogGroupId[] = [
  "builtin",
  "npm",
  "domestic",
  "experimental",
];

/** Display priority: built-in first, then npm-installable, dual-client, experimental. */
const AGENT_ORDER: readonly string[] = [
  "doctor",
  "codex",
  "claude-code",
  "openclaw",
  "pi",
  "antigravity",
  "cursor",
  "workbuddy",
  "qoder",
  "zcode",
  "deepseek-harness",
  "hermes",
];

export const BUILTIN_RUNTIME_IDS = ["doctor"] as const;

export const EXPERIMENTAL_RUNTIME_IDS = [
  "hermes",
  "deepseek-harness",
] as const;

export const DOMESTIC_DUAL_RUNTIME_IDS = [
  "cursor",
  "workbuddy",
  "qoder",
  "zcode",
] as const;

export const DUAL_INSTALL_RUNTIME_IDS = DOMESTIC_DUAL_RUNTIME_IDS;

const builtinIds = new Set<string>(BUILTIN_RUNTIME_IDS);
const experimentalIds = new Set<string>(EXPERIMENTAL_RUNTIME_IDS);
const domesticDualIds = new Set<string>(DOMESTIC_DUAL_RUNTIME_IDS);
const orderRank = new Map<string, number>(
  AGENT_ORDER.map((id, idx) => [id, idx]),
);

export function isExperimentalRuntime(runtimeId: string): boolean {
  return experimentalIds.has(runtimeId);
}

/** True when tapping Install sets up both the command line and the desktop app. */
export function installsDesktopAndCli(runtimeId: string): boolean {
  return domesticDualIds.has(runtimeId);
}

export function agentCatalogGroup(runtimeId: string): AgentCatalogGroupId {
  if (builtinIds.has(runtimeId)) return "builtin";
  if (experimentalIds.has(runtimeId)) return "experimental";
  if (domesticDualIds.has(runtimeId)) return "domestic";
  return "npm";
}

export function agentCatalogRank(runtimeId: string): number {
  return orderRank.get(runtimeId) ?? 999;
}

export function groupAgentCatalog<T extends { id: string }>(
  rows: readonly T[],
): Record<AgentCatalogGroupId, T[]> {
  const grouped: Record<AgentCatalogGroupId, T[]> = {
    builtin: [],
    npm: [],
    domestic: [],
    experimental: [],
  };
  for (const row of rows) {
    grouped[agentCatalogGroup(row.id)].push(row);
  }
  return grouped;
}

export function splitAgentCatalog<T extends { id: string }>(rows: readonly T[]): {
  ready: T[];
  experimental: T[];
} {
  const ready: T[] = [];
  const experimental: T[] = [];
  for (const row of rows) {
    if (isExperimentalRuntime(row.id)) experimental.push(row);
    else ready.push(row);
  }
  return { ready, experimental };
}
