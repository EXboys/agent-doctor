import type { DoctorReport, RepairPreviewResponse, RuntimeDoctorResult } from "./types";

const STORAGE_KEY = "agent-doctor.personal-first-run.v1";

/** Prefer these when choosing what to install or diagnose first. */
export const FIRST_RUN_RUNTIME_ORDER = [
  "hermes",
  "openclaw",
  "claude-code",
  "codex",
  "deepseek-harness",
] as const;

export type FirstRunPhase =
  | "welcome"
  | "scanning"
  | "probing"
  | "issue"
  | "fixing"
  | "installing"
  | "awaitingWiring"
  | "awaitingVerify"
  | "error"
  | "success"
  | "hidden";

export type FirstRunActionKind = "repair" | "install" | "wiring" | "none";

export interface FirstRunTarget {
  runtimeId: string;
  displayName: string;
  kind: FirstRunActionKind;
  headline: string;
  detail: string;
  fail: number;
  warn: number;
  canApply: boolean;
  topCheck?: string;
  /** When set, wiring can one-click re-apply this saved personal provider. */
  applyProviderId?: string;
  applyProviderName?: string;
}

interface FirstRunStorage {
  dismissed?: boolean;
  completed?: boolean;
}

export function readFirstRunStorage(): FirstRunStorage {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) {
      return {};
    }
    return JSON.parse(raw) as FirstRunStorage;
  } catch {
    return {};
  }
}

export function writeFirstRunStorage(patch: FirstRunStorage): void {
  const next = { ...readFirstRunStorage(), ...patch };
  localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
}

export function shouldShowPersonalFirstRun(isPersonal: boolean): boolean {
  if (!isPersonal) {
    return false;
  }
  const stored = readFirstRunStorage();
  return !stored.dismissed && !stored.completed;
}

export function markFirstRunDismissed(): void {
  writeFirstRunStorage({ dismissed: true });
}

export function markFirstRunCompleted(): void {
  writeFirstRunStorage({ completed: true });
}

function runtimeRank(id: string): number {
  const idx = FIRST_RUN_RUNTIME_ORDER.indexOf(
    id as (typeof FIRST_RUN_RUNTIME_ORDER)[number],
  );
  return idx === -1 ? 100 : idx;
}

function sortRuntimes(runtimes: RuntimeDoctorResult[]): RuntimeDoctorResult[] {
  return [...runtimes].sort((a, b) => runtimeRank(a.id) - runtimeRank(b.id));
}

function issueScore(preview: RepairPreviewResponse): number {
  const blocking = preview.checks.filter(isFirstRunBlockingCheck);
  const fail = blocking.filter((check) => check.status === "fail").length;
  const warn = blocking.filter((check) => check.status === "warn").length;
  if (fail === 0 && warn === 0) {
    return 0;
  }
  // Prefer auto-fixable failures so the primary CTA can succeed.
  const fixBoost = preview.can_apply_repair ? 25 : 0;
  return fail * 100 + warn * 10 + fixBoost;
}

/** Soft network/version noise should not drive first-run CTAs. */
function isFirstRunBlockingCheck(check: {
  id: string;
  status: string;
}): boolean {
  if (check.status !== "fail" && check.status !== "warn") {
    return false;
  }
  return !/gateway\.connectivity|binary\.upstream_version/i.test(check.id);
}

function isCredentialWiringPreview(preview: RepairPreviewResponse): boolean {
  const missingCredential = preview.checks.some(
    (check) =>
      (check.status === "fail" || check.status === "warn") &&
      /api_key\.(configured|required)|provider\.(missing|required)/i.test(check.id),
  );
  if (missingCredential) {
    return true;
  }
  return preview.suggested_repairs.some((item) => {
    const blob = `${item.id} ${item.title}`;
    if (/gateway|connectivity|dns|unreachable|upstream.?version/i.test(blob)) {
      return false;
    }
    return /scaffold|wire-provider|configure-.*(?:api-?key|provider)|fix-.*api-key-scaffold/i.test(
      blob,
    );
  });
}

/** Pick the single highest-impact problem for the first-run hero. */
export function pickBiggestFirstRunTarget(
  report: DoctorReport,
  previews: Map<string, RepairPreviewResponse>,
  copy: {
    missingInstall: (name: string) => { headline: string; detail: string };
    needsRepair: (name: string, fail: number, warn: number, top?: string) => {
      headline: string;
      detail: string;
    };
    needsWiring: (name: string) => { headline: string; detail: string };
    allGood: () => { headline: string; detail: string };
  },
): FirstRunTarget {
  const ordered = sortRuntimes(report.runtimes);
  const installed = ordered.filter((r) => r.installed);

  let best: FirstRunTarget | null = null;
  let bestScore = 0;

  for (const runtime of installed) {
    const preview = previews.get(runtime.id);
    if (!preview) {
      continue;
    }
    const score = issueScore(preview);
    if (score <= 0) {
      continue;
    }
    const topFail = preview.checks.find(
      (c) => c.status === "fail" && isFirstRunBlockingCheck(c),
    );
    const topWarn = preview.checks.find(
      (c) => c.status === "warn" && isFirstRunBlockingCheck(c),
    );
    const top = topFail?.title ?? topWarn?.title;
    const kind: FirstRunActionKind = preview.can_apply_repair
      ? "repair"
      : isCredentialWiringPreview(preview)
        ? "wiring"
        : "repair";
    const text =
      kind === "wiring"
        ? copy.needsWiring(runtime.display_name)
        : copy.needsRepair(
            runtime.display_name,
            preview.summary.fail,
            preview.summary.warn,
            top,
          );
    if (score > bestScore) {
      bestScore = score;
      best = {
        runtimeId: runtime.id,
        displayName: runtime.display_name,
        kind,
        headline: text.headline,
        detail: text.detail,
        fail: preview.summary.fail,
        warn: preview.summary.warn,
        canApply: preview.can_apply_repair,
        topCheck: top,
      };
    }
  }

  if (best) {
    return best;
  }

  if (installed.length === 0) {
    const candidate =
      ordered.find((r) => !r.installed) ?? ordered[0];
    if (candidate) {
      const text = copy.missingInstall(candidate.display_name);
      return {
        runtimeId: candidate.id,
        displayName: candidate.display_name,
        kind: "install",
        headline: text.headline,
        detail: text.detail,
        fail: 0,
        warn: 0,
        canApply: false,
      };
    }
  }

  const good = copy.allGood();
  return {
    runtimeId: installed[0]?.id ?? "",
    displayName: installed[0]?.display_name ?? "",
    kind: "none",
    headline: good.headline,
    detail: good.detail,
    fail: 0,
    warn: 0,
    canApply: false,
  };
}
