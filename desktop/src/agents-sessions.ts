import { opensOwnWindowOnly } from "./agents-ui";
import { t } from "./i18n";
import { openSession, openAskWindow as openAskWindowCommand, closeAskWindow } from "./ipc";

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
const openingAskRuntimes = new Set<string>();

function isWindowsShell(): boolean {
  return document.documentElement.dataset.os === "windows";
}

/** Keep the pre-created WebView2 shell on Windows; macOS can destroy/rebuild. */
function askCloseDestroyOnFailure(): boolean {
  return !isWindowsShell();
}

export function createAgentsSessions(deps: AgentsSessionsDeps) {
  async function openDesktopApp(runtime: string): Promise<void> {
    await withTimeout(
      openSession({
        runtime,
        cwd: null,
        prompt: null,
        terminal: null,
      }),
      15_000,
      new Error(t("runtime.openTimeout")),
    );
  }

  async function openAskWindow(runtime: string, card?: HTMLElement | null): Promise<void> {
    if (openingAskRuntimes.has(runtime)) {
      return;
    }
    openingAskRuntimes.add(runtime);
    const hint =
      card?.querySelector<HTMLElement>("[data-repair-hint]") ??
      card?.querySelector<HTMLElement>("[data-ask-hint]");
    const askButtons = card
      ? [
          ...card.querySelectorAll<HTMLButtonElement>('[data-action="ask-doctor"]'),
          ...card.querySelectorAll<HTMLButtonElement>('[data-action="ask-session"]'),
          ...card.querySelectorAll<HTMLButtonElement>('[data-action="ask-verify"]'),
        ]
      : [];
    for (const button of askButtons) {
      button.setAttribute("disabled", "true");
    }
    if (hint) {
      hint.hidden = false;
      hint.textContent = t("runtime.opening");
    }
    try {
      if (opensOwnWindowOnly(runtime)) {
        await openDesktopApp(runtime);
        if (hint) {
          hint.hidden = true;
          hint.replaceChildren();
        }
        return;
      }
      await withTimeout(
        openAskWindowCommand({ runtime }),
        15_000,
        new Error(t("runtime.openTimeout")),
      );
      if (hint) {
        hint.hidden = true;
        hint.replaceChildren();
      }
    } catch (error) {
      try {
        await closeAskWindow({ destroy: askCloseDestroyOnFailure() });
      } catch {
        /* window may already be gone or the UI thread is stuck */
      }
      const message = t("runtime.openFailed", { error: String(error) });
      if (hint) {
        hint.hidden = false;
        hint.textContent = message;
      }
      deps.setStatusBanner("error", message);
    } finally {
      openingAskRuntimes.delete(runtime);
      for (const button of askButtons) {
        button.removeAttribute("disabled");
      }
    }
  }

  async function openAskWindowForVerify(runtime: string, card?: HTMLElement | null): Promise<void> {
    if (opensOwnWindowOnly(runtime)) {
      await openDesktopApp(runtime);
      return;
    }
    localStorage.setItem(
      ASK_VERIFY_DRAFT_KEY,
      JSON.stringify({ prompt: t("ask.verifyPrompt"), autoSend: true }),
    );
    await openAskWindow(runtime, card);
  }

  async function openSessionFromCard(card: HTMLElement, forceTerminal = false) {
    const runtime = card.dataset.runtime;
    const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
    const openButtons = [
      ...card.querySelectorAll<HTMLButtonElement>('[data-action="open-session"]'),
      ...card.querySelectorAll<HTMLButtonElement>('[data-action="open-desktop-app"]'),
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
        openSession({
          runtime,
          cwd: null,
          prompt: null,
          terminal: forceTerminal ? true : null,
        }),
        20_000,
        new Error(t("runtime.openTimeout")),
      );
      const method =
        report.method === "deep-link"
          ? "deep-link"
          : report.method === "app"
            ? "app"
            : "terminal";
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

  async function openDesktopAppFromCard(card: HTMLElement): Promise<void> {
    const runtime = card.dataset.runtime;
    const hint = card.querySelector<HTMLElement>("[data-repair-hint]");
    const openButtons = [
      ...card.querySelectorAll<HTMLButtonElement>('[data-action="open-session"]'),
      ...card.querySelectorAll<HTMLButtonElement>('[data-action="open-desktop-app"]'),
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
        openSession({
          runtime,
          cwd: null,
          prompt: null,
          terminal: null,
          app: true,
        }),
        20_000,
        new Error(t("runtime.openTimeout")),
      );
      if (hint) {
        hint.textContent = t("runtime.openOk", { method: report.method });
      }
      deps.setStatusBanner("ok", t("runtime.openOk", { method: report.method }));
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
    openDesktopAppFromCard,
  };
}

export type AgentsSessionsApi = ReturnType<typeof createAgentsSessions>;
