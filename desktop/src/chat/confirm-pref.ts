import type { ChatSession } from "./types";

const KEY = "agent-doctor-settings-confirm";

export type ConfirmMode = "ask" | "session" | "auto";

export function readConfirmMode(): ConfirmMode {
  try {
    const saved = localStorage.getItem(KEY);
    if (saved === "session" || saved === "auto") return saved;
    return "ask";
  } catch {
    return "ask";
  }
}

export function writeConfirmMode(mode: ConfirmMode): void {
  try {
    localStorage.setItem(KEY, mode);
  } catch {
    /* ignore */
  }
}

/** Later file changes in this chat skip the allow card. Questions still wait. */
export function sessionSkipsApproval(session: Pick<ChatSession, "rememberApprovals"> | null | undefined): boolean {
  return readConfirmMode() === "session" && session?.rememberApprovals === true;
}

export function confirmHintKey(mode: ConfirmMode): "chat.settingsConfirmAskHint" | "chat.settingsConfirmSessionHint" | "chat.settingsConfirmAutoHint" {
  if (mode === "auto") return "chat.settingsConfirmAutoHint";
  if (mode === "session") return "chat.settingsConfirmSessionHint";
  return "chat.settingsConfirmAskHint";
}

export function noteApprovalRemembered(session: ChatSession, inputMode: string | undefined): void {
  if (readConfirmMode() !== "session") return;
  if (inputMode === "line" || inputMode === "secret" || inputMode === "options") return;
  session.rememberApprovals = true;
}
