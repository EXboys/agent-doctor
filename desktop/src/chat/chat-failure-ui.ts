import { focusMainTab, openSession, runRepairExecute } from "../ipc";
import {
  formatChatFailureLine,
  type ChatFailureAction,
  type ChatFailureExplain,
} from "../friendly-error";
import { t } from "../i18n";
export type ChatFailureHandlers = {
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  selectedRuntime: () => string;
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

  const actions = document.createElement("div");
  actions.className = "chat-failure-actions";
  actions.setAttribute("role", "group");

  for (const action of explain.actions) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "chat-failure-btn";
    btn.textContent = actionLabel(action);
    btn.addEventListener("click", () => {
      void runChatFailureAction(action, explain, handlers);
    });
    actions.appendChild(btn);
  }

  wrap.appendChild(actions);
  logEl.appendChild(wrap);
  logEl.scrollTop = logEl.scrollHeight;
  return bubble;
}

function actionLabel(action: ChatFailureAction): string {
  switch (action) {
    case "provider":
      return t("chat.fail.actionProvider");
    case "repair":
      return t("chat.fail.actionRepair");
    case "native":
      return t("chat.fail.actionNative");
  }
}

async function runChatFailureAction(
  action: ChatFailureAction,
  explain: ChatFailureExplain,
  handlers: ChatFailureHandlers,
): Promise<void> {
  if (action === "provider") {
    try {
      await focusMainTab({ tab: "provider" });
      handlers.setStatus(t("chat.fail.openedProvider"), "muted");
    } catch (error) {
      handlers.setStatus(String(error), "error");
    }
    return;
  }
  if (action === "repair") {
    try {
      await focusMainTab({ tab: "diagnose" });
      handlers.setStatus(t("chat.fail.repairRunning"), "muted");
      const runtime = explain.kind === "glm_codex_url" ? "codex" : handlers.selectedRuntime();
      await runRepairExecute({ runtime });
      handlers.setStatus(t("chat.fail.repairDone"), "ok");
    } catch (error) {
      handlers.setStatus(t("chat.fail.repairFailed"), "warn");
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
    handlers.setStatus(t("chat.terminalOpened"), "ok");
  } catch {
    handlers.setStatus(t("chat.terminalFailed"), "error");
  }
}
