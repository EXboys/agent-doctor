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

export function resolveInitialRuntime(): string {
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
