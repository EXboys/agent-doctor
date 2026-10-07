import { t } from "../i18n";
import { durationLabel } from "./activity";
import type { ChatMessage } from "./types";

const MAX_THINKING_CHARS = 12_000;

export function renderThinkingBlock(text: string, opts?: { id?: string; live?: boolean }): HTMLDetailsElement {
  const block = document.createElement("details");
  block.className = "chat-thinking";
  if (opts?.id) block.dataset.messageId = opts.id;
  block.innerHTML = `
    <summary class="chat-thinking-summary">
      <span class="chat-thinking-label"></span>
      <span class="chat-activity-elapsed"></span>
      <span class="chat-thinking-chevron" aria-hidden="true"></span>
    </summary>
    <div class="chat-thinking-body"></div>
  `;
  block.querySelector<HTMLElement>(".chat-thinking-body")!.textContent = text;
  if (opts?.live) {
    markLive(block);
  } else {
    block.querySelector<HTMLElement>(".chat-thinking-label")!.textContent = t("chat.thinkingSaved");
  }
  return block;
}

function markLive(block: HTMLDetailsElement): void {
  block.classList.add("is-live");
  block.open = true;
  block.querySelector<HTMLElement>(".chat-thinking-label")!.textContent = t("chat.thinkingLive");
}

export type ThinkingDeps = {
  logEl: HTMLElement;
  isViewingRunningSession: () => boolean;
  persistMessage: (content: string) => ChatMessage;
  updateMessage: (id: string, content: string) => void;
  /** Finish the reply streamed so far, so text after this block lands below it. */
  endReply: () => void;
  /** Close the live rows above so tools after this block start a new group. */
  beforeBlock: () => void;
};

export type ThinkingApi = ReturnType<typeof createThinkingController>;

export function createThinkingController(deps: ThinkingDeps) {
  let messageId: string | null = null;
  let text = "";
  let startedAt = 0;
  let el: HTMLDetailsElement | null = null;

  function findBlock(): HTMLDetailsElement | null {
    if (el?.isConnected) return el;
    if (!messageId) return null;
    el = deps.logEl.querySelector<HTMLDetailsElement>(`.chat-thinking[data-message-id="${CSS.escape(messageId)}"]`);
    return el;
  }

  function liveBlock(): HTMLDetailsElement | null {
    if (!deps.isViewingRunningSession()) return null;
    const found = findBlock();
    if (found) {
      if (!found.classList.contains("is-live")) markLive(found);
      return found;
    }
    deps.beforeBlock();
    el = renderThinkingBlock(text, { id: messageId ?? undefined, live: true });
    deps.logEl.appendChild(el);
    return el;
  }

  function append(chunk: string): void {
    if (!chunk) return;
    if (!messageId) {
      text = "";
      startedAt = Date.now();
      deps.endReply();
      messageId = deps.persistMessage("").id;
    }
    if (text.length >= MAX_THINKING_CHARS) return;
    text = (text + chunk).slice(0, MAX_THINKING_CHARS);
    deps.updateMessage(messageId, text);
    const block = liveBlock();
    if (!block) return;
    const body = block.querySelector<HTMLElement>(".chat-thinking-body")!;
    body.textContent = text;
    body.scrollTop = body.scrollHeight;
    deps.logEl.scrollTop = deps.logEl.scrollHeight;
  }

  function seal(): void {
    if (!messageId) return;
    const block = findBlock();
    if (block) {
      block.classList.remove("is-live");
      block.open = false;
      const elapsed = durationLabel(Date.now() - startedAt);
      block.querySelector<HTMLElement>(".chat-thinking-label")!.textContent = elapsed
        ? t("chat.thinkingDone", { elapsed })
        : t("chat.thinkingSaved");
      block.querySelector<HTMLElement>(".chat-activity-elapsed")!.textContent = "";
    }
    messageId = null;
    text = "";
    el = null;
  }

  return { append, seal, isLive: () => messageId !== null };
}
