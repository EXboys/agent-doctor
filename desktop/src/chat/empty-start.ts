import type { ChatSession } from "./types";

/** True once the user or assistant has said anything in this session. */
export function sessionHasChatContent(session: ChatSession): boolean {
  return session.messages.some((m) => m.role === "user" || m.role === "assistant");
}

export function clearEmptyChatStart(log: HTMLElement): void {
  log.querySelectorAll(".chat-start").forEach((el) => el.remove());
}
