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
}

let deps!: FirstRunUiDeps;
let firstRunPhase: FirstRunPhase = "hidden";
let firstRunTarget: FirstRunTarget | null = null;
let firstRunBusy = false;
let firstRunAutoStarted = false;

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
  const active = phase !== "hidden";
  firstRunEl.hidden = !active;
  deps.panelDiagnoseEl.classList.toggle("is-first-run", active);
  deps.panelDiagnoseEl.classList.toggle("is-first-run-done", !active);
  if (!active) {
    return;
  }
  applyStaticI18n(firstRunEl);
}

function renderFirstRunUi(): void {
  if (firstRunPhase === "hidden") {
    setFirstRunPhase("hidden");
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
  firstRunSecondaryEl.disabled = busy && firstRunPhase !== "success";

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
  } else if (target.kind === "wiring") {
    firstRunTargetMetaEl.textContent = t("firstRun.targetMetaWiring");
    firstRunPrimaryLabelEl.textContent = t("firstRun.wiring");
  } else {
    firstRunTargetMetaEl.textContent = t("firstRun.targetMetaRepair", {
      fail: String(target.fail),
      warn: String(target.warn),
    });
    firstRunPrimaryLabelEl.textContent =
      firstRunPhase === "fixing" ? t("firstRun.fixing") : t("firstRun.fix");
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
  setFirstRunPhase("hidden");
  deps.panelDiagnoseEl.classList.remove("is-first-run");
  deps.panelDiagnoseEl.classList.add("is-first-run-done");
  firstRunEl.hidden = true;
  const lastReport = deps.getLastReport();
  if (lastReport) {
    void deps.renderReport(lastReport);
  }
}

async function probeInstalledForFirstRun(report: DoctorReport): Promise<void> {
  const installed = report.runtimes.filter((runtime) => runtime.installed);
  await Promise.all(
    installed.map(async (runtime) => {
      if (deps.repairPreviewByRuntime.has(runtime.id)) {
        return;
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

async function evaluateFirstRunFromReport(report: DoctorReport): Promise<void> {
  if (firstRunPhase === "hidden") {
    return;
  }
  firstRunPhase = "probing";
  firstRunBusy = true;
  renderFirstRunUi();
  try {
    await probeInstalledForFirstRun(report);
    firstRunTarget = pickBiggestFirstRunTarget(report, deps.repairPreviewByRuntime, firstRunCopy());
    firstRunPhase = firstRunTarget.kind === "none" ? "success" : "issue";
  } finally {
    firstRunBusy = false;
    renderFirstRunUi();
    deps.updateAgentsSecurityOverview(report);
  }
}

async function runFirstRunScan(): Promise<void> {
  if (firstRunBusy) {
    return;
  }
  firstRunBusy = true;
  firstRunPhase = "scanning";
  renderFirstRunUi();
  deps.setLoading(true);
  try {
    const report = await invoke<DoctorReport>("run_doctor_command");
    await deps.renderReport(report);
    firstRunBusy = false;
    await evaluateFirstRunFromReport(report);
  } catch (error) {
    firstRunBusy = false;
    firstRunPhase = "welcome";
    firstRunDescEl.textContent = t("firstRun.scanFailed", { error: String(error) });
    renderFirstRunUi();
    deps.setStatusBanner("error", t("doctor.failed", { error: String(error) }));
  } finally {
    deps.setLoading(false);
  }
}

async function runFirstRunPrimaryAction(): Promise<void> {
  if (firstRunBusy) {
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
    await runFirstRunScan();
    return;
  }
  if (target.kind === "wiring") {
    exitFirstRun({ completed: false });
    deps.setMainTab("provider");
    return;
  }
  if (target.kind === "install") {
    firstRunBusy = true;
    firstRunPhase = "installing";
    renderFirstRunUi();
    try {
      await invoke<InstallRuntimeResponse>("install_runtime_command", {
        runtime: target.runtimeId,
        force: false,
      });
      firstRunBusy = false;
      await runFirstRunScan();
    } catch (error) {
      firstRunBusy = false;
      firstRunPhase = "issue";
      firstRunDescEl.textContent = t("firstRun.installFailed", { error: String(error) });
      renderFirstRunUi();
    }
    return;
  }
  if (target.kind === "repair") {
    firstRunBusy = true;
    firstRunPhase = "fixing";
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
      await runFirstRunScan();
    } catch (error) {
      firstRunBusy = false;
      firstRunPhase = "issue";
      firstRunDescEl.textContent = t("firstRun.fixFailed", { error: String(error) });
      renderFirstRunUi();
    }
  }
}

function initPersonalFirstRun(): void {
  if (!shouldShowPersonalFirstRun(isPersonalEdition())) {
    setFirstRunPhase("hidden");
    firstRunEl.hidden = true;
    deps.panelDiagnoseEl.classList.remove("is-first-run");
    return;
  }
  firstRunPhase = "welcome";
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
    exitFirstRun({ completed: firstRunPhase === "success" });
  });
  return {
    getPhase: () => firstRunPhase,
    isBusy: () => firstRunBusy,
    renderFirstRunUi,
    evaluateFirstRunFromReport,
    initPersonalFirstRun,
    exitFirstRun,
  };
}
