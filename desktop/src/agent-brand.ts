import claudeUrl from "./assets/agent-icons/claude.svg?url";
import codexUrl from "./assets/agent-icons/codex.svg?url";
import cursorUrl from "./assets/agent-icons/cursor.svg?url";
import deepseekUrl from "./assets/agent-icons/deepseek.svg?url";
import hermesUrl from "./assets/agent-icons/hermes.svg?url";
import openclawUrl from "./assets/agent-icons/openclaw.svg?url";
import qoderUrl from "./assets/agent-icons/qoder.svg?url";
import workbuddyUrl from "./assets/agent-icons/workbuddy.svg?url";

/** Official brand marks (Lobe Icons pack of the vendors’ logos). */
const ICONS: Record<string, string> = {
  cursor: cursorUrl,
  "claude-code": claudeUrl,
  codex: codexUrl,
  "deepseek-harness": deepseekUrl,
  hermes: hermesUrl,
  openclaw: openclawUrl,
  qoder: qoderUrl,
  workbuddy: workbuddyUrl,
};

function iconImg(src: string): string {
  return `<img class="agent-brand-img" src="${src}" alt="" draggable="false" />`;
}

export function agentBrandIconHtml(runtimeId: string): string {
  const src = ICONS[runtimeId];
  return src ? iconImg(src) : "";
}

export function setAgentBrandIcon(el: HTMLElement, runtimeId: string): void {
  const src = ICONS[runtimeId];
  el.classList.add("agent-brand-icon");
  if (src) {
    el.innerHTML = iconImg(src);
    return;
  }
  el.textContent = (runtimeId.charAt(0) || "?").toUpperCase();
}
