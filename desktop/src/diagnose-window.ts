import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyStaticI18n, getLocale, t } from "./i18n";
import { isPersonalEdition } from "./edition";
import { escapeHtml } from "./format";
import {
  computeDiagnoseScore,
  isPreviewHealthy,
  needsWiringFromPreview,
  pickInitialStep,
  scoreLooksGood,
  stepStatesFor,
  type DiagnoseScore,
  type DiagnoseStepId,
} from "./diagnose-flow";
import { PROVIDER_PRESETS, PRESET_PICKER_GROUPS } from "./provider-presets";
import { repairCheckStatusLabel, repairStatusClass } from "./repair-ui";
import type {
  DoctorReport,
  EvotownStatus,
  InstallProgressEvent,
  InstallRuntimeResponse,
  PersonalProviderSetupReport,
  PersonalProviderStatus,
  PersonalProvidersDocument,
  PersonalProviderVerifyReport,
  ProviderProtocol,
  RepairPreviewResponse,
  RuntimeDoctorResult,
} from "./types";

declare global {
  interface Window {
    __AD_DIAGNOSE_RUNTIME__?: string;
    __AD_DIAGNOSE_APPLY_RUNTIME__?: (runtime: string) => void;
  }
}

const titleEl = document.querySelector<HTMLElement>("#diagnose-title")!;
const heroEl = document.querySelector<HTMLElement>("#diagnose-hero")!;
const healthCardEl = document.querySelector<HTMLElement>("#diagnose-health-card")!;
const headlineEl = document.querySelector<HTMLElement>("#diagnose-headline")!;
const detailEl = document.querySelector<HTMLElement>("#diagnose-detail")!;
const resultEl = document.querySelector<HTMLElement>("#diagnose-result")!;
const heroKickerEl = document.querySelector<HTMLElement>("#diagnose-hero-kicker")!;
const statRowEl = document.querySelector<HTMLElement>("#diagnose-stat-row")!;
const statPassEl = document.querySelector<HTMLElement>("#diagnose-stat-pass")!;
const statWarnEl = document.querySelector<HTMLElement>("#diagnose-stat-warn")!;
const statFailEl = document.querySelector<HTMLElement>("#diagnose-stat-fail")!;
const testHintEl = document.querySelector<HTMLElement>("#diagnose-test-hint")!;
const stepsEl = document.querySelector<HTMLElement>("#diagnose-steps")!;
const scoreRingEl = document.querySelector<HTMLElement>("#diagnose-score-ring")!;
const scoreValueEl = document.querySelector<HTMLElement>("#diagnose-score-value")!;
const panelInstallEl = document.querySelector<HTMLElement>("#diagnose-panel-install")!;
const panelConfigEl = document.querySelector<HTMLElement>("#diagnose-panel-config")!;
const panelTestEl = document.querySelector<HTMLElement>("#diagnose-panel-test")!;
const installProgressEl = document.querySelector<HTMLElement>("#diagnose-install-progress")!;
const installStatusEl = document.querySelector<HTMLElement>("#diagnose-install-status")!;
const installPercentEl = document.querySelector<HTMLElement>("#diagnose-install-percent")!;
const installFillEl = document.querySelector<HTMLElement>("#diagnose-install-fill")!;
const installLogEl = document.querySelector<HTMLElement>("#diagnose-install-log")!;
const configPersonalEl = document.querySelector<HTMLElement>("#diagnose-config-personal")!;
const configTeamEl = document.querySelector<HTMLElement>("#diagnose-config-team")!;
const teamStatusEl = document.querySelector<HTMLElement>("#diagnose-team-status")!;
const presetChipsEl = document.querySelector<HTMLElement>("#diagnose-preset-chips")!;
const urlEl = document.querySelector<HTMLInputElement>("#diagnose-url")!;
const keyEl = document.querySelector<HTMLInputElement>("#diagnose-key")!;
const modelEl = document.querySelector<HTMLSelectElement>("#diagnose-model")!;
const protocolEl = document.querySelector<HTMLInputElement>("#diagnose-protocol")!;
const providerNameEl = document.querySelector<HTMLInputElement>("#diagnose-provider-name")!;
const primaryEl = document.querySelector<HTMLButtonElement>("#diagnose-primary")!;
const secondaryEl = document.querySelector<HTMLButtonElement>("#diagnose-secondary")!;
const checkListEl = document.querySelector<HTMLElement>("#diagnose-check-list")!;
const checkScrollEl = document.querySelector<HTMLElement>("#diagnose-check-scroll")!;
const detailsEl = document.querySelector<HTMLElement>("#diagnose-details")!;
const scanMeterEl = document.querySelector<HTMLElement>("#diagnose-scan-meter")!;
const scanMeterLabelEl = document.querySelector<HTMLElement>("#diagnose-scan-meter-label")!;
const scanMeterCountEl = document.querySelector<HTMLElement>("#diagnose-scan-meter-count")!;
const scanMeterFillEl = document.querySelector<HTMLElement>("#diagnose-scan-meter-fill")!;
const closeEl = document.querySelector<HTMLButtonElement>("#diagnose-close")!;

/** Perceived scan length even when the backend returns instantly. */
const SCORE_MIN_MS = 2800;
const SCORE_MAX_MS = 4000;

type PrimaryAction =
  | "install"
  | "verify-save"
  | "open-team-wiring"
  | "run-score"
  | "auto-fix"
  | "rescan"
  | "none";

let runtimeId = resolveInitialRuntime();
let displayName = runtimeId;
let activeStep: DiagnoseStepId = "install";
let installed = false;
let configured = false;
let testedOk = false;
let busy = false;
let preview: RepairPreviewResponse | null = null;
let lastScore: DiagnoseScore | null = null;
let primaryAction: PrimaryAction = "none";
let canAutoFix = false;
type CheckFilter = "all" | "pass" | "warn" | "fail";
let checkFilter: CheckFilter = "all";
/** After a failed score, nudge the user to fill key / address on this page. */
let guideFillConfig = false;

function resolveInitialRuntime(): string {
  const injected = window.__AD_DIAGNOSE_RUNTIME__?.trim();
  if (injected) {
    return injected;
  }
  return "openclaw";
}

function setResult(kind: "ok" | "error" | "busy" | "hide", message = ""): void {
  if (kind === "hide" || !message) {
    resultEl.hidden = true;
    resultEl.textContent = "";
    resultEl.className = "diagnose-result is-slot";
    return;
  }
  resultEl.hidden = false;
  resultEl.textContent = message;
  resultEl.className = `diagnose-result is-slot is-${kind}`;
}

function paintSkeletonChecks(count = 8): void {
  checkListEl.innerHTML = Array.from({ length: count }, () => {
    return `<li class="is-pending is-skeleton" aria-hidden="true"><span class="diagnose-check-badge muted">—</span><span>—</span></li>`;
  }).join("");
}

function paintBootShell(): void {
  // First paint: reserve the full test layout so the window does not stretch open.
  titleEl.textContent = displayName;
  ensureDetailsOpen();
  paintHeroTone("busy");
  headlineEl.textContent = t("diagnose.flow.testHeadline");
  detailEl.textContent = t("diagnose.flow.confirming");
  setResult("busy", t("diagnose.flow.scoring"));
  paintScore({ pass: 0, warn: 0, fail: 0, total: 0, percent: 0 }, { animateFromZero: true });
  setScanMeter(0, 0, t("diagnose.flow.confirming"));
  testHintEl.hidden = true;
  primaryEl.hidden = false;
  primaryEl.textContent = t("diagnose.flow.testCta");
  secondaryEl.hidden = false;
  secondaryEl.textContent = t("diagnose.flow.openAskYourself");
  secondaryEl.dataset.fallback = "open-ask";
  if (!checkListEl.querySelector(".is-skeleton") && checkListEl.children.length === 0) {
    paintSkeletonChecks();
  }
}

function setBusy(next: boolean): void {
  busy = next;
  primaryEl.disabled = next;
  secondaryEl.disabled = next;
}

function paintScore(score: DiagnoseScore | null, opts?: { animateFromZero?: boolean }): void {
  // Keep ring + stats in layout to avoid open-time stretch.
  scoreRingEl.hidden = false;
  statRowEl.hidden = false;
  if (!score) {
    scoreValueEl.textContent = "—";
    scoreRingEl.style.setProperty("--readiness", "0");
    scoreRingEl.classList.remove("is-warn", "is-fail", "is-busy", "is-scanning");
    statPassEl.textContent = "—";
    statWarnEl.textContent = "—";
    statFailEl.textContent = "—";
    heroKickerEl.hidden = true;
    syncStatFilterUi(null);
    return;
  }
  scoreRingEl.classList.toggle("is-warn", score.fail === 0 && score.warn > 0);
  scoreRingEl.classList.toggle("is-fail", score.fail > 0);
  scoreRingEl.classList.toggle("is-busy", Boolean(opts?.animateFromZero));
  scoreRingEl.classList.toggle("is-scanning", Boolean(opts?.animateFromZero));
  statPassEl.textContent = String(score.pass);
  statWarnEl.textContent = String(score.warn);
  statFailEl.textContent = String(score.fail);
  heroKickerEl.hidden = Boolean(opts?.animateFromZero);
  if (opts?.animateFromZero) {
    scoreValueEl.textContent = "0";
    scoreRingEl.style.setProperty("--readiness", "0");
    syncStatFilterUi(score);
    return;
  }
  scoreValueEl.textContent = String(score.percent);
  scoreRingEl.style.setProperty("--readiness", String(score.percent));
  syncStatFilterUi(score);
}

function paintHeroTone(tone: "ok" | "warn" | "fail" | "busy" | "neutral"): void {
  for (const el of [heroEl, healthCardEl]) {
    el.classList.toggle("is-ok", tone === "ok");
    el.classList.toggle("is-warn", tone === "warn");
    el.classList.toggle("is-fail", tone === "fail");
    el.classList.toggle("is-busy", tone === "busy");
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function ensureDetailsOpen(): void {
  detailsEl.hidden = false;
}

function focusConfigFill(): void {
  panelConfigEl.hidden = false;
  const target = !urlEl.value.trim() ? urlEl : keyEl;
  try {
    target.focus({ preventScroll: true });
  } catch {
    target.focus();
  }
  panelConfigEl.scrollIntoView({ block: "nearest", behavior: "smooth" });
}

function setScanMeter(current: number, total: number, label: string): void {
  scanMeterEl.hidden = false;
  scanMeterEl.classList.remove("is-idle");
  scanMeterLabelEl.textContent = label;
  scanMeterCountEl.textContent = `${current}/${total}`;
  const percent = total <= 0 ? 0 : Math.round((current / total) * 100);
  scanMeterFillEl.style.width = `${percent}%`;
}

function hideScanMeter(): void {
  // Keep height reserved; only fade so the foot does not collapse.
  scanMeterEl.hidden = false;
  scanMeterEl.classList.add("is-idle");
  scanMeterFillEl.style.width = "0%";
}

function syncStatFilterUi(score: DiagnoseScore | null): void {
  const counts: Record<"pass" | "warn" | "fail", number> = {
    pass: score?.pass ?? 0,
    warn: score?.warn ?? 0,
    fail: score?.fail ?? 0,
  };
  statRowEl.querySelectorAll<HTMLButtonElement>("[data-check-filter]").forEach((btn) => {
    const filter = btn.dataset.checkFilter as "pass" | "warn" | "fail" | undefined;
    if (!filter) {
      return;
    }
    const active = checkFilter === filter;
    btn.classList.toggle("is-active", active);
    btn.setAttribute("aria-selected", active ? "true" : "false");
    btn.disabled = !score || counts[filter] === 0;
    btn.title = t("diagnose.flow.filterHint");
  });
}

function scrollActiveCheckIntoView(): void {
  const active = checkListEl.querySelector<HTMLElement>("li.is-checking, li.is-just-revealed");
  if (!active) {
    return;
  }
  const top = active.offsetTop;
  const bottom = top + active.offsetHeight;
  const viewTop = checkScrollEl.scrollTop;
  const viewBottom = viewTop + checkScrollEl.clientHeight;
  if (top < viewTop + 4) {
    checkScrollEl.scrollTop = Math.max(0, top - 8);
  } else if (bottom > viewBottom - 4) {
    checkScrollEl.scrollTop = bottom - checkScrollEl.clientHeight + 8;
  }
}

function paintChecks(
  report: RepairPreviewResponse | null,
  opts?: { mode?: "final" | "pending" | "reveal"; revealThrough?: number },
): void {
  if (!report) {
    // Keep reserved rows so the details pane does not collapse while scanning.
    if (!checkListEl.querySelector(".is-skeleton")) {
      paintSkeletonChecks();
    }
    return;
  }
  const mode = opts?.mode ?? "final";
  const revealThrough = opts?.revealThrough ?? -1;
  const checks =
    mode === "final" && checkFilter !== "all"
      ? report.checks.filter((check) => check.status === checkFilter)
      : report.checks;

  if (mode === "final" && checks.length === 0) {
    checkListEl.innerHTML = `<li class="is-revealed">${escapeHtml(t("diagnose.flow.filterEmpty"))}</li>`;
    return;
  }

  // Reveal updates rows in place so the list height does not reflow each tick.
  if (mode === "reveal") {
    applyRevealState(report, revealThrough);
    return;
  }

  checkListEl.innerHTML = checks
    .map((check, index) => {
      const body = `<span><strong>${escapeHtml(check.title)}</strong> — ${escapeHtml(check.message)}</span>`;
      if (mode === "pending") {
        return `<li class="is-pending" data-check-index="${index}">
          <span class="diagnose-check-badge muted">${escapeHtml(t("diagnose.flow.waitingConfirm"))}</span>
          ${body}
        </li>`;
      }
      const statusClass = repairStatusClass(check.status);
      return `<li class="is-revealed" data-check-index="${index}">
        <span class="diagnose-check-badge ${statusClass}">${escapeHtml(repairCheckStatusLabel(check.status))}</span>
        ${body}
      </li>`;
    })
    .join("");
}

function applyRevealState(report: RepairPreviewResponse, revealThrough: number): void {
  const total = report.checks.length;
  if (checkListEl.children.length !== total) {
    paintChecks(report, { mode: "pending" });
  }
  report.checks.forEach((check, index) => {
    const li = checkListEl.children[index] as HTMLElement | undefined;
    if (!li) {
      return;
    }
    const badge = li.querySelector(".diagnose-check-badge");
    if (index < revealThrough) {
      li.className = "is-revealed";
      if (badge) {
        badge.className = `diagnose-check-badge ${repairStatusClass(check.status)}`;
        badge.textContent = repairCheckStatusLabel(check.status);
      }
    } else if (index === revealThrough) {
      li.className = "is-just-revealed";
      if (badge) {
        badge.className = `diagnose-check-badge ${repairStatusClass(check.status)}`;
        badge.textContent = repairCheckStatusLabel(check.status);
      }
    } else if (index === revealThrough + 1) {
      li.className = "is-checking";
      if (badge) {
        badge.className = "diagnose-check-badge muted";
        badge.textContent = t("diagnose.flow.checkingNow");
      }
    } else {
      li.className = "is-pending";
      if (badge) {
        badge.className = "diagnose-check-badge muted";
        badge.textContent = t("diagnose.flow.waitingConfirm");
      }
    }
  });
  scrollActiveCheckIntoView();
}

async function animateScoreCount(target: DiagnoseScore): Promise<void> {
  scoreRingEl.hidden = false;
  scoreRingEl.classList.remove("is-scanning", "is-busy");
  scoreRingEl.classList.toggle("is-warn", target.fail === 0 && target.warn > 0);
  scoreRingEl.classList.toggle("is-fail", target.fail > 0);
  const frames = 18;
  for (let i = 1; i <= frames; i += 1) {
    const percent = Math.round((target.percent * i) / frames);
    scoreValueEl.textContent = String(percent);
    scoreRingEl.style.setProperty("--readiness", String(percent));
    await sleep(28);
  }
  scoreValueEl.textContent = String(target.percent);
  scoreRingEl.style.setProperty("--readiness", String(target.percent));
  statPassEl.textContent = String(target.pass);
  statWarnEl.textContent = String(target.warn);
  statFailEl.textContent = String(target.fail);
  statRowEl.hidden = false;
  heroKickerEl.hidden = false;
  syncStatFilterUi(target);
}

async function playSequentialConfirm(report: RepairPreviewResponse): Promise<void> {
  ensureDetailsOpen();
  panelTestEl.hidden = true;
  testHintEl.hidden = true;
  paintHeroTone("busy");
  checkFilter = "all";
  const total = report.checks.length;
  const started = performance.now();
  // List is already mounted by refreshState / runScoreTest; only reset if needed.
  if (checkListEl.children.length !== total) {
    paintChecks(report, { mode: "pending" });
  }
  setScanMeter(0, total, t("diagnose.flow.confirming"));
  paintScore(computeDiagnoseScore(report), { animateFromZero: true });

  if (total === 0) {
    const remain = Math.max(0, SCORE_MIN_MS - (performance.now() - started));
    await sleep(remain);
    return;
  }

  // Spread reveals across 2–4s; keep each tick readable.
  const budget = Math.min(
    SCORE_MAX_MS,
    Math.max(SCORE_MIN_MS, total * 120),
  );
  const perItem = Math.max(90, Math.min(220, Math.floor(budget / total)));

  for (let i = 0; i < total; i += 1) {
    paintChecks(report, { mode: "reveal", revealThrough: i - 1 });
    setScanMeter(
      i,
      total,
      t("diagnose.flow.checkingItem", {
        current: String(i + 1),
        total: String(total),
      }),
    );
    await sleep(Math.floor(perItem * 0.35));
    paintChecks(report, { mode: "reveal", revealThrough: i });
    setScanMeter(
      i + 1,
      total,
      t("diagnose.flow.checkingItem", {
        current: String(i + 1),
        total: String(total),
      }),
    );
    await sleep(Math.floor(perItem * 0.65));
  }

  const elapsed = performance.now() - started;
  if (elapsed < SCORE_MIN_MS) {
    await sleep(SCORE_MIN_MS - elapsed);
  }
  paintChecks(report, { mode: "final" });
  setScanMeter(total, total, t("diagnose.flow.scoreReveal"));
}

function paintSteps(): void {
  const states = stepStatesFor(activeStep, { installed, configured, testedOk });
  const labels: Record<DiagnoseStepId, string> = {
    install: "1",
    config: "2",
    test: "3",
  };
  stepsEl.querySelectorAll<HTMLElement>(".diagnose-step").forEach((el) => {
    const step = el.dataset.step as DiagnoseStepId | undefined;
    if (!step) {
      return;
    }
    const state = states[step];
    el.classList.toggle("is-active", state === "active");
    el.classList.toggle("is-done", state === "done");
    el.classList.toggle("is-error", state === "error");
    const index = el.querySelector<HTMLElement>(".diagnose-step-index");
    if (index) {
      index.textContent = state === "done" ? "✓" : labels[step];
    }
  });
}

function applyPreset(presetId: string): void {
  const preset = PROVIDER_PRESETS[presetId];
  if (!preset) {
    return;
  }
  providerNameEl.value = preset.name;
  urlEl.value = preset.url;
  protocolEl.value = preset.protocol;
  modelEl.innerHTML = "";
  for (const model of preset.models) {
    const option = document.createElement("option");
    option.value = model;
    option.textContent = model;
    modelEl.appendChild(option);
  }
  if (preset.models[0]) {
    modelEl.value = preset.models[0];
  }
  presetChipsEl.querySelectorAll<HTMLButtonElement>(".provider-chip").forEach((chip) => {
    chip.classList.toggle("is-active", chip.dataset.presetId === presetId);
  });
}

function renderPresetChips(): void {
  presetChipsEl.innerHTML = "";
  for (const group of PRESET_PICKER_GROUPS) {
    for (const id of group.ids) {
      const preset = PROVIDER_PRESETS[id];
      if (!preset) {
        continue;
      }
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "provider-chip";
      chip.dataset.presetId = id;
      chip.textContent = preset.chip ?? preset.name;
      chip.addEventListener("click", () => applyPreset(id));
      presetChipsEl.appendChild(chip);
    }
  }
  applyPreset("deepseek");
}

function runtimeFromDoctor(report: DoctorReport): RuntimeDoctorResult | undefined {
  return report.runtimes.find((item) => item.id === runtimeId);
}

async function loadPreview(): Promise<RepairPreviewResponse> {
  return invoke<RepairPreviewResponse>("run_repair_preview_command", { runtime: runtimeId });
}

async function refreshState(opts?: { preferStep?: DiagnoseStepId }): Promise<void> {
  setBusy(true);
  setResult("busy", t("diagnose.flow.scanning"));
  let autoScore = false;
  try {
    const doctor = await invoke<DoctorReport>("run_doctor_command");
    const runtime = runtimeFromDoctor(doctor);
    displayName = runtime?.display_name ?? runtimeId;
    installed = Boolean(runtime?.installed);
    titleEl.textContent = t("diagnose.flow.title", { name: displayName });

    if (installed) {
      preview = await loadPreview();
      lastScore = computeDiagnoseScore(preview);
      canAutoFix = preview.can_apply_repair;
      const wiringNeeded = needsWiringFromPreview(preview);
      if (isPersonalEdition()) {
        const status = await invoke<PersonalProviderStatus>("get_personal_provider_status_command");
        configured = status.configured && !wiringNeeded;
      } else {
        const status = await invoke<EvotownStatus>("get_evotown_status_command");
        configured = status.configured && !wiringNeeded;
        teamStatusEl.textContent = status.configured
          ? t("diagnose.flow.teamConfigured", {
              url: status.base_url ?? "—",
            })
          : t("diagnose.flow.teamMissing");
      }
      testedOk = configured && isPreviewHealthy(preview) && scoreLooksGood(lastScore);
    } else {
      preview = null;
      lastScore = null;
      configured = false;
      testedOk = false;
      canAutoFix = false;
    }

    activeStep =
      opts?.preferStep ??
      pickInitialStep({
        installed,
        needsConfig: installed && !configured,
        healthy: testedOk,
      });

    // Land on config with in-window fill guidance when key / address is still missing.
    guideFillConfig = activeStep === "config" && !configured;

    autoScore = activeStep === "test";
    if (autoScore && preview && lastScore) {
      // Paint the full test shell in one frame so the window does not stretch open.
      testedOk = false;
      checkFilter = "all";
      panelInstallEl.hidden = true;
      panelConfigEl.hidden = true;
      panelTestEl.hidden = true;
      testHintEl.hidden = true;
      ensureDetailsOpen();
      paintHeroTone("busy");
      paintSteps();
      headlineEl.textContent = t("diagnose.flow.testHeadline");
      detailEl.textContent = t("diagnose.flow.confirming");
      setResult("busy", t("diagnose.flow.scoring"));
      paintScore(lastScore, { animateFromZero: true });
      paintChecks(preview, { mode: "pending" });
      setScanMeter(0, preview.checks.length, t("diagnose.flow.confirming"));
      primaryAction = "run-score";
      primaryEl.hidden = false;
      primaryEl.textContent = t("diagnose.flow.testCta");
      secondaryEl.hidden = false;
      secondaryEl.textContent = t("diagnose.flow.openAskYourself");
      secondaryEl.dataset.fallback = "open-ask";
    } else {
      paintAll();
      setResult("hide");
    }
  } catch (error) {
    setResult("error", t("diagnose.flow.scanFailed", { error: String(error) }));
  } finally {
    setBusy(false);
  }
  if (autoScore) {
    void runScoreTest({ reusePreview: true });
  }
}

function paintAll(): void {
  ensureDetailsOpen();
  paintSteps();
  if (activeStep === "test" || testedOk) {
    // Keep the score ring visible during the whole confirm animation.
    paintScore(lastScore ?? { pass: 0, warn: 0, fail: 0, total: 0, percent: 0 }, {
      animateFromZero: !lastScore || (!testedOk && busy),
    });
  } else {
    paintScore(null);
  }
  paintChecks(preview);
  panelInstallEl.hidden = activeStep !== "install";
  panelConfigEl.hidden = activeStep !== "config";
  panelConfigEl.classList.toggle("is-guide", activeStep === "config" && guideFillConfig && !configured);
  panelTestEl.hidden = true;
  testHintEl.hidden = activeStep !== "test";
  if (activeStep !== "test") {
    hideScanMeter();
  }
  configPersonalEl.hidden = !isPersonalEdition();
  configTeamEl.hidden = isPersonalEdition();

  if (activeStep === "install") {
    paintHeroTone("neutral");
    headlineEl.textContent = installed
      ? t("diagnose.flow.installDoneHeadline", { name: displayName })
      : t("diagnose.flow.installHeadline", { name: displayName });
    detailEl.textContent = installed
      ? t("diagnose.flow.installDoneDetail")
      : t("diagnose.flow.installDetail");
    primaryAction = installed ? "none" : "install";
    primaryEl.textContent = t("diagnose.flow.installCta", { name: displayName });
    primaryEl.hidden = installed;
    secondaryEl.hidden = true;
  } else if (activeStep === "config") {
    paintHeroTone(configured ? "ok" : "busy");
    if (guideFillConfig && !configured) {
      headlineEl.textContent = t("diagnose.flow.configGuideHeadline");
      detailEl.textContent = isPersonalEdition()
        ? t("diagnose.flow.configGuideDetail")
        : t("diagnose.flow.configGuideDetailTeam");
    } else {
      headlineEl.textContent = configured
        ? t("diagnose.flow.configDoneHeadline")
        : t("diagnose.flow.configHeadline");
      detailEl.textContent = isPersonalEdition()
        ? t("diagnose.flow.configDetail")
        : t("diagnose.flow.configDetailTeam");
    }
    if (canAutoFix) {
      primaryAction = "auto-fix";
      primaryEl.hidden = false;
      primaryEl.textContent = t("diagnose.flow.autoFixCta");
      secondaryEl.hidden = false;
      secondaryEl.textContent = isPersonalEdition()
        ? guideFillConfig
          ? t("diagnose.flow.configGuideCta")
          : t("diagnose.flow.configCta")
        : t("diagnose.flow.openTeamWiring");
      secondaryEl.dataset.fallback = isPersonalEdition() ? "verify-save" : "open-team-wiring";
    } else if (isPersonalEdition()) {
      primaryAction = "verify-save";
      primaryEl.hidden = false;
      primaryEl.textContent = guideFillConfig
        ? t("diagnose.flow.configGuideCta")
        : t("diagnose.flow.configCta");
      secondaryEl.hidden = true;
    } else {
      primaryAction = "open-team-wiring";
      primaryEl.hidden = false;
      primaryEl.textContent = t("diagnose.flow.openTeamWiring");
      secondaryEl.hidden = true;
    }
    if (guideFillConfig && !configured && isPersonalEdition()) {
      window.requestAnimationFrame(() => focusConfigFill());
    }
  } else {
    if (busy && !testedOk) {
      paintHeroTone("busy");
    } else if (testedOk) {
      paintHeroTone("ok");
    } else if (lastScore && lastScore.fail > 0) {
      paintHeroTone("fail");
    } else if (lastScore && lastScore.warn > 0) {
      paintHeroTone("warn");
    } else {
      paintHeroTone("busy");
    }
    headlineEl.textContent = testedOk
      ? t("diagnose.flow.testOkHeadline")
      : t("diagnose.flow.testHeadline");
    detailEl.textContent = testedOk
      ? t("diagnose.flow.testOkDetail")
      : t("diagnose.flow.testDetail");
    primaryAction = testedOk ? "rescan" : "run-score";
    primaryEl.hidden = false;
    primaryEl.textContent = testedOk
      ? t("diagnose.flow.rescan")
      : t("diagnose.flow.testCta");
    secondaryEl.hidden = false;
    secondaryEl.textContent = t("diagnose.flow.openAskYourself");
    secondaryEl.dataset.fallback = "open-ask";
  }
}

async function runInstall(): Promise<void> {
  setBusy(true);
  installProgressEl.hidden = false;
  installStatusEl.textContent = t("diagnose.flow.installing");
  installPercentEl.textContent = "0%";
  installFillEl.style.width = "0%";
  installLogEl.textContent = "";
  setResult("busy", t("diagnose.flow.installing"));

  const unlisten = await listen<InstallProgressEvent>("install-progress", (event) => {
    if (event.payload.runtime_id !== runtimeId) {
      return;
    }
    const percent = Math.max(0, Math.min(100, Math.round(event.payload.percent)));
    installStatusEl.textContent = event.payload.message || t("diagnose.flow.installing");
    installPercentEl.textContent = `${percent}%`;
    installFillEl.style.width = `${percent}%`;
    if (event.payload.message) {
      installLogEl.textContent = `${installLogEl.textContent}${event.payload.message}\n`.trimStart();
      installLogEl.scrollTop = installLogEl.scrollHeight;
    }
  });

  try {
    const report = await invoke<InstallRuntimeResponse>("install_runtime_command", {
      runtime: runtimeId,
      force: false,
    });
    if (report.install_succeeded || report.after_installed || !report.install_needed) {
      setResult("ok", t("diagnose.flow.installOk"));
      await refreshState({ preferStep: "config" });
    } else {
      const detail =
        report.skipped.map((item) => item.reason).find(Boolean) ||
        report.manual_fallback[0] ||
        t("diagnose.flow.installFailed");
      setResult("error", t("diagnose.flow.installFailedDetail", { error: detail }));
    }
  } catch (error) {
    setResult("error", t("diagnose.flow.installFailedDetail", { error: String(error) }));
  } finally {
    unlisten();
    setBusy(false);
  }
}

async function runAutoFix(): Promise<void> {
  setBusy(true);
  setResult("busy", t("diagnose.flow.autoFixing"));
  try {
    preview = await invoke<RepairPreviewResponse>("run_repair_execute_command", {
      runtime: runtimeId,
    });
    setResult("ok", t("diagnose.flow.autoFixOk"));
    await refreshState({ preferStep: "config" });
  } catch (error) {
    setResult("error", t("diagnose.flow.autoFixFailed", { error: String(error) }));
  } finally {
    setBusy(false);
  }
}

async function runVerifyAndSave(): Promise<void> {
  const url = urlEl.value.trim();
  const key = keyEl.value.trim();
  const model = modelEl.value.trim();
  const name = providerNameEl.value.trim() || "Provider";
  const protocol = (protocolEl.value === "anthropic" ? "anthropic" : "openai") as ProviderProtocol;
  if (!url || !key || !model) {
    setResult("error", t("diagnose.flow.missingFields"));
    return;
  }

  setBusy(true);
  setResult("busy", t("diagnose.flow.verifying"));
  try {
    const verify = await invoke<PersonalProviderVerifyReport>("verify_personal_provider_command", {
      url,
      key,
      protocol,
    });
    if (!verify.ok) {
      setResult("error", t("diagnose.flow.verifyFailed", { error: verify.message }));
      return;
    }

    setResult("busy", t("diagnose.flow.saving"));
    const doc = await invoke<PersonalProvidersDocument>("upsert_personal_provider_command", {
      id: null,
      name,
      url,
      key,
      model,
      protocol,
      activate: false,
    });
    const targetId =
      doc.providers.find((item) => item.name === name && item.url === url)?.id ??
      doc.providers[doc.providers.length - 1]?.id;
    if (!targetId) {
      throw new Error("saved provider id missing");
    }
    const setup = await invoke<PersonalProviderSetupReport>("activate_personal_provider_command", {
      id: targetId,
    });
    keyEl.value = "";
    const probeOk = setup.verify?.ok !== false;
    if (!probeOk) {
      setResult(
        "error",
        t("diagnose.flow.saveProbeFailed", {
          error: setup.verify?.message ?? t("diagnose.flow.verifyFailedShort"),
        }),
      );
      configured = false;
      activeStep = "config";
      paintAll();
      return;
    }
    setResult("ok", t("diagnose.flow.configOk", { name: setup.provider_name ?? name }));
    guideFillConfig = false;
    await refreshState({ preferStep: "test" });
  } catch (error) {
    setResult("error", t("diagnose.flow.configFailed", { error: String(error) }));
  } finally {
    setBusy(false);
  }
}

async function runScoreTest(opts?: { reusePreview?: boolean }): Promise<void> {
  setBusy(true);
  ensureDetailsOpen();
  activeStep = "test";
  guideFillConfig = false;
  panelTestEl.hidden = true;
  testHintEl.hidden = true;
  paintHeroTone("busy");
  headlineEl.textContent = t("diagnose.flow.testHeadline");
  detailEl.textContent = t("diagnose.flow.confirming");
  setResult("busy", t("diagnose.flow.scoring"));

  try {
    let providerOk = true;
    if (opts?.reusePreview && preview && lastScore) {
      paintScore(lastScore, { animateFromZero: true });
      paintChecks(preview, { mode: "pending" });
      setScanMeter(0, preview.checks.length, t("diagnose.flow.confirming"));
      if (isPersonalEdition()) {
        const status = await invoke<PersonalProviderStatus>(
          "get_personal_provider_status_command",
        );
        providerOk = status.configured;
      } else {
        const status = await invoke<EvotownStatus>("get_evotown_status_command");
        providerOk = status.configured;
      }
    } else {
      paintScore({ pass: 0, warn: 0, fail: 0, total: 0, percent: 0 }, { animateFromZero: true });
      setScanMeter(0, preview?.checks.length ?? 0, t("diagnose.flow.confirming"));
      const [nextPreview, ok] = await Promise.all([
        loadPreview(),
        (async () => {
          if (isPersonalEdition()) {
            const status = await invoke<PersonalProviderStatus>(
              "get_personal_provider_status_command",
            );
            return status.configured;
          }
          const status = await invoke<EvotownStatus>("get_evotown_status_command");
          return status.configured;
        })(),
      ]);
      preview = nextPreview;
      lastScore = computeDiagnoseScore(preview);
      providerOk = ok;
      paintScore(lastScore, { animateFromZero: true });
      paintChecks(preview, { mode: "pending" });
      setScanMeter(0, preview.checks.length, t("diagnose.flow.confirming"));
    }

    configured = providerOk && !needsWiringFromPreview(preview);

    await playSequentialConfirm(preview);
    await animateScoreCount(lastScore);

    testedOk = providerOk && scoreLooksGood(lastScore) && preview.summary.fail === 0;
    hideScanMeter();
    scoreRingEl.classList.remove("is-scanning");
    testHintEl.hidden = false;

    if (testedOk) {
      guideFillConfig = false;
      setResult(
        "ok",
        t("diagnose.flow.scoreOk", {
          score: String(lastScore.percent),
          pass: String(lastScore.pass),
          total: String(lastScore.total || lastScore.pass),
        }),
      );
      activeStep = "test";
      paintAll();
    } else {
      const needsFill =
        !providerOk || needsWiringFromPreview(preview);
      const canFixHere = Boolean(preview.can_apply_repair);
      canAutoFix = canFixHere;
      if (needsFill || canFixHere) {
        guideFillConfig = needsFill;
        configured = false;
        activeStep = "config";
        setResult(
          "error",
          needsFill
            ? t("diagnose.flow.scoreNeedsConfig", { score: String(lastScore.percent) })
            : t("diagnose.flow.scoreNeedsAutoFix", { score: String(lastScore.percent) }),
        );
      } else {
        guideFillConfig = false;
        setResult(
          "error",
          t("diagnose.flow.scoreNeedsFix", { score: String(lastScore.percent) }),
        );
      }
      paintAll();
    }
  } catch (error) {
    hideScanMeter();
    setResult("error", t("diagnose.flow.scoreFailed", { error: String(error) }));
  } finally {
    setBusy(false);
  }
}

async function openAskYourself(): Promise<void> {
  try {
    await invoke("open_ask_window_command", { runtime: runtimeId });
  } catch (error) {
    setResult("error", t("runtime.openFailed", { error: String(error) }));
  }
}

async function openTeamWiring(): Promise<void> {
  try {
    await invoke("focus_main_tab_command", { tab: "provider" });
    await invoke("close_diagnose_window_command", { destroy: false });
  } catch (error) {
    setResult("error", String(error));
  }
}

async function closeWindow(): Promise<void> {
  try {
    await invoke("close_diagnose_window_command", { destroy: false });
  } catch {
    try {
      await getCurrentWindow().hide();
    } catch {
      /* ignore */
    }
  }
}

function applyRuntime(next: string): void {
  const trimmed = next.trim();
  if (!trimmed || trimmed === runtimeId) {
    return;
  }
  runtimeId = trimmed;
  void refreshState();
}

window.__AD_DIAGNOSE_APPLY_RUNTIME__ = applyRuntime;

statRowEl.addEventListener("click", (event) => {
  const btn = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-check-filter]");
  if (!btn || btn.disabled || busy) {
    return;
  }
  const next = btn.dataset.checkFilter as CheckFilter | undefined;
  if (!next || (next !== "pass" && next !== "warn" && next !== "fail")) {
    return;
  }
  checkFilter = checkFilter === next ? "all" : next;
  syncStatFilterUi(lastScore);
  paintChecks(preview);
  ensureDetailsOpen();
  checkScrollEl.scrollTop = 0;
});

primaryEl.addEventListener("click", () => {
  if (busy) {
    return;
  }
  switch (primaryAction) {
    case "install":
      void runInstall();
      break;
    case "verify-save":
      void runVerifyAndSave();
      break;
    case "open-team-wiring":
      void openTeamWiring();
      break;
    case "run-score":
      void runScoreTest();
      break;
    case "auto-fix":
      void runAutoFix();
      break;
    case "rescan":
      void refreshState({ preferStep: "test" });
      break;
    default:
      break;
  }
});

secondaryEl.addEventListener("click", () => {
  if (busy) {
    return;
  }
  const fallback = secondaryEl.dataset.fallback;
  if (fallback === "verify-save") {
    void runVerifyAndSave();
  } else if (fallback === "open-team-wiring") {
    void openTeamWiring();
  } else if (fallback === "open-ask") {
    void openAskYourself();
  }
});

closeEl.addEventListener("click", () => {
  void closeWindow();
});

document.documentElement.classList.add("is-opaque-shell");
document.documentElement.lang = getLocale() === "zh" ? "zh-CN" : "en";
applyStaticI18n(document);
renderPresetChips();
paintBootShell();
void refreshState();

void listen<{ runtime?: string }>("diagnose-window-focus", (event) => {
  const next = event.payload?.runtime?.trim();
  if (next) {
    applyRuntime(next);
  } else {
    void refreshState();
  }
});
