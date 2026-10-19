const SIDE_KEY = "agent-doctor-ask-side-w";
const CHAT_SPLIT_KEY = "agent-doctor-ask-chat-split-w";
const TREE_KEY = "agent-doctor-ask-files-tree-w";
const SIDE_DEFAULT = 250;
const CHAT_SPLIT_DEFAULT = 410;
const TREE_DEFAULT = 280;

const SIDE_MIN = 200;
const SIDE_MAX = 420;
const FILES_MIN = 280;
const TREE_MIN = 96;
/** File preview keeps this much so the folder list cannot cover it. */
const PREVIEW_MIN = 160;

function chatSplitMinPx(): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue("--chat-split-chat-min").trim();
  const n = Number.parseInt(raw, 10);
  return Number.isFinite(n) && n > 0 ? n : 410;
}

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

function currentChatSplitPx(): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue("--chat-split-chat-w").trim();
  const n = Number.parseInt(raw, 10);
  return Number.isFinite(n) ? n : readPx(CHAT_SPLIT_KEY, CHAT_SPLIT_DEFAULT);
}

export function applyLayoutVars(): void {
  const root = document.documentElement;
  root.style.setProperty("--chat-side-w", `${readPx(SIDE_KEY, SIDE_DEFAULT)}px`);
  const chatW = Math.max(readPx(CHAT_SPLIT_KEY, CHAT_SPLIT_DEFAULT), chatSplitMinPx());
  root.style.setProperty("--chat-split-chat-w", `${chatW}px`);
  fitFilesTree();
}

/** Folder list width. A saved drag wins; otherwise the list takes half the files area. */
function preferredTreePx(panelW: number): number {
  const raw = localStorage.getItem(TREE_KEY);
  const saved = raw ? Number.parseInt(raw, 10) : NaN;
  if (Number.isFinite(saved) && saved > 0) return saved;
  return Math.max(TREE_DEFAULT, Math.round(panelW * 0.5));
}

/** Keep the folder list inside the files area. A narrow panel shrinks it; the saved width stays. */
function fitFilesTree(): void {
  const panel = document.querySelector<HTMLElement>("#chat-files-panel");
  const handle = document.querySelector<HTMLElement>("#chat-splitter-files-tree");
  if (!panel || handle?.classList.contains("is-dragging")) return;
  if (panel.classList.contains("is-tree-collapsed")) return;
  const panelW = panel.getBoundingClientRect().width;
  if (panelW < 40) return;
  const splitW = handle?.getBoundingClientRect().width ?? 0;
  const max = Math.max(TREE_MIN, panelW - PREVIEW_MIN - splitW);
  const next = clamp(preferredTreePx(panelW), TREE_MIN, max);
  panel.style.setProperty("--files-tree-w-user", `${Math.round(next)}px`);
}

function clamp(n: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, n));
}

type DragCtx = { startX: number; startW: number };

function bindDrag(
  handle: HTMLElement,
  onMove: (clientX: number, ctx: DragCtx) => void,
  onEnd: () => void,
  canStart?: () => boolean,
): void {
  // WKWebView treats a press on this line as a text selection and then
  // sends no move events. Cancel that, and follow the mouse on the window.
  handle.draggable = false;
  handle.addEventListener("dragstart", (event) => event.preventDefault());

  let pressed = false;
  const ctx: DragCtx = { startX: 0, startW: 0 };

  const move = (event: MouseEvent): void => {
    if (!pressed) return;
    event.preventDefault();
    onMove(event.clientX, ctx);
  };
  const end = (): void => {
    if (!pressed) return;
    pressed = false;
    window.removeEventListener("pointermove", move, true);
    window.removeEventListener("mousemove", move, true);
    window.removeEventListener("pointerup", end, true);
    window.removeEventListener("mouseup", end, true);
    window.removeEventListener("pointercancel", end, true);
    window.removeEventListener("blur", end);
    handle.classList.remove("is-dragging");
    document.body.classList.remove("chat-is-resizing");
    onEnd();
  };
  const press = (event: MouseEvent): void => {
    if (event.button !== 0 || pressed) return;
    if (canStart && !canStart()) return;
    event.preventDefault();
    pressed = true;
    ctx.startX = event.clientX;
    ctx.startW = 0;
    handle.classList.add("is-dragging");
    document.body.classList.add("chat-is-resizing");
    if (event instanceof PointerEvent) {
      try {
        handle.setPointerCapture(event.pointerId);
      } catch {
        /* Mouse moves on the window cover this. */
      }
    }
    window.addEventListener("pointermove", move, true);
    window.addEventListener("mousemove", move, true);
    window.addEventListener("pointerup", end, true);
    window.addEventListener("mouseup", end, true);
    window.addEventListener("pointercancel", end, true);
    window.addEventListener("blur", end);
  };

  handle.addEventListener("pointerdown", press);
  handle.addEventListener("mousedown", press);
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

  const filesTree = document.querySelector<HTMLElement>("#chat-splitter-files-tree");
  const filesPanel = document.querySelector<HTMLElement>("#chat-files-panel");
  if (filesTree && filesPanel) {
    bindDrag(
      filesTree,
      (clientX, ctx) => {
        if (!ctx.startW) {
          const tree = filesPanel.querySelector(".chat-files-tree-col");
          ctx.startW = tree?.getBoundingClientRect().width || readPx(TREE_KEY, TREE_DEFAULT);
        }
        const panelW = filesPanel.getBoundingClientRect().width;
        const splitW = filesTree.getBoundingClientRect().width;
        const max = Math.max(TREE_MIN, panelW - PREVIEW_MIN - splitW);
        const next = clamp(ctx.startW + (ctx.startX - clientX), TREE_MIN, max);
        filesPanel.style.setProperty("--files-tree-w-user", `${Math.round(next)}px`);
      },
      () => {
        const tree = filesPanel.querySelector(".chat-files-tree-col");
        const width = tree?.getBoundingClientRect().width;
        if (width && width > 0) savePx(TREE_KEY, width);
      },
      () => !filesPanel.classList.contains("is-tree-collapsed"),
    );
    if (typeof ResizeObserver !== "undefined") {
      new ResizeObserver(() => fitFilesTree()).observe(filesPanel);
    }
  }

  if (chatFiles && main) {
    bindDrag(
      chatFiles,
      (clientX, ctx) => {
        if (!ctx.startW) ctx.startW = currentChatSplitPx();
        const mainW = main.getBoundingClientRect().width;
        const splitterW = chatFiles.getBoundingClientRect().width || 5;
        const max = Math.max(chatSplitMinPx(), mainW - FILES_MIN - splitterW);
        const delta = clientX - ctx.startX;
        const next = clamp(ctx.startW + delta, chatSplitMinPx(), max);
        document.documentElement.style.setProperty("--chat-split-chat-w", `${Math.round(next)}px`);
      },
      () => savePx(CHAT_SPLIT_KEY, currentChatSplitPx()),
    );
  }
}

export function syncChatFilesSplitter(visible: boolean): void {
  const el = document.querySelector<HTMLElement>("#chat-splitter-chat-files");
  if (!el) return;
  el.hidden = !visible;
  el.setAttribute("aria-hidden", visible ? "false" : "true");
}
