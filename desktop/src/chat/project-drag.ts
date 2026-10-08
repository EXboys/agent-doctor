/**
 * Pointer drag reorder for project blocks: the block follows the mouse and the
 * other projects slide aside live, then the new order is committed on release.
 *
 * macOS WKWebView starts a native drag/selection on mousedown unless the default
 * is prevented, and then delivers no move events until release. Prevent the
 * default on press, block dragstart, and track both pointer and mouse moves.
 */

const DRAG_THRESHOLD_PX = 4;
const SETTLE_MS = 160;

let suppressProjectHeadClick = false;

/** Call from fold / head click handlers right after a drag-sort. */
export function consumeProjectDragClick(): boolean {
  if (!suppressProjectHeadClick) return false;
  suppressProjectHeadClick = false;
  return true;
}

export type ProjectDropPosition = "before" | "after";

type Slot = { el: HTMLElement; name: string; top: number; height: number };

export function attachProjectReorderPointer(
  listEl: HTMLElement,
  head: HTMLElement,
  projectName: string,
  onReorder: (from: string, to: string, position: ProjectDropPosition) => void,
): void {
  head.classList.add("chat-session-group-drag-handle");
  head.setAttribute("draggable", "false");
  head.addEventListener("dragstart", (event) => event.preventDefault());

  let pressed = false;
  let dragging = false;
  let startX = 0;
  let startY = 0;
  let dragged: HTMLElement | null = null;
  let draggedIndex = 0;
  let draggedTop = 0;
  let draggedHeight = 0;
  let gap = 0;
  let others: Slot[] = [];
  let targetIndex = 0;

  const startDrag = (): boolean => {
    dragged = head.closest<HTMLElement>(".chat-session-group");
    if (!dragged) return false;
    const blocks = [
      ...listEl.querySelectorAll<HTMLElement>(".chat-session-group[data-project-name]"),
    ];
    draggedIndex = blocks.indexOf(dragged);
    if (draggedIndex < 0 || blocks.length < 2) return false;
    gap = parseFloat(getComputedStyle(listEl).rowGap) || 0;
    const outerHeight = (el: HTMLElement): number => {
      const box = el.getBoundingClientRect();
      return box.height + (parseFloat(getComputedStyle(el).marginBottom) || 0);
    };
    const rect = dragged.getBoundingClientRect();
    draggedTop = rect.top;
    draggedHeight = outerHeight(dragged);
    others = blocks
      .filter((el) => el !== dragged)
      .map((el) => {
        const box = el.getBoundingClientRect();
        return { el, name: el.dataset.projectName ?? "", top: box.top, height: outerHeight(el) };
      });
    targetIndex = draggedIndex;
    dragged.classList.add("is-dragging");
    for (const slot of others) slot.el.classList.add("is-shifting");
    document.body.classList.add("chat-project-dragging");
    return true;
  };

  const layout = (dy: number): void => {
    if (!dragged) return;
    const top = draggedTop + dy;
    const bottom = top + draggedHeight;
    let next = 0;
    others.forEach((slot, i) => {
      const mid = slot.top + slot.height / 2;
      const wasAbove = i < draggedIndex;
      if (wasAbove ? top > mid : bottom > mid) next += 1;
    });
    targetIndex = next;
    const shift = draggedHeight + gap;
    others.forEach((slot, i) => {
      const original = i < draggedIndex ? i : i + 1;
      let offset = 0;
      if (original > draggedIndex && i < targetIndex) offset = -shift;
      else if (original < draggedIndex && i >= targetIndex) offset = shift;
      slot.el.style.transform = offset ? `translateY(${offset}px)` : "";
    });
    dragged.style.transform = `translateY(${dy}px) scale(1.02)`;
  };

  const finalOffset = (): number => {
    let offset = 0;
    others.forEach((slot, i) => {
      const original = i < draggedIndex ? i : i + 1;
      if (original > draggedIndex && i < targetIndex) offset += slot.height + gap;
      else if (original < draggedIndex && i >= targetIndex) offset -= slot.height + gap;
    });
    return offset;
  };

  const resetStyles = (): void => {
    for (const slot of others) {
      slot.el.classList.remove("is-shifting");
      slot.el.style.transform = "";
    }
    if (dragged) {
      dragged.classList.remove("is-dragging", "is-settling");
      dragged.style.transform = "";
    }
    document.body.classList.remove("chat-project-dragging");
  };

  const onMove = (clientX: number, clientY: number): void => {
    if (!pressed) return;
    if (!dragging) {
      if (
        Math.abs(clientX - startX) < DRAG_THRESHOLD_PX &&
        Math.abs(clientY - startY) < DRAG_THRESHOLD_PX
      ) {
        return;
      }
      if (!startDrag()) {
        pressed = false;
        detach();
        return;
      }
      dragging = true;
    }
    layout(clientY - startY);
  };

  const handlePointerMove = (event: PointerEvent): void => {
    if (dragging) event.preventDefault();
    onMove(event.clientX, event.clientY);
  };
  const handleMouseMove = (event: MouseEvent): void => {
    if (dragging) event.preventDefault();
    onMove(event.clientX, event.clientY);
  };

  function detach(): void {
    window.removeEventListener("pointermove", handlePointerMove, true);
    window.removeEventListener("mousemove", handleMouseMove, true);
    window.removeEventListener("pointerup", handleUp, true);
    window.removeEventListener("mouseup", handleUp, true);
    window.removeEventListener("pointercancel", handleUp, true);
    window.removeEventListener("blur", handleUp);
  }

  function handleUp(): void {
    if (!pressed) return;
    pressed = false;
    detach();
    if (!dragging || !dragged) {
      dragging = false;
      return;
    }
    dragging = false;

    const moved = targetIndex !== draggedIndex;
    let to = "";
    let position: ProjectDropPosition = "before";
    if (moved) {
      if (targetIndex < others.length) {
        to = others[targetIndex]!.name;
      } else {
        to = others[others.length - 1]!.name;
        position = "after";
      }
    }

    const el = dragged;
    el.classList.add("is-settling");
    el.style.transform = `translateY(${finalOffset()}px)`;
    window.setTimeout(() => {
      resetStyles();
      dragged = null;
      others = [];
      if (moved && to) {
        suppressProjectHeadClick = true;
        onReorder(projectName, to, position);
      }
    }, SETTLE_MS);
  }

  const press = (event: MouseEvent): void => {
    if (event.button !== 0 || pressed) return;
    const target = event.target;
    if (target instanceof Element && target.closest("button")) return;
    event.preventDefault();

    pressed = true;
    dragging = false;
    startX = event.clientX;
    startY = event.clientY;

    window.addEventListener("pointermove", handlePointerMove, true);
    window.addEventListener("mousemove", handleMouseMove, true);
    window.addEventListener("pointerup", handleUp, true);
    window.addEventListener("mouseup", handleUp, true);
    window.addEventListener("pointercancel", handleUp, true);
    window.addEventListener("blur", handleUp);
  };

  head.addEventListener("pointerdown", press);
  head.addEventListener("mousedown", press);
}
