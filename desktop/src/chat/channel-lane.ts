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

export function dropChannelJob(jobId: string): ChannelQueued | undefined {
  for (const [sessionId, list] of queues) {
    const index = list.findIndex((job) => job.id === jobId);
    if (index < 0) continue;
    const [job] = list.splice(index, 1);
    if (list.length === 0) queues.delete(sessionId);
    return job;
  }
  return undefined;
}

export function takeNextChannelJob(sessionId: string): ChannelQueued | undefined {
  const list = queues.get(sessionId);
  if (!list?.length) return undefined;
  const next = list.shift();
  if (list.length === 0) queues.delete(sessionId);
  return next;
}

type TurnStep = {
  id?: string;
  name?: string;
  status?: string;
  title?: string;
  path?: string;
  command?: string;
  query?: string;
  output?: string;
};

type TurnMessage = { role: string; content: string; at: number; step?: TurnStep };

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

export type ChannelToolTrace = {
  id: string;
  name: string;
  status: "running" | "done" | "failed";
  detail: string;
  output: string;
};

export type ChannelDelta = {
  delta: true;
  text: string;
  thinking: string;
  resetText: boolean;
  resetThinking: boolean;
  tools: ChannelToolTrace[];
};

const DELTA_CAP = 2000;

export type ChannelTrace = {
  thinking: string;
  tools: ChannelToolTrace[];
};

const THINKING_CAP = 8000;
const TOOL_DETAIL_CAP = 500;
const TOOL_OUTPUT_CAP = 2000;
const MAX_TOOLS = 30;

/** Thinking and tool runs for this sentence, capped so the channel can carry them. */
export function channelTurnTrace(
  messages: TurnMessage[],
  startedAt: number,
  userText: string,
): ChannelTrace {
  const slice = turnMessages(messages, startedAt, userText);
  const thinking = clipTail(
    slice
      .filter((message) => message.role === "thinking")
      .map((message) => message.content.trim())
      .filter(Boolean)
      .join("\n\n"),
    THINKING_CAP,
  );
  const tools: ChannelToolTrace[] = [];
  for (let i = 0; i < slice.length; i += 1) {
    const message = slice[i];
    if (message.role !== "tool") continue;
    const next = slice[i + 1];
    if (!message.step && next?.role === "tool" && next.step && sameTool(message, next)) continue;
    const tool = toolTrace(message, i);
    if (tool) tools.push(tool);
  }
  return { thinking, tools: tools.slice(-MAX_TOOLS) };
}

function turnMessages(messages: TurnMessage[], startedAt: number, userText: string): TurnMessage[] {
  const wanted = userText.trim();
  if (!wanted) return [];
  let userAt = 0;
  for (const message of messages) {
    if (message.role !== "user" || message.at < startedAt) continue;
    if (message.content.trim() !== wanted) continue;
    userAt = message.at;
    break;
  }
  if (!userAt) return [];
  const slice: TurnMessage[] = [];
  for (const message of messages) {
    if (message.at < userAt) continue;
    if (message.role === "user" && message.at > userAt) break;
    slice.push(message);
  }
  return slice;
}

/** The new piece since `previous`. Callers send this and keep the rest locally. */
export function channelDelta(
  previous: { text: string; thinking: string; tools: ChannelToolTrace[] },
  next: { text: string; thinking: string; tools: ChannelToolTrace[] },
): ChannelDelta | null {
  const text = takePiece(previous.text, next.text);
  const thinking = takePiece(previous.thinking, next.thinking);
  const tools = changedTools(previous.tools, next.tools);
  if (!text.piece && !thinking.piece && tools.length === 0) return null;
  return {
    delta: true,
    text: text.piece,
    thinking: thinking.piece,
    resetText: text.reset,
    resetThinking: thinking.reset,
    tools,
  };
}

/** Advance the cursor by the piece that was actually sent. */
export function advanceChannelCursor(
  previous: { text: string; thinking: string; tools: ChannelToolTrace[] },
  delta: ChannelDelta,
): { text: string; thinking: string; tools: ChannelToolTrace[] } {
  return {
    text: delta.resetText ? delta.text : previous.text + delta.text,
    thinking: delta.resetThinking ? delta.thinking : previous.thinking + delta.thinking,
    tools: mergeToolCursor(previous.tools, delta.tools),
  };
}

function takePiece(previous: string, next: string): { piece: string; reset: boolean } {
  if (next === previous) return { piece: "", reset: false };
  if (!previous || next.startsWith(previous)) {
    const start = previous.length;
    return { piece: next.slice(start, start + DELTA_CAP), reset: false };
  }
  return { piece: next.slice(0, DELTA_CAP), reset: true };
}

function changedTools(previous: ChannelToolTrace[], next: ChannelToolTrace[]): ChannelToolTrace[] {
  const seen = new Map(previous.map((tool) => [tool.id, toolKey(tool)]));
  return next.filter((tool) => seen.get(tool.id) !== toolKey(tool));
}

function mergeToolCursor(previous: ChannelToolTrace[], updates: ChannelToolTrace[]): ChannelToolTrace[] {
  const tools = previous.slice();
  for (const update of updates) {
    const at = tools.findIndex((tool) => tool.id === update.id);
    if (at >= 0) tools[at] = update;
    else tools.push(update);
  }
  return tools;
}

function toolKey(tool: ChannelToolTrace): string {
  return `${tool.name}\n${tool.status}\n${tool.detail}\n${tool.output}`;
}

function toolTrace(message: TurnMessage, index: number): ChannelToolTrace | null {
  const step = message.step;
  const rawLines = message.content.split("\n");
  const head = oneLine(rawLines[0] ?? "");
  const body = rawLines.slice(1).join("\n").trim();
  const name = oneLine(step?.title || step?.name || head);
  if (!name && !step?.command && !step?.output && !body) return null;
  const detail = clipTail(step?.command || step?.path || step?.query || "", TOOL_DETAIL_CAP);
  const output = clipTail(step?.output || (step ? "" : body), TOOL_OUTPUT_CAP);
  return {
    id: step?.id || `t${index}:${name || "tool"}`,
    name: name || "工具",
    status: toolStatus(step?.status),
    detail,
    output,
  };
}

function sameTool(bare: TurnMessage, step: TurnMessage): boolean {
  const left = oneLine(bare.content.split("\n")[0] ?? "").toLowerCase();
  const right = oneLine(step.step?.name || step.step?.title || "").toLowerCase();
  if (!left || !right) return false;
  return left === right || left.endsWith(right) || right.endsWith(left);
}

function toolStatus(value: string | undefined): ChannelToolTrace["status"] {
  if (value === "running" || value === "failed") return value;
  return "done";
}

function oneLine(value: string): string {
  return value.trim().replace(/^(?:调用工具|call(?:ing)? tool)\s*/i, "").replace(/[….]+$/g, "").trim();
}

function clipTail(value: string, max: number): string {
  const text = value.trim();
  if (text.length <= max) return text;
  return `…${text.slice(text.length - max)}`;
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
