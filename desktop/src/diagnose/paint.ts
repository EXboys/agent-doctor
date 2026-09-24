import {
  computeDiagnoseScore,
  scoreLooksGood,
  type DiagnoseScore,
  stepStatesFor,
} from "../diagnose-flow";
import { isPersonalEdition } from "../edition";
import { escapeHtml } from "../format";
import { t } from "../i18n";
import { isDesktopAppRuntimeId, supportsBrowserMcp } from "../agents-ui";
import { repairCheckStatusLabel, repairStatusClass } from "../repair-ui";
import { renderPlainRepairCheckBody } from "../repair-plain";
import type { RepairPreviewResponse } from "../types";
import * as dom from "./dom";
import {
  SCORE_MAX_MS,
  SCORE_MIN_MS,
  sleep,
  type DiagnoseSession,
} from "./session";

export type DiagnosePaintApi = ReturnType<typeof createDiagnosePaint>;

export function createDiagnosePaint(session: DiagnoseSession) {
  function setResult(kind: "ok" | "error" | "busy" | "hide", message = ""): void {
    if (kind === "hide" || !message) {
      dom.resultEl.hidden = true;
      dom.resultEl.textContent = "";
      dom.resultEl.className = "diagnose-result is-slot";
      return;
    }
    dom.resultEl.hidden = false;
    dom.resultEl.textContent = message;
    dom.resultEl.className = `diagnose-result is-slot is-${kind}`;
  }

  function paintSkeletonChecks(count = 8): void {
    dom.checkListEl.innerHTML = Array.from({ length: count }, () => {
      return `<li class="is-pending is-skeleton" aria-hidden="true"><span class="diagnose-check-badge muted">—</span><span>—</span></li>`;
    }).join("");
  }

  function ensureDetailsOpen(): void {
    dom.detailsEl.hidden = false;
  }

  function paintHeroTone(tone: "ok" | "warn" | "fail" | "busy" | "neutral"): void {
    for (const el of [dom.heroEl, dom.healthCardEl]) {
      el.classList.toggle("is-ok", tone === "ok");
      el.classList.toggle("is-warn", tone === "warn");
      el.classList.toggle("is-fail", tone === "fail");
      el.classList.toggle("is-busy", tone === "busy");
    }
  }

  function setBusy(next: boolean): void {
    session.busy = next;
    dom.primaryEl.disabled = next;
    dom.secondaryEl.disabled = next;
  }

  function syncStatFilterUi(score: DiagnoseScore | null): void {
    const counts: Record<"pass" | "warn" | "fail", number> = {
      pass: score?.pass ?? 0,
      warn: score?.warn ?? 0,
      fail: score?.fail ?? 0,
    };
    if (dom.tabPassEl) {
      dom.tabPassEl.textContent = score ? String(counts.pass) : "—";
    }
    if (dom.tabWarnEl) {
      dom.tabWarnEl.textContent = score ? String(counts.warn) : "—";
    }
    if (dom.tabFailEl) {
      dom.tabFailEl.textContent = score ? String(counts.fail) : "—";
    }
    dom.checkTabsEl?.querySelectorAll<HTMLButtonElement>("[data-check-filter]").forEach((btn) => {
      const filter = btn.dataset.checkFilter as "all" | "pass" | "warn" | "fail" | undefined;
      if (!filter) {
        return;
      }
      const active = session.checkFilter === filter;
      btn.classList.toggle("is-active", active);
      btn.setAttribute("aria-selected", active ? "true" : "false");
      if (filter === "all") {
        btn.disabled = !score;
        return;
      }
      btn.disabled = !score || counts[filter] === 0;
    });
  }

  function paintScore(score: DiagnoseScore | null, opts?: { animateFromZero?: boolean }): void {
    // Keep ring + stats in layout to avoid open-time stretch.
    dom.scoreRingEl.hidden = false;
    dom.statRowEl.hidden = false;
    if (!score) {
      dom.scoreValueEl.textContent = "—";
      dom.scoreRingEl.style.setProperty("--readiness", "0");
      dom.scoreRingEl.classList.remove("is-warn", "is-fail", "is-busy", "is-scanning");
      dom.statPassEl.textContent = "—";
      dom.statWarnEl.textContent = "—";
      dom.statFailEl.textContent = "—";
      dom.heroKickerEl.hidden = true;
      syncStatFilterUi(null);
      return;
    }
    // Green when usable (≥80, no fail). Yellow only below that bar; red if any fail.
    const good = scoreLooksGood(score);
    dom.scoreRingEl.classList.toggle("is-warn", !good && score.fail === 0);
    dom.scoreRingEl.classList.toggle("is-fail", score.fail > 0);
    dom.scoreRingEl.classList.toggle("is-busy", Boolean(opts?.animateFromZero));
    dom.scoreRingEl.classList.toggle("is-scanning", Boolean(opts?.animateFromZero));
    dom.statPassEl.textContent = String(score.pass);
    dom.statWarnEl.textContent = String(score.warn);
    dom.statFailEl.textContent = String(score.fail);
    dom.heroKickerEl.hidden = Boolean(opts?.animateFromZero);
    if (opts?.animateFromZero) {
      dom.scoreValueEl.textContent = "0";
      dom.scoreRingEl.style.setProperty("--readiness", "0");
      syncStatFilterUi(score);
      return;
    }
    dom.scoreValueEl.textContent = String(score.percent);
    dom.scoreRingEl.style.setProperty("--readiness", String(score.percent));
    syncStatFilterUi(score);
  }

  function focusConfigFill(): void {
    dom.panelConfigEl.hidden = false;
    const target = !dom.urlEl.value.trim() ? dom.urlEl : dom.keyEl;
    try {
      target.focus({ preventScroll: true });
    } catch {
      target.focus();
    }
    dom.panelConfigEl.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }

  function setScanMeter(current: number, total: number, label: string): void {
    dom.scanMeterEl.hidden = false;
    dom.scanMeterEl.classList.remove("is-idle");
    dom.scanMeterLabelEl.textContent = label;
    dom.scanMeterCountEl.textContent = total > 0 ? `${current}/${total}` : "";
    const percent = total <= 0 ? 0 : Math.round((current / total) * 100);
    dom.scanMeterFillEl.style.width = `${percent}%`;
  }

  function hideScanMeter(): void {
    dom.scanMeterEl.classList.add("is-idle");
    dom.scanMeterEl.hidden = true;
    dom.scanMeterFillEl.style.width = "0%";
  }

  function scrollActiveCheckIntoView(): void {
    const active = dom.checkListEl.querySelector<HTMLElement>(
      "li.is-checking, li.is-just-revealed",
    );
    if (!active) {
      return;
    }
    const top = active.offsetTop;
    const bottom = top + active.offsetHeight;
    const viewTop = dom.checkScrollEl.scrollTop;
    const viewBottom = viewTop + dom.checkScrollEl.clientHeight;
    if (top < viewTop + 4) {
      dom.checkScrollEl.scrollTop = Math.max(0, top - 8);
    } else if (bottom > viewBottom - 4) {
      dom.checkScrollEl.scrollTop = bottom - dom.checkScrollEl.clientHeight + 8;
    }
  }

  function applyRevealState(report: RepairPreviewResponse, revealThrough: number): void {
    const total = report.checks.length;
    if (dom.checkListEl.children.length !== total) {
      paintChecks(report, { mode: "pending" });
    }
    report.checks.forEach((check, index) => {
      const li = dom.checkListEl.children[index] as HTMLElement | undefined;
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

  function paintChecks(
    report: RepairPreviewResponse | null,
    opts?: { mode?: "final" | "pending" | "reveal"; revealThrough?: number },
  ): void {
    if (!report) {
      // Keep reserved rows so the details pane does not collapse while scanning.
      if (!dom.checkListEl.querySelector(".is-skeleton")) {
        paintSkeletonChecks();
      }
      return;
    }
    const mode = opts?.mode ?? "final";
    const revealThrough = opts?.revealThrough ?? -1;
    const checks =
      mode === "final" && session.checkFilter !== "all"
        ? report.checks.filter((check) => check.status === session.checkFilter)
        : report.checks;

    if (mode === "final" && checks.length === 0) {
      dom.checkListEl.innerHTML = `<li class="is-revealed">${escapeHtml(t("diagnose.flow.filterEmpty"))}</li>`;
      return;
    }

    // Reveal updates rows in place so the list height does not reflow each tick.
    if (mode === "reveal") {
      applyRevealState(report, revealThrough);
      return;
    }

    dom.checkListEl.innerHTML = checks
      .map((check, index) => {
        const body = renderPlainRepairCheckBody(check);
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

  async function animateScoreCount(target: DiagnoseScore): Promise<void> {
    dom.scoreRingEl.hidden = false;
    dom.scoreRingEl.classList.remove("is-scanning", "is-busy");
    const good = scoreLooksGood(target);
    dom.scoreRingEl.classList.toggle("is-warn", !good && target.fail === 0);
    dom.scoreRingEl.classList.toggle("is-fail", target.fail > 0);
    const frames = 18;
    for (let i = 1; i <= frames; i += 1) {
      const percent = Math.round((target.percent * i) / frames);
      dom.scoreValueEl.textContent = String(percent);
      dom.scoreRingEl.style.setProperty("--readiness", String(percent));
      await sleep(28);
    }
    dom.scoreValueEl.textContent = String(target.percent);
    dom.scoreRingEl.style.setProperty("--readiness", String(target.percent));
    dom.statPassEl.textContent = String(target.pass);
    dom.statWarnEl.textContent = String(target.warn);
    dom.statFailEl.textContent = String(target.fail);
    dom.statRowEl.hidden = false;
    dom.heroKickerEl.hidden = false;
    syncStatFilterUi(target);
  }

  async function playSequentialConfirm(report: RepairPreviewResponse): Promise<void> {
    ensureDetailsOpen();
    dom.panelTestEl.hidden = true;
    dom.testHintEl.hidden = true;
    paintHeroTone("busy");
    session.checkFilter = "all";
    const total = report.checks.length;
    const started = performance.now();
    // List is already mounted by refreshState / runScoreTest; only reset if needed.
    if (dom.checkListEl.children.length !== total) {
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
    const budget = Math.min(SCORE_MAX_MS, Math.max(SCORE_MIN_MS, total * 120));
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
    const states = stepStatesFor(session.activeStep, {
      installed: session.installed,
      configured: session.configured,
      testedOk: session.testedOk,
    });
    const labels: Record<"install" | "config" | "test", string> = {
      install: "1",
      config: "2",
      test: "3",
    };
    dom.stepsEl.querySelectorAll<HTMLElement>(".diagnose-step").forEach((el) => {
      const step = el.dataset.step as "install" | "config" | "test" | undefined;
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

  function paintBootShell(): void {
    // First paint: reserve the full test layout so the window does not stretch open.
    dom.titleEl.textContent = session.displayName;
    ensureDetailsOpen();
    paintHeroTone("busy");
    dom.headlineEl.textContent = t("diagnose.flow.testHeadline");
    dom.detailEl.textContent = t("diagnose.flow.confirming");
    setResult("busy", t("diagnose.flow.scoring"));
    paintScore({ pass: 0, warn: 0, fail: 0, total: 0, percent: 0 }, { animateFromZero: true });
    setScanMeter(0, 0, t("diagnose.flow.confirming"));
    dom.testHintEl.hidden = true;
    dom.primaryEl.hidden = false;
    dom.primaryEl.textContent = t("diagnose.flow.testCta");
    dom.secondaryEl.hidden = false;
    dom.secondaryEl.textContent = t("diagnose.flow.openAskYourself");
    dom.secondaryEl.dataset.fallback = "open-ask";
    if (!dom.checkListEl.querySelector(".is-skeleton") && dom.checkListEl.children.length === 0) {
      paintSkeletonChecks();
    }
  }

  function paintAll(): void {
    ensureDetailsOpen();
    paintSteps();
    if (session.activeStep === "test" || session.testedOk) {
      // Keep the score ring visible during the whole confirm animation.
      paintScore(
        session.lastScore ?? { pass: 0, warn: 0, fail: 0, total: 0, percent: 0 },
        {
          animateFromZero: !session.lastScore || (!session.testedOk && session.busy),
        },
      );
    } else {
      paintScore(null);
    }
    paintChecks(session.preview);
    dom.panelInstallEl.hidden = session.activeStep !== "install";
    dom.panelConfigEl.hidden = session.activeStep !== "config";
    dom.panelConfigEl.classList.toggle(
      "is-guide",
      session.activeStep === "config" && session.guideFillConfig && !session.configured,
    );
    dom.panelTestEl.hidden = true;
    dom.testHintEl.hidden = session.activeStep !== "test";
    if (session.activeStep === "test") {
      dom.testHintEl.textContent = isDesktopAppRuntimeId(session.runtimeId)
        ? t("diagnose.flow.testHintDesktop", { name: session.displayName })
        : t("diagnose.flow.testHint");
    }
    if (session.activeStep !== "test") {
      hideScanMeter();
    }
    dom.configPersonalEl.hidden = !isPersonalEdition();
    dom.configTeamEl.hidden = isPersonalEdition();

    if (session.activeStep === "install") {
      paintHeroTone("neutral");
      dom.headlineEl.textContent = session.installed
        ? t("diagnose.flow.installDoneHeadline", { name: session.displayName })
        : t("diagnose.flow.installHeadline", { name: session.displayName });
      dom.detailEl.textContent = session.installed
        ? t("diagnose.flow.installDoneDetail")
        : t("diagnose.flow.installDetail");
      session.primaryAction = session.installed ? "none" : "install";
      dom.primaryEl.textContent = t("diagnose.flow.installCta", { name: session.displayName });
      dom.primaryEl.hidden = session.installed;
      dom.secondaryEl.hidden = true;
    } else if (session.activeStep === "config") {
      paintHeroTone(session.configured ? "ok" : "busy");
      if (session.guideFillConfig && !session.configured) {
        dom.headlineEl.textContent = t("diagnose.flow.configGuideHeadline");
        dom.detailEl.textContent = isPersonalEdition()
          ? t("diagnose.flow.configGuideDetail")
          : t("diagnose.flow.configGuideDetailTeam");
      } else {
        dom.headlineEl.textContent = session.configured
          ? t("diagnose.flow.configDoneHeadline")
          : t("diagnose.flow.configHeadline");
        dom.detailEl.textContent = isPersonalEdition()
          ? t("diagnose.flow.configDetail")
          : t("diagnose.flow.configDetailTeam");
      }
      if (session.canAutoFix) {
        session.primaryAction = "auto-fix";
        dom.primaryEl.hidden = false;
        dom.primaryEl.textContent = t("diagnose.flow.autoFixCta");
        dom.secondaryEl.hidden = false;
        dom.secondaryEl.textContent = isPersonalEdition()
          ? session.guideFillConfig
            ? t("diagnose.flow.configGuideCta")
            : t("diagnose.flow.configCta")
          : t("diagnose.flow.openTeamWiring");
        dom.secondaryEl.dataset.fallback = isPersonalEdition()
          ? "verify-save"
          : "open-team-wiring";
      } else if (isPersonalEdition()) {
        session.primaryAction = "verify-save";
        dom.primaryEl.hidden = false;
        dom.primaryEl.textContent = session.guideFillConfig
          ? t("diagnose.flow.configGuideCta")
          : t("diagnose.flow.configCta");
        dom.secondaryEl.hidden = true;
      } else {
        session.primaryAction = "open-team-wiring";
        dom.primaryEl.hidden = false;
        dom.primaryEl.textContent = t("diagnose.flow.openTeamWiring");
        dom.secondaryEl.hidden = true;
      }
      if (session.guideFillConfig && !session.configured && isPersonalEdition()) {
        window.requestAnimationFrame(() => focusConfigFill());
      }
    } else {
      if (session.busy && !session.testedOk) {
        paintHeroTone("busy");
      } else if (session.testedOk || (session.lastScore && scoreLooksGood(session.lastScore))) {
        paintHeroTone("ok");
      } else if (session.lastScore && session.lastScore.fail > 0) {
        paintHeroTone("fail");
      } else if (session.lastScore) {
        paintHeroTone("warn");
      } else {
        paintHeroTone("busy");
      }
      const browserVerify = supportsBrowserMcp(session.runtimeId);
      const desktopApp = isDesktopAppRuntimeId(session.runtimeId);
      dom.headlineEl.textContent = session.testedOk
        ? desktopApp
          ? t("diagnose.flow.testOkHeadlineDesktop", { name: session.displayName })
          : t("diagnose.flow.testOkHeadline")
        : t("diagnose.flow.testHeadline");
      dom.detailEl.textContent = session.testedOk
        ? desktopApp
          ? t("diagnose.flow.testOkDetailDesktop", { name: session.displayName })
          : browserVerify
            ? t("diagnose.flow.testOkDetail")
            : t("diagnose.flow.testOkDetailNoBrowser")
        : desktopApp
          ? t("diagnose.flow.testDetailDesktop", { name: session.displayName })
          : t("diagnose.flow.testDetail");
      if (session.testedOk) {
        // Keep「重新检查」on the right for every agent; left is the next step.
        if (browserVerify) {
          session.primaryAction = "ask-verify";
          dom.primaryEl.textContent = t("diagnose.flow.askVerifyCta");
        } else {
          session.primaryAction = "open-ask";
          dom.primaryEl.textContent = desktopApp
            ? t("diagnose.flow.openDesktopApp", { name: session.displayName })
            : t("diagnose.flow.openAskYourself");
        }
        dom.primaryEl.hidden = false;
        dom.secondaryEl.hidden = false;
        dom.secondaryEl.textContent = t("diagnose.flow.rescan");
        dom.secondaryEl.dataset.fallback = "rescan";
      } else {
        session.primaryAction = "run-score";
        dom.primaryEl.hidden = false;
        dom.primaryEl.textContent = t("diagnose.flow.testCta");
        if (desktopApp) {
          dom.secondaryEl.hidden = true;
        } else {
          dom.secondaryEl.hidden = false;
          dom.secondaryEl.textContent = t("diagnose.flow.openAskYourself");
          dom.secondaryEl.dataset.fallback = "open-ask";
        }
      }
    }
  }

  return {
    setResult,
    setBusy,
    paintScore,
    paintHeroTone,
    paintChecks,
    paintSteps,
    paintAll,
    paintBootShell,
    ensureDetailsOpen,
    focusConfigFill,
    setScanMeter,
    hideScanMeter,
    syncStatFilterUi,
    animateScoreCount,
    playSequentialConfirm,
  };
}
