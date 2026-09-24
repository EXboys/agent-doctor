import { invoke } from "@tauri-apps/api/core";
import { isDesktopAppRuntimeId } from "./agents-ui";
import { t } from "./i18n";
import type { OpenSessionReport } from "./types";

export interface AgentsSessionsDeps {
  setStatusBanner: (kind: "ok" | "warn" | "error" | "neutral", message: string) => void;
}

function withTimeout<T>(promise: Promise<T>, ms: number, timeoutError: Error): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => reject(timeoutError), ms);
    promise.then(
      (value) => {
        window.clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        window.clearTimeout(timer);
        reject(error);
      },
    );
  });
}

const ASK_VERIFY_DRAFT_KEY = "agent-doctor.ask.verifyDraft";
const openingSessionRuntimes = new Set<string>();

export function createAgentsSessions(deps: AgentsSessionsDeps) {
  async function openDesktopApp(runtime: string): Promise<void> {
    await withTimeout(
      invoke("open_session_command", {
        runtime,
        cwd: null,
        prompt: null,
        terminal: null,
      }),
      15_000,
      new Error(t("runtime.openTimeout")),
    );
  }

  async function openAskWindow(runtime: string): Promise<void> {
    try {
      if (isDesktopAppRuntimeId(runtime)) {
        await openDesktopApp(runtime);
        return;
      }
      await withTimeout(
        invoke("open_ask_window_command", { runtime }),
        15_000,
        new Error(t("runtime.openTimeout")),
      );
    } catch (error) {
      try {
        await invoke("close_ask_window_command", { destroy: true });
      } catch {
        /* window may already be gone or the UI thread is stuck */
      }
      deps.setStatusBanner("error", t("runtime.openFailed", { error: String(error) }));
    }
  }

  async function openAskWindowForVerify(runtime: string): Promise<void> {
    try {
      if (isDesktopAppRuntimeId(runtime)) {
        await openDesktopApp(runtime);
        return;
      }
      localStorage.setItem(
        ASK_VERIFY_DRAFT_KEY,
        JSON.stringify({ prompt: t("ask.verifyPrompt"), autoSend: true }),
      );
      await withTimeout(
        invoke("open_ask_window_command", { runtime }),
        15_000,
        new Error(t("runtime.openTimeout")),
      );
    } catch (error) {
      try {
        await invoke("close_ask_window_command", { destroy: true });
      } catch {
        /* window may already be gone or the UI thread is stuck */
      }
      deps.setStatusBanner("error", t("runtime.openFailed", { error: String(error) }));
    }
  }

  async function openSessionFromCard(card: HTMLElement, forceTerminal = false) {
    const runtime = card.dataset.runtime;
    const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
    const openButtons = [
      ...card.querySelectorAll<HTMLButtonElement>('[data-action="open-session"]'),
    ];
    if (!runtime || openingSessionRuntimes.has(runtime)) {
      return;
    }
    openingSessionRuntimes.add(runtime);
    for (const button of openButtons) {
      button.setAttribute("disabled", "true");
    }
    if (hint) {
      hint.hidden = false;
      hint.textContent = t("runtime.opening");
    }
    try {
      const report = await withTimeout(
        invoke<OpenSessionReport>("open_session_command", {
          runtime,
          cwd: null,
          prompt: null,
          terminal: forceTerminal ? true : null,
        }),
        20_000,
        new Error(t("runtime.openTimeout")),
      );
      const method = report.method === "deep-link" ? "deep-link" : "terminal";
      if (hint) {
        hint.textContent = t("runtime.openOk", { method });
      }
      deps.setStatusBanner("ok", t("runtime.openOk", { method }));
    } catch (error) {
      if (hint) {
        hint.hidden = false;
        hint.textContent = t("runtime.openFailed", { error: String(error) });
      }
      deps.setStatusBanner("error", t("runtime.openFailed", { error: String(error) }));
    } finally {
      openingSessionRuntimes.delete(runtime);
      for (const button of openButtons) {
        button.removeAttribute("disabled");
      }
    }
  }

  return {
    openAskWindow,
    openAskWindowForVerify,
    openSessionFromCard,
  };
}

export type AgentsSessionsApi = ReturnType<typeof createAgentsSessions>;
