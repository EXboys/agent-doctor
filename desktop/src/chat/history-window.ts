import { isLogMessage } from "./format";

/** Recent rows painted before the first frame. Older rows follow in short slices. */
export const HISTORY_FIRST_SCREEN = 24;
/** How many older rows to mount per slice. */
export const HISTORY_BATCH = 8;
/** A tool/permission run snaps its slice boundary up to the run's start; one
 * agent turn can hold thousands of rows, so a slice never absorbs more than
 * this many. The rest of the run fills the following slices - and each slice
 * stays a slice, which is what keeps a first paint from rendering minutes of
 * transcript in one go. */
export const HISTORY_RUN_SLICE = 48;
/** Stop the first screen early once this much text is already included. */
const FIRST_SCREEN_CHARS = 24_000;
/** Lines of conversation to pull into the first screen when it holds none. */
const FIRST_SCREEN_CONVERSATION = 4;
/** And never further back than this, even when it finds none. */
const FIRST_SCREEN_EXTEND_CAP = 400;

type HistoryRow = { role: string; content?: string; log?: boolean };

function asLogRow(row: HistoryRow): { role: string; content: string; log?: boolean } {
  return { role: row.role, content: row.content ?? "", log: row.log };
}

function isConversationRow(row: HistoryRow | undefined): boolean {
  return row?.role === "user" || row?.role === "assistant";
}

/** True when the rows painted first contain something the user actually said. */
function firstScreenHasConversation(messages: readonly HistoryRow[], index: number): boolean {
  for (let cursor = index; cursor < messages.length; cursor += 1) {
    if (isConversationRow(messages[cursor])) return true;
  }
  return false;
}

/**
 * Pull the last lines of conversation into the first screen. The newest rows
 * of a long turn are all tool output and those rows are painted first, so
 * without this the user's own words wait for the older rows to be pumped in —
 * and that pump does not run while the window is hidden.
 */
function extendToConversation(messages: readonly HistoryRow[], index: number): number {
  let seen = 0;
  let cursor = index;
  // A transcript tail with no conversation rows at all must not walk this
  // loop to the very top - bound the search like every other walk here.
  let walked = 0;
  while (cursor > 0 && seen < FIRST_SCREEN_CONVERSATION && walked < FIRST_SCREEN_EXTEND_CAP) {
    if (isConversationRow(messages[cursor - 1])) seen += 1;
    cursor -= 1;
    walked += 1;
  }
  return cursor;
}

/** Extend a boundary up through the run it belongs to, at most `cap` rows:
 * `messageContinuesRun` without a cap snapped a slice boundary back to the
 * start of a thousands-of-rows run and rendered it all at once.
 */
export function snapRunBoundary(
  messages: readonly HistoryRow[],
  index: number,
  cap: number,
): number {
  let start = index;
  let taken = 0;
  while (start > 0 && taken < cap && messageContinuesRun(messages, start)) {
    start -= 1;
    taken += 1;
  }
  return start;
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
  index = snapRunBoundary(messages, index, HISTORY_RUN_SLICE);
  if (!firstScreenHasConversation(messages, index)) {
    const extended = extendToConversation(messages, index);
    if (extended < index) {
      index = extended;
      index = snapRunBoundary(messages, index, HISTORY_RUN_SLICE);
    }
  }
  // A short older section is cheaper to paint now than to schedule.
  if (index <= HISTORY_BATCH) return 0;
  return index;
}
