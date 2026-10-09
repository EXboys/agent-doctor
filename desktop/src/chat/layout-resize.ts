const SIDE_KEY = "agent-doctor-ask-side-w";
const FILES_KEY = "agent-doctor-ask-files-w";
const SIDE_DEFAULT = 250;
const FILES_DEFAULT = 400;

const SIDE_MIN = 200;
const SIDE_MAX = 420;
const FILES_MIN = 280;

function readPx(key: string, fallback: number): number {
  const raw = localStorage.getItem(key);
  const n = raw ? Number.parseInt(raw, 10) : NaN;
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

function savePx(key: string, value: number): void {
  localStorage.setItem(key, String(Math.round(value)));
}

function currentSidePx(): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue("--chat-side-w").trim();
  const n = Number.parseInt(raw, 10);
  return Number.isFinite(n) ? n : readPx(SIDE_KEY, SIDE_DEFAULT);
}

function currentFilesPx(): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue("--chat-files-panel-w").trim();
  const n = Number.parseInt(raw, 10);
  return Number.isFinite(n) ? n : readPx(FILES_KEY, FILES_DEFAULT);
}

export function applyLayoutVars(): void {
  const root = document.documentElement;
  root.style.setProperty("--chat-side-w", `${readPx(SIDE_KEY, SIDE_DEFAULT)}px`);
  root.style.setProperty("--chat-files-panel-w", `${readPx(FILES_KEY, FILES_DEFAULT)}px`);
}

function clamp(n: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, n));
}

type DragCtx = { startX: number; startW: number };

function bindDrag(
  handle: HTMLElement,
  onMove: (clientX: number, ctx: DragCtx) => void,
  onEnd: () => void,
): void {
  handle.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    event.preventDefault();
    handle.setPointerCapture(event.pointerId);
    handle.classList.add("is-dragging");
    document.body.classList.add("chat-is-resizing");
    const ctx: DragCtx = { startX: event.clientX, startW: 0 };

    const move = (e: PointerEvent) => {
      if (e.pointerId !== event.pointerId) return;
      onMove(e.clientX, ctx);
    };
    const up = (e: PointerEvent) => {
      if (e.pointerId !== event.pointerId) return;
      handle.releasePointerCapture(event.pointerId);
      handle.classList.remove("is-dragging");
      document.body.classList.remove("chat-is-resizing");
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      handle.removeEventListener("pointercancel", up);
      onEnd();
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
    handle.addEventListener("pointercancel", up);
  });
}

/** 对话栏 | 聊天栏 | 文件 — 中间两条竖线可拖拽。 */
export function bindLayoutResize(): void {
  applyLayoutVars();

  const sessionsChat = document.querySelector<HTMLElement>("#chat-splitter-sessions");
  const chatFiles = document.querySelector<HTMLElement>("#chat-splitter-chat-files");
  const main = document.querySelector<HTMLElement>("#chat-main");

  if (sessionsChat) {
    bindDrag(
      sessionsChat,
      (clientX, ctx) => {
        if (!ctx.startW) ctx.startW = currentSidePx();
        const delta = clientX - ctx.startX;
        const next = clamp(ctx.startW + delta, SIDE_MIN, SIDE_MAX);
        document.documentElement.style.setProperty("--chat-side-w", `${next}px`);
      },
      () => savePx(SIDE_KEY, currentSidePx()),
    );
  }

  if (chatFiles && main) {
    bindDrag(
      chatFiles,
      (clientX, ctx) => {
        if (!ctx.startW) ctx.startW = currentFilesPx();
        const mainW = main.getBoundingClientRect().width;
        const max = Math.max(FILES_MIN, mainW - 280);
        const delta = ctx.startX - clientX;
        const next = clamp(ctx.startW + delta, FILES_MIN, max);
        document.documentElement.style.setProperty("--chat-files-panel-w", `${next}px`);
      },
      () => savePx(FILES_KEY, currentFilesPx()),
    );
  }
}

export function syncChatFilesSplitter(visible: boolean): void {
  const el = document.querySelector<HTMLElement>("#chat-splitter-chat-files");
  if (!el) return;
  el.hidden = !visible;
  el.setAttribute("aria-hidden", visible ? "false" : "true");
}
