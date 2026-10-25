/**
 * Page layout numbers for the home window and the Ask columns.
 * Native window insets stay in `src-tauri/src/windows.rs` — they place the OS window.
 * `chat.css` repeats the column widths for the first paint; `tests/layout-rules.test.ts` checks that copy.
 */
export const layout = {
  /** Space kept above and below the home window, in logical pixels. */
  homeEdgeMargin: 48,
  /** Project list. Windows is wider because its UI font runs wider. */
  chatSide: 250,
  chatSideWindows: 272,
  /** Chat column when files are open, and the narrowest it may be dragged. */
  chatSplit: 410,
  splitter: 5,
  tree: 280,
  treeMin: 96,
  sideMin: 200,
  sideMax: 420,
  filesMin: 280,
  /** File preview keeps this much so the folder list cannot cover it. */
  previewMin: 160,
} as const;

export function chatSideDefault(os: string | undefined): number {
  return os === "windows" ? layout.chatSideWindows : layout.chatSide;
}

/**
 * One height for every home tab.
 * Start from the tallest tab, then sit halfway to the screen so the
 * window is not cramped and not full-height. Never taller than Ask.
 */
export function dockedHomeHeight(content: number, screen: number, ask: number): number {
  const room = Math.max(0, screen - layout.homeEdgeMargin * 2);
  const upper = Math.min(room, ask > 0 ? ask : room);
  if (upper <= 0) return Math.max(0, Math.round(content));
  if (content >= upper) return Math.round(upper);
  const mid = (content + screen) / 2;
  return Math.round(Math.min(Math.max(mid, content), upper));
}
