import "./platform";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { bindLocaleSync } from "./locale-sync";
import { t, type MessageKey } from "./i18n";

let mounted = false;

const GLYPHS = {
  minimize: `<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M2 6h8" fill="none" stroke="currentColor" stroke-width="1.1"/></svg>`,
  maximize: `<svg class="icon-max" viewBox="0 0 12 12" aria-hidden="true"><rect x="2.2" y="2.2" width="7.6" height="7.6" fill="none" stroke="currentColor" stroke-width="1.1"/></svg><svg class="icon-restore" viewBox="0 0 12 12" aria-hidden="true"><rect x="3.4" y="2.1" width="6.2" height="6.2" fill="none" stroke="currentColor" stroke-width="1.1"/><path d="M2.2 4.2v5.6h5.6" fill="none" stroke="currentColor" stroke-width="1.1"/></svg>`,
  close: `<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M3 3l6 6M9 3L3 9" fill="none" stroke="currentColor" stroke-width="1.1"/></svg>`,
} as const;

function labelKey(kind: string, maximized: boolean): MessageKey {
  if (kind === "minimize") return "window.minimize";
  if (kind === "close") return "window.close";
  return maximized ? "window.restore" : "window.maximize";
}

export function syncWindowControlLabels(): void {
  const maximized =
    document.querySelector<HTMLButtonElement>('[data-window-control="maximize"]')?.dataset
      .maximized === "true";
  document.querySelectorAll<HTMLButtonElement>("[data-window-control]").forEach((button) => {
    const kind = button.dataset.windowControl ?? "";
    const label = t(labelKey(kind, kind === "maximize" && maximized));
    button.title = label;
    button.setAttribute("aria-label", label);
  });
}

function currentWindow() {
  if (!("__TAURI_INTERNALS__" in window)) return null;
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
}

async function readWindowState(): Promise<void> {
  const win = currentWindow();
  if (!win) return;
  const [fullscreen, maximized] = await Promise.all([win.isFullscreen(), win.isMaximized()]);
  document.documentElement.classList.toggle("is-window-fullscreen", fullscreen);
  const maxBtn = document.querySelector<HTMLButtonElement>('[data-window-control="maximize"]');
  if (maxBtn) {
    maxBtn.dataset.maximized = maximized ? "true" : "false";
    maxBtn.setAttribute("aria-pressed", maximized ? "true" : "false");
  }
  syncWindowControlLabels();
}

function updateWindowFocusClass(focused: boolean): void {
  document.documentElement.classList.toggle("is-window-unfocused", !focused);
}

async function trackWindowState(): Promise<void> {
  const win = currentWindow();
  if (!win) return;
  await win.onResized(() => {
    void readWindowState();
  });
  await win.onFocusChanged(({ payload }) => {
    updateWindowFocusClass(payload);
  });
  updateWindowFocusClass(await win.isFocused());
  await readWindowState();
}

function mountWindowsControls(): void {
  const bar = document.createElement("div");
  bar.className = "window-controls";
  bar.setAttribute("role", "group");
  const kinds = ["minimize", "maximize", "close"] as const;
  for (const kind of kinds) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = `window-control window-control-${kind}`;
    button.dataset.windowControl = kind;
    button.innerHTML = GLYPHS[kind];
    button.addEventListener("click", () => {
      const win = currentWindow();
      if (!win) return;
      if (kind === "minimize") {
        void win.minimize();
        return;
      }
      if (kind === "maximize") {
        void win.toggleMaximize().then(() => readWindowState());
        return;
      }
      // CloseRequested handlers hide the home window and dismiss the others.
      void win.close();
    });
    bar.appendChild(button);
  }
  document.body.appendChild(bar);
  syncWindowControlLabels();
}

/** macOS: overlay title bar, system traffic lights. Windows: caption buttons. */
export function mountWindowChrome(): void {
  if (mounted) return;
  mounted = true;
  const os = document.documentElement.dataset.os;
  if (os === "windows") mountWindowsControls();
  if (os === "macos" || os === "windows") {
    void trackWindowState();
    bindLocaleSync(() => syncWindowControlLabels());
  }
}
