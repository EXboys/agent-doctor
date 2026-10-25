import { listen } from "@tauri-apps/api/event";
import { resizeMainWindow } from "./ipc";
import { dockedHomeHeight } from "./layout-rules";

type DockLayout = {
  docked?: boolean;
  workHeight?: number;
  secondaryHeight?: number;
  fitHeight?: boolean;
};

let docked = false;
let fitHeight = false;
let workHeight = 0;
let secondaryHeight = 0;
let fitTimer: ReturnType<typeof setTimeout> | undefined;
let fitting = false;
let lastApplied = 0;

function clearWidgetInlineSize(widget: HTMLElement): void {
  widget.style.height = "";
  widget.style.maxHeight = "";
  delete widget.dataset.fit;
}

/** Tallest of Agent / 资源 / 服务商 / 项目, including the shared chrome. */
function measureTallestHome(widget: HTMLElement): number {
  const panels = [...widget.querySelectorAll<HTMLElement>("[data-main-panel]")];
  if (panels.length === 0) return Math.ceil(widget.getBoundingClientRect().height);

  const snapshot = panels.map((panel) => ({
    hidden: panel.hidden,
    active: panel.classList.contains("is-active"),
  }));
  const prevHeight = widget.style.height;
  const prevMax = widget.style.maxHeight;
  widget.style.height = "auto";
  widget.style.maxHeight = "none";

  let max = 0;
  for (const panel of panels) {
    for (const other of panels) {
      const on = other === panel;
      other.hidden = !on;
      other.classList.toggle("is-active", on);
    }
    max = Math.max(max, Math.ceil(widget.getBoundingClientRect().height));
  }

  panels.forEach((panel, index) => {
    const saved = snapshot[index];
    if (!saved) return;
    panel.hidden = saved.hidden;
    panel.classList.toggle("is-active", saved.active);
  });
  widget.style.height = prevHeight;
  widget.style.maxHeight = prevMax;
  return max;
}

export async function fitMainWindowToContent(): Promise<void> {
  if (!docked || !fitHeight || fitting || workHeight <= 0) return;
  const widget = document.querySelector<HTMLElement>(".widget");
  if (!widget) return;

  fitting = true;
  document.documentElement.classList.add("is-main-docked");
  const content = measureTallestHome(widget);
  const target = dockedHomeHeight(content, workHeight, secondaryHeight);
  widget.dataset.fit = content > target ? "tall" : "compact";
  widget.style.height = "100%";
  widget.style.maxHeight = "100%";
  fitting = false;

  if (Math.abs(target - lastApplied) < 2) return;
  try {
    await resizeMainWindow({ height: target });
    lastApplied = target;
  } catch {
    // Window may be mid-layout; a later tick will retry.
  }
}

export function scheduleMainWindowFit(): void {
  if (!docked || !fitHeight) return;
  if (fitTimer !== undefined) clearTimeout(fitTimer);
  fitTimer = setTimeout(() => {
    fitTimer = undefined;
    void fitMainWindowToContent();
  }, 60);
}

export function initMainWindowFit(): void {
  const widget = document.querySelector<HTMLElement>(".widget");
  if (widget && typeof ResizeObserver !== "undefined") {
    const observer = new ResizeObserver(() => {
      if (!fitting) scheduleMainWindowFit();
    });
    observer.observe(widget);
  }

  void listen<DockLayout>("main-dock-layout", (event) => {
    docked = Boolean(event.payload?.docked);
    fitHeight = Boolean(event.payload?.fitHeight);
    workHeight = Number(event.payload?.workHeight) || 0;
    secondaryHeight = Number(event.payload?.secondaryHeight) || 0;
    lastApplied = 0;
    if (docked && fitHeight) {
      scheduleMainWindowFit();
      return;
    }
    document.documentElement.classList.remove("is-main-docked");
    const el = document.querySelector<HTMLElement>(".widget");
    if (el) clearWidgetInlineSize(el);
  });
}
