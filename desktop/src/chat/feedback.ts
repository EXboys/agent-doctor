/**
 * One rating pair per assistant turn, on the turn-owner fragment's action row
 * next to the copy button. A rating click is anonymous consent: the rolling
 * prompt-session log (ids and error classes, never chat content) travels with
 * it. DOM only lives here; session access and the send calls stay in
 * bubbles.ts.
 */
import { t } from "../i18n";
import { listen } from "@tauri-apps/api/event";
import { reportFeedback } from "../ipc";

export type FeedbackDirection = "up" | "down";

const THUMB_UP = `<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 7 7 2.2c.9 0 1.8.7 1.8 1.7v2.6h3.6c.9 0 1.6.9 1.4 1.8l-.9 4.2a1.5 1.5 0 0 1-1.5 1.2H4.5A1.5 1.5 0 0 1 3 12.2V8.5C3 7.7 3.7 7 4.5 7Z"/></svg>`;
const THUMB_DOWN = `<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M11.5 9 9 13.8c-.9 0-1.8-.7-1.8-1.7V9.5H3.6c-.9 0-1.6-.9-1.4-1.8l.9-4.2A1.5 1.5 0 0 1 4.6 2.3h6.9A1.5 1.5 0 0 1 13 3.8v3.7c0 .8-.7 1.5-1.5 1.5Z"/></svg>`;

/** The 👍/👎 pair plus its transient confirmation and optional supplement form. */
export function createFeedbackControls(): HTMLElement {
  const box = document.createElement("div");
  box.className = "chat-feedback";
  box.hidden = true;

  const up = document.createElement("button");
  up.type = "button";
  up.className = "chat-feedback-btn";
  up.dataset.feedback = "up";
  up.title = t("chat.feedbackUpTitle");
  up.innerHTML = THUMB_UP;

  const down = document.createElement("button");
  down.type = "button";
  down.className = "chat-feedback-btn";
  down.dataset.feedback = "down";
  down.title = t("chat.feedbackDownTitle");
  down.innerHTML = THUMB_DOWN;

  const note = document.createElement("span");
  note.className = "chat-feedback-note";

  const supplement = document.createElement("div");
  supplement.className = "chat-feedback-supplement";
  supplement.hidden = true;
  const input = document.createElement("input");
  input.type = "text";
  input.className = "chat-feedback-input";
  input.maxLength = 2000;
  input.placeholder = t("chat.feedbackSupplementPlaceholder");
  const send = document.createElement("button");
  send.type = "button";
  send.className = "chat-feedback-send";
  send.textContent = t("chat.feedbackSend");
  supplement.append(input, send);

  box.append(up, down, note, supplement);
  return box;
}

export function feedbackControls(wrap: HTMLElement): HTMLElement | null {
  return wrap.querySelector<HTMLElement>(":scope > .chat-msg-actions > .chat-feedback");
}

/**
 * Owner fragments show the pair; streaming or non-owner fragments hide it.
 * A rated turn keeps its buttons visible with the chosen side highlighted.
 * Runs after the copy sync so the row's own `hidden` accounts for feedback.
 */
export function syncFeedbackControls(
  wrap: HTMLElement,
  opts: { owner: boolean; streaming: boolean; rated?: FeedbackDirection },
): void {
  const actions = wrap.querySelector<HTMLElement>(":scope > .chat-msg-actions");
  const box = feedbackControls(wrap);
  if (!actions || !box) return;
  const show = opts.owner && !opts.streaming;
  box.hidden = !show;
  for (const btn of box.querySelectorAll<HTMLButtonElement>(".chat-feedback-btn")) {
    btn.classList.toggle("is-rated", btn.dataset.feedback === opts.rated);
  }
  const copy = actions.querySelector<HTMLButtonElement>(":scope > .chat-msg-copy");
  const time = actions.querySelector<HTMLTimeElement>(":scope > time.chat-msg-time");
  const copyVisible = Boolean(copy && !copy.hidden);
  const timeVisible = Boolean(time && !time.hidden);
  actions.hidden = !copyVisible && !timeVisible && !show;
}

/** Reveal the supplement input under the 👎 and focus it. */
export function openFeedbackSupplement(wrap: HTMLElement): void {
  const supplement = feedbackControls(wrap)?.querySelector<HTMLElement>(
    ":scope > .chat-feedback-supplement",
  );
  if (!supplement) return;
  supplement.hidden = false;
  supplement.querySelector<HTMLInputElement>(".chat-feedback-input")?.focus();
}

/** The typed supplement, trimmed; the input clears on hand-off. */
export function takeFeedbackSupplement(wrap: HTMLElement): string {
  const input = feedbackControls(wrap)?.querySelector<HTMLInputElement>(".chat-feedback-input");
  const text = (input?.value ?? "").trim();
  if (input) input.value = "";
  const supplement = feedbackControls(wrap)?.querySelector<HTMLElement>(
    ":scope > .chat-feedback-supplement",
  );
  if (supplement) supplement.hidden = true;
  return text;
}

/**
 * Inline confirmation in the actions row. Successes fade after a few seconds;
 * errors stick around (styled `.is-error`) until the next message replaces them.
 */
export function showFeedbackNote(wrap: HTMLElement, text: string, isError = false): void {
  const note = feedbackControls(wrap)?.querySelector<HTMLElement>(".chat-feedback-note");
  if (!note) return;
  note.textContent = text;
  note.hidden = false;
  note.classList.toggle("is-error", isError);
  if (isError) return;
  window.setTimeout(() => {
    note.textContent = "";
    note.hidden = true;
  }, 3000);
}

/**
 * The manual feedback dialog: description (optional) + anonymous report.
 * One instance at a time; the tray, the account bar, and the settings page
 * all land here.
 */
export function openFeedbackDialog(): void {
  if (document.querySelector(".chat-feedback-dialog")) return;
  const overlay = document.createElement("div");
  overlay.className = "chat-feedback-dialog";

  const card = document.createElement("div");
  card.className = "chat-feedback-card";

  const title = document.createElement("h3");
  title.className = "chat-feedback-title";
  title.textContent = t("chat.feedbackTitle");

  const textarea = document.createElement("textarea");
  textarea.className = "chat-feedback-textarea";
  textarea.placeholder = t("chat.feedbackPlaceholder");
  textarea.maxLength = 2000;

  const privacy = document.createElement("p");
  privacy.className = "chat-feedback-privacy";
  privacy.textContent = t("chat.feedbackPrivacyNote");

  const actions = document.createElement("div");
  actions.className = "chat-feedback-dialog-actions";
  const cancel = document.createElement("button");
  cancel.type = "button";
  cancel.className = "chat-feedback-dialog-btn";
  cancel.textContent = t("chat.feedbackCancel");
  const submit = document.createElement("button");
  submit.type = "button";
  submit.className = "chat-feedback-dialog-btn is-primary";
  submit.textContent = t("chat.feedbackSend");
  actions.append(cancel, submit);

  const result = document.createElement("p");
  result.className = "chat-feedback-result";
  result.hidden = true;

  const close = () => overlay.remove();
  cancel.addEventListener("click", close);
  overlay.addEventListener("pointerdown", (event) => {
    if (event.target === overlay) close();
  });
  overlay.addEventListener("keydown", (event) => {
    if (event.key === "Escape") close();
  });
  submit.addEventListener("click", () => {
    const note = textarea.value.trim();
    if (!note) {
      result.hidden = false;
      result.textContent = t("chat.feedbackEmptyNote");
      return;
    }
    submit.disabled = true;
    cancel.disabled = true;
    void reportFeedback({ reason: "manual", note })
      .then(() => {
        result.hidden = false;
        result.textContent = t("chat.feedbackSent");
        window.setTimeout(close, 1200);
      })
      .catch(() => {
        submit.disabled = false;
        cancel.disabled = false;
        result.hidden = false;
        result.textContent = t("chat.feedbackSendFailed");
      });
  });

  card.append(title, textarea, privacy, actions, result);
  overlay.appendChild(card);
  document.body.appendChild(overlay);
  textarea.focus();
}

/**
 * Wire the standing entries: the account-bar icon, the settings-page action,
 * and the tray's `ad-open-feedback` event. Called once from the chat boot.
 */
export function bindFeedbackLaunchers(): void {
  document.querySelector("#chat-feedback-open")?.addEventListener("click", () => openFeedbackDialog());
  document
    .querySelector("#chat-settings-feedback")
    ?.addEventListener("click", () => openFeedbackDialog());
  void listen("ad-open-feedback", () => openFeedbackDialog());
}
