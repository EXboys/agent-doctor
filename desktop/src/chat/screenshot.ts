import { listen } from "@tauri-apps/api/event";
import { screenshotChipsEl } from "../chat-dom";
import { withErrorDetail } from "../friendly-error";
import { t } from "../i18n";
import { startRegionCapture } from "../ipc";
import type { AttachmentsApi } from "./attachments";

export type ScreenshotDeps = {
  getAttachments: () => AttachmentsApi | undefined;
  promptEl: HTMLTextAreaElement;
  autoResizePrompt: () => void;
  isComposerLocked: () => boolean;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
};

export type ScreenshotApi = ReturnType<typeof createScreenshotController>;

/** Presets shown while a picture sits in the composer. Deliberately plain
 * prompts: the capture pipeline stays generic, a scenario is just one chip. */
const CHIPS = [
  { label: "chat.screenshotChipDiagnose", prompt: "chat.screenshotChipDiagnosePrompt" },
  { label: "chat.screenshotChipExtract", prompt: "chat.screenshotChipExtractPrompt" },
  { label: "chat.screenshotChipExplain", prompt: "chat.screenshotChipExplainPrompt" },
] as const;

export function createScreenshotController(deps: ScreenshotDeps) {
  /** The picker reports back here, because the overlay webview cannot reach
   * the composer on its own. */
  async function setupScreenshotEvents(): Promise<void> {
    try {
      await listen<{ path: string }>("screenshot-captured", (event) => {
        deps.getAttachments()?.addAttachmentPaths([event.payload.path]);
      });
      await listen("screenshot-cancelled", () => {
        deps.setStatus(t("chat.screenshotCancelled"), "muted");
      });
      await listen<string>("screenshot-failed", (event) => {
        deps.setStatus(withErrorDetail(t("chat.screenshotFailed"), event.payload), "error");
      });
    } catch {
      /* browser preview without Tauri events */
    }
  }
  void setupScreenshotEvents();

  async function captureScreen(): Promise<void> {
    if (deps.isComposerLocked()) {
      // A silent no-op here reads as "the button is broken" — say so.
      deps.setStatus(t("chat.screenshotLocked"), "warn");
      return;
    }
    try {
      await startRegionCapture();
      deps.setStatus(t("chat.screenshotRegionHint"), "muted");
    } catch (error) {
      deps.setStatus(withErrorDetail(t("chat.screenshotFailed"), error), "error");
    }
  }

  /** Rebuilt on every attachment render; visible only while a picture waits. */
  function renderChips(): void {
    if (!screenshotChipsEl) return;
    const show =
      !deps.isComposerLocked() && (deps.getAttachments()?.hasImageAttachment() ?? false);
    screenshotChipsEl.replaceChildren();
    if (!show) {
      screenshotChipsEl.hidden = true;
      return;
    }
    for (const chip of CHIPS) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "chat-screenshot-chip";
      button.textContent = t(chip.label);
      button.addEventListener("click", () => {
        deps.promptEl.value = t(chip.prompt);
        deps.autoResizePrompt();
        deps.promptEl.focus();
      });
      screenshotChipsEl.appendChild(button);
    }
    screenshotChipsEl.hidden = false;
  }

  return { captureScreen, renderChips };
}
