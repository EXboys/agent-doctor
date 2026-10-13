/**
 * Pointer drag reorder for skill rows.
 *
 * macOS WKWebView starts a native selection on mousedown unless the default
 * is prevented, and then delivers no move events until release. Prevent the
 * default on press, block dragstart, and track both pointer and mouse moves.
 */

const DRAG_THRESHOLD_PX = 4;
const SETTLE_MS = 160;

export type SkillDropPosition = "before" | "after";

type Slot = { el: HTMLElement; id: string; top: number; height: number };

export function attachSkillReorder(
  listEl: HTMLElement,
  handle: HTMLElement,
  skillId: string,
  onReorder: (from: string, to: string, position: SkillDropPosition) => void,
): void {
  handle.setAttribute("draggable", "false");
  handle.addEventListener("dragstart", (event) => event.preventDefault());

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

  const rows = (): HTMLElement[] => [
    ...listEl.querySelectorAll<HTMLElement>('.chat-res-row[data-kind="skill"]'),
  ];

  const startDrag = (): boolean => {
    dragged = handle.closest<HTMLElement>('.chat-res-row[data-kind="skill"]');
    if (!dragged) return false;
    const blocks = rows();
    draggedIndex = blocks.indexOf(dragged);
    if (draggedIndex < 0 || blocks.length < 2) return false;
    gap = parseFloat(getComputedStyle(listEl).rowGap) || 0;
    const outerHeight = (el: HTMLElement): number => el.getBoundingClientRect().height;
    const rect = dragged.getBoundingClientRect();
    draggedTop = rect.top;
    draggedHeight = outerHeight(dragged);
    others = blocks
      .filter((el) => el !== dragged)
      .map((el) => {
        const box = el.getBoundingClientRect();
        return { el, id: el.dataset.id ?? "", top: box.top, height: outerHeight(el) };
      });
    targetIndex = draggedIndex;
    dragged.classList.add("is-dragging");
    for (const slot of others) slot.el.classList.add("is-shifting");
    document.body.classList.add("chat-skill-dragging");
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
    dragged.style.transform = `translateY(${dy}px)`;
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
    document.body.classList.remove("chat-skill-dragging");
  };

  const onMove = (clientY: number, clientX: number): void => {
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
    onMove(event.clientY, event.clientX);
  };
  const handleMouseMove = (event: MouseEvent): void => {
    if (dragging) event.preventDefault();
    onMove(event.clientY, event.clientX);
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
    let position: SkillDropPosition = "before";
    if (moved) {
      if (targetIndex < others.length) {
        to = others[targetIndex]!.id;
      } else {
        to = others[others.length - 1]!.id;
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
      if (moved && to) onReorder(skillId, to, position);
    }, SETTLE_MS);
  }

  const press = (event: MouseEvent): void => {
    if (event.button !== 0 || pressed) return;
    event.preventDefault();
    event.stopPropagation();
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

  handle.addEventListener("pointerdown", press);
  handle.addEventListener("mousedown", press);
  handle.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
  });
}
