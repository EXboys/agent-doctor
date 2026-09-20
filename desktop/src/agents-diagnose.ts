import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { escapeHtml } from "./format";
import {
  isAskRuntimeId,
  renderRuntimeCardActions,
  runtimeAdvancedMeta,
  supportsBrowserMcp,
  type RuntimeCardActionContext,
} from "./agents-ui";
import {
  preferredRepairFilter,
  renderDiagnosePendingHtml,
  renderRelatedResourcesHtml,
  renderRepairPreview,
} from "./repair-ui";
import {
  appState,
  repairConfirmRuntimeIds,
  repairFilterByRuntime,
  repairPreviewByRuntime,
} from "./app-state";
import type {
  DoctorReport,
  RepairPreviewResponse,
  RepairStatusFilter,
  RestoreSummary,
  RuntimeDoctorResult,
  WindowSizeReport,
} from "./types";

export const MAIN_COMPACT_WIDTH = 420;
export const MAIN_DETAIL_EXTRA = 380;

export interface AgentsDiagnoseDeps {
  setStatusBanner: (kind: "ok" | "warn" | "error" | "neutral", message: string) => void;
  updateAgentsSecurityOverview: (report: DoctorReport) => void;
  loadHermesModel: () => Promise<void>;
  getRuntimesEl: () => HTMLElement;
  hasActiveWorkspace: () => boolean;
  openAskWindow: (runtime: string) => Promise<void>;
  openAskWindowForVerify: (runtime: string) => Promise<void>;
  setMainTab: (tab: "provider") => void;
}

export function createAgentsDiagnose(deps: AgentsDiagnoseDeps) {
  const diagnoseDetailEl = document.querySelector<HTMLElement>("#diagnose-detail")!;
  const diagnoseDetailBodyEl = document.querySelector<HTMLElement>("#diagnose-detail-body")!;

  let diagnoseDetailOpen = false;
  let compactWidthBeforeDetail: number | null = null;
  const dismissedDiagnoseRuntimes = new Set<string>();

  function runtimeCardEl(runtime: string): HTMLElement | null {
    return deps.getRuntimesEl().querySelector<HTMLElement>(`[data-runtime="${runtime}"]`);
  }

  function isDiagnoseDetailOpenFor(runtimeId: string): boolean {
    return !diagnoseDetailEl.hidden && diagnoseDetailEl.dataset.runtime === runtimeId;
  }

  function hasDismissed(runtimeId: string): boolean {
    return dismissedDiagnoseRuntimes.has(runtimeId);
  }

  async function setMainWindowWidth(width: number): Promise<WindowSizeReport> {
    return invoke<WindowSizeReport>("resize_main_window_command", {
      width,
      height: null,
    });
  }

  async function readMainWindowSize(): Promise<WindowSizeReport> {
    return invoke<WindowSizeReport>("resize_main_window_command", {
      width: null,
      height: null,
    });
  }

  function diagnoseRuntimeLabel(runtimeId: string): string {
    return (
      appState.lastReport?.runtimes.find((item) => item.id === runtimeId)?.display_name ?? runtimeId
    );
  }

  async function expandDiagnoseWindowIfNeeded(): Promise<void> {
    diagnoseDetailEl.hidden = false;
    document.body.classList.add("is-diagnose-layout");
    if (!diagnoseDetailOpen) {
      try {
        compactWidthBeforeDetail = (await readMainWindowSize()).width;
      } catch {
        compactWidthBeforeDetail = MAIN_COMPACT_WIDTH;
      }
      const compact = compactWidthBeforeDetail ?? MAIN_COMPACT_WIDTH;
      document.body.style.setProperty("--compact-window-width", `${compact}px`);
      diagnoseDetailOpen = true;
      try {
        await setMainWindowWidth(compact + MAIN_DETAIL_EXTRA);
      } catch (error) {
        deps.setStatusBanner("error", t("runtime.openFailed", { error: String(error) }));
      }
    }
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    document.body.classList.add("is-diagnose-open");
  }

  function showDiagnosePending(
    runtimeId: string,
    message: string,
    step: "diagnose" | "repair" = "diagnose",
  ): void {
    diagnoseDetailBodyEl.innerHTML = renderDiagnosePendingHtml(
      runtimeId,
      diagnoseRuntimeLabel(runtimeId),
      message,
      step,
    );
    diagnoseDetailEl.dataset.runtime = runtimeId;
    void expandDiagnoseWindowIfNeeded();
  }

  function runtimeCardActionContext(runtime: RuntimeDoctorResult): RuntimeCardActionContext {
    return {
      preview: repairPreviewByRuntime.get(runtime.id),
      confirmPending: repairConfirmRuntimeIds.has(runtime.id),
      diagnoseOpenForRuntime: isDiagnoseDetailOpenFor(runtime.id),
      dismissed: dismissedDiagnoseRuntimes.has(runtime.id),
      hasActiveWorkspace: deps.hasActiveWorkspace(),
      isAskRuntime: isAskRuntimeId(runtime.id),
      supportsBrowserMcp: supportsBrowserMcp(runtime.id),
    };
  }

  function refreshRuntimeCardActions(card: HTMLElement, runtimeId: string): void {
    const runtime = appState.lastReport?.runtimes.find((item) => item.id === runtimeId);
    if (!runtime) {
      return;
    }
    const actions = card.querySelector(".card-actions");
    if (!actions) {
      return;
    }
    const html = renderRuntimeCardActions(
      runtime,
      runtimeAdvancedMeta(runtime, appState.hermesModel),
      runtimeCardActionContext(runtime),
    );
    if (html) {
      actions.innerHTML = html;
    }
  }

  async function openDiagnoseDetail(report: RepairPreviewResponse): Promise<void> {
    const filter = repairFilterByRuntime.get(report.runtime_id) ?? "all";
    diagnoseDetailBodyEl.innerHTML = renderRepairPreview(report, filter, {
      confirmPending: repairConfirmRuntimeIds.has(report.runtime_id),
      isAskRuntime: isAskRuntimeId(report.runtime_id),
      supportsBrowserMcp: supportsBrowserMcp(report.runtime_id),
    });
    diagnoseDetailEl.dataset.runtime = report.runtime_id;
    await expandDiagnoseWindowIfNeeded();
    const card = runtimeCardEl(report.runtime_id);
    if (card) {
      refreshRuntimeCardActions(card, report.runtime_id);
    }
  }

  async function closeDiagnoseDetail(opts?: {
    keepContent?: boolean;
    skipDismiss?: boolean;
  }): Promise<void> {
    const runtime = diagnoseDetailEl.dataset.runtime;
    if (runtime) {
      repairConfirmRuntimeIds.delete(runtime);
    }
    if (!opts?.keepContent && !opts?.skipDismiss && runtime) {
      dismissedDiagnoseRuntimes.add(runtime);
    }
    document.body.classList.remove("is-diagnose-open");
    if (diagnoseDetailOpen) {
      await new Promise((resolve) => window.setTimeout(resolve, 180));
    }
    diagnoseDetailEl.hidden = true;
    if (!opts?.keepContent) {
      diagnoseDetailBodyEl.replaceChildren();
      delete diagnoseDetailEl.dataset.runtime;
    }
    diagnoseDetailOpen = false;
    if (runtime) {
      const card = runtimeCardEl(runtime);
      if (card) {
        refreshRuntimeCardActions(card, runtime);
      }
    }
    const compact = compactWidthBeforeDetail ?? MAIN_COMPACT_WIDTH;
    compactWidthBeforeDetail = null;
    await setMainWindowWidth(compact);
    document.body.classList.remove("is-diagnose-layout");
    document.body.style.removeProperty("--compact-window-width");
  }

  function mountRelatedResources(card: HTMLElement, runtime: string): void {
    const el = card.querySelector<HTMLElement>("[data-related-resources]");
    if (!el) {
      return;
    }
    el.outerHTML = renderRelatedResourcesHtml(repairPreviewByRuntime.get(runtime));
  }

  function mountRepairPreview(report: RepairPreviewResponse, opts?: { resetFilter?: boolean }): void {
    const runtime = report.runtime_id;
    dismissedDiagnoseRuntimes.delete(runtime);
    repairPreviewByRuntime.set(runtime, report);
    if (appState.lastReport) {
      deps.updateAgentsSecurityOverview(appState.lastReport);
    }
    if (opts?.resetFilter || !repairFilterByRuntime.has(runtime)) {
      repairFilterByRuntime.set(runtime, preferredRepairFilter(report));
    }
    const card = runtimeCardEl(runtime);
    const hint = card?.querySelector<HTMLElement>("[data-repair-hint]");
    if (hint && !hint.querySelector("[data-install-progress]")) {
      hint.hidden = true;
      hint.replaceChildren();
    }
    if (card) {
      mountRelatedResources(card, runtime);
      refreshRuntimeCardActions(card, runtime);
    }
    void openDiagnoseDetail(report);
  }

  function applyRepairFilter(runtime: string, filter: RepairStatusFilter): void {
    const report = repairPreviewByRuntime.get(runtime);
    if (!report) {
      return;
    }
    const current = repairFilterByRuntime.get(runtime) ?? "all";
    const next = current === filter && filter !== "all" ? "all" : filter;
    repairFilterByRuntime.set(runtime, next);
    diagnoseDetailBodyEl.innerHTML = renderRepairPreview(report, next, {
      confirmPending: repairConfirmRuntimeIds.has(report.runtime_id),
      isAskRuntime: isAskRuntimeId(report.runtime_id),
      supportsBrowserMcp: supportsBrowserMcp(report.runtime_id),
    });
  }

  async function openRepairGuide(path: string) {
    await invoke("open_path_command", { path });
  }

  async function runBrowserSmokeFromCard(root: HTMLElement): Promise<void> {
    const host = diagnoseDetailEl.hidden ? root : diagnoseDetailEl;
    const slot = host.querySelector<HTMLElement>("[data-browser-smoke-slot]");
    const button = host.querySelector<HTMLButtonElement>('[data-action="browser-smoke"]');
    button?.setAttribute("disabled", "true");
    if (slot) {
      slot.className = "repair-smoke-slot";
      slot.textContent = t("repair.browserSmokeRunning");
    }
    try {
      const smoke = await invoke<{ ok: boolean; detail: string }>("run_browser_smoke_command");
      if (slot) {
        slot.className = `repair-smoke-slot ${smoke.ok ? "ok" : "fail"}`;
        slot.textContent = `${smoke.ok ? t("repair.browserSmokeOk") : t("repair.browserSmokeFail")}: ${smoke.detail}`;
      }
    } catch (error) {
      if (slot) {
        slot.className = "repair-smoke-slot fail";
        slot.textContent = `${t("repair.browserSmokeFail")}: ${String(error)}`;
      }
    } finally {
      button?.removeAttribute("disabled");
    }
  }

  async function rollbackRepairRuntimeCard(card: HTMLElement) {
    const runtime = card.dataset.runtime;
    const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
    const diagnoseButton = card.querySelector<HTMLButtonElement>('[data-action="diagnose-runtime"]');
    const applyButton = card.querySelector<HTMLButtonElement>('[data-action="apply-repair"]');
    const rollbackButton = card.querySelector<HTMLButtonElement>('[data-action="rollback-repair"]');
    if (!runtime || !hint) {
      return;
    }
    diagnoseButton?.setAttribute("disabled", "true");
    applyButton?.setAttribute("disabled", "true");
    rollbackButton?.setAttribute("disabled", "true");
    hint.hidden = false;
    hint.textContent = t("repair.rollingBack");
    try {
      const restore = await invoke<RestoreSummary>("run_repair_rollback_command", {
        runtime,
        backup: null,
      });
      const report = await invoke<RepairPreviewResponse>("run_repair_preview_command", { runtime });
      mountRepairPreview(report, { resetFilter: true });
      diagnoseDetailBodyEl.insertAdjacentHTML(
        "afterbegin",
        `<p class="repair-rollback-ok">${escapeHtml(
          t("repair.rollbackDone", {
            id: restore.backup_id,
            count: String(restore.restored_files.length),
          }),
        )}</p>`,
      );
      if (runtime === "hermes") {
        await deps.loadHermesModel();
      }
    } catch (error) {
      hint.textContent = String(error);
    } finally {
      diagnoseButton?.removeAttribute("disabled");
      applyButton?.removeAttribute("disabled");
      rollbackButton?.removeAttribute("disabled");
    }
  }

  async function applyRepairRuntimeCard(card: HTMLElement) {
    const runtime = card.dataset.runtime;
    const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
    const diagnoseButton = card.querySelector<HTMLButtonElement>('[data-action="diagnose-runtime"]');
    const applyButton = card.querySelector<HTMLButtonElement>('[data-action="apply-repair"]');
    if (!runtime || !hint) {
      return;
    }
    diagnoseButton?.setAttribute("disabled", "true");
    applyButton?.setAttribute("disabled", "true");
    hint.hidden = false;
    hint.textContent = t("repair.applying");
    showDiagnosePending(runtime, t("repair.applying"), "repair");
    try {
      const report = await invoke<RepairPreviewResponse>("run_repair_execute_command", { runtime });
      mountRepairPreview(report, { resetFilter: true });
      if (runtime === "hermes") {
        await deps.loadHermesModel();
      }
    } catch (error) {
      hint.textContent = String(error);
    } finally {
      diagnoseButton?.removeAttribute("disabled");
      applyButton?.removeAttribute("disabled");
    }
  }

  /** Diagnose if needed, then apply playbook (gateway + browser MCP) in one click. */
  async function oneClickRepairRuntimeCard(card: HTMLElement) {
    const runtime = card.dataset.runtime;
    const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
    if (!runtime || !hint) {
      return;
    }

    let preview = repairPreviewByRuntime.get(runtime);
    if (!preview) {
      hint.hidden = false;
      hint.textContent = t("runtime.diagnosing");
      showDiagnosePending(runtime, t("runtime.diagnosing"));
      try {
        preview = await invoke<RepairPreviewResponse>("run_repair_preview_command", { runtime });
        mountRepairPreview(preview, { resetFilter: true });
      } catch (error) {
        hint.hidden = false;
        hint.textContent = String(error);
        return;
      }
    }

    if (!preview.can_apply_repair) {
      hint.hidden = false;
      mountRepairPreview(preview);
      return;
    }

    await applyRepairRuntimeCard(card);
  }

  async function diagnoseRuntimeCard(card: HTMLElement) {
    const runtime = card.dataset.runtime;
    const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
    const button = card.querySelector<HTMLButtonElement>('[data-action="diagnose-runtime"]');
    if (!runtime || !hint) {
      return;
    }
    button?.setAttribute("disabled", "true");
    hint.hidden = false;
    hint.textContent = t("runtime.diagnosing");
    showDiagnosePending(runtime, t("runtime.diagnosing"));
    try {
      const report = await invoke<RepairPreviewResponse>("run_repair_preview_command", { runtime });
      mountRepairPreview(report, { resetFilter: true });
    } catch (error) {
      hint.textContent = String(error);
    } finally {
      button?.removeAttribute("disabled");
    }
  }

  function bindEvents(): void {
    diagnoseDetailEl.addEventListener("click", (event) => {
      const target = event.target as HTMLElement;
      const filterBtn = target.closest<HTMLButtonElement>("[data-repair-filter]");
      if (filterBtn && !filterBtn.disabled) {
        const runtime =
          filterBtn.closest<HTMLElement>("[data-runtime]")?.dataset.runtime ||
          diagnoseDetailEl.dataset.runtime;
        const filter = filterBtn.dataset.repairFilter as RepairStatusFilter | undefined;
        if (runtime && filter) {
          applyRepairFilter(runtime, filter);
        }
        return;
      }

      const action = target.closest<HTMLElement>("[data-action]")?.dataset.action;
      if (!action) {
        return;
      }

      if (action === "close-diagnose-detail") {
        void closeDiagnoseDetail();
        return;
      }

      const runtime =
        target.closest<HTMLElement>("[data-runtime]")?.dataset.runtime ||
        diagnoseDetailEl.dataset.runtime;
      const card = runtime ? runtimeCardEl(runtime) : null;

      if (action === "ask-session" && runtime) {
        if (isAskRuntimeId(runtime)) {
          void deps.openAskWindow(runtime);
        }
        return;
      }

      if (action === "ask-verify" && runtime) {
        if (supportsBrowserMcp(runtime)) {
          void deps.openAskWindowForVerify(runtime);
        }
        return;
      }

      if (action === "browser-smoke") {
        void runBrowserSmokeFromCard(diagnoseDetailEl);
        return;
      }

      if (action === "go-wiring") {
        void closeDiagnoseDetail();
        deps.setMainTab("provider");
        return;
      }

      if (action === "preview-repair" && runtime) {
        repairConfirmRuntimeIds.add(runtime);
        const report = repairPreviewByRuntime.get(runtime);
        if (report) {
          void openDiagnoseDetail(report);
        } else if (card) {
          void diagnoseRuntimeCard(card);
        } else {
          deps.setStatusBanner("error", t("doctor.empty"));
        }
        return;
      }

      if (action === "cancel-repair-preview" && runtime) {
        repairConfirmRuntimeIds.delete(runtime);
        const report = repairPreviewByRuntime.get(runtime);
        if (report) {
          void openDiagnoseDetail(report);
        }
        return;
      }

      if ((action === "confirm-repair" || action === "apply-repair") && card) {
        if (runtime) {
          repairConfirmRuntimeIds.delete(runtime);
        }
        void oneClickRepairRuntimeCard(card);
        return;
      }

      if (action === "rollback-repair" && card) {
        void rollbackRepairRuntimeCard(card);
        return;
      }

      const guideBtn = target.closest<HTMLButtonElement>('[data-action="open-repair-guide"]');
      if (guideBtn?.dataset.guidePath) {
        void openRepairGuide(decodeURIComponent(guideBtn.dataset.guidePath));
      }
    });
  }

  return {
    diagnoseDetailEl,
    MAIN_COMPACT_WIDTH,
    MAIN_DETAIL_EXTRA,
    setMainWindowWidth,
    readMainWindowSize,
    isDiagnoseDetailOpenFor,
    hasDismissed,
    runtimeCardEl,
    runtimeCardActionContext,
    refreshRuntimeCardActions,
    mountRepairPreview,
    mountRelatedResources,
    applyRepairFilter,
    openDiagnoseDetail,
    closeDiagnoseDetail,
    showDiagnosePending,
    expandDiagnoseWindowIfNeeded,
    diagnoseRuntimeCard,
    applyRepairRuntimeCard,
    oneClickRepairRuntimeCard,
    rollbackRepairRuntimeCard,
    openRepairGuide,
    runBrowserSmokeFromCard,
    bindEvents,
  };
}

export type AgentsDiagnoseApi = ReturnType<typeof createAgentsDiagnose>;
