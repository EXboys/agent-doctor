import { t } from "../i18n";
import { elapsedLabel } from "./activity";

const MAX_TEXT = 60;

export type RunningApi = ReturnType<typeof createRunningIndicator>;

/** Bar on top of the composer while a reply is running: what it is doing and for how long. */
export function createRunningIndicator(boxEl: HTMLElement) {
  const el = document.createElement("div");
  el.className = "chat-running";
  el.hidden = true;
  el.setAttribute("role", "status");
  el.innerHTML = `
    <span class="chat-running-dot" aria-hidden="true"></span>
    <span class="chat-running-text"></span>
    <span class="chat-running-time"></span>
  `;
  boxEl.prepend(el);
  const textEl = el.querySelector<HTMLElement>(".chat-running-text")!;
  const timeEl = el.querySelector<HTMLElement>(".chat-running-time")!;
  let text = "";
  let timer = 0;

  function paint(): void {
    textEl.textContent = text || t("chat.runningDefault");
    timeEl.textContent = elapsedLabel();
  }

  function setRunning(on: boolean): void {
    boxEl.classList.toggle("is-running", on);
    el.hidden = !on;
    if (on && !timer) timer = window.setInterval(paint, 1000);
    if (!on && timer) {
      window.clearInterval(timer);
      timer = 0;
    }
    paint();
  }

  function setText(message: string): void {
    const line = message.split("\n")[0]?.trim() ?? "";
    if (!line) return;
    text = line.length > MAX_TEXT ? `${line.slice(0, MAX_TEXT - 1)}…` : line;
    if (!el.hidden) paint();
  }

  return { setRunning, setText };
}
