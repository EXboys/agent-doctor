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

/** Wide composer shows the two switches in the bar. A narrow one keeps the menu. */
function fitTune(): void {
  const bar = document.querySelector<HTMLElement>(".chat-composer-bar");
  const tools = document.querySelector<HTMLElement>(".chat-composer-tools");
  const send = document.querySelector<HTMLElement>(".chat-composer-send");
  const row = wrap();
  const m = menu();
  if (!bar || !tools || !send || !row || !m) return;

  const wasInline = row.classList.contains("is-inline");
  const wasHidden = m.hidden;
  row.classList.add("is-inline");
  m.hidden = false;
  m.style.left = "";
  m.style.bottom = "";
  m.style.width = "";

  const model = tools.querySelector<HTMLElement>(".chat-model-btn");
  const modelWrap = model?.parentElement;
  const modelMax = model?.style.maxWidth ?? "";
  const modelShrink = modelWrap?.style.flexShrink ?? "";
  if (model) model.style.maxWidth = "none";
  if (modelWrap) modelWrap.style.flexShrink = "0";
  let others = 0;
  let visible = 0;
  for (const child of Array.from(tools.children) as HTMLElement[]) {
    if (child === row) {
      visible += 1;
      continue;
    }
    const width = child.offsetWidth;
    if (width <= 0) continue;
    others += width;
    visible += 1;
  }
  const chips = m.offsetWidth;
  if (model) model.style.maxWidth = modelMax;
  if (modelWrap) modelWrap.style.flexShrink = modelShrink;

  const toolGap = parseFloat(getComputedStyle(tools).columnGap) || 0;
  const barGap = parseFloat(getComputedStyle(bar).columnGap) || 0;
  const available = bar.clientWidth - send.offsetWidth - barGap;
  const need = others + chips + toolGap * Math.max(0, visible - 1);
  const slack = wasInline ? 0 : 20;
  const inline = available >= need + slack;

  row.classList.toggle("is-inline", inline);
  if (inline) {
    btn()?.setAttribute("aria-expanded", "false");
    btn()?.classList.remove("is-open");
    return;
  }
  m.hidden = wasInline ? true : wasHidden;
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
