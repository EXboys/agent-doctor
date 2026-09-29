
import { listen } from "@tauri-apps/api/event";
import { applyStaticI18n, getLocale, t } from "./i18n";
import { withErrorDetail } from "./friendly-error";
import { isPersonalEdition } from "./edition";
import {
  computeDiagnoseScore,
  isPreviewHealthy,
  needsWiringFromPreview,
  pickInitialStep,
  scoreLooksGood,
  type DiagnoseStepId,
} from "./diagnose-flow";
import type {
  DoctorReport,
  RepairPreviewResponse,
  RuntimeDoctorResult,
} from "./types";
import { createDiagnoseActions } from "./diagnose/actions";
import { createDeepChat, type DeepPresetId } from "./diagnose/deep-chat";
import * as dom from "./diagnose/dom";
import { createDiagnosePaint } from "./diagnose/paint";
import { renderPresetChips } from "./diagnose/presets";
import {
  createDiagnoseSession,
  rememberDiagnoseRuntime,
  resolveInitialRuntime,
  type CheckFilter,
} from "./diagnose/session";
import { runRepairPreview, runDoctor, getPersonalProviderStatus, getEvotownStatus } from "./ipc";

declare global {
  interface Window {
    __AD_DIAGNOSE_RUNTIME__?: string;
    __AD_DIAGNOSE_APPLY_RUNTIME__?: (runtime: string) => void;
  }
}

const session = createDiagnoseSession(resolveInitialRuntime());
rememberDiagnoseRuntime(session.runtimeId);
const deepChat = createDeepChat(session, {
  onRepaired: () => refreshState({ preferStep: "test" }),
});
const paint = createDiagnosePaint(session, deepChat);

function runtimeFromDoctor(report: DoctorReport): RuntimeDoctorResult | undefined {
  return report.runtimes.find((item) => item.id === session.runtimeId);
}

async function loadPreview(): Promise<RepairPreviewResponse> {
  return runRepairPreview({
    runtime: session.runtimeId,
  });
}

async function refreshState(opts?: { preferStep?: DiagnoseStepId }): Promise<void> {
  paint.setBusy(true);
  paint.setResult("busy", t("diagnose.flow.scanning"));
  let autoScore = false;
  try {
    const doctor = await runDoctor();
    const runtime = runtimeFromDoctor(doctor);
    session.displayName = runtime?.display_name ?? session.runtimeId;
    session.installed = Boolean(runtime?.installed);
    dom.titleEl.textContent = t("diagnose.flow.title", { name: session.displayName });

    if (session.installed) {
      session.preview = await loadPreview();
      session.lastScore = computeDiagnoseScore(session.preview);
      session.canAutoFix = session.preview.can_apply_repair;
      const wiringNeeded = needsWiringFromPreview(session.preview);
      if (isPersonalEdition()) {
        const status = await getPersonalProviderStatus();
        session.configured = status.configured && !wiringNeeded;
      } else {
        const status = await getEvotownStatus();
        session.configured = status.configured && !wiringNeeded;
        dom.teamStatusEl.textContent = status.configured
          ? t("diagnose.flow.teamConfigured", {
              url: status.base_url ?? "—",
            })
          : t("diagnose.flow.teamMissing");
      }
      session.testedOk =
        session.configured &&
        isPreviewHealthy(session.preview) &&
        scoreLooksGood(session.lastScore);
    } else {
      session.preview = null;
      session.lastScore = null;
      session.configured = false;
      session.testedOk = false;
      session.canAutoFix = false;
    }

    session.activeStep =
      opts?.preferStep ??
      pickInitialStep({
        installed: session.installed,
        needsConfig: session.installed && !session.configured,
        healthy: session.testedOk,
      });

    // Land on config with in-window fill guidance when key / address is still missing.
    session.guideFillConfig = session.activeStep === "config" && !session.configured;

    autoScore = session.activeStep === "test";
    if (autoScore && session.preview && session.lastScore) {
      // Paint the full test shell in one frame so the window does not stretch open.
      session.testedOk = false;
      session.checkFilter = "all";
      dom.panelInstallEl.hidden = true;
      dom.panelConfigEl.hidden = true;
      dom.panelTestEl.hidden = true;
      dom.testHintEl.hidden = true;
      paint.ensureDetailsOpen();
      paint.paintHeroTone("busy");
      paint.paintSteps();
      dom.headlineEl.textContent = t("diagnose.flow.testHeadline");
      dom.detailEl.textContent = t("diagnose.flow.confirming");
      paint.setResult("busy", t("diagnose.flow.scoring"));
      paint.paintScore(session.lastScore, { animateFromZero: true });
      paint.paintChecks(session.preview, { mode: "pending" });
      paint.setScanMeter(0, session.preview.checks.length, t("diagnose.flow.confirming"));
      session.primaryAction = "run-score";
      dom.primaryEl.hidden = false;
      dom.primaryEl.textContent = t("diagnose.flow.testCta");
      dom.secondaryEl.hidden = false;
      dom.secondaryEl.textContent = t("diagnose.flow.openAskYourself");
      dom.secondaryEl.dataset.fallback = "open-ask";
    } else {
      paint.paintAll();
      paint.setResult("hide");
    }
  } catch (error) {
    paint.setResult("error", withErrorDetail(t("diagnose.flow.scanFailed"), error));
  } finally {
    paint.setBusy(false);
  }
  if (autoScore) {
    void actions.runScoreTest({ reusePreview: true });
  }
}

const actions = createDiagnoseActions({
  session,
  paint,
  refreshState,
  loadPreview,
});

function applyRuntime(next: string): void {
  const trimmed = next.trim();
  if (!trimmed) {
    return;
  }
  rememberDiagnoseRuntime(trimmed);
  if (trimmed === session.runtimeId) {
    return;
  }
  session.runtimeId = trimmed;
  void refreshState();
}

window.__AD_DIAGNOSE_APPLY_RUNTIME__ = applyRuntime;

dom.checkTabsEl.addEventListener("click", (event) => {
  const btn = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-check-filter]");
  if (!btn || btn.disabled || session.busy) {
    return;
  }
  const next = btn.dataset.checkFilter as CheckFilter | undefined;
  if (!next || (next !== "all" && next !== "pass" && next !== "warn" && next !== "fail")) {
    return;
  }
  session.checkFilter = next;
  paint.syncStatFilterUi(session.lastScore);
  paint.paintChecks(session.preview);
  paint.ensureDetailsOpen();
  dom.checkScrollEl.scrollTop = 0;
});

dom.primaryEl.addEventListener("click", () => {
  if (session.busy) {
    return;
  }
  switch (session.primaryAction) {
    case "install":
      void actions.runInstall();
      break;
    case "verify-save":
      void actions.runVerifyAndSave();
      break;
    case "open-team-wiring":
      void actions.openTeamWiring();
      break;
    case "run-score":
      void actions.runScoreTest();
      break;
    case "auto-fix":
      void actions.runAutoFix();
      break;
    case "ask-verify":
      void actions.openAskForVerify();
      break;
    case "open-ask":
      void actions.openAskYourself();
      break;
    case "rescan":
      void refreshState({ preferStep: "test" });
      break;
    default:
      break;
  }
});

dom.secondaryEl.addEventListener("click", () => {
  if (session.busy) {
    return;
  }
  const fallback = dom.secondaryEl.dataset.fallback;
  if (fallback === "verify-save") {
    void actions.runVerifyAndSave();
  } else if (fallback === "open-team-wiring") {
    void actions.openTeamWiring();
  } else if (fallback === "rescan") {
    void refreshState({ preferStep: "test" });
  } else if (fallback === "open-ask") {
    void actions.openAskYourself();
  }
});

dom.deepToggleEl.addEventListener("click", () => {
  // Do not gate on session.busy — scoring must not block opening the AI chat.
  actions.toggleDeep();
});

dom.deepCloseEl.addEventListener("click", () => {
  actions.toggleDeep(false);
});

window.addEventListener("keydown", (event) => {
  if (event.key !== "Escape" || event.defaultPrevented || event.isComposing) {
    return;
  }
  if (!session.deepOpen) {
    return;
  }
  event.preventDefault();
  actions.toggleDeep(false);
});

function handleDeepBodyClick(event: Event): void {
  const target = event.target as HTMLElement | null;
  if (!target) {
    return;
  }
  const preset = target.closest<HTMLButtonElement>("[data-deep-preset]");
  if (preset?.dataset.deepPreset) {
    event.preventDefault();
    void deepChat.usePreset(preset.dataset.deepPreset as DeepPresetId);
    return;
  }
  if (target.closest("#diagnose-deep-stop")) {
    event.preventDefault();
    void deepChat.stop();
    return;
  }
  if (target.closest("[data-deep-repair]")) {
    event.preventDefault();
    void deepChat.repair();
  }
}

// Listen on the drawer root so clicks still work after the body HTML is replaced.
dom.deepEl.addEventListener("click", handleDeepBodyClick);

dom.deepBodyEl.addEventListener("submit", (event) => {
  const form = (event.target as HTMLElement).closest("#diagnose-deep-form");
  if (!form) {
    return;
  }
  event.preventDefault();
  void deepChat.send();
});

document.documentElement.classList.add("is-opaque-shell");
document.documentElement.lang = getLocale() === "zh" ? "zh-CN" : "en";
applyStaticI18n(document);
renderPresetChips();
paint.paintBootShell();
void refreshState();

void listen<{ runtime?: string }>("diagnose-window-focus", (event) => {
  const next = event.payload?.runtime?.trim();
  if (next) {
    applyRuntime(next);
  } else {
    void refreshState();
  }
});

window.addEventListener("beforeunload", () => {
  deepChat.dispose();
});
