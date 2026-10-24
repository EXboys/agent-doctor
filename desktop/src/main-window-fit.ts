import { listen } from "@tauri-apps/api/event";
import { resizeMainWindow } from "./ipc";

/** Matches `MAIN_WINDOW_HEIGHT` / `MAIN_WINDOW_MIN_HEIGHT` in windows.rs (logical px). */
const MAIN_HOME_MAX = 800;
const MAIN_HOME_MIN = 480;

let docked = false;
let fitTimer: ReturnType<typeof setTimeout> | undefined;
let observer: ResizeObserver | undefined;

function clearWidgetInlineSize(widget: HTMLElement): void {
  widget.style.height = "";
  widget.style.maxHeight = "";
}

function measureWidgetHeight(widget: HTMLElement): number {
  const prevHeight = widget.style.height;
  const prevMax = widget.style.maxHeight;
  widget.style.height = "auto";
  widget.style.maxHeight = "none";
  const natural = Math.ceil(widget.getBoundingClientRect().height);
  widget.style.height = prevHeight;
  widget.style.maxHeight = prevMax;
  return natural;
}

export async function fitMainWindowToContent(): Promise<void> {
  if (!docked) return;
  const widget = document.querySelector<HTMLElement>(".widget");
  if (!widget) return;

  document.documentElement.classList.add("is-main-docked");
  const natural = measureWidgetHeight(widget);
  const target = Math.min(MAIN_HOME_MAX, Math.max(MAIN_HOME_MIN, natural));

  if (natural > MAIN_HOME_MAX) {
    widget.dataset.fit = "tall";
    widget.style.height = "100%";
    widget.style.maxHeight = "100%";
  } else {
    widget.dataset.fit = "compact";
    widget.style.height = "auto";
    widget.style.maxHeight = "none";
  }

  try {
    await resizeMainWindow({ height: target });
  } catch {
    // Window may be mid-layout; a later observer tick will retry.
  }
}

export function scheduleMainWindowFit(): void {
  if (!docked) return;
  if (fitTimer !== undefined) clearTimeout(fitTimer);
  fitTimer = setTimeout(() => {
    fitTimer = undefined;
    void fitMainWindowToContent();
  }, 60);
}

export function initMainWindowFit(): void {
  const widget = document.querySelector<HTMLElement>(".widget");
  if (widget && typeof ResizeObserver !== "undefined") {
    observer = new ResizeObserver(() => scheduleMainWindowFit());
    observer.observe(widget);
  }

  void listen<{ docked?: boolean }>("main-dock-layout", (event) => {
    docked = Boolean(event.payload?.docked);
    if (docked) {
      scheduleMainWindowFit();
      return;
    }
    document.documentElement.classList.remove("is-main-docked");
    const el = document.querySelector<HTMLElement>(".widget");
    if (el) {
      clearWidgetInlineSize(el);
      delete el.dataset.fit;
    }
  });
}
