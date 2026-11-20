import { mergeToolStep, runLogAlreadyHas, splitToolActivity, toolSignature, toolStepText } from "./format";
import type { ChatMessage, ToolStep } from "./types";

export type ToolRecord = string | { text: string; step?: ToolStep; skipped?: number };

function toolFace(text: string): string {
  return splitToolActivity(text).summary.replace(/[….]+$/g, "").toLowerCase();
}

/**
 * A tool is announced once before its command arrives, then saved again with the
 * step. The announcement has no command. Drop it when the next record is that step,
 * so the list does not show a blank row above every real one.
 */
export function omitBareToolStarts(records: ToolRecord[]): ToolRecord[] {
  const out: ToolRecord[] = [];
  const claimedSteps = new Set<number>();
  for (let i = 0; i < records.length; i += 1) {
    const record = records[i];
    if (record && typeof record === "string") {
      const name = toolFace(record);
      const { detail } = splitToolActivity(record);
      if (!detail && name) {
        // Match either the immediately following step or, in a parallel tool
        // batch ([bare A, bare B, step A, step B]), the next unclaimed step
        // with the same tool face.
        let matched = -1;
        for (let j = i + 1; j < records.length; j += 1) {
          if (claimedSteps.has(j)) continue;
          const candidate = records[j];
          if (!candidate || typeof candidate === "string" || !candidate.step) continue;
          const stepName = toolFace(candidate.step.name || candidate.text);
          if (stepName === name) {
            matched = j;
            break;
          }
        }
        if (matched >= 0) {
          claimedSteps.add(matched);
          continue;
        }
      }
    }
    if (record) out.push(record);
  }
  return out;
}

export function toolRecordKey(record: ToolRecord): string {
  if (typeof record !== "string" && record.step?.id) return `id:${record.step.id}`;
  const text = typeof record === "string" ? record : record.text;
  return `t:${toolSignature(text)}`;
}

/**
 * The steps a new tool chip owns: everything after the last step an earlier
 * chip of this turn already shows. Steps are stored in order, so the earlier
 * chips always cover a prefix. A key is matched at its first occurrence so a
 * command repeated in this chip is not mistaken for the earlier run.
 */
export function turnSegmentRecords(all: ToolRecord[], shownKeys: Iterable<string>): ToolRecord[] {
  const firstIndex = new Map<string, number>();
  all.forEach((record, index) => {
    const key = toolRecordKey(record);
    if (!firstIndex.has(key)) firstIndex.set(key, index);
  });
  let start = 0;
  for (const key of shownKeys) {
    const at = firstIndex.get(key);
    if (at !== undefined && at >= start) start = at + 1;
  }
  return all.slice(start);
}

const SAVED_OUTPUT_CHARS = 32_000;
const SAVED_LINES = 80;
const SAVED_LINE_CHARS = 400;

/** How many tool rows a turn shows before the reader scrolls. */
export const MAX_SHOWN_TOOL_ROWS = 40;
/** Upper bound on rows kept in the page while scrolling through a long turn. */
export const MAX_TOOL_DOM_ROWS = 80;

/** The previous page of steps when `firstIndex` is the first row on screen. */
export function earlierToolRange(firstIndex: number): { from: number; to: number } | null {
  if (firstIndex <= 0) return null;
  return { from: Math.max(0, firstIndex - MAX_SHOWN_TOOL_ROWS), to: firstIndex };
}

/** The next page of steps when `lastIndex` is the last row on screen. */
export function laterToolRange(lastIndex: number, count: number): { from: number; to: number } | null {
  const from = lastIndex + 1;
  if (from >= count) return null;
  return { from, to: Math.min(count, from + MAX_SHOWN_TOOL_ROWS) };
}

const PAGE_TEXT_CHARS = 4_000;
const PAGE_DIFF_LINES = 40;

function capText(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}\n…` : text;
}

/** The saved copy keeps the row and a taste of the output, not the whole log. */
function slim(step: ToolStep): ToolStep {
  const out = { ...step };
  if (out.output) out.output = capText(out.output, SAVED_OUTPUT_CHARS);
  if (out.command) out.command = capText(out.command, SAVED_OUTPUT_CHARS);
  if (out.query) out.query = capText(out.query, SAVED_OUTPUT_CHARS);
  if (out.added_lines) {
    out.added_lines = out.added_lines.slice(0, SAVED_LINES).map((line) => capText(line, SAVED_LINE_CHARS));
  }
  if (out.deleted_lines) {
    out.deleted_lines = out.deleted_lines.slice(0, SAVED_LINES).map((line) => capText(line, SAVED_LINE_CHARS));
  }
  return out;
}

/** What one row may hold in the page. The full step can be megabytes; the row cannot. */
export function clipStepForPage(step: ToolStep): ToolStep {
  const out = { ...step };
  if (out.output) out.output = capText(out.output, PAGE_TEXT_CHARS);
  if (out.command) out.command = capText(out.command, PAGE_TEXT_CHARS);
  if (out.query) out.query = capText(out.query, PAGE_TEXT_CHARS);
  if (out.added_lines) {
    out.added_lines = out.added_lines.slice(0, PAGE_DIFF_LINES).map((line) => capText(line, SAVED_LINE_CHARS));
  }
  if (out.deleted_lines) {
    out.deleted_lines = out.deleted_lines.slice(0, PAGE_DIFF_LINES).map((line) => capText(line, SAVED_LINE_CHARS));
  }
  return out;
}

/** Add raw output to this turn's saved log message, starting one if needed. */
export function appendRunLogMessage(messages: ChatMessage[], text: string, newId: () => string): void {
  const detail = text.trim();
  if (!detail) return;
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i];
    if (message?.role === "user") break;
    if (message?.role === "meta" && message.log) {
      if (runLogAlreadyHas(message.content, detail)) return;
      const next = `${message.content}\n${detail}`;
      message.content = next.length > SAVED_LOG_CHARS ? next.slice(next.length - SAVED_LOG_CHARS) : next;
      return;
    }
  }
  messages.push({ id: newId(), role: "meta", content: detail, at: Date.now(), log: true });
}

const SAVED_LOG_CHARS = 64_000;

/**
 * Put a step on this turn's tool messages. A result merges into the message with the
 * same id; a new step takes over the plain status message just saved for it.
 */
export function storeToolStep(
  messages: ChatMessage[],
  step: ToolStep,
  streamingAssistantId: string | null,
  newId: () => string,
): boolean {
  let userAt = -1;
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (messages[i]?.role === "user") {
      userAt = i;
      break;
    }
  }
  for (let i = messages.length - 1; i > userAt; i -= 1) {
    const message = messages[i];
    if (message?.role === "tool" && message.step?.id === step.id) {
      message.step = slim(mergeToolStep(message.step, step));
      message.content = toolStepText(message.step);
      // If a `Status(tool)` announcement landed right before this merge update
      // (e.g. `content_block_start` already created the step, then `assistant`
      // sent `Status` + `Tool`), drop that redundant bare status message.
      const tailAt = messages.length - 1;
      const tail = messages[tailAt];
      const checkAt = tail?.role === "assistant" && tail.id === streamingAssistantId ? tailAt - 1 : tailAt;
      const check = messages[checkAt];
      if (checkAt > i && check?.role === "tool" && !check.step) {
        const bareFace = toolFace(check.content);
        const stepFace = toolFace(message.step.name || step.name || "");
        if (!bareFace || !stepFace || bareFace === stepFace) {
          messages.splice(checkAt, 1);
        }
      }
      return true;
    }
  }
  if (!step.kind) return false;
  const stored = slim(step);
  const tailAt = messages.length - 1;
  const tail = messages[tailAt];
  const anchorEnd = tail?.role === "assistant" ? tailAt - 1 : tailAt;
  // Walk the current contiguous tool run so parallel `[Status(A), Status(B), Tool(A), Tool(B)]`
  // pairs each step with its matching status message instead of stranding the first one.
  let runStart = anchorEnd;
  while (runStart >= userAt + 1 && messages[runStart]?.role === "tool") {
    runStart -= 1;
  }
  const stepFace = toolFace(stored.name || "");
  let adoptAt = -1;
  for (let i = runStart + 1; i <= anchorEnd; i += 1) {
    const candidate = messages[i];
    if (candidate?.role !== "tool" || candidate.step) continue;
    if (stepFace && toolFace(candidate.content) === stepFace) {
      adoptAt = i;
      break;
    }
  }
  if (adoptAt < 0 && anchorEnd > userAt && messages[anchorEnd]?.role === "tool" && !messages[anchorEnd]?.step) {
    adoptAt = anchorEnd;
  }
  if (adoptAt >= 0) {
    const adopted = messages[adoptAt]!;
    adopted.step = stored;
    adopted.content = toolStepText(stored);
    return true;
  }
  const message: ChatMessage = {
    id: newId(),
    role: "tool",
    content: toolStepText(stored),
    at: Date.now(),
    step: stored,
  };
  if (tail?.role === "assistant" && tail.id === streamingAssistantId) {
    messages.splice(tailAt, 0, message);
  } else {
    messages.push(message);
  }
  return true;
}
