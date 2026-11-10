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

/** The home window never gets shorter than this: the agent list needs the room. */
export const homeMinHeight = 480;

/**
 * How much empty height the home may leave before the big card is worth
 * drawing, and how little it may leave before that card has to go.
 *
 * Two numbers, not one: a single threshold flips the card on and off while the
 * user drags a window edge across it. `heroShouldShow` keeps the current state
 * inside the band between them.
 */
export const heroRoom = {
  /** Draw the big card once this much height is going spare. */
  show: 168,
  /** Drop it again only once the page is this tight. */
  hide: 104,
};

/**
 * Whether the home paints the big agent card beside the tiles.
 *
 * `spare` is the height left over after the tiles are laid out. `shown` is
 * what the page is doing now, so a value inside the band keeps its answer and
 * the card does not flicker.
 */
export function heroShouldShow(spare: number, shown: boolean): boolean {
  if (shown) return spare > heroRoom.hide;
  return spare >= heroRoom.show;
}

/**
 * Height the agent panel has to spare once the tiles are laid out.
 *
 * Measured with the big card taken back out: a card that grows to fill the
 * panel would otherwise measure its own empty space and hide itself, then see
 * room again the moment it left — the card's own height must never reach this
 * number.
 *
 * `card` is the natural height of the card as painted (children, not the
 * stretched box), `hero` the block inside it that may go away.
 */
export function heroSpare(input: {
  /** Height the agent panel has to give. */
  panel: number;
  /** Natural height of the card, big block included. */
  card: number;
  /** Height of the big block; 0 when the card paints none. */
  hero: number;
  /** Row gap between the block and the rest of the card. */
  gap: number;
}): number {
  const tiles = input.hero > 0 ? input.card - input.hero - input.gap : input.card;
  return Math.round(input.panel - tiles);
}


