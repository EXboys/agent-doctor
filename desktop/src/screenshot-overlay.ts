import { getCurrentWindow } from "@tauri-apps/api/window";
import { cancelRegionCapture, completeRegionCapture } from "./ipc";

let dragging = false;
let done = false;
let start: { x: number; y: number } | null = null;
const rectEl = document.querySelector<HTMLDivElement>("#shot-rect")!;
const hintEl = document.querySelector<HTMLDivElement>("#shot-hint")!;

function finish(x: number, y: number, width: number, height: number): void {
  if (done) return;
  done = true;
  document.body.style.cursor = "default";
  hintEl.hidden = true;
  drawRect(x, y, width, height);
  completeRegionCapture({ x, y, width, height }).catch(() => {
    cancelRegionCapture().catch(() => {});
  });
}

function cancel(): void {
  if (done) return;
  done = true;
  cancelRegionCapture().catch(() => {});
}

function drawRect(x: number, y: number, width: number, height: number): void {
  rectEl.hidden = false;
  rectEl.style.left = `${x}px`;
  rectEl.style.top = `${y}px`;
  rectEl.style.width = `${width}px`;
  rectEl.style.height = `${height}px`;
}

window.addEventListener("mousedown", (event) => {
  if (done || event.button !== 0) return;
  dragging = true;
  start = { x: event.clientX, y: event.clientY };
  drawRect(start.x, start.y, 0, 0);
});

window.addEventListener("mousemove", (event) => {
  if (!dragging || !start || done) return;
  const x = Math.min(start.x, event.clientX);
  const y = Math.min(start.y, event.clientY);
  const width = Math.abs(event.clientX - start.x);
  const height = Math.abs(event.clientY - start.y);
  drawRect(x, y, width, height);
  hintEl.textContent = `${Math.round(width)} × ${Math.round(height)}`;
});

window.addEventListener("mouseup", (event) => {
  if (!dragging || done || !start) return;
  dragging = false;
  const width = Math.abs(event.clientX - start.x);
  const height = Math.abs(event.clientY - start.y);
  if (width < 4 || height < 4) {
    // A plain click takes the whole screen, so the old one-shot feel survives.
    finish(0, 0, window.innerWidth, window.innerHeight);
    return;
  }
  finish(
    Math.min(start.x, event.clientX),
    Math.min(start.y, event.clientY),
    width,
    height,
  );
});

window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    cancel();
  } else if (event.key === "Enter" && !done) {
    finish(0, 0, window.innerWidth, window.innerHeight);
  }
});

window.addEventListener("contextmenu", (event) => {
  event.preventDefault();
  cancel();
});

// The picker is a live see-through dim over whatever is on screen right now,
// so switching desktops mid-pick selects from the space you landed on.

// Built hidden so it never flashes white. Show, then grab key focus — without
// it the first drag only activates the window and Esc never arrives, leaving
// a dead mask over the whole screen.
async function reveal(): Promise<void> {
  const current = getCurrentWindow();
  await current.show();
  await current.setFocus();
}
void reveal();
