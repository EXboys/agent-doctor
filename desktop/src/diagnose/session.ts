import type { DiagnoseScore, DiagnoseStepId } from "../diagnose-flow";
import type { RepairPreviewResponse } from "../types";

export const SCORE_MIN_MS = 2800;
export const SCORE_MAX_MS = 4000;

export type PrimaryAction =
  | "install"
  | "verify-save"
  | "open-team-wiring"
  | "run-score"
  | "auto-fix"
  | "ask-verify"
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
  checkFilter: CheckFilter;
  guideFillConfig: boolean;
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
    checkFilter: "all",
    guideFillConfig: false,
  };
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}
