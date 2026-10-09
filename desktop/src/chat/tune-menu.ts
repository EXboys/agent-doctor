import { t } from "../i18n";

const btn = () => document.querySelector<HTMLButtonElement>("#chat-tune");
const menu = () => document.querySelector<HTMLElement>("#chat-tune-menu");

function setOpen(open: boolean): void {
  const b = btn();
  const m = menu();
  if (!b || !m) return;
  if (open) {
    const rect = b.getBoundingClientRect();
    const width = Math.min(264, window.innerWidth - 16);
    const left = Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
    m.style.left = `${left}px`;
    m.style.bottom = `${window.innerHeight - rect.top + 8}px`;
    m.style.width = `${width}px`;
  }
  m.hidden = !open;
  b.setAttribute("aria-expanded", open ? "true" : "false");
  b.classList.toggle("is-open", open);
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
    if (m.hidden) return;
    const target = event.target as Node;
    if (m.contains(target) || b.contains(target)) return;
    setOpen(false);
  });
  window.addEventListener("resize", () => setOpen(false));
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && !m.hidden) {
      setOpen(false);
      b.focus();
    }
  });
}
