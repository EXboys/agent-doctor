import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t } from "./i18n";
import { escapeHtml } from "./format";
import type { DoctorReport, InstallProgressEvent, InstallRuntimeResponse } from "./types";

export interface AgentsInstallDeps {
  refresh: () => Promise<void>;
}

/** Survives card re-render after install → refresh(). Cleared once runtime looks installed. */
const stickyInstallHints = new Map<string, { html: string | null; text: string | null }>();

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
      applyInstallHint(runtime, hint.html, hint.text);
    }
  }

  async function installRuntimeFromCard(card: HTMLElement, options: { force?: boolean } = {}) {
    const runtime = card.dataset.runtime;
    const force = Boolean(options.force);
    const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
    const installButton = card.querySelector<HTMLButtonElement>('[data-action="install-runtime"]');
    const forceButton = card.querySelector<HTMLButtonElement>(
      '[data-action="force-reinstall-runtime"]',
    );
    const diagnoseButton = card.querySelector<HTMLButtonElement>('[data-action="diagnose-runtime"]');
    if (!runtime) {
      return;
    }
    if (force) {
      const ok = window.confirm(t("runtime.forceReinstallConfirm", { runtime }));
      if (!ok) {
        return;
      }
    }
    installButton?.setAttribute("disabled", "true");
    forceButton?.setAttribute("disabled", "true");
    diagnoseButton?.setAttribute("disabled", "true");
    if (hint) {
      hint.hidden = false;
      hint.innerHTML = `
      <div class="install-progress" data-install-progress>
        <div class="install-progress-head">
          <span data-install-status>${escapeHtml(t("runtime.installing"))}</span>
          <span data-install-percent>0%</span>
        </div>
        <div class="install-progress-track" aria-hidden="true">
          <div class="install-progress-fill is-indeterminate" data-install-fill></div>
        </div>
        <pre class="install-progress-log" data-install-log></pre>
      </div>
    `;
    }
    const statusEl = hint?.querySelector<HTMLElement>("[data-install-status]");
    const percentEl = hint?.querySelector<HTMLElement>("[data-install-percent]");
    const fillEl = hint?.querySelector<HTMLElement>("[data-install-fill]");
    const logEl = hint?.querySelector<HTMLElement>("[data-install-log]");
    const logLines: string[] = [];

    const unlisten = await listen<InstallProgressEvent>("install-progress", (event) => {
      if (event.payload.runtime_id !== runtime) {
        return;
      }
      const { phase, message, percent } = event.payload;
      const clamped = Math.min(100, Math.max(0, percent));
      if (statusEl) {
        // Do not treat phase "done" as success — the invoke result decides.
        // Shell lines stay in the log; the headline is one short sentence.
        const text = message.trim();
        statusEl.textContent =
          phase === "verifying"
            ? t("runtime.installVerifying")
            : text.startsWith("$ ")
              ? t("runtime.installing")
              : text || t("runtime.installing");
      }
      if (percentEl) {
        percentEl.textContent = `${clamped}%`;
      }
      if (fillEl) {
        fillEl.style.width = `${clamped}%`;
        fillEl.classList.toggle("is-indeterminate", clamped < 2 && phase !== "done");
      }
      if (logEl && message.trim()) {
        const isByteProgress = /Downloading Node\.js .+\/|正在下载/.test(message);
        if (
          isByteProgress &&
          logLines.length > 0 &&
          /Downloading Node\.js .+\/|正在下载/.test(logLines[logLines.length - 1] ?? "")
        ) {
          logLines[logLines.length - 1] = message;
        } else {
          logLines.push(message);
        }
        while (logLines.length > 40) {
          logLines.shift();
        }
        logEl.textContent = logLines.join("\n");
        logEl.scrollTop = logEl.scrollHeight;
      }
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
        const last = logLines.slice(-3).join("\n");
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
          )}<p class="footnote">${escapeHtml(t("runtime.installLogHint", { path: logPath }))}</p><button type="button" class="btn-ghost" data-action="open-install-log" data-log-path="${escapeHtml(
            logPath,
          )}">${escapeHtml(t("runtime.openInstallLog"))}</button></div>`;
          stickyText = null;
        }
      }
      await deps.refresh();
      setStickyInstallHint(runtime, stickyHtml, stickyText);
    } catch (error) {
      const message = String(error);
      if (hint) {
        hint.hidden = false;
        hint.textContent = message;
      }
      try {
        await deps.refresh();
      } catch {
        // keep the error on the original hint if re-probe fails
      }
      setStickyInstallHint(runtime, null, message);
    } finally {
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
    if (!window.confirm(t("runtime.uninstallConfirm", { name }))) {
      return;
    }
    const card = document.querySelector<HTMLElement>(`.runtime[data-runtime="${CSS.escape(runtime)}"]`);
    const hint = card?.querySelector<HTMLElement>("[data-repair-hint]");
    if (hint) {
      hint.hidden = false;
      hint.textContent = t("runtime.uninstalling");
    }
    try {
      await invoke("uninstall_runtime_command", { runtime });
      if (hint) {
        hint.textContent = t("runtime.uninstallOk");
      }
      await deps.refresh();
    } catch (error) {
      const message = String(error);
      if (hint) {
        hint.hidden = false;
        hint.textContent = message;
      }
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
