const mirrors = new Map<HTMLTextAreaElement, HTMLTextAreaElement>();

function pruneMirrors(): void {
  for (const [field, mirror] of mirrors) {
    if (field.isConnected) continue;
    mirror.remove();
    mirrors.delete(field);
  }
}

/** Content height, measured on a throwaway field so the focused one is never collapsed. */
function measureTextarea(field: HTMLTextAreaElement, width: number): number {
  pruneMirrors();
  let mirror = mirrors.get(field);
  if (!mirror) {
    mirror = document.createElement("textarea");
    mirror.setAttribute("aria-hidden", "true");
    mirror.tabIndex = -1;
    mirror.readOnly = true;
    const style = mirror.style;
    style.position = "fixed";
    style.left = "-10000px";
    style.top = "0";
    style.visibility = "hidden";
    style.pointerEvents = "none";
    style.height = "auto";
    style.minHeight = "0";
    style.maxHeight = "none";
    style.overflow = "hidden";
    style.zIndex = "-1";
    document.body.appendChild(mirror);
    mirrors.set(field, mirror);
  }
  mirror.className = field.className;
  mirror.style.width = `${width}px`;
  if (mirror.value !== field.value) mirror.value = field.value;
  return mirror.scrollHeight;
}

/**
 * Grow or shrink a textarea to its text.
 * Never set the live field to `height: auto` — that relayout drops the caret,
 * so an IME's composing characters jump or vanish on every key.
 */
export function fitTextarea(field: HTMLTextAreaElement, maxPx?: number): number {
  const styles = getComputedStyle(field);
  const minHeight = Number.parseFloat(styles.minHeight);
  const maxHeight = maxPx ?? Number.parseFloat(styles.maxHeight);
  const width = field.clientWidth;
  const current = field.offsetHeight;
  if (width <= 0) return current;

  const measured = measureTextarea(field, width);
  let next = measured;
  if (Number.isFinite(minHeight)) next = Math.max(next, minHeight);
  const capped = Number.isFinite(maxHeight) && measured > maxHeight;
  if (capped) next = maxHeight;
  next = Math.ceil(next);

  // While the input method is painting marked text, only grow. Shrinking moves the caret.
  if (field.dataset.composing === "1" && next <= current + 1) return current;

  if (Math.abs(next - current) > 1) field.style.height = `${next}px`;
  const overflow = capped ? "auto" : "hidden";
  if (styles.overflowY !== overflow) field.style.overflowY = overflow;
  return Math.abs(next - current) > 1 ? next : current;
}
