import { t } from "../i18n";
import { clearThinkingStarted, durationLabel, followChatLog, markThinkingStartedAt } from "./activity";
import { noteThinkingLive, placeProcessBlock } from "./work-trail";
import type { ChatMessage } from "./types";

const MAX_THINKING_CHARS = 12_000;
/**
 * While live, only the tail of the thinking text is in the DOM: the peek is
 * pinned to the bottom, so the earlier text is scrolled out of view anyway.
 * Renders O(tail) per frame instead of O(full text); the full text stays in
 * the session message and comes back in the folded block after the seal.
 */
const THINKING_TAIL_CHARS = 600;

/** The trailing slice of the thinking text, cut above a surrogate pair. */
function liveTail(text: string): string {
  if (text.length <= THINKING_TAIL_CHARS) return text;
  let start = text.length - THINKING_TAIL_CHARS;
  const code = text.charCodeAt(start);
  if (code >= 0xdc00 && code <= 0xdfff) start -= 1; // land on the high surrogate
  return text.slice(start);
}

export function renderThinkingBlock(text: string, opts?: { id?: string; live?: boolean }): HTMLDetailsElement {
  const block = document.createElement("details");
  block.className = "chat-thinking";
  if (opts?.id) block.dataset.messageId = opts.id;
  block.innerHTML = `
    <summary class="chat-thinking-summary">
      <span class="chat-thinking-icon" aria-hidden="true"></span>
      <span class="chat-thinking-label"></span>
      <span class="chat-activity-elapsed"></span>
      <span class="chat-thinking-chevron" aria-hidden="true"></span>
    </summary>
    <div class="chat-thinking-body"></div>
  `;
  block.querySelector<HTMLElement>(".chat-thinking-body")!.textContent = text;
  // While thinking the row stays open so the peek keeps streaming; once done
  // the block folds into its header row and that row opens it again.
  block.querySelector("summary")?.addEventListener("click", (event) => {
    if (block.classList.contains("is-live")) event.preventDefault();
  });
  if (opts?.live) {
    markThinkingLive(block);
  } else {
    block.open = false;
    block.querySelector<HTMLElement>(".chat-thinking-label")!.textContent = t("chat.thinkingSaved");
  }
  return block;
}

/** Live, fresh or after a redraw: open, shimmering, and about to show the newest lines. */
export function markThinkingLive(block: HTMLDetailsElement): void {
  block.classList.add("is-live");
  block.open = true;
  block.dataset.follow = "1";
  block.querySelector<HTMLElement>(".chat-thinking-label")!.textContent = t("chat.thinkingLive");
  noteThinkingLive(block);
}

export type ThinkingDeps = {
  logEl: HTMLElement;
  /** Land thinking blocks inside this turn's process container (optional). */
  placeBlock?: (el: HTMLDetailsElement) => void;
  isViewingRunningSession: () => boolean;
  persistMessage: (content: string) => ChatMessage;
  updateMessage: (id: string, content: string) => void;
  /** Finish the reply streamed so far, so text after this block lands below it. */
  endReply: () => void;
  /** Settle rows above a fresh thinking block; a live tool group stays put and keeps collecting. */
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
      if (!found.classList.contains("is-live")) markThinkingLive(found);
      return found;
    }
    deps.beforeBlock();
    el = renderThinkingBlock(text, { id: messageId ?? undefined, live: true });
    if (deps.placeBlock) deps.placeBlock(el);
    else placeProcessBlock(deps.logEl, el);
    return el;
  }

  let pendingThink = "";
  let thinkRaf = 0;
  function append(chunk: string): void {
    if (!chunk) return;
    if (!messageId) {
      text = "";
      startedAt = Date.now();
      markThinkingStartedAt(startedAt);
      deps.endReply();
      messageId = deps.persistMessage("").id;
    }
    if (text.length >= MAX_THINKING_CHARS) return;
    // Thinking events arrive faster than frames; rewrite the block once per
    // frame instead of per event — every rewrite forced a reflow.
    pendingThink += chunk;
    if (thinkRaf) return;
    thinkRaf = window.requestAnimationFrame(() => {
      thinkRaf = 0;
      paintThinkChunk();
    });
  }
  function paintThinkChunk(): void {
    if (thinkRaf) {
      window.cancelAnimationFrame(thinkRaf);
      thinkRaf = 0;
    }
    if (!pendingThink || !messageId) return;
    text = (text + pendingThink).slice(0, MAX_THINKING_CHARS);
    pendingThink = "";
    deps.updateMessage(messageId, text);
    const block = liveBlock();
    if (!block) return;
    const body = block.querySelector<HTMLElement>(".chat-thinking-body")!;
    // The peek keeps showing the newest lines unless the reader scrolled up inside it.
    const follow = block.dataset.follow === "1" || body.scrollHeight - body.scrollTop - body.clientHeight < 12;
    delete block.dataset.follow;
    body.textContent = liveTail(text);
    if (follow) body.scrollTop = body.scrollHeight;
    followChatLog(deps.logEl);
  }

  function seal(): void {
    const elapsed = messageId ? durationLabel(Date.now() - startedAt) : "";
    if (!messageId) {
      startedAt = 0;
      clearThinkingStarted();
      return;
    }
    // A frame callback may still hold the last chunk — land it before folding.
    paintThinkChunk();
    const block = findBlock();
    if (block) {
      block.classList.remove("is-live");
      block.open = false;
      // The live body only carried the tail; restore the full text so the
      // folded block still reads complete when expanded later.
      block.querySelector<HTMLElement>(".chat-thinking-body")!.textContent = text;
      block.querySelector<HTMLElement>(".chat-thinking-label")!.textContent = elapsed
        ? t("chat.thinkingDone", { elapsed })
        : t("chat.thinkingSaved");
      block.querySelector<HTMLElement>(".chat-activity-elapsed")!.textContent = "";
    }
    messageId = null;
    text = "";
    el = null;
    startedAt = 0;
    clearThinkingStarted();
  }

  function exportState(): { messageId: string | null; text: string } {
    return { messageId, text };
  }

  function importState(next: { messageId: string | null; text: string }): void {
    messageId = next.messageId;
    text = next.text;
    el = null;
    if (messageId && !startedAt) startedAt = Date.now();
    if (messageId) markThinkingStartedAt(startedAt);
  }

  return { append, seal, isLive: () => messageId !== null, exportState, importState };
}
