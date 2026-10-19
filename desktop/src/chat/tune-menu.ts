import { t } from "../i18n";

const btn = () => document.querySelector<HTMLButtonElement>("#chat-tune");
const menu = () => document.querySelector<HTMLElement>("#chat-tune-menu");
const wrap = () => document.querySelector<HTMLElement>(".chat-tune-wrap");

function inlineNow(): boolean {
  return wrap()?.classList.contains("is-inline") ?? false;
}

function setOpen(open: boolean): void {
  const b = btn();
  const m = menu();
  if (!b || !m || inlineNow()) return;
  if (open) {
    const rect = b.getBoundingClientRect();
    const width = Math.min(264, window.innerWidth - 16);
    const left = Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
    m.style.left = `${left}px`;
    m.style.bottom = `${window.innerHeight - rect.top + 8}px`;
    m.style.width = `${width}px`;
  } else {
    m.style.left = "";
    m.style.bottom = "";
    m.style.width = "";
  }
  m.hidden = !open;
  b.setAttribute("aria-expanded", open ? "true" : "false");
  b.classList.toggle("is-open", open);
}

/** Auto-approve and chat memory stay in the menu, so the bar stays one line. */
function fitTune(): void {
  wrap()?.classList.remove("is-inline");
}

export function syncTuneLabels(): void {
  const b = btn();
  if (!b) return;
  b.title = t("chat.tune");
  b.setAttribute("aria-label", t("chat.tune"));
}

export function bindTuneMenu(): void {
  const b = btn();
  const m = menu();
  if (!b || !m) return;
  syncTuneLabels();
  b.addEventListener("click", (event) => {
    event.stopPropagation();
    setOpen(m.hidden);
  });
  document.addEventListener("pointerdown", (event) => {
    if (m.hidden || inlineNow()) return;
    const target = event.target as Node;
    if (m.contains(target) || b.contains(target)) return;
    setOpen(false);
  });
  const bar = document.querySelector<HTMLElement>(".chat-composer-bar");
  if (bar && "ResizeObserver" in window) {
    new ResizeObserver(() => fitTune()).observe(bar);
  } else {
    window.addEventListener("resize", () => fitTune());
  }
  fitTune();
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && !m.hidden && !inlineNow()) {
      setOpen(false);
      b.focus();
    }
  });
}
