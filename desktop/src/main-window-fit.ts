import { listen } from "@tauri-apps/api/event";
import { readMainDockLayout, resizeMainWindow, type MainDockLayout } from "./ipc";
import { dockedHomeHeight, fittedHomeHeight } from "./layout-rules";

type DockLayout = Partial<MainDockLayout>;

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
}

/**
 * Height the home page needs: the fixed chrome plus the tab on screen.
 *
 * The tab is measured with the scroll box collapsed, so it reports the height
 * of its own content instead of the height it was stretched to. Measuring the
 * tallest tab instead would keep the window tall enough for a tab nobody is
 * looking at, and the visible tab would end in a blank gap again.
 */
function measureHomeContent(widget: HTMLElement): number {
  const panels = [...widget.querySelectorAll<HTMLElement>("[data-main-panel]")];
  if (panels.length === 0) return Math.ceil(widget.getBoundingClientRect().height);

  const active = panels.find((panel) => !panel.hidden) ?? panels[0];
  const snapshot = panels.map((panel) => ({
    hidden: panel.hidden,
    active: panel.classList.contains("is-active"),
  }));

  const body = widget.querySelector<HTMLElement>(".widget-body");
  const scrollBox = widget.querySelector<HTMLElement>(".main-panels");
  const saved = {
    height: widget.style.height,
    maxHeight: widget.style.maxHeight,
    bodyFlex: body?.style.flex ?? "",
    boxFlex: scrollBox?.style.flex ?? "",
    boxHeight: scrollBox?.style.height ?? "",
  };

  // The chrome decides the height for a moment: no stretched body, no box
  // that hides the content behind a scrollbar. The box is a flex item, so its
  // height alone would not collapse it — the flex basis has to go as well.
  widget.style.height = "auto";
  widget.style.maxHeight = "none";
  if (body) body.style.flex = "none";
  if (scrollBox) {
    scrollBox.style.flex = "0 0 0px";
    scrollBox.style.height = "0px";
  }

  const chrome = Math.ceil(widget.getBoundingClientRect().height);

  for (const panel of panels) {
    const on = panel === active;
    panel.hidden = !on;
    panel.classList.toggle("is-active", on);
  }
  if (scrollBox) {
    scrollBox.style.flex = "0 0 0px";
    scrollBox.style.height = "0px";
  }
  const content = Math.ceil(active.scrollHeight);

  panels.forEach((panel, index) => {
    const savedPanel = snapshot[index];
    if (!savedPanel) return;
    panel.hidden = savedPanel.hidden;
    panel.classList.toggle("is-active", savedPanel.active);
  });

  widget.style.height = saved.height;
  widget.style.maxHeight = saved.maxHeight;
  if (body) body.style.flex = saved.bodyFlex;
  if (scrollBox) {
    scrollBox.style.flex = saved.boxFlex;
    scrollBox.style.height = saved.boxHeight;
  }

  return chrome + content;
}

export async function fitMainWindowToContent(): Promise<void> {
  if (!fitHeight || fitting || workHeight <= 0) return;
  const widget = document.querySelector<HTMLElement>(".widget");
  if (!widget) return;

  fitting = true;
  document.documentElement.classList.add("is-main-fitted");
  const content = measureHomeContent(widget);
  // A window beside Ask has to share its height; a window on its own may
  // end with its content.
  const target =
    docked && secondaryHeight > 0
      ? dockedHomeHeight(content, workHeight, secondaryHeight)
      : fittedHomeHeight(content, workHeight);
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
  if (!fitHeight) return;
  if (fitTimer !== undefined) clearTimeout(fitTimer);
  fitTimer = setTimeout(() => {
    fitTimer = undefined;
    void fitMainWindowToContent();
  }, 120);
}

function applyDockLayout(payload: DockLayout): void {
  docked = Boolean(payload?.docked);
  fitHeight = Boolean(payload?.fitHeight);
  workHeight = Number(payload?.workHeight) || 0;
  secondaryHeight = Number(payload?.secondaryHeight) || 0;
  lastApplied = 0;
  if (fitHeight) {
    scheduleMainWindowFit();
    return;
  }
  document.documentElement.classList.remove("is-main-fitted");
  const el = document.querySelector<HTMLElement>(".widget");
  if (el) clearWidgetInlineSize(el);
}

export function initMainWindowFit(): void {
  const widget = document.querySelector<HTMLElement>(".widget");
  if (widget && typeof ResizeObserver !== "undefined") {
    const observer = new ResizeObserver(() => {
      if (!fitting) scheduleMainWindowFit();
    });
    observer.observe(widget);
  }

  // The page grows and shrinks on its own — a finished scan, an opened
  // section. The window has to follow, or the page ends in a blank gap.
  const panels = widget?.querySelector<HTMLElement>(".main-panels");
  if (panels && typeof MutationObserver !== "undefined") {
    const observer = new MutationObserver(() => scheduleMainWindowFit());
    observer.observe(panels, { childList: true, subtree: true });
  }

  void listen<DockLayout>("main-dock-layout", (event) => {
    applyDockLayout(event.payload ?? {});
  });

  // The first layout event is emitted while the webview is still booting, so
  // the page asks for the layout itself.
  void readMainDockLayout()
    .then((layout) => applyDockLayout(layout))
    .catch(() => undefined);
}
