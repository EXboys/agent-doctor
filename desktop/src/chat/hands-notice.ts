import { openUrl } from "@tauri-apps/plugin-opener";
import { t } from "../i18n";
import { openMacAccessibility } from "./computer-grant";
import type { HandsBlock } from "./hands-intent";
import { openChatSettings } from "./settings-page";

const noticeEl = () => document.querySelector<HTMLElement>("#chat-hands-notice");

export function hideHandsNotice(): void {
  const el = noticeEl();
  if (!el) return;
  el.hidden = true;
  el.replaceChildren();
}

/** Stop the send and tell the person which switch to open. */
export function showHandsNotice(
  block: HandsBlock,
  mode: "settings" | "system" | "pending" = "settings",
  settingsUrl?: string | null,
  promptAccessibility = false,
): void {
  const el = noticeEl();
  if (!el) return;
  const text =
    mode === "pending"
      ? t("chat.handsComputerPending")
      : block === "both"
        ? t("chat.handsNeedBoth")
        : block === "computer"
          ? t("chat.handsNeedComputer")
          : t("chat.handsNeedBrowser");
  const sentence = document.createElement("span");
  sentence.textContent = text;
  if (mode === "pending") {
    el.replaceChildren(sentence);
    el.hidden = false;
    return;
  }
  const button = document.createElement("button");
  button.type = "button";
  button.className = "chat-hands-notice-btn";
  if (mode === "system") {
    button.textContent = t("chat.handsOpenSystem");
    const open = () => {
      if (promptAccessibility) {
        void openMacAccessibility().catch(() => {
          sentence.textContent = t("chat.handsSystemFailed");
        });
        return;
      }
      void openUrl(settingsUrl || "").catch(() => {
        sentence.textContent = t("chat.handsSystemFailed");
      });
    };
    button.addEventListener("click", open);
  } else {
    button.textContent = t("chat.handsOpenSettings");
    button.addEventListener("click", () => {
      hideHandsNotice();
      openChatSettings("conversation", block === "computer" ? "computer" : "browser");
    });
  }
  el.replaceChildren(sentence, button);
  el.hidden = false;
}
