/** Space kept above and below the home window, in logical pixels. */
export const HOME_EDGE_MARGIN = 48;

/**
 * One height for every home tab.
 * Start from the tallest tab, then sit halfway to the screen so the
 * window is not cramped and not full-height. Never taller than Ask.
 */
export function dockedHomeHeight(content: number, screen: number, ask: number): number {
  const room = Math.max(0, screen - HOME_EDGE_MARGIN * 2);
  const upper = Math.min(room, ask > 0 ? ask : room);
  if (upper <= 0) return Math.max(0, Math.round(content));
  if (content >= upper) return Math.round(upper);
  const mid = (content + screen) / 2;
  return Math.round(Math.min(Math.max(mid, content), upper));
}
