import { t } from "../i18n";
import { looksLikeChoiceQuestion } from "./format";

export type DecisionEls = {
  decisionDockEl: HTMLElement;
  decisionKickerEl: HTMLElement;
  decisionTitleEl: HTMLElement;
  decisionDetailEl: HTMLElement;
  decisionActionsEl: HTMLElement;
  logEl: HTMLElement;
  promptEl: HTMLTextAreaElement;
};

export type DecisionDeps = DecisionEls & {
  getBusy: () => boolean;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  autoResizePrompt: () => void;
  sendAsk: () => Promise<void>;
};

export type DecisionApi = ReturnType<typeof createDecisionController>;

export function createDecisionController(deps: DecisionDeps) {
  function hideDecisionDock(): void {
    deps.decisionDockEl.hidden = true;
    deps.decisionKickerEl.textContent = "";
    deps.decisionTitleEl.textContent = "";
    deps.decisionDetailEl.textContent = "";
    deps.decisionDetailEl.hidden = true;
    deps.decisionActionsEl.replaceChildren();
  }

  function showDecisionDock(opts: {
    kicker: string;
    title: string;
    detail?: string;
    onDismiss?: () => void;
    actions: Array<{
      label: string;
      kind: "allow" | "deny" | "yes" | "no";
      onClick: () => void;
    }>;
  }): void {
    deps.decisionKickerEl.textContent = opts.kicker;
    deps.decisionTitleEl.textContent = opts.title;
    const detail = opts.detail?.trim() ?? "";
    deps.decisionDetailEl.textContent = detail;
    deps.decisionDetailEl.hidden = !detail;
    deps.decisionActionsEl.replaceChildren();
    for (const action of opts.actions) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = `chat-decision-btn is-${action.kind}`;
      btn.textContent = action.label;
      btn.addEventListener("click", () => action.onClick());
      deps.decisionActionsEl.appendChild(btn);
    }
    if (opts.onDismiss) {
      const close = document.createElement("button");
      close.type = "button";
      close.className = "chat-decision-dismiss";
      close.textContent = "×";
      close.setAttribute("aria-label", t("chat.dismissChoice"));
      close.title = t("chat.dismissChoice");
      const dismiss = opts.onDismiss;
      close.addEventListener("click", () => dismiss());
      deps.decisionActionsEl.appendChild(close);
    }
    deps.decisionDockEl.hidden = false;
  }

  function clearQuickReplies(): void {
    deps.logEl.querySelectorAll(".chat-quick-replies").forEach((el) => el.remove());
    if (
      !deps.decisionDockEl.querySelector(".chat-decision-btn.is-allow, .chat-decision-btn.is-deny")
    ) {
      // Only clear dock when it is showing quick replies, not a permission prompt.
      if (deps.decisionDockEl.querySelector(".chat-decision-btn.is-yes, .chat-decision-btn.is-no")) {
        hideDecisionDock();
      }
    }
  }

  function dismissChoice(): void {
    hideDecisionDock();
    deps.setStatus("", "muted");
  }

  function showQuickReplies(sourceText: string): void {
    clearQuickReplies();
    if (!looksLikeChoiceQuestion(sourceText)) return;

    showDecisionDock({
      kicker: t("chat.needYourChoiceShort"),
      title: t("chat.decisionQuestionTitle"),
      onDismiss: dismissChoice,
      actions: [
        {
          label: t("chat.quickYes"),
          kind: "yes",
          onClick: () => {
            if (deps.getBusy()) return;
            hideDecisionDock();
            deps.promptEl.value = t("chat.quickYesSend");
            deps.autoResizePrompt();
            void deps.sendAsk();
          },
        },
        {
          label: t("chat.quickNo"),
          kind: "no",
          onClick: () => {
            if (deps.getBusy()) return;
            dismissChoice();
          },
        },
      ],
    });
    deps.setStatus(t("chat.needYourChoiceShort"), "warn");
  }

  return {
    hideDecisionDock,
    showDecisionDock,
    clearQuickReplies,
    dismissChoice,
    showQuickReplies,
  };
}
