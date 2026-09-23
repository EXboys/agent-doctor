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
        /api_key\.(configured|required)|provider\.(missing|required)/i.test(check.id),
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
