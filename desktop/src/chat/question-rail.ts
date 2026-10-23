import { t } from "../i18n";
import { currentQuestionId, questionMarks, type QuestionMark, type QuestionSource } from "./question-marks";

const HIGHLIGHT = "is-question-target";

let latest: QuestionMark[] = [];
let openId: string | null = null;
let logBound: HTMLElement | null = null;
let scrollBound = false;
let railBound = false;
let highlighted: HTMLElement | null = null;
let highlightTimer = 0;

function railEl(): HTMLElement | null {
  return document.getElementById("chat-question-rail");
}

function ticksEl(): HTMLElement | null {
  return document.getElementById("chat-question-rail-ticks");
}

function cardEl(): HTMLElement | null {
  return document.getElementById("chat-question-rail-card");
}

function replyText(mark: Pick<QuestionMark, "reply" | "pending">): string {
  if (mark.reply) return mark.reply;
  return mark.pending ? t("chat.questionPending") : t("chat.questionUnanswered");
}

function currentInView(log: HTMLElement): string | null {
  const bubbles = [...log.querySelectorAll<HTMLElement>(".chat-bubble.user[data-message-id]")];
  const view = log.getBoundingClientRect();
  const atEnd = log.scrollHeight - log.scrollTop - log.clientHeight <= 32;
  return currentQuestionId(
    bubbles.flatMap((bubble) => {
      const id = bubble.dataset.messageId;
      return id ? [{ id, top: bubble.getBoundingClientRect().top }] : [];
    }),
    { top: view.top, atEnd },
  );
}

function placeCard(tick: HTMLButtonElement): void {
  const card = cardEl();
  const rail = railEl();
  if (!card || !rail) return;
  const railBox = rail.getBoundingClientRect();
  const tickBox = tick.getBoundingClientRect();
  card.style.top = `${tickBox.top - railBox.top + tickBox.height / 2}px`;
}

function paintCurrent(id: string | null): void {
  const ticks = ticksEl();
  if (!ticks) return;
  for (const button of ticks.querySelectorAll<HTMLButtonElement>(".chat-question-tick")) {
    const on = button.dataset.questionId === id;
    button.classList.toggle("is-current", on);
    if (on) button.setAttribute("aria-current", "true");
    else button.removeAttribute("aria-current");
  }
}

function showCard(id: string, title: string, reply: string, tick: HTMLButtonElement): void {
  const card = cardEl();
  const titleEl = document.getElementById("chat-question-rail-title");
  const replyEl = document.getElementById("chat-question-rail-reply");
  if (!card || !titleEl || !replyEl) return;
  openId = id;
  titleEl.textContent = title;
  replyEl.textContent = reply;
  card.hidden = false;
  placeCard(tick);
  paintCurrent(id);
}

function hideCard(): void {
  openId = null;
  const card = cardEl();
  if (card) card.hidden = true;
}

function jumpTo(log: HTMLElement, id: string): void {
  const bubble = log.querySelector<HTMLElement>(
    `.chat-bubble.user[data-message-id="${CSS.escape(id)}"]`,
  );
  if (!bubble) return;
  bubble.scrollIntoView({ block: "start", behavior: "smooth" });
  highlighted?.classList.remove(HIGHLIGHT);
  bubble.classList.add(HIGHLIGHT);
  highlighted = bubble;
  window.clearTimeout(highlightTimer);
  highlightTimer = window.setTimeout(() => {
    bubble.classList.remove(HIGHLIGHT);
    if (highlighted === bubble) highlighted = null;
  }, 1200);
}

function bindScroll(log: HTMLElement): void {
  logBound = log;
  if (scrollBound) return;
  scrollBound = true;
  log.addEventListener(
    "scroll",
    () => {
      if (openId) return;
      paintCurrent(currentInView(log));
    },
    { passive: true },
  );
  const rail = railEl();
  const ticks = ticksEl();
  const card = cardEl();
  if (rail && !railBound) {
    railBound = true;
    const close = () => {
      hideCard();
      if (logBound) paintCurrent(currentInView(logBound));
    };
    rail.addEventListener("pointerleave", close);
    rail.addEventListener("focusout", (event) => {
      const next = event.relatedTarget;
      if (next instanceof Node && rail.contains(next)) return;
      close();
    });
    card?.addEventListener("click", () => {
      if (!openId || !logBound) return;
      const id = openId;
      hideCard();
      jumpTo(logBound, id);
      paintCurrent(id);
    });
  }
  ticks?.addEventListener(
    "scroll",
    () => {
      if (!openId) return;
      const tick = ticksEl()?.querySelector<HTMLButtonElement>(
        `.chat-question-tick[data-question-id="${CSS.escape(openId)}"]`,
      );
      if (tick) placeCard(tick);
    },
    { passive: true },
  );
}

function sameMarks(ticks: HTMLElement, marks: QuestionMark[]): boolean {
  const buttons = ticks.querySelectorAll<HTMLButtonElement>(".chat-question-tick");
  if (buttons.length !== marks.length) return false;
  return marks.every((mark, index) => buttons[index]?.dataset.questionId === mark.id);
}

function refreshOpenCard(marks: QuestionMark[]): void {
  if (!openId) return;
  const mark = marks.find((item) => item.id === openId);
  const tick = ticksEl()?.querySelector<HTMLButtonElement>(
    `.chat-question-tick[data-question-id="${CSS.escape(openId)}"]`,
  );
  if (!mark || !tick) {
    hideCard();
    return;
  }
  showCard(mark.id, mark.title, replyText(mark), tick);
}

/** Fixed ticks for this chat's questions. Hidden until there are two. */
export function syncQuestionRail(messages: QuestionSource[], log: HTMLElement): void {
  const rail = railEl();
  const ticks = ticksEl();
  if (!rail || !ticks) return;
  bindScroll(log);
  const marks = questionMarks(messages);
  latest = marks;
  if (marks.length < 2) {
    rail.hidden = true;
    ticks.replaceChildren();
    hideCard();
    return;
  }
  rail.hidden = false;
  rail.setAttribute("aria-label", t("chat.questionRail"));
  if (sameMarks(ticks, marks)) {
    const buttons = ticks.querySelectorAll<HTMLButtonElement>(".chat-question-tick");
    marks.forEach((mark, index) => {
      const button = buttons[index];
      if (!button) return;
      button.setAttribute("aria-label", mark.title);
    });
    refreshOpenCard(marks);
    if (!openId) paintCurrent(currentInView(log));
    return;
  }
  const kept = openId;
  ticks.replaceChildren();
  for (const mark of marks) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "chat-question-tick";
    button.dataset.questionId = mark.id;
    button.setAttribute("aria-label", mark.title);
    const open = () => {
      const current = latest.find((item) => item.id === button.dataset.questionId);
      if (current) showCard(current.id, current.title, replyText(current), button);
    };
    button.addEventListener("pointerenter", open);
    button.addEventListener("focus", open);
    button.addEventListener("click", () => {
      hideCard();
      jumpTo(log, mark.id);
      paintCurrent(mark.id);
    });
    ticks.appendChild(button);
  }
  const reopen = kept
    ? ticks.querySelector<HTMLButtonElement>(`.chat-question-tick[data-question-id="${CSS.escape(kept)}"]`)
    : null;
  const keptMark = marks.find((item) => item.id === kept);
  if (reopen && keptMark) showCard(keptMark.id, keptMark.title, replyText(keptMark), reopen);
  else {
    hideCard();
    paintCurrent(currentInView(log));
  }
}
