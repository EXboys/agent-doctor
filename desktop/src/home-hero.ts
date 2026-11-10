/**
 * The big agent card on the Windows home.
 *
 * A single agent paints one tall card; two or more paint tiles and a shorter
 * card. On Windows the window is as tall as the work area in both cases, so
 * the multi-agent home can keep the big card *beside* the tiles whenever the
 * panel has height going spare — and drop it when the panel is full.
 *
 * The decision runs on measured height, never on a guess about screen size:
 * a 150% display and a 4K one give the panel very different room.
 */
import { heroShouldShow, heroSpare } from "./layout-rules";

/** What the page is showing now; the hysteresis band needs it. */
let shown = false;

function isWindows(): boolean {
  return document.documentElement.dataset.os === "windows";
}

/** Natural height of the card's contents — not the box, which stretches. */
function contentHeight(card: HTMLElement): number {
  const style = getComputedStyle(card);
  const gap = Number.parseFloat(style.rowGap) || 0;
  let total = (Number.parseFloat(style.paddingTop) || 0) + (Number.parseFloat(style.paddingBottom) || 0);
  let first = true;
  for (const child of Array.from(card.children)) {
    const el = child as HTMLElement;
    if (el.hidden || el.offsetParent === null) continue;
    total += el.getBoundingClientRect().height + (first ? 0 : gap);
    first = false;
  }
  return total;
}

/**
 * Paint or drop the big card. Call it after the home re-renders or resizes;
 * it reads layout and toggles one class, nothing else.
 */
export function syncHomeHero(): void {
  if (!isWindows()) return;
  const panels = document.querySelector<HTMLElement>(".runtime-panels");
  const card = panels?.querySelector<HTMLElement>("article.runtime");
  if (!panels || !card) return;
  // A solo home always paints its big card; there are no tiles to compare to.
  if (document.querySelector(".runtime-section.is-solo")) return;

  const hero = card.querySelector<HTMLElement>(".runtime-solo-hero");
  const gap = Number.parseFloat(getComputedStyle(card).rowGap) || 0;
  const spare = heroSpare({
    panel: panels.clientHeight,
    card: contentHeight(card),
    hero: hero ? hero.getBoundingClientRect().height : 0,
    gap,
  });
  shown = heroShouldShow(spare, shown);
  card.classList.toggle("has-hero", shown);
}

/** A fresh home has no measurement yet; the next sync decides from scratch. */
export function resetHomeHero(): void {
  shown = false;
}
