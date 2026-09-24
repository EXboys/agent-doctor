import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  computeDiagnoseScore,
  needsWiringFromPreview,
  scoreLooksGood,
  type DiagnoseStepId,
} from "../diagnose-flow";
import { isPersonalEdition } from "../edition";
import { withErrorDetail, formatProviderFailure, withProviderFailure } from "../friendly-error";
import { isDesktopAppRuntimeId } from "../agents-ui";
import { t } from "../i18n";
import { ASK_VERIFY_DRAFT_KEY } from "../chat/types";
import type {
  EvotownStatus,
  InstallProgressEvent,
  InstallRuntimeResponse,
  PersonalProviderSetupReport,
  PersonalProviderStatus,
  PersonalProvidersDocument,
  PersonalProviderVerifyReport,
  ProviderProtocol,
  RepairPreviewResponse,
} from "../types";
import * as dom from "./dom";
import type { DiagnosePaintApi } from "./paint";
import type { DiagnoseSession } from "./session";

export type DiagnoseActionsDeps = {
  session: DiagnoseSession;
  paint: DiagnosePaintApi;
  refreshState: (opts?: { preferStep?: DiagnoseStepId }) => Promise<void>;
  loadPreview: () => Promise<RepairPreviewResponse>;
};

export type DiagnoseActionsApi = ReturnType<typeof createDiagnoseActions>;

export function createDiagnoseActions(deps: DiagnoseActionsDeps) {
  const { session, paint } = deps;

  async function runInstall(): Promise<void> {
    paint.setBusy(true);
    dom.installProgressEl.hidden = false;
    dom.installStatusEl.textContent = t("diagnose.flow.installing");
    dom.installPercentEl.textContent = "0%";
    dom.installFillEl.style.width = "0%";
    dom.installLogEl.textContent = "";
    paint.setResult("busy", t("diagnose.flow.installing"));

    const unlisten = await listen<InstallProgressEvent>("install-progress", (event) => {
      if (event.payload.runtime_id !== session.runtimeId) {
        return;
      }
      const percent = Math.max(0, Math.min(100, Math.round(event.payload.percent)));
      dom.installStatusEl.textContent = event.payload.message || t("diagnose.flow.installing");
      dom.installPercentEl.textContent = `${percent}%`;
      dom.installFillEl.style.width = `${percent}%`;
      if (event.payload.message) {
        dom.installLogEl.textContent =
          `${dom.installLogEl.textContent}${event.payload.message}\n`.trimStart();
        dom.installLogEl.scrollTop = dom.installLogEl.scrollHeight;
      }
    });

    try {
      const report = await invoke<InstallRuntimeResponse>("install_runtime_command", {
        runtime: session.runtimeId,
        force: false,
      });
      if (report.install_succeeded || report.after_installed || !report.install_needed) {
        paint.setResult("ok", t("diagnose.flow.installOk"));
        await deps.refreshState({ preferStep: "config" });
      } else {
        const detail =
          report.skipped.map((item) => item.reason).find(Boolean) ||
          report.manual_fallback[0] ||
          t("diagnose.flow.installFailed");
        paint.setResult("error", t("diagnose.flow.installFailedDetail", { error: detail }));
      }
    } catch (error) {
      paint.setResult("error", withErrorDetail(t("diagnose.flow.installFailedDetail"), error));
    } finally {
      unlisten();
      paint.setBusy(false);
    }
  }

  async function runAutoFix(): Promise<void> {
    paint.setBusy(true);
    paint.setResult("busy", t("diagnose.flow.autoFixing"));
    try {
      session.preview = await invoke<RepairPreviewResponse>("run_repair_execute_command", {
        runtime: session.runtimeId,
      });
      paint.setResult("ok", t("diagnose.flow.autoFixOk"));
      await deps.refreshState({ preferStep: "test" });
    } catch (error) {
      paint.setResult("error", withErrorDetail(t("diagnose.flow.autoFixFailed"), error));
    } finally {
      paint.setBusy(false);
    }
  }

  async function runVerifyAndSave(): Promise<void> {
    const url = dom.urlEl.value.trim();
    const key = dom.keyEl.value.trim();
    const model = dom.modelEl.value.trim();
    const name = dom.providerNameEl.value.trim() || "Provider";
    const protocol = (
      dom.protocolEl.value === "anthropic" ? "anthropic" : "openai"
    ) as ProviderProtocol;
    if (!url || !key || !model) {
      paint.setResult("error", t("diagnose.flow.missingFields"));
      return;
    }

    paint.setBusy(true);
    paint.setResult("busy", t("diagnose.flow.verifying"));
    try {
      const verify = await invoke<PersonalProviderVerifyReport>("verify_personal_provider_command", {
        url,
        key,
        protocol,
      });
      if (!verify.ok) {
        paint.setResult(
          "error",
          formatProviderFailure(verify.message, { statusCode: verify.status_code }),
        );
        return;
      }

      paint.setResult("busy", t("diagnose.flow.saving"));
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
      dom.keyEl.value = "";
      const probeOk = setup.verify?.ok !== false;
      if (!probeOk) {
        paint.setResult(
          "error",
          formatProviderFailure(
            setup.verify?.message ?? t("diagnose.flow.verifyFailedShort"),
            { statusCode: setup.verify?.status_code },
          ),
        );
        session.configured = false;
        session.activeStep = "config";
        paint.paintAll();
        return;
      }
      paint.setResult("ok", t("diagnose.flow.configOk", { name: setup.provider_name ?? name }));
      session.guideFillConfig = false;
      await deps.refreshState({ preferStep: "test" });
    } catch (error) {
      paint.setResult("error", withProviderFailure("diagnose.flow.configFailed", error));
    } finally {
      paint.setBusy(false);
    }
  }

  async function runScoreTest(opts?: { reusePreview?: boolean }): Promise<void> {
    paint.setBusy(true);
    paint.ensureDetailsOpen();
    session.activeStep = "test";
    session.guideFillConfig = false;
    dom.panelTestEl.hidden = true;
    dom.testHintEl.hidden = true;
    paint.paintHeroTone("busy");
    dom.headlineEl.textContent = t("diagnose.flow.testHeadline");
    dom.detailEl.textContent = t("diagnose.flow.confirming");
    paint.setResult("busy", t("diagnose.flow.scoring"));

    try {
      let providerOk = true;
      if (opts?.reusePreview && session.preview && session.lastScore) {
        paint.paintScore(session.lastScore, { animateFromZero: true });
        paint.paintChecks(session.preview, { mode: "pending" });
        paint.setScanMeter(0, session.preview.checks.length, t("diagnose.flow.confirming"));
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
        paint.paintScore(
          { pass: 0, warn: 0, fail: 0, total: 0, percent: 0 },
          { animateFromZero: true },
        );
        paint.setScanMeter(0, session.preview?.checks.length ?? 0, t("diagnose.flow.confirming"));
        const [nextPreview, ok] = await Promise.all([
          deps.loadPreview(),
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
        session.preview = nextPreview;
        session.lastScore = computeDiagnoseScore(session.preview);
        providerOk = ok;
        paint.paintScore(session.lastScore, { animateFromZero: true });
        paint.paintChecks(session.preview, { mode: "pending" });
        paint.setScanMeter(0, session.preview.checks.length, t("diagnose.flow.confirming"));
      }

      session.configured = providerOk && !needsWiringFromPreview(session.preview!);

      await paint.playSequentialConfirm(session.preview!);
      await paint.animateScoreCount(session.lastScore!);

      session.testedOk =
        providerOk &&
        scoreLooksGood(session.lastScore!) &&
        session.preview!.summary.fail === 0;
      paint.hideScanMeter();
      dom.scoreRingEl.classList.remove("is-scanning");
      dom.testHintEl.hidden = false;

      if (session.testedOk) {
        session.guideFillConfig = false;
        paint.setResult(
          "ok",
          t("diagnose.flow.scoreOk", {
            score: String(session.lastScore!.percent),
            pass: String(session.lastScore!.pass),
            total: String(session.lastScore!.total || session.lastScore!.pass),
          }),
        );
        session.activeStep = "test";
        paint.paintAll();
      } else {
        const needsFill = !providerOk || needsWiringFromPreview(session.preview!);
        const canFixHere = Boolean(session.preview!.can_apply_repair);
        session.canAutoFix = canFixHere;
        if (needsFill || canFixHere) {
          session.guideFillConfig = needsFill;
          session.configured = false;
          session.activeStep = "config";
          paint.setResult(
            "error",
            needsFill
              ? t("diagnose.flow.scoreNeedsConfig", {
                  score: String(session.lastScore!.percent),
                })
              : t("diagnose.flow.scoreNeedsAutoFix", {
                  score: String(session.lastScore!.percent),
                }),
          );
        } else {
          session.guideFillConfig = false;
          paint.setResult(
            "error",
            t("diagnose.flow.scoreNeedsFix", { score: String(session.lastScore!.percent) }),
          );
        }
        paint.paintAll();
      }
    } catch (error) {
      paint.hideScanMeter();
      paint.setResult("error", withErrorDetail(t("diagnose.flow.scoreFailed"), error));
    } finally {
      paint.setBusy(false);
    }
  }

  async function openDesktopApp(): Promise<void> {
    await invoke("open_session_command", {
      runtime: session.runtimeId,
      cwd: null,
      prompt: null,
      terminal: null,
    });
    paint.setResult("ok", t("diagnose.flow.openedDesktop", { name: session.displayName }));
  }

  async function openAskYourself(): Promise<void> {
    try {
      if (isDesktopAppRuntimeId(session.runtimeId)) {
        await openDesktopApp();
        return;
      }
      await invoke("open_ask_window_command", { runtime: session.runtimeId });
    } catch (error) {
      paint.setResult("error", withErrorDetail(t("runtime.openFailed"), error));
    }
  }

  async function openAskForVerify(): Promise<void> {
    try {
      if (isDesktopAppRuntimeId(session.runtimeId)) {
        await openDesktopApp();
        return;
      }
      localStorage.setItem(
        ASK_VERIFY_DRAFT_KEY,
        JSON.stringify({ prompt: t("ask.verifyPrompt"), autoSend: true }),
      );
      await invoke("open_ask_window_command", { runtime: session.runtimeId });
      paint.setResult("ok", t("diagnose.flow.askVerifyHint"));
    } catch (error) {
      paint.setResult("error", withErrorDetail(t("runtime.openFailed"), error));
    }
  }

  async function openTeamWiring(): Promise<void> {
    try {
      await invoke("focus_main_tab_command", { tab: "provider" });
      await invoke("close_diagnose_window_command", { destroy: false });
    } catch (error) {
      paint.setResult("error", withErrorDetail(t("diagnose.flow.scanFailed"), error));
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

  return {
    runInstall,
    runAutoFix,
    runVerifyAndSave,
    runScoreTest,
    openAskYourself,
    openAskForVerify,
    openTeamWiring,
    closeWindow,
  };
}
