import type { DiagnoseScore, DiagnoseStepId } from "../diagnose-flow";
import type { RepairPreviewResponse, RepairStatusFilter } from "../types";

export const SCORE_MIN_MS = 2800;
export const SCORE_MAX_MS = 4000;

export type PrimaryAction =
  | "install"
  | "verify-save"
  | "open-team-wiring"
  | "run-score"
  | "auto-fix"
  | "deep-repair"
  | "ask-fix"
  | "ask-verify"
  | "open-ask"
  | "rescan"
  | "none";

export type CheckFilter = "all" | "pass" | "warn" | "fail";

export type DiagnoseSession = {
  runtimeId: string;
  displayName: string;
  activeStep: DiagnoseStepId;
  installed: boolean;
  configured: boolean;
  testedOk: boolean;
  busy: boolean;
  preview: RepairPreviewResponse | null;
  lastScore: DiagnoseScore | null;
  primaryAction: PrimaryAction;
  canAutoFix: boolean;
  /** Rule auto-fix already ran; failures left after it go to one-click repair, not back to config. */
  autoFixTried: boolean;
  /** Set after a repair that changed nothing useful, so the button stops offering it again. */
  repairTried: boolean;
  /** Result of the last repair, shown in front of the next score line. */
  repairNotice: string | null;
  checkFilter: CheckFilter;
  guideFillConfig: boolean;
  /** Full repair panel (one-click / rollback / Chrome check). */
  deepOpen: boolean;
  repairConfirmPending: boolean;
  deepFilter: RepairStatusFilter;
};

const RUNTIME_STORAGE_KEY = "ad-diagnose-runtime";

/** Persist so reload does not fall back to the window's first-create init script. */
export function rememberDiagnoseRuntime(runtime: string): void {
  const trimmed = runtime.trim();
  if (!trimmed) {
    return;
  }
  try {
    sessionStorage.setItem(RUNTIME_STORAGE_KEY, trimmed);
  } catch {
    /* private / blocked storage */
  }
  window.__AD_DIAGNOSE_RUNTIME__ = trimmed;
}

export function resolveInitialRuntime(): string {
  try {
    const stored = sessionStorage.getItem(RUNTIME_STORAGE_KEY)?.trim();
    if (stored) {
      return stored;
    }
  } catch {
    /* private / blocked storage */
  }
  const injected = window.__AD_DIAGNOSE_RUNTIME__?.trim();
  if (injected) {
    return injected;
  }
  return "openclaw";
}

export function createDiagnoseSession(runtimeId: string): DiagnoseSession {
  return {
    runtimeId,
    displayName: runtimeId,
    activeStep: "install",
    installed: false,
    configured: false,
    testedOk: false,
    busy: false,
    preview: null,
    lastScore: null,
    primaryAction: "none",
    canAutoFix: false,
    autoFixTried: false,
    repairTried: false,
    repairNotice: null,
    checkFilter: "all",
    guideFillConfig: false,
    deepOpen: false,
    repairConfirmPending: false,
    deepFilter: "all",
  };
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}
