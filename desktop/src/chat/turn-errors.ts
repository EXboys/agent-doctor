import { explainChatFailure, type ChatFailureExplain } from "../friendly-error";

let reported: string[] = [];
let turnLines: string[] = [];
let failureBubbleShown = false;
let noticeShown = false;

export function resetChatTurnErrors(): void {
  reported = [];
  turnLines = [];
  failureBubbleShown = false;
  noticeShown = false;
}

/** A sentence for the person already says why this turn failed. */
export function markTurnNoticeShown(): void {
  noticeShown = true;
}

export function turnNoticeShown(): boolean {
  return noticeShown;
}

export function pushChatTurnError(line: string): void {
  const text = line.trim();
  if (!text) return;
  turnLines.push(text);
}

/** The runtime's own report of why the turn failed. Explained before the raw log. */
export function pushChatTurnReport(message: string): void {
  const text = message.trim();
  if (!text) return;
  reported.push(text);
}

export function turnErrorBlob(): string {
  return turnLines.join("\n");
}

/** Best explain for the current turn: the runtime's report first, then the raw log. */
export function explainCurrentChatTurnFailure(): ChatFailureExplain | null {
  return explainChatFailure(reported.join("\n")) ?? explainChatFailure(turnErrorBlob());
}

export function markChatFailureBubbleShown(): boolean {
  if (failureBubbleShown) return false;
  failureBubbleShown = true;
  return true;
}

export function chatFailureBubbleAlreadyShown(): boolean {
  return failureBubbleShown;
}
