import { explainChatFailure, type ChatFailureExplain } from "../friendly-error";

let turnLines: string[] = [];
let failureBubbleShown = false;

export function resetChatTurnErrors(): void {
  turnLines = [];
  failureBubbleShown = false;
}

export function pushChatTurnError(line: string): void {
  const text = line.trim();
  if (!text) return;
  turnLines.push(text);
}

export function turnErrorBlob(): string {
  return turnLines.join("\n");
}

/** Best explain for the current turn (stderr + API errors accumulated this run). */
export function explainCurrentChatTurnFailure(): ChatFailureExplain | null {
  return explainChatFailure(turnErrorBlob());
}

export function markChatFailureBubbleShown(): boolean {
  if (failureBubbleShown) return false;
  failureBubbleShown = true;
  return true;
}

export function chatFailureBubbleAlreadyShown(): boolean {
  return failureBubbleShown;
}
