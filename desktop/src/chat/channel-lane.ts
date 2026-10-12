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

/**
 * A Feishu "1" / "2" while steps are still waiting confirms them.
 * The same word sent again must not become a new queued sentence.
 */
export function channelConfirmAction(opts: {
  confirmToken: boolean;
  pendingChoices: number;
  busy: boolean;
}): "confirm" | "repeat" | "queue" | "run" {
  if (opts.confirmToken && opts.pendingChoices > 0) return "confirm";
  if (opts.confirmToken && opts.busy) return "repeat";
  if (opts.busy) return "queue";
  return "run";
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

/**
 * The last words sit in a paint frame after the turn returns.
 * Wait until the text stops growing, then send that to Feishu.
 */
export async function settleChannelAnswer(
  read: () => string,
  opts?: { flush?: () => void; wait?: (ms: number) => Promise<void> },
): Promise<string> {
  const wait = opts?.wait ?? ((ms) => new Promise<void>((resolve) => setTimeout(resolve, ms)));
  let last = "";
  let same = 0;
  for (let i = 0; i < 20; i += 1) {
    opts?.flush?.();
    const now = read();
    if (now && now === last) {
      same += 1;
      if (same >= 2) return now;
    } else {
      last = now;
      same = 0;
    }
    await wait(50);
  }
  opts?.flush?.();
  return last || read();
}
