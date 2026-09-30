import type { MentionRef } from "../ask-resources";
import { t } from "../i18n";
import type { ChatAttachment } from "./types";

export type FollowDraft = {
  id: string;
  sessionId: string;
  text: string;
  attachments: ChatAttachment[];
  mentions: MentionRef[];
  fromVoice?: boolean;
  verifyMcp?: boolean;
};

const queues = new Map<string, FollowDraft[]>();
let drainIntent: "when-ready" | "now" | "hold" = "when-ready";
let onInsertNow: (() => void) | null = null;

export function setFollowQueueInsertHandler(handler: () => void): void {
  onInsertNow = handler;
}

/** Stop should not send the queue. "现在插入" sets now first, and must win. */
export function holdFollowUps(): void {
  if (drainIntent !== "now") drainIntent = "hold";
}

export function insertFollowUpsNow(): void {
  drainIntent = "now";
}

export function consumeDrainIntent(): "when-ready" | "now" | "hold" {
  const intent = drainIntent;
  drainIntent = "when-ready";
  return intent;
}

export function followUpsFor(sessionId: string): FollowDraft[] {
  return queues.get(sessionId) ?? [];
}

export function enqueueFollowUp(sessionId: string, draft: Omit<FollowDraft, "id">): FollowDraft {
  const item: FollowDraft = { ...draft, id: crypto.randomUUID() };
  const list = queues.get(sessionId) ?? [];
  list.push(item);
  queues.set(sessionId, list);
  return item;
}

export function removeFollowUp(sessionId: string, id: string): void {
  const list = (queues.get(sessionId) ?? []).filter((item) => item.id !== id);
  if (list.length === 0) queues.delete(sessionId);
  else queues.set(sessionId, list);
}

export function takeFollowUps(sessionId: string): FollowDraft[] {
  const list = queues.get(sessionId) ?? [];
  queues.delete(sessionId);
  return list;
}

/** One follow-up. Several notes collapse so the latest sentence is the task. */
export function mergedFollowUp(items: FollowDraft[]): FollowDraft | null {
  if (items.length === 0) return null;
  if (items.length === 1) return items[0];
  const latest = items[items.length - 1];
  const earlier = items
    .slice(0, -1)
    .map((item) => item.text)
    .join("\n\n");
  return {
    ...latest,
    text: t("chat.queueSteer", { earlier, latest: latest.text }),
    attachments: items.flatMap((item) => item.attachments),
    mentions: items.flatMap((item) => item.mentions),
  };
}

export function renderFollowQueue(
  host: HTMLElement | null,
  sessionId: string,
  canInsertNow: boolean,
): void {
  if (!host) return;
  const items = followUpsFor(sessionId);
  host.replaceChildren();
  host.hidden = items.length === 0;
  if (items.length === 0) return;

  const head = document.createElement("div");
  head.className = "chat-follow-queue-head";
  const label = document.createElement("span");
  label.textContent = t("chat.queueTitle");
  head.appendChild(label);
  if (canInsertNow) {
    const now = document.createElement("button");
    now.type = "button";
    now.className = "chat-follow-now";
    now.textContent = t("chat.queueNow");
    now.addEventListener("click", () => {
      insertFollowUpsNow();
      onInsertNow?.();
    });
    head.appendChild(now);
  }
  host.appendChild(head);

  const list = document.createElement("ul");
  list.className = "chat-follow-list";
  for (const item of items) {
    const li = document.createElement("li");
    li.className = "chat-follow-item";
    const text = document.createElement("p");
    text.textContent = item.text || t("chat.attachOnlyPrompt");
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "chat-follow-remove";
    remove.textContent = t("chat.queueRemove");
    remove.addEventListener("click", () => {
      removeFollowUp(sessionId, item.id);
      renderFollowQueue(host, sessionId, canInsertNow);
    });
    li.append(text, remove);
    list.appendChild(li);
  }
  host.appendChild(list);
}
