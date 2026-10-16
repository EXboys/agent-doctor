import { settingsApplyKind } from "../diagnose-flow";
import { focusMainTab, openSession, resignNativeSubscription, runRepairExecute } from "../ipc";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  formatChatFailureLine,
  type ChatFailureAction,
  type ChatFailureExplain,
} from "../friendly-error";
import { t } from "../i18n";

const MAC_FILES_PRIVACY =
  "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders";

export type ChatFailureHandlers = {
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  selectedRuntime: () => string;
  focusPrompt?: () => void;
};

export function mountChatFailureBubble(
  logEl: HTMLElement,
  explain: ChatFailureExplain,
  handlers: ChatFailureHandlers,
): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "chat-failure-wrap";

  const bubble = document.createElement("div");
  bubble.className = "chat-bubble meta is-failure";
  bubble.textContent = formatChatFailureLine(explain);
  wrap.appendChild(bubble);

  if (explain.actions.length > 0) {
    const actions = document.createElement("div");
    actions.className = "chat-failure-actions";
    actions.setAttribute("role", "group");

    for (const action of explain.actions) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "chat-failure-btn";
      btn.textContent = actionLabel(action, explain);
      btn.addEventListener("click", () => {
        void runChatFailureAction(action, explain, handlers, actions);
      });
      actions.appendChild(btn);
    }

    wrap.appendChild(actions);
  }
  logEl.appendChild(wrap);
  logEl.scrollTop = logEl.scrollHeight;
  return bubble;
}

function actionLabel(action: ChatFailureAction, explain: ChatFailureExplain): string {
  switch (action) {
    case "provider":
      return explain.kind === "provider_url" ||
        explain.kind === "network" ||
        explain.kind === "unknown"
        ? t("repair.gatewayUnreachableNext")
        : t("chat.fail.actionProvider");
    case "repair":
      return t("diagnose.flow.autoFixCta");
    case "native":
      return t("chat.fail.actionNative");
    case "privacy":
      return t("chat.fail.actionPrivacy");
    case "askAgain":
      return t("chat.fail.actionAskAgain");
    case "relogin":
      return t("chat.fail.actionRelogin");
  }
}

function writeActionResult(
  actions: HTMLElement,
  text: string,
  tone: "ok" | "warn" | "muted",
): void {
  let note = actions.querySelector<HTMLElement>(".chat-failure-result");
  if (!note) {
    note = document.createElement("p");
    note.className = "chat-failure-result";
    actions.appendChild(note);
  }
  note.hidden = false;
  note.dataset.tone = tone;
  note.textContent = text;
}

function setActionsBusy(actions: HTMLElement, busy: boolean): void {
  actions.querySelectorAll<HTMLButtonElement>("button").forEach((btn) => {
    btn.disabled = busy;
  });
}

async function runChatFailureAction(
  action: ChatFailureAction,
  explain: ChatFailureExplain,
  handlers: ChatFailureHandlers,
  actions: HTMLElement,
): Promise<void> {
  if (action === "provider") {
    setActionsBusy(actions, true);
    try {
      await focusMainTab({ tab: "provider" });
      writeActionResult(
        actions,
        explain.kind === "provider_url" || explain.kind === "network" || explain.kind === "unknown"
          ? t("chat.fail.openedAddress")
          : t("chat.fail.openedProvider"),
        "muted",
      );
    } catch {
      writeActionResult(actions, t("chat.fail.repairFailed"), "warn");
    } finally {
      setActionsBusy(actions, false);
    }
    return;
  }
  if (action === "repair") {
    setActionsBusy(actions, true);
    writeActionResult(actions, t("chat.fail.repairRunning"), "muted");
    try {
      const runtime = explain.kind === "glm_codex_url" ? "codex" : handlers.selectedRuntime();
      const report = await runRepairExecute({ runtime });
      const kind = settingsApplyKind(report);
      writeActionResult(
        actions,
        kind === "nothing"
          ? t("chat.fail.repairNothing")
          : kind === "still-fail"
            ? t("chat.fail.repairStillFail")
            : t("chat.fail.repairChanged"),
        kind === "changed" ? "ok" : "warn",
      );
    } catch {
      writeActionResult(actions, t("chat.fail.repairFailed"), "warn");
    } finally {
      setActionsBusy(actions, false);
    }
    return;
  }
  if (action === "relogin") {
    setActionsBusy(actions, true);
    try {
      const result = await resignNativeSubscription(handlers.selectedRuntime());
      const text =
        result.outcome === "using"
          ? t("chat.fail.reloginDone")
          : result.outcome === "unreachable"
            ? t("native.unreachable")
            : result.outcome === "missing_tool"
              ? handlers.selectedRuntime() === "codex"
                ? t("native.missingCodex")
                : t("native.missingClaude")
              : t("native.incomplete");
      writeActionResult(actions, text, result.outcome === "using" ? "ok" : "warn");
    } catch {
      writeActionResult(actions, t("chat.fail.reloginFailed"), "warn");
    } finally {
      setActionsBusy(actions, false);
    }
    return;
  }
  if (action === "askAgain") {
    handlers.focusPrompt?.();
    writeActionResult(actions, t("chat.fail.askAgainReady"), "muted");
    return;
  }
  if (action === "privacy") {
    setActionsBusy(actions, true);
    try {
      await openUrl(MAC_FILES_PRIVACY);
      writeActionResult(actions, t("chat.fail.privacyOpened"), "ok");
    } catch {
      writeActionResult(actions, t("chat.fail.privacyFailed"), "warn");
    } finally {
      setActionsBusy(actions, false);
    }
    return;
  }
  try {
    await openSession({
      runtime: handlers.selectedRuntime(),
      cwd: null,
      prompt: null,
      terminal: true,
    });
    writeActionResult(actions, t("chat.terminalOpened"), "ok");
  } catch {
    writeActionResult(actions, t("chat.terminalFailed"), "warn");
  }
}
