import type { RepairPreviewResponse } from "./types";

export type DiagnoseStepId = "install" | "config" | "test";

export type DiagnoseStepState = "todo" | "active" | "done" | "error";

export interface DiagnoseScore {
  pass: number;
  warn: number;
  fail: number;
  total: number;
  /** 0–100 style score, similar to a security/readiness ring. */
  percent: number;
}

export function needsWiringFromPreview(preview: RepairPreviewResponse): boolean {
  if (preview.can_apply_repair) {
    return false;
  }
  // Missing key/provider — not mere gateway reachability or version noise.
  if (
    preview.checks.some(
      (check) =>
        (check.status === "fail" || check.status === "warn") &&
        /api_key\.(configured|required)|provider\.(missing|required)/i.test(check.id ?? ""),
    )
  ) {
    return true;
  }
  return preview.suggested_repairs.some((item) => {
    const blob = `${item.id} ${item.title}`;
    if (/gateway|connectivity|dns|unreachable|upstream.?version/i.test(blob)) {
      return false;
    }
    return /wire|provider|scaffold|credential|api.?key/i.test(blob);
  });
}

export function isPreviewHealthy(preview: RepairPreviewResponse): boolean {
  return preview.summary.fail === 0 && !needsWiringFromPreview(preview);
}

export function computeDiagnoseScore(preview: RepairPreviewResponse): DiagnoseScore {
  const pass = preview.summary.pass;
  const warn = preview.summary.warn;
  const fail = preview.summary.fail;
  const total = pass + warn + fail;
  if (total === 0) {
    return { pass, warn, fail, total: 0, percent: 100 };
  }
  // Fail weighs heavier than warn so the ring reflects "usable" more than raw pass count.
  const weighted = pass * 1 + warn * 0.5;
  const percent = Math.max(0, Math.min(100, Math.round((weighted / total) * 100)));
  return { pass, warn, fail, total, percent };
}

export function scoreLooksGood(score: DiagnoseScore): boolean {
  return score.fail === 0 && score.percent >= 80;
}

type FailCheck = {
  id?: string;
  title?: string;
  status: string;
  message?: string;
};

/** True when every failure is the model address, so the next step is the provider page. */
export function failsNeedAddress(preview: {
  summary: { fail: number };
  checks?: FailCheck[];
}): boolean {
  if (preview.summary.fail <= 0) {
    return false;
  }
  const fails = (preview.checks ?? []).filter((check) => check.status === "fail");
  // Callers that only pass a fail count keep the previous address button.
  if (fails.length === 0) {
    return true;
  }
  return fails.every(
    (check) =>
      check.id === "gateway.connectivity" ||
      check.title === "Gateway connectivity" ||
      /gateway (TCP|DNS|host|URL)/i.test(check.message ?? ""),
  );
}

/** Saved key + only a browser-tool write left. The key form should stay closed. */
export function autoFixNoticeKey(preview: {
  suggested_repairs: Array<{ id: string; auto_fixable: boolean }>;
}): "diagnose.flow.scoreNeedsBrowser" | "diagnose.flow.scoreNeedsAutoFix" {
  const ids = preview.suggested_repairs
    .filter((item) => item.auto_fixable)
    .map((item) => item.id);
  if (ids.length > 0 && ids.every((id) => id.includes("browser-mcp"))) {
    return "diagnose.flow.scoreNeedsBrowser";
  }
  return "diagnose.flow.scoreNeedsAutoFix";
}

/** Main button when checks failed: only offer a settings write when one exists. */
export function failurePrimary(
  preview: {
    can_apply_repair: boolean;
    summary: { fail: number };
    checks?: FailCheck[];
  },
  autoFixTried: boolean,
): "apply-settings" | "check-address" | null {
  if (preview.can_apply_repair && !autoFixTried) {
    return "apply-settings";
  }
  if (preview.summary.fail <= 0) {
    return null;
  }
  if (failsNeedAddress(preview)) {
    return "check-address";
  }
  return null;
}

/** What a settings apply actually changed, so the button note does not claim a fix. */
export function settingsApplyKind(report: {
  summary: { fail: number };
  last_execute: { executed: string[] } | null;
}): "changed" | "still-fail" | "nothing" {
  const changed = report.last_execute?.executed.some((id) => id.startsWith("fix-")) ?? false;
  if (!changed) {
    return "nothing";
  }
  if (report.summary.fail > 0) {
    return "still-fail";
  }
  return "changed";
}

/** Pick the first incomplete step for the wizard. */
export function pickInitialStep(input: {
  installed: boolean;
  needsConfig: boolean;
  healthy: boolean;
}): DiagnoseStepId {
  if (!input.installed) {
    return "install";
  }
  if (input.needsConfig) {
    return "config";
  }
  // Always land on test so the system can score; user opens Ask themselves.
  if (input.healthy) {
    return "test";
  }
  return "test";
}

export function stepStatesFor(
  active: DiagnoseStepId,
  flags: { installed: boolean; configured: boolean; testedOk: boolean },
): Record<DiagnoseStepId, DiagnoseStepState> {
  const order: DiagnoseStepId[] = ["install", "config", "test"];
  const doneFlags = {
    install: flags.installed,
    config: flags.configured,
    test: flags.testedOk,
  };
  const result = {} as Record<DiagnoseStepId, DiagnoseStepState>;
  for (const step of order) {
    if (step === active) {
      result[step] = "active";
    } else if (doneFlags[step]) {
      result[step] = "done";
    } else {
      result[step] = "todo";
    }
  }
  return result;
}
