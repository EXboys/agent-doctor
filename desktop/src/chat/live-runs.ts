import type { PendingPermission } from "./types";

/** How many chats may be working at the same time. */
export const MAX_PARALLEL_RUNS = 4;

export type LiveRun = {
  chatSessionId: string;
  backendSessionId: string | null;
  busyGen: number;
  assistantMessageId: string | null;
  assistantRaw: string;
  pendingText: string;
  turnHadAssistantText: boolean;
  pendingPermissions: PendingPermission[];
  thinkingMessageId: string | null;
  thinkingText: string;
};

const runs = new Map<string, LiveRun>();
const backendToChat = new Map<string, string>();

export function isChatRunning(id: string | null | undefined): boolean {
  return Boolean(id && runs.has(id));
}

export function runningCount(): number {
  return runs.size;
}

export function liveRun(id: string | null | undefined): LiveRun | undefined {
  if (!id) return undefined;
  return runs.get(id);
}

export function liveRuns(): Iterable<LiveRun> {
  return runs.values();
}

export function backgroundChoiceCount(id: string): number {
  return runs.get(id)?.pendingPermissions.length ?? 0;
}

export function beginLiveRun(id: string): LiveRun {
  const existing = runs.get(id);
  if (existing) return existing;
  const run: LiveRun = {
    chatSessionId: id,
    backendSessionId: null,
    busyGen: 0,
    assistantMessageId: null,
    assistantRaw: "",
    pendingText: "",
    turnHadAssistantText: false,
    pendingPermissions: [],
    thinkingMessageId: null,
    thinkingText: "",
  };
  runs.set(id, run);
  return run;
}

export function endLiveRun(id: string): void {
  const run = runs.get(id);
  if (!run) return;
  if (run.backendSessionId) backendToChat.delete(run.backendSessionId);
  runs.delete(id);
}

export function bindBackendSession(chatId: string, backendId: string): void {
  if (!backendId) return;
  const run = runs.get(chatId);
  if (!run) return;
  if (run.backendSessionId && run.backendSessionId !== backendId) {
    backendToChat.delete(run.backendSessionId);
  }
  run.backendSessionId = backendId;
  backendToChat.set(backendId, chatId);
}

export function chatIdForBackend(backendId: string | undefined): string | null {
  if (!backendId) return null;
  const chatId = backendToChat.get(backendId);
  if (!chatId || !runs.has(chatId)) return null;
  return chatId;
}
