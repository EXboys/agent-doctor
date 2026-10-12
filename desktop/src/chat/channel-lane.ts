export type ChannelQueued = {
  id: string;
  sessionId: string;
  text: string;
  noticed: boolean;
};

const queues = new Map<string, ChannelQueued[]>();
let onIdle: ((sessionId: string) => void) | null = null;

export function setChannelSessionIdle(handler: (sessionId: string) => void): void {
  onIdle = handler;
}

export function notifyChannelSessionIdle(sessionId: string): void {
  onIdle?.(sessionId);
}

export function channelQueueLength(sessionId: string): number {
  return queues.get(sessionId)?.length ?? 0;
}

export function enqueueChannelJob(job: ChannelQueued): void {
  const list = queues.get(job.sessionId) ?? [];
  list.push(job);
  queues.set(job.sessionId, list);
}

/** The sentence that lost the race to start goes back to the front. */
export function unshiftChannelJob(job: ChannelQueued): void {
  const list = queues.get(job.sessionId) ?? [];
  list.unshift(job);
  queues.set(job.sessionId, list);
}

export function takeNextChannelJob(sessionId: string): ChannelQueued | undefined {
  const list = queues.get(sessionId);
  if (!list?.length) return undefined;
  const next = list.shift();
  if (list.length === 0) queues.delete(sessionId);
  return next;
}

type TurnMessage = { role: string; content: string; at: number };

/** Assistant text written for this sentence, not a later one in the same chat. */
export function assistantForChannelTurn(
  messages: TurnMessage[],
  startedAt: number,
  userText: string,
): string {
  const wanted = userText.trim();
  if (!wanted) return "";
  let userAt = 0;
  for (const message of messages) {
    if (message.role !== "user" || message.at < startedAt) continue;
    if (message.content.trim() !== wanted) continue;
    userAt = message.at;
    break;
  }
  if (!userAt) return "";
  let assistant = "";
  for (const message of messages) {
    if (message.at < userAt) continue;
    if (message.role === "user" && message.at > userAt) break;
    if (message.role === "assistant") assistant = message.content.trim();
  }
  return assistant;
}
