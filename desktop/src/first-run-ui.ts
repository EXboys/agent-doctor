import { invoke } from "@tauri-apps/api/core";
import { applyStaticI18n, t } from "./i18n";
import { isPersonalEdition } from "./edition";
import {
  markFirstRunCompleted,
  markFirstRunDismissed,
  pickBiggestFirstRunTarget,
  shouldShowPersonalFirstRun,
  type FirstRunPhase,
  type FirstRunTarget,
} from "./first-run";
import { preferredRepairFilter } from "./repair-ui";
import type {
  DoctorReport,
  InstallRuntimeResponse,
  RepairPreviewResponse,
  RepairStatusFilter,
} from "./types";

export interface FirstRunUiDeps {
  panelDiagnoseEl: HTMLElement;
  repairPreviewByRuntime: Map<string, RepairPreviewResponse>;
  repairFilterByRuntime: Map<string, RepairStatusFilter>;
  getLastReport: () => DoctorReport | null;
  renderReport: (report: DoctorReport) => Promise<void>;
  setLoading: (loading: boolean) => void;
  setStatusBanner: (kind: "ok" | "warn" | "error" | "neutral", message: string) => void;
  updateAgentsSecurityOverview: (report: DoctorReport) => void;
  setMainTab: (tab: "provider") => void;
}

export interface FirstRunUiApi {
  getPhase: () => FirstRunPhase;
  isBusy: () => boolean;
  renderFirstRunUi: () => void;
  evaluateFirstRunFromReport: (report: DoctorReport) => Promise<void>;
  initPersonalFirstRun: () => void;
  exitFirstRun: (opts?: { completed?: boolean }) => void;
  /** Called when user returns to the Agents tab (e.g. after wiring). */
  onDiagnoseTabVisible: () => void;
}

let deps!: FirstRunUiDeps;
let firstRunPhase: FirstRunPhase = "hidden";
let firstRunTarget: FirstRunTarget | null = null;
let firstRunBusy = false;
let firstRunAutoStarted = false;
let firstRunError: string | null = null;
/** Last action that failed — retry button re-runs this instead of a bare welcome scan. */
let firstRunRetryKind: "scan" | "install" | "repair" = "scan";

const firstRunEl = document.querySelector<HTMLElement>("#first-run")!;
const firstRunTitleEl = document.querySelector<HTMLElement>("#first-run-title")!;
const firstRunDescEl = document.querySelector<HTMLElement>("#first-run-desc")!;
const firstRunTargetEl = document.querySelector<HTMLElement>("#first-run-target")!;
const firstRunTargetNameEl = document.querySelector<HTMLElement>("#first-run-target-name")!;
const firstRunTargetMetaEl = document.querySelector<HTMLElement>("#first-run-target-meta")!;
const firstRunPrimaryEl = document.querySelector<HTMLButtonElement>("#first-run-primary")!;
const firstRunPrimaryLabelEl = document.querySelector<HTMLElement>("#first-run-primary-label")!;
const firstRunSpinnerEl = document.querySelector<HTMLElement>("#first-run-spinner")!;
const firstRunSecondaryEl = document.querySelector<HTMLButtonElement>("#first-run-secondary")!;
const firstRunFootnoteEl = document.querySelector<HTMLElement>("#first-run-footnote")!;

function firstRunCopy() {
  return {
    missingInstall: (name: string) => ({
      headline: t("firstRun.missingHeadline", { name }),
      detail: t("firstRun.missingDetail"),
    }),
    needsRepair: (name: string, fail: number, warn: number, top?: string) => ({
      headline: t("firstRun.repairHeadline", { name }),
      detail: t("firstRun.repairDetail", {
        fail: String(fail),
        warn: String(warn),
        top: top ? t("firstRun.repairTop", { top }) : "",
      }),
    }),
    needsWiring: (name: string) => ({
      headline: t("firstRun.wiringHeadline", { name }),
      detail: t("firstRun.wiringDetail"),
    }),
    allGood: () => ({
      headline: t("firstRun.successHeadline"),
      detail: t("firstRun.successDetail"),
    }),
  };
}

function setFirstRunPhase(phase: FirstRunPhase): void {
  firstRunPhase = phase;
  const active = phase !== "hidden" && phase !== "awaitingWiring";
  firstRunEl.hidden = !active;
  firstRunEl.classList.toggle("is-error", phase === "error");
  deps.panelDiagnoseEl.classList.toggle("is-first-run", active);
  deps.panelDiagnoseEl.classList.toggle("is-first-run-done", !active);
  if (!active) {
    return;
  }
  applyStaticI18n(firstRunEl);
}

function showError(message: string, retry: "scan" | "install" | "repair"): void {
  firstRunError = message;
  firstRunRetryKind = retry;
  firstRunBusy = false;
  firstRunPhase = "error";
  renderFirstRunUi();
  deps.setStatusBanner("error", message);
}

function clearError(): void {
  firstRunError = null;
}

function renderFirstRunUi(): void {
  if (firstRunPhase === "hidden" || firstRunPhase === "awaitingWiring") {
    setFirstRunPhase(firstRunPhase);
    return;
  }

  setFirstRunPhase(firstRunPhase);
  const busy =
    firstRunBusy ||
    firstRunPhase === "scanning" ||
    firstRunPhase === "probing" ||
    firstRunPhase === "fixing" ||
    firstRunPhase === "installing";
  firstRunSpinnerEl.hidden = !busy;
  firstRunPrimaryEl.disabled = busy;
  // Allow skip even while scanning so users are not trapped.
  firstRunSecondaryEl.disabled = false;

  if (firstRunPhase === "error") {
    firstRunTitleEl.textContent = t("firstRun.errorHeadline");
    firstRunDescEl.textContent = firstRunError ?? t("firstRun.scanFailed", { error: "unknown" });
    firstRunTargetEl.hidden = !firstRunTarget;
    if (firstRunTarget) {
      firstRunTargetNameEl.textContent = firstRunTarget.displayName;
      firstRunTargetMetaEl.textContent = t("firstRun.targetMetaError");
    }
    firstRunPrimaryLabelEl.textContent = t("firstRun.retry");
    firstRunSecondaryEl.textContent = t("firstRun.skip");
    firstRunFootnoteEl.textContent = t("firstRun.footnoteError");
    return;
  }

  if (firstRunPhase === "welcome") {
    firstRunTitleEl.textContent = t("firstRun.title");
    firstRunDescEl.textContent = t("firstRun.desc");
    firstRunTargetEl.hidden = true;
    firstRunPrimaryLabelEl.textContent = t("firstRun.scan");
    firstRunSecondaryEl.textContent = t("firstRun.skip");
    firstRunFootnoteEl.textContent = t("firstRun.footnote");
    return;
  }

  if (firstRunPhase === "scanning" || firstRunPhase === "probing") {
    firstRunTitleEl.textContent = t("firstRun.title");
    firstRunDescEl.textContent =
      firstRunPhase === "scanning" ? t("firstRun.scanning") : t("firstRun.probing");
    firstRunTargetEl.hidden = true;
    firstRunPrimaryLabelEl.textContent =
      firstRunPhase === "scanning" ? t("firstRun.scanning") : t("firstRun.probing");
    firstRunSecondaryEl.textContent = t("firstRun.skip");
    firstRunFootnoteEl.textContent = t("firstRun.footnote");
    return;
  }

  if (firstRunPhase === "success" || firstRunTarget?.kind === "none") {
    firstRunTitleEl.textContent = t("firstRun.successHeadline");
    firstRunDescEl.textContent = t("firstRun.successDetail");
    firstRunTargetEl.hidden = true;
    firstRunPrimaryLabelEl.textContent = t("firstRun.done");
    firstRunSecondaryEl.textContent = t("firstRun.skip");
    firstRunFootnoteEl.textContent = t("firstRun.footnote");
    firstRunPhase = "success";
    return;
  }

  const target = firstRunTarget!;
  firstRunTitleEl.textContent = target.headline;
  firstRunDescEl.textContent = target.detail;
  firstRunTargetEl.hidden = false;
  firstRunTargetNameEl.textContent = target.displayName;
  if (target.kind === "install") {
    firstRunTargetMetaEl.textContent = t("firstRun.targetMetaInstall");
    firstRunPrimaryLabelEl.textContent =
      firstRunPhase === "installing"
        ? t("firstRun.installing")
        : t("firstRun.install", { name: target.displayName });
    firstRunFootnoteEl.textContent = t("firstRun.footnote");
  } else if (target.kind === "wiring") {
    firstRunTargetMetaEl.textContent = t("firstRun.targetMetaWiring");
    firstRunPrimaryLabelEl.textContent = t("firstRun.wiring");
    firstRunFootnoteEl.textContent = t("firstRun.footnoteWiring");
  } else {
    firstRunTargetMetaEl.textContent = t("firstRun.targetMetaRepair", {
      fail: String(target.fail),
      warn: String(target.warn),
    });
    firstRunPrimaryLabelEl.textContent =
      firstRunPhase === "fixing" ? t("firstRun.fixing") : t("firstRun.fix");
    firstRunFootnoteEl.textContent = t("firstRun.footnote");
  }
  firstRunSecondaryEl.textContent = t("firstRun.skip");
}

function exitFirstRun(opts: { completed?: boolean } = {}): void {
  if (opts.completed) {
    markFirstRunCompleted();
  } else {
    markFirstRunDismissed();
  }
  firstRunTarget = null;
  firstRunBusy = false;
  clearError();
  setFirstRunPhase("hidden");
  deps.panelDiagnoseEl.classList.remove("is-first-run");
  deps.panelDiagnoseEl.classList.add("is-first-run-done");
  firstRunEl.hidden = true;
  const lastReport = deps.getLastReport();
  if (lastReport) {
    void deps.renderReport(lastReport);
  }
}

/** Leave for Wiring without dismissing — return to Agents to resume. */
function suspendForWiring(): void {
  clearError();
  firstRunBusy = false;
  firstRunPhase = "awaitingWiring";
  setFirstRunPhase("awaitingWiring");
  deps.setMainTab("provider");
}

function invalidatePreview(runtimeId: string): void {
  deps.repairPreviewByRuntime.delete(runtimeId);
  deps.repairFilterByRuntime.delete(runtimeId);
}

async function probeInstalledForFirstRun(
  report: DoctorReport,
  opts?: { force?: boolean },
): Promise<void> {
  const installed = report.runtimes.filter((runtime) => runtime.installed);
  await Promise.all(
    installed.map(async (runtime) => {
      if (!opts?.force && deps.repairPreviewByRuntime.has(runtime.id)) {
        return;
      }
      if (opts?.force) {
        invalidatePreview(runtime.id);
      }
      try {
        const preview = await invoke<RepairPreviewResponse>("run_repair_preview_command", {
          runtime: runtime.id,
        });
        deps.repairPreviewByRuntime.set(runtime.id, preview);
        deps.repairFilterByRuntime.set(runtime.id, preferredRepairFilter(preview));
      } catch {
        // Keep scanning other runtimes; missing preview just lowers ranking.
      }
    }),
  );
}

async function evaluateFirstRunFromReport(
  report: DoctorReport,
  opts?: { forceProbe?: boolean },
): Promise<void> {
  if (firstRunPhase === "hidden") {
    return;
  }
  // Resume from wiring park into the normal probe flow.
  if (firstRunPhase === "awaitingWiring") {
    firstRunPhase = "probing";
  }
  clearError();
  firstRunPhase = "probing";
  firstRunBusy = true;
  renderFirstRunUi();
  try {
    await probeInstalledForFirstRun(report, { force: opts?.forceProbe });
    firstRunTarget = pickBiggestFirstRunTarget(report, deps.repairPreviewByRuntime, firstRunCopy());
    firstRunPhase = firstRunTarget.kind === "none" ? "success" : "issue";
  } finally {
    firstRunBusy = false;
    renderFirstRunUi();
    deps.updateAgentsSecurityOverview(report);
  }
}

async function runFirstRunScan(opts?: { forceProbe?: boolean }): Promise<void> {
  if (firstRunBusy) {
    return;
  }
  clearError();
  firstRunBusy = true;
  firstRunPhase = "scanning";
  firstRunRetryKind = "scan";
  renderFirstRunUi();
  deps.setLoading(true);
  try {
    const report = await invoke<DoctorReport>("run_doctor_command");
    await deps.renderReport(report);
    firstRunBusy = false;
    await evaluateFirstRunFromReport(report, { forceProbe: opts?.forceProbe });
  } catch (error) {
    showError(t("firstRun.scanFailed", { error: String(error) }), "scan");
  } finally {
    deps.setLoading(false);
  }
}

function installSucceeded(report: InstallRuntimeResponse): boolean {
  if (!report.install_needed) {
    return true;
  }
  return report.install_succeeded || report.after_installed;
}

function installFailureDetail(report: InstallRuntimeResponse): string {
  return (
    report.skipped.map((item) => item.reason).find(Boolean) ||
    report.manual_fallback[0] ||
    (report.install_log_path
      ? t("firstRun.installIncomplete", { detail: report.install_log_path })
      : t("firstRun.installFailed", { error: "unknown" }))
  );
}

async function runFirstRunInstall(target: FirstRunTarget): Promise<void> {
  firstRunBusy = true;
  firstRunPhase = "installing";
  firstRunRetryKind = "install";
  renderFirstRunUi();
  try {
    const report = await invoke<InstallRuntimeResponse>("install_runtime_command", {
      runtime: target.runtimeId,
      force: false,
    });
    if (!installSucceeded(report)) {
      showError(installFailureDetail(report), "install");
      return;
    }
    invalidatePreview(target.runtimeId);
    firstRunBusy = false;
    // Force re-probe so a freshly installed runtime is ranked correctly.
    await runFirstRunScan({ forceProbe: true });
  } catch (error) {
    showError(t("firstRun.installFailed", { error: String(error) }), "install");
  }
}

async function runFirstRunRepair(target: FirstRunTarget): Promise<void> {
  firstRunBusy = true;
  firstRunPhase = "fixing";
  firstRunRetryKind = "repair";
  renderFirstRunUi();
  try {
    let preview = deps.repairPreviewByRuntime.get(target.runtimeId);
    if (!preview) {
      preview = await invoke<RepairPreviewResponse>("run_repair_preview_command", {
        runtime: target.runtimeId,
      });
      deps.repairPreviewByRuntime.set(target.runtimeId, preview);
    }
    if (!preview.can_apply_repair) {
      // Re-rank: may become wiring or another runtime.
      firstRunBusy = false;
      const lastReport = deps.getLastReport();
      if (lastReport) {
        firstRunTarget = pickBiggestFirstRunTarget(
          lastReport,
          deps.repairPreviewByRuntime,
          firstRunCopy(),
        );
      }
      firstRunPhase = firstRunTarget?.kind === "none" ? "success" : "issue";
      renderFirstRunUi();
      return;
    }
    const report = await invoke<RepairPreviewResponse>("run_repair_execute_command", {
      runtime: target.runtimeId,
    });
    deps.repairPreviewByRuntime.set(target.runtimeId, report);
    firstRunBusy = false;
    await runFirstRunScan({ forceProbe: true });
  } catch (error) {
    showError(t("firstRun.fixFailed", { error: String(error) }), "repair");
  }
}

async function runFirstRunPrimaryAction(): Promise<void> {
  if (firstRunBusy) {
    return;
  }
  if (firstRunPhase === "error") {
    if (firstRunRetryKind === "install" && firstRunTarget?.kind === "install") {
      await runFirstRunInstall(firstRunTarget);
      return;
    }
    if (firstRunRetryKind === "repair" && firstRunTarget?.kind === "repair") {
      await runFirstRunRepair(firstRunTarget);
      return;
    }
    await runFirstRunScan({ forceProbe: true });
    return;
  }
  if (firstRunPhase === "welcome" || firstRunPhase === "scanning") {
    await runFirstRunScan();
    return;
  }
  if (firstRunPhase === "success") {
    exitFirstRun({ completed: true });
    return;
  }
  const target = firstRunTarget;
  if (!target) {
    await runFirstRunScan({ forceProbe: true });
    return;
  }
  if (target.kind === "wiring") {
    suspendForWiring();
    return;
  }
  if (target.kind === "install") {
    await runFirstRunInstall(target);
    return;
  }
  if (target.kind === "repair") {
    await runFirstRunRepair(target);
  }
}

function onDiagnoseTabVisible(): void {
  if (firstRunPhase !== "awaitingWiring" || firstRunBusy) {
    return;
  }
  void runFirstRunScan({ forceProbe: true });
}

function initPersonalFirstRun(): void {
  if (!shouldShowPersonalFirstRun(isPersonalEdition())) {
    setFirstRunPhase("hidden");
    firstRunEl.hidden = true;
    deps.panelDiagnoseEl.classList.remove("is-first-run");
    return;
  }
  firstRunPhase = "welcome";
  clearError();
  renderFirstRunUi();
  if (!firstRunAutoStarted) {
    firstRunAutoStarted = true;
    void runFirstRunScan();
  }
}

export function initFirstRunUi(d: FirstRunUiDeps): FirstRunUiApi {
  deps = d;
  firstRunPrimaryEl.addEventListener("click", () => {
    void runFirstRunPrimaryAction();
  });
  firstRunSecondaryEl.addEventListener("click", () => {
    if (firstRunBusy && firstRunPhase !== "error") {
      // Soft cancel: dismiss even mid-scan so users are never trapped.
    }
    exitFirstRun({ completed: firstRunPhase === "success" });
  });
  return {
    getPhase: () => firstRunPhase,
    isBusy: () => firstRunBusy,
    renderFirstRunUi,
    evaluateFirstRunFromReport,
    initPersonalFirstRun,
    exitFirstRun,
    onDiagnoseTabVisible,
  };
}
