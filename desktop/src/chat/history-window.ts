import { isLogMessage } from "./format";

/** Recent rows painted before the first frame. Older rows follow in short slices. */
export const HISTORY_FIRST_SCREEN = 24;
/** How many older rows to mount per slice. */
export const HISTORY_BATCH = 8;
/** Stop the first screen early once this much text is already included. */
const FIRST_SCREEN_CHARS = 24_000;

type HistoryRow = { role: string; content?: string; log?: boolean };

function asLogRow(row: HistoryRow): { role: string; content: string; log?: boolean } {
  return { role: row.role, content: row.content ?? "", log: row.log };
}

/**
 * True when this row belongs with the row above: one tool/thinking run,
 * one permission run, or one collapsed runtime log.
 */
export function messageContinuesRun(messages: readonly HistoryRow[], index: number): boolean {
  if (index <= 0) return false;
  const current = messages[index];
  const previous = messages[index - 1];
  if (!current || !previous) return false;
  const role = current.role;
  const prev = previous.role;
  if ((role === "tool" || role === "thinking") && (prev === "tool" || prev === "thinking")) return true;
  if (role === "permission" && prev === "permission") return true;
  return isLogMessage(asLogRow(current)) && isLogMessage(asLogRow(previous));
}

/**
 * Index of the first row to paint immediately.
 * 0 means the whole chat is small enough to paint in one pass.
 * The cut never lands in the middle of a tool, permission, or log run.
 */
export function historyRenderCut(messages: readonly HistoryRow[]): number {
  if (messages.length <= HISTORY_FIRST_SCREEN) return 0;
  let index = messages.length;
  let rows = 0;
  let chars = 0;
  while (index > 0 && rows < HISTORY_FIRST_SCREEN && chars < FIRST_SCREEN_CHARS) {
    index -= 1;
    rows += 1;
    chars += messages[index]?.content?.length ?? 0;
  }
  while (index > 0 && messageContinuesRun(messages, index)) index -= 1;
  // A short older section is cheaper to paint now than to schedule.
  if (index <= HISTORY_BATCH) return 0;
  return index;
}
