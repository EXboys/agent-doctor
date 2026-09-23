import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import { t } from "./i18n";
import { withErrorDetail } from "./friendly-error";
import { escapeHtml } from "./format";
import type { DoctorReport, InstallProgressEvent, InstallRuntimeResponse } from "./types";

export interface AgentsInstallDeps {
  refresh: () => Promise<void>;
  setStatusBanner: (tone: "ok" | "warn" | "error" | "neutral", message: string) => void;
}

/** Survives card re-render after install → refresh(). Cleared once runtime looks installed. */
const stickyInstallHints = new Map<string, { html: string | null; text: string | null }>();

type LiveInstall = {
  status: string;
  percent: number;
  indeterminate: boolean;
  logLines: string[];
};

/** In-flight installs. Switching agent tabs rebuilds the card, so progress lives here. */
const liveInstalls = new Map<string, LiveInstall>();

function installWaitNote(runtime: string): string {
  if (runtime !== "hermes") {
    return "";
  }
  return `<p class="footnote" data-install-wait>${escapeHtml(t("runtime.hermesInstallWait"))}</p>`;
}

function installProgressHtml(runtime: string, live: LiveInstall): string {
  return `
      <div class="install-progress" data-install-progress>
        <div class="install-progress-head">
          <span data-install-status>${escapeHtml(live.status)}</span>
          <span data-install-percent>${live.percent}%</span>
        </div>
        <div class="install-progress-track" aria-hidden="true">
          <div class="install-progress-fill${live.indeterminate ? " is-indeterminate" : ""}" data-install-fill style="width:${live.percent}%"></div>
        </div>
        ${installWaitNote(runtime)}
        <pre class="install-progress-log" data-install-log>${escapeHtml(live.logLines.join("\n"))}</pre>
      </div>
    `;
}

function paintLiveInstall(runtime: string): void {
  const live = liveInstalls.get(runtime);
  if (!live) {
    return;
  }
  const card = document.querySelector<HTMLElement>(`.runtime[data-runtime="${CSS.escape(runtime)}"]`);
  const hint = card?.querySelector<HTMLElement>("[data-repair-hint]");
  if (hint) {
    hint.hidden = false;
    const progress = hint.querySelector<HTMLElement>("[data-install-progress]");
    if (!progress) {
      hint.innerHTML = installProgressHtml(runtime, live);
    } else {
      if (runtime === "hermes" && !progress.querySelector("[data-install-wait]")) {
        const track = progress.querySelector(".install-progress-track");
        track?.insertAdjacentHTML("afterend", installWaitNote(runtime));
      }
      const statusEl = progress.querySelector<HTMLElement>("[data-install-status]");
      const percentEl = progress.querySelector<HTMLElement>("[data-install-percent]");
      const fillEl = progress.querySelector<HTMLElement>("[data-install-fill]");
      const logEl = progress.querySelector<HTMLElement>("[data-install-log]");
      if (statusEl) {
        statusEl.textContent = live.status;
      }
      if (percentEl) {
        percentEl.textContent = `${live.percent}%`;
      }
      if (fillEl) {
        fillEl.style.width = `${live.percent}%`;
        fillEl.classList.toggle("is-indeterminate", live.indeterminate);
      }
      if (logEl && logEl.textContent !== live.logLines.join("\n")) {
        logEl.textContent = live.logLines.join("\n");
        logEl.scrollTop = logEl.scrollHeight;
      }
    }
  }
  for (const action of ["install-runtime", "force-reinstall-runtime", "diagnose-runtime"]) {
    card
      ?.querySelector<HTMLButtonElement>(`[data-action="${action}"]`)
      ?.setAttribute("disabled", "true");
  }
}

export function createAgentsInstall(deps: AgentsInstallDeps) {
  function applyInstallHint(runtime: string, html: string | null, text: string | null) {
    const freshCard = document.querySelector<HTMLElement>(
      `.runtime[data-runtime="${CSS.escape(runtime)}"]`,
    );
    const freshHint = freshCard?.querySelector<HTMLElement>("[data-repair-hint]");
    if (!freshHint) {
      return;
    }
    freshHint.hidden = false;
    if (html) {
      freshHint.innerHTML = html;
    } else if (text) {
      freshHint.textContent = text;
    }
  }

  function setStickyInstallHint(runtime: string, html: string | null, text: string | null) {
    stickyInstallHints.set(runtime, { html, text });
    applyInstallHint(runtime, html, text);
  }

  function reapplyStickyInstallHints(report: DoctorReport) {
    for (const runtime of report.runtimes) {
      if (runtime.installed) {
        stickyInstallHints.delete(runtime.id);
      }
    }
    for (const [runtime, hint] of stickyInstallHints) {
      if (liveInstalls.has(runtime)) {
        continue;
      }
      applyInstallHint(runtime, hint.html, hint.text);
    }
    for (const runtime of liveInstalls.keys()) {
      paintLiveInstall(runtime);
    }
  }

  async function installRuntimeFromCard(card: HTMLElement, options: { force?: boolean } = {}) {
    const runtime = card.dataset.runtime;
    const force = Boolean(options.force);
    const installButton = card.querySelector<HTMLButtonElement>('[data-action="install-runtime"]');
    const forceButton = card.querySelector<HTMLButtonElement>(
      '[data-action="force-reinstall-runtime"]',
    );
    const diagnoseButton = card.querySelector<HTMLButtonElement>('[data-action="diagnose-runtime"]');
    if (!runtime) {
      return;
    }
    if (liveInstalls.has(runtime)) {
      paintLiveInstall(runtime);
      return;
    }
    if (force) {
      const ok = await ask(t("runtime.forceReinstallConfirm", { runtime }), {
        title: t("runtime.forceReinstall"),
        kind: "warning",
        okLabel: t("runtime.forceReinstall"),
        cancelLabel: t("runtime.cancel"),
      });
      if (!ok) {
        return;
      }
    }
    liveInstalls.set(runtime, {
      status: t("runtime.installing"),
      percent: 0,
      indeterminate: true,
      logLines: [],
    });
    paintLiveInstall(runtime);

    const unlisten = await listen<InstallProgressEvent>("install-progress", (event) => {
      if (event.payload.runtime_id !== runtime) {
        return;
      }
      const live = liveInstalls.get(runtime);
      if (!live) {
        return;
      }
      const { phase, message, percent } = event.payload;
      const clamped = Math.min(100, Math.max(0, percent));
      const text = message.trim();
      live.status =
        phase === "verifying"
          ? t("runtime.installVerifying")
          : text.startsWith("$ ")
            ? t("runtime.installing")
            : text || t("runtime.installing");
      live.percent = clamped;
      live.indeterminate = clamped < 2 && phase !== "done";
      if (text) {
        const isByteProgress = /Downloading Node\.js .+\/|正在下载|正在确认|正在安装依赖|依赖装好了/.test(text);
        if (
          isByteProgress &&
          live.logLines.length > 0 &&
          /Downloading Node\.js .+\/|正在下载|正在确认|正在安装依赖|依赖装好了/.test(live.logLines[live.logLines.length - 1] ?? "")
        ) {
          live.logLines[live.logLines.length - 1] = text;
        } else {
          live.logLines.push(text);
        }
        while (live.logLines.length > 40) {
          live.logLines.shift();
        }
      }
      paintLiveInstall(runtime);
    });

    try {
      const report = await invoke<InstallRuntimeResponse>("install_runtime_command", {
        runtime,
        force,
      });
      let nextHintHtml: string | null = null;
      let nextHintText: string | null = null;
      if (!report.install_needed) {
        nextHintText = t("runtime.installAlready");
      } else if (report.install_succeeded || report.after_installed) {
        const last = liveInstalls.get(runtime)?.logLines.slice(-3).join("\n") ?? "";
        nextHintHtml = `<div class="install-progress-done">${escapeHtml(t("runtime.installOk"))}<p class="footnote">${escapeHtml(
          t("runtime.installWireNext"),
        )}</p>${last ? `<pre class="install-progress-log">${escapeHtml(last)}</pre>` : ""}</div>`;
      } else {
        const detail =
          report.skipped.map((item) => item.reason).find(Boolean) ||
          report.manual_fallback[0] ||
          t("runtime.installFailed");
        nextHintText = `${t("runtime.installFailed")} ${detail}`;
      }
      // refresh() re-renders cards and would wipe the in-card hint — keep sticky.
      let stickyHtml = nextHintHtml;
      let stickyText = nextHintText;
      if (report.install_needed && !(report.install_succeeded || report.after_installed)) {
        const logPath = report.install_log_path;
        if (logPath) {
          stickyHtml = `<div class="install-progress-done">${escapeHtml(
            nextHintText ?? t("runtime.installFailed"),
          )}<p class="footnote">${escapeHtml(t("runtime.installLogHint"))}</p><button type="button" class="btn-ghost" data-action="open-install-log" data-log-path="${escapeHtml(
            logPath,
          )}">${escapeHtml(t("runtime.openInstallLog"))}</button></div>`;
          stickyText = null;
        }
      }
      await deps.refresh();
      setStickyInstallHint(runtime, stickyHtml, stickyText);
    } catch (error) {
      const message = withErrorDetail(t("runtime.installFailed"), error);
      setStickyInstallHint(runtime, null, message);
      try {
        await deps.refresh();
      } catch {
        // keep the error on the original hint if re-probe fails
      }
      setStickyInstallHint(runtime, null, message);
    } finally {
      liveInstalls.delete(runtime);
      unlisten();
      const freshCard = document.querySelector<HTMLElement>(
        `.runtime[data-runtime="${CSS.escape(runtime)}"]`,
      );
      freshCard
        ?.querySelector<HTMLButtonElement>('[data-action="install-runtime"]')
        ?.removeAttribute("disabled");
      freshCard
        ?.querySelector<HTMLButtonElement>('[data-action="force-reinstall-runtime"]')
        ?.removeAttribute("disabled");
      freshCard
        ?.querySelector<HTMLButtonElement>('[data-action="diagnose-runtime"]')
        ?.removeAttribute("disabled");
      installButton?.removeAttribute("disabled");
      forceButton?.removeAttribute("disabled");
      diagnoseButton?.removeAttribute("disabled");
    }
  }

  async function uninstallRuntime(runtime: string, name: string): Promise<void> {
    let ok = false;
    try {
      ok = await ask(t("runtime.uninstallConfirm", { name }), {
        title: t("runtime.uninstall"),
        kind: "warning",
        okLabel: t("runtime.uninstall"),
        cancelLabel: t("runtime.cancel"),
      });
    } catch (error) {
      deps.setStatusBanner("error", withErrorDetail(t("runtime.uninstallFailed"), error));
      return;
    }
    if (!ok) {
      return;
    }

    const card = document.querySelector<HTMLElement>(
      `.runtime[data-runtime="${CSS.escape(runtime)}"]`,
    );
    const hint = card?.querySelector<HTMLElement>("[data-repair-hint]");
    const uninstallButton = card?.querySelector<HTMLButtonElement>(
      '[data-action="uninstall-runtime"]',
    );
    uninstallButton?.setAttribute("disabled", "true");
    if (hint) {
      hint.hidden = false;
      hint.textContent = t("runtime.uninstalling");
    }
    deps.setStatusBanner("neutral", t("runtime.uninstalling"));
    try {
      await invoke("uninstall_runtime_command", { runtime });
      if (hint) {
        hint.textContent = t("runtime.uninstallOk");
      }
      deps.setStatusBanner("ok", t("runtime.uninstallOk"));
      await deps.refresh();
    } catch (error) {
      uninstallButton?.removeAttribute("disabled");
      if (hint) {
        hint.hidden = false;
        hint.textContent = withErrorDetail(t("runtime.uninstallFailed"), error);
      }
      deps.setStatusBanner("error", withErrorDetail(t("runtime.uninstallFailed"), error));
    }
  }

  return {
    reapplyStickyInstallHints,
    setStickyInstallHint,
    applyInstallHint,
    installRuntimeFromCard,
    uninstallRuntime,
  };
}

export type AgentsInstallApi = ReturnType<typeof createAgentsInstall>;
