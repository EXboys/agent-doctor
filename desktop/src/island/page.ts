import { parseStoreRaw } from "../chat/store";
import { STORAGE_KEY } from "../chat/types";
import { applyStaticI18n, t } from "../i18n";
import { renderMarkdown } from "../markdown";
import { doingText, planProgress, renderPlanCard } from "../plan";
import {
  currentIslandView,
  islandClaimKeyboard,
  islandOpenSession,
  islandSendText,
  islandSetHover,
  islandSetContentHeight,
  islandSetReading,
  resolvePermissionSession,
} from "../ipc";
import { permissionView } from "./permission";
import { buildIslandRows, type IslandPending, type IslandRow, type IslandView } from "./track";

const root = document.querySelector<HTMLElement>("#island");
const titleEl = document.querySelector<HTMLElement>("#island-title");
const detailEl = document.querySelector<HTMLElement>("#island-detail");
const actionsEl = document.querySelector<HTMLElement>("#island-actions");
const errorEl = document.querySelector<HTMLElement>("#island-error");

let hoverTimer = 0;
let pointerInside = false;
let pointerX = 0;
let pointerY = 0;
let renderedRequest = "";
let sending = false;
let feedSig = "";
let openRowId = "";
let lastRows: IslandFeedRow[] = [];
let activePending: IslandPending | null = null;
const READ_MESSAGES = 40;

type IslandFeedRow = IslandRow;

function setHover(hovering: boolean, sticky = false): void {
  window.clearTimeout(hoverTimer);
  if (hovering) {
    if (pointerInside && !sticky) return;
    pointerInside = true;
    void islandSetHover(true, sticky).catch(() => {});
    return;
  }
  hoverTimer = window.setTimeout(() => {
    // A shrinking card can slip out from under the pointer. That is not a leave.
    if (pointerOverIsland()) return;
    const field = document.activeElement;
    const typing =
      (field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement) &&
      field.value.trim() !== "";
    if (!typing && field instanceof HTMLElement) field.blur();
    pointerInside = false;
    window.clearTimeout(rowHoverTimer);
    void islandSetHover(false, typing).catch(() => {});
  }, 180);
}

function notePointer(event: MouseEvent): void {
  pointerX = event.clientX;
  pointerY = event.clientY;
}

function pointerOverIsland(): boolean {
  if (!root) return false;
  const hit = document.elementFromPoint(pointerX, pointerY);
  return hit === root || Boolean(hit && root.contains(hit));
}

function showSendResult(status: string): void {
  const field = actionsEl?.querySelector<HTMLInputElement | HTMLTextAreaElement>(".island-reply");
  if (status === "sent" || status === "queued") {
    if (field) {
      field.value = "";
      if (field instanceof HTMLTextAreaElement) fitReplyField(field);
      field.closest(".island-compose")?.classList.remove("is-ready");
    }
  }
  if (!errorEl) return;
  if (status === "sent") {
    errorEl.hidden = true;
    return;
  }
  errorEl.hidden = false;
  errorEl.classList.toggle("is-info", status === "queued");
  errorEl.textContent =
    status === "queued"
      ? t("island.queued")
      : status === "busy"
        ? t("island.busyOther")
        : t("island.failed");
}

function showError(error: unknown): void {
  errorEl?.classList.remove("is-info");
  if (!errorEl) return;
  const raw = String(error);
  errorEl.hidden = false;
  errorEl.textContent = /no active ask session/i.test(raw)
    ? t("chat.permissionSessionGone")
    : t("island.failed");
}

const RESULT_MS = 2500;
let lastResult: { rowId: string; text: string; short: string; ok: boolean; until: number } | null = null;
let resultTimer = 0;

function resultText(pending: IslandPending, allow: boolean): { text: string; short: string } {
  if (pending.kind === "choice") {
    return allow
      ? { text: t("island.allowedResult"), short: t("island.allowedShort") }
      : { text: t("island.deniedResult"), short: t("island.deniedShort") };
  }
  return allow
    ? { text: t("island.answeredResult"), short: t("island.answeredShort") }
    : { text: t("island.skippedResult"), short: t("island.skippedShort") };
}

function resultLine(): HTMLElement | null {
  if (!lastResult) return null;
  const line = document.createElement("p");
  line.className = "island-result";
  line.dataset.ok = String(lastResult.ok);
  line.setAttribute("role", "status");
  line.textContent = lastResult.text;
  return line;
}

function showResult(pending: IslandPending, allow: boolean): void {
  const rowId = lastRows.find((row) => row.needsYou)?.id ?? "";
  lastResult = { rowId, ...resultText(pending, allow), ok: allow, until: Date.now() + RESULT_MS };
  const line = resultLine();
  if (actionsEl && line) actionsEl.replaceChildren(line);
  window.clearTimeout(resultTimer);
  resultTimer = window.setTimeout(() => {
    lastResult = null;
    feedSig = "";
    if (lastView) render(lastView);
  }, RESULT_MS);
}

function appendResult(parent: HTMLElement, row: IslandFeedRow): void {
  if (!lastResult || lastResult.rowId !== row.id || Date.now() > lastResult.until) return;
  const line = resultLine();
  if (line) parent.append(line);
}

async function resolve(pending: IslandPending, allow: boolean, text?: string): Promise<void> {
  if (sending || !actionsEl) return;
  sending = true;
  if (errorEl) errorEl.hidden = true;
  const controls = [
    ...actionsEl.querySelectorAll<HTMLButtonElement | HTMLInputElement | HTMLTextAreaElement>(
      "button, input, textarea",
    ),
  ];
  controls.forEach((el) => {
    el.disabled = true;
  });
  try {
    await resolvePermissionSession({
      sessionId: pending.sessionId,
      requestId: pending.requestId,
      allow,
      text: text ?? null,
    });
    showResult(pending, allow);
  } catch (error) {
    sending = false;
    controls.forEach((el) => {
      el.disabled = false;
    });
    showError(error);
  }
}

type IslandOption = { label: string; description: string };

type IslandQuestion = {
  question: string;
  options: IslandOption[];
  multiSelect: boolean;
};

function questionsOf(pending: IslandPending): IslandQuestion[] {
  const raw = pending.inputJson?.trim();
  if (!raw) return [];
  try {
    const value = JSON.parse(raw) as { questions?: unknown };
    if (!Array.isArray(value.questions)) return [];
    const questions: IslandQuestion[] = [];
    for (const item of value.questions) {
      if (!item || typeof item !== "object") continue;
      const record = item as { question?: unknown; options?: unknown; multiSelect?: unknown };
      const question = typeof record.question === "string" ? record.question.trim() : "";
      if (!question) continue;
      const options: IslandOption[] = [];
      if (Array.isArray(record.options)) {
        for (const option of record.options) {
          if (!option || typeof option !== "object") continue;
          const { label, description } = option as { label?: unknown; description?: unknown };
          if (typeof label !== "string" || !label.trim()) continue;
          options.push({
            label: label.trim(),
            description: typeof description === "string" ? description.trim() : "",
          });
        }
      }
      questions.push({ question, options, multiSelect: record.multiSelect === true });
    }
    return questions;
  } catch {
    return [];
  }
}

const REPLY_MAX_HEIGHT = 120;

function fitReplyField(field: HTMLTextAreaElement): void {
  field.style.height = "auto";
  const next = Math.min(field.scrollHeight, REPLY_MAX_HEIGHT);
  field.style.height = `${next}px`;
  field.closest(".island-compose")?.classList.toggle("is-multiline", next > 40);
}

function appendReplyField(
  parent: HTMLElement,
  placeholder: string,
  secret: boolean,
  onSend: (text: string) => void,
): HTMLInputElement | HTMLTextAreaElement {
  const compose = document.createElement("div");
  compose.className = "island-compose";
  const field = secret ? document.createElement("input") : document.createElement("textarea");
  if (field instanceof HTMLInputElement) field.type = "password";
  else field.rows = 1;
  field.className = "island-reply";
  field.placeholder = placeholder;
  field.autocomplete = "off";
  field.spellcheck = false;
  field.addEventListener("pointerdown", (event) => {
    event.stopPropagation();
    void islandClaimKeyboard().finally(() => field.focus());
  });
  field.addEventListener("input", () => {
    if (field instanceof HTMLTextAreaElement) fitReplyField(field);
    compose.classList.toggle("is-ready", field.value.trim().length > 0);
  });
  const send = () => {
    const text = field.value.trim();
    if (!text) return;
    onSend(text);
  };
  field.addEventListener("keydown", (event) => {
    if (!(event instanceof KeyboardEvent)) return;
    if (event.key !== "Enter" || event.isComposing || event.shiftKey) return;
    event.preventDefault();
    send();
  });
  const button = document.createElement("button");
  button.type = "button";
  button.className = "island-send";
  button.textContent = t("island.send");
  button.addEventListener("click", send);
  compose.append(field, button);
  parent.append(compose);
  return field;
}

/** The question in the card goes with the row that asked it. A reply only goes to the row that is open. */
function replyPlan(pending: IslandPending | null): { pending: IslandPending | null; target: IslandFeedRow | null } {
  const asking = lastRows.find((row) => row.needsYou) ?? null;
  if (pending) return { pending, target: asking };
  const open = openRowId ? lastRows.find((row) => row.id === openRowId) ?? null : null;
  return { pending: null, target: open };
}

function renderActions(view: IslandView): void {
  if (!actionsEl) return;
  const expanded = view.expanded;
  const plan = replyPlan(view.pending);
  const pending = plan.pending;
  const target = plan.target;
  const requestId = !expanded
    ? ""
    : pending?.requestId || (target ? `follow:${target.id}` : "");
  const keepReply =
    requestId !== "" &&
    requestId === renderedRequest &&
    actionsEl.childElementCount > 0;
  if (keepReply) return;
  renderedRequest = requestId;
  sending = false;
  actionsEl.replaceChildren();
  if (!expanded || !requestId) return;

  if (pending?.kind === "choice") {
    const row = document.createElement("div");
    row.className = "island-ask-buttons";
    const deny = document.createElement("button");
    deny.type = "button";
    deny.className = "island-ask-deny";
    deny.textContent = t("island.deny");
    deny.addEventListener("click", () => void resolve(pending, false));
    const allow = document.createElement("button");
    allow.type = "button";
    allow.className = "island-ask-allow";
    allow.textContent = t("island.allow");
    allow.addEventListener("click", () => void resolve(pending, true));
    row.append(deny, allow);
    actionsEl.append(row);
    return;
  }

  if (pending?.kind === "options") {
    renderOptions(actionsEl, pending);
    return;
  }

  if (pending?.kind === "line" || pending?.kind === "secret") {
    appendReplyField(
      actionsEl,
      pending.kind === "secret" ? t("chat.pasteSecret") : t("island.replyPlaceholder"),
      pending.kind === "secret",
      (text) => void resolve(pending, true, text),
    );
    return;
  }

  if (!target) return;
  appendReplyField(actionsEl, t("island.replyTo", { agent: target.agent }), false, (text) => {
    void sendFollowUp(target.id, text);
  });
}

function renderOptions(parent: HTMLElement, pending: IslandPending): void {
  const questions = questionsOf(pending);
  const picks = new Map<string, string[]>();
  const others = new Map<string, HTMLInputElement>();
  const submit = document.createElement("button");
  submit.type = "button";
  submit.className = "island-ask-submit";
  submit.textContent = t("chat.inputSubmit");
  submit.disabled = true;

  const answers = (): Record<string, string> | null => {
    const out: Record<string, string> = {};
    for (const question of questions) {
      const typed = others.get(question.question)?.value.trim() ?? "";
      const value = typed || (picks.get(question.question) ?? []).join(", ");
      if (!value) return null;
      out[question.question] = value;
    }
    return questions.length > 0 ? out : null;
  };
  const refresh = () => {
    submit.disabled = answers() === null;
  };

  questions.forEach((question, index) => {
    if (index > 0 || questions.length > 1) {
      const heading = document.createElement("p");
      heading.className = "island-ask-question";
      heading.textContent = question.question;
      parent.append(heading);
    }
    const grid = document.createElement("div");
    grid.className = "island-choices";
    question.options.forEach((option, optionIndex) => {
      const card = document.createElement("button");
      card.type = "button";
      card.className = "island-choice";
      const num = document.createElement("span");
      num.className = "island-choice-num";
      num.textContent = String(optionIndex + 1);
      const copy = document.createElement("span");
      copy.className = "island-choice-copy";
      const label = document.createElement("span");
      label.className = "island-choice-label";
      label.textContent = option.label;
      copy.append(label);
      if (option.description) {
        const note = document.createElement("span");
        note.className = "island-choice-note";
        note.textContent = option.description;
        copy.append(note);
      }
      card.append(num, copy);
      card.addEventListener("click", () => {
        const current = picks.get(question.question) ?? [];
        const next = question.multiSelect
          ? current.includes(option.label)
            ? current.filter((item) => item !== option.label)
            : [...current, option.label]
          : [option.label];
        picks.set(question.question, next);
        const typed = others.get(question.question);
        if (typed && !question.multiSelect) typed.value = "";
        grid.querySelectorAll<HTMLElement>(".island-choice").forEach((el) => {
          const text = el.querySelector(".island-choice-label")?.textContent ?? "";
          el.classList.toggle("is-selected", next.includes(text));
        });
        refresh();
      });
      grid.append(card);
    });
    parent.append(grid);
    const other = document.createElement("input");
    other.type = "text";
    other.className = "island-choice-other";
    other.placeholder = t("chat.answerOther");
    other.autocomplete = "off";
    other.spellcheck = false;
    other.addEventListener("pointerdown", (event) => {
      event.stopPropagation();
      void islandClaimKeyboard().finally(() => other.focus());
    });
    other.addEventListener("input", () => {
      if (other.value.trim()) {
        picks.set(question.question, []);
        grid.querySelectorAll(".island-choice").forEach((el) => el.classList.remove("is-selected"));
      }
      refresh();
    });
    others.set(question.question, other);
    parent.append(other);
  });

  if (questions.length === 0) {
    appendReplyField(parent, t("island.replyPlaceholder"), false, (text) => {
      void resolve(pending, true, JSON.stringify({ answers: { [pending.detail || "value"]: text } }));
    });
    return;
  }

  submit.addEventListener("click", () => {
    const picked = answers();
    if (picked) void resolve(pending, true, JSON.stringify({ answers: picked }));
  });
  const skip = document.createElement("button");
  skip.type = "button";
  skip.className = "island-ask-skip";
  skip.textContent = t("chat.answerSkip");
  skip.addEventListener("click", () => void resolve(pending, false));
  const footer = document.createElement("div");
  footer.className = "island-ask-footer";
  footer.append(submit, skip);
  parent.append(footer);
}

async function sendFollowUp(sessionId: string, text: string): Promise<void> {
  if (sending || !actionsEl) return;
  sending = true;
  if (errorEl) errorEl.hidden = true;
  const controls = [
    ...actionsEl.querySelectorAll<HTMLButtonElement | HTMLInputElement | HTMLTextAreaElement>(
      "button, input, textarea",
    ),
  ];
  controls.forEach((el) => {
    el.disabled = true;
  });
  try {
    await islandSendText(sessionId, text);
  } catch (error) {
    showError(error);
  } finally {
    sending = false;
    controls.forEach((el) => {
      el.disabled = false;
    });
  }
}

function appendState(meta: HTMLElement, row: IslandFeedRow): void {
  if (!row.needsYou && !row.working) return;
  const pill = document.createElement("span");
  pill.className = "island-state";
  pill.dataset.state = row.needsYou ? "waiting" : "working";
  pill.textContent = row.needsYou ? t("island.stateWaiting") : t("island.stateWorking");
  meta.prepend(pill);
}

function ago(at: number): string {
  const minutes = Math.max(0, Math.round((Date.now() - at) / 60000));
  if (minutes < 1) return t("island.justNow");
  if (minutes < 60) return t("island.minutesAgo", { n: String(minutes) });
  const hours = Math.round(minutes / 60);
  if (hours < 24) return t("island.hoursAgo", { n: String(hours) });
  return t("island.daysAgo", { n: String(Math.round(hours / 24)) });
}

function parseFeed(raw: string): IslandFeedRow[] | null {
  const detail = raw.trim();
  if (!detail.startsWith("{")) return null;
  try {
    const data = JSON.parse(detail) as { v?: number; rows?: IslandFeedRow[] };
    if (data.v !== 1 || !Array.isArray(data.rows)) return null;
    return data.rows;
  } catch {
    return null;
  }
}

function renderPlain(raw: string): void {
  if (!detailEl) return;
  const detail = raw.trim();
  const parts = detail.split("\u0001");
  const paired = parts.length > 1;
  const sent = paired ? parts[0].trim() : "";
  const spoken = paired ? parts.slice(1).join("\u0001").trim() : detail;
  const key = `plain\u0001${sent}\u0001${spoken}`;
  detailEl.hidden = !sent && !spoken;
  if (detailEl.dataset.shown === key) return;
  detailEl.dataset.shown = key;
  feedSig = "";
  detailEl.replaceChildren();
  if (sent) {
    const you = document.createElement("p");
    you.className = "island-sent-text";
    you.textContent = sent;
    detailEl.append(you);
  }
  if (spoken) {
    const reply = document.createElement("p");
    reply.className = "island-spoken-text";
    reply.textContent = spoken;
    detailEl.append(reply);
  }
}

function rowsFromChat(): IslandFeedRow[] {
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return [];
  const store = parseStoreRaw(raw);
  if (!store) return [];
  return buildIslandRows(
    store.sessions.map((session) => ({
      id: session.id,
      runtime: session.runtime,
      title: session.title,
      updatedAt: session.updatedAt,
      messages: session.messages ?? [],
      plan: session.plan?.items,
    })),
    store.activeId,
  );
}

function oneLine(text: string, max = 78): string {
  const line = text
    .split("\n")
    .map((part) => part.trim())
    .find(Boolean);
  if (!line) return "";
  return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}

function messagesFor(id: string): { role: string; content: string }[] {
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return [];
  const store = parseStoreRaw(raw);
  const session = store?.sessions.find((item) => item.id === id);
  if (!session) return [];
  return (session.messages ?? [])
    .filter((m) => (m.role === "user" || m.role === "assistant") && m.content.trim())
    .slice(-READ_MESSAGES)
    .map((m) => ({ role: m.role, content: m.content.trim() }));
}

/** The latest thing you said, and the latest reply after it. The rest stays in the chat. */
function latestExchange(row: IslandFeedRow): { you: string; reply: string } {
  const messages = messagesFor(row.id);
  let you = row.sent.trim();
  let reply = row.spoken.trim();
  for (const message of messages) {
    if (message.role === "user") {
      you = message.content.trim();
      reply = "";
    } else {
      reply = message.content.trim();
    }
  }
  return { you, reply };
}

function renderConversation(row: IslandFeedRow): HTMLElement {
  const body = document.createElement("div");
  body.className = "island-row-body";
  const { you, reply } = latestExchange(row);
  if (you) {
    const said = document.createElement("div");
    said.className = "island-msg-you";
    said.textContent = you;
    body.append(said);
  }
  if (reply) {
    const longReply = reply.length > 320 || reply.split(/\r?\n/).length > 6;
    const replyWrap = document.createElement("div");
    replyWrap.className = "island-reply-scroll";
    const expanded = longReply && expandedReplyId === row.id;
    replyWrap.classList.toggle("is-expanded", expanded);
    const answer = document.createElement("div");
    answer.className = "island-msg-reply island-md";
    answer.innerHTML = renderMarkdown(reply);
    for (const link of answer.querySelectorAll("a")) link.removeAttribute("href");
    replyWrap.append(answer);
    body.append(replyWrap);
    if (longReply) {
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.className = "island-reply-toggle";
      toggle.textContent = expanded ? t("island.collapseReply") : t("island.expandReply");
      toggle.addEventListener("click", (event) => {
        event.stopPropagation();
        expandedReplyId = expanded ? "" : row.id;
        feedSig = "";
        if (lastView) render(lastView);
      });
      body.append(toggle);
    }
  }
  return body;
}

let rowHoverTimer: number | undefined;
let scrollSettleTimer = 0;
let snappedRowId = "";
let expandedReplyId = "";

function openRow(id: string): void {
  window.clearTimeout(rowHoverTimer);
  if (openRowId === id) return;
  toggleRow(id);
}

const SCROLL_QUIET_MS = 600;
let lastWheelAt = 0;

function scrolling(): boolean {
  return Date.now() - lastWheelAt < SCROLL_QUIET_MS;
}

function rowUnderPointer(): string {
  return detailEl?.querySelector<HTMLElement>(".island-row:hover")?.dataset.id ?? "";
}

/** A reply already typed in this row stays put when the pointer leaves. */
function rowHasDraft(id: string): boolean {
  const row = detailEl?.querySelector<HTMLElement>(`.island-row[data-id="${CSS.escape(id)}"]`);
  const field = row?.querySelector("input, textarea");
  return (
    (field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement) &&
    field.value.trim() !== ""
  );
}

function hoverRow(item: HTMLElement, id: string): void {
  item.addEventListener("mouseenter", () => {
    window.clearTimeout(rowHoverTimer);
    if (openRowId === id || scrolling() || activePending) return;
    rowHoverTimer = window.setTimeout(() => {
      if (!scrolling() && !activePending && rowUnderPointer() === id) openRow(id);
    }, 180);
  });
  item.addEventListener("mouseleave", () => {
    window.clearTimeout(rowHoverTimer);
    if (openRowId !== id) return;
    rowHoverTimer = window.setTimeout(() => {
      if (scrolling() || activePending) return;
      const hovered = rowUnderPointer();
      if (hovered && hovered !== openRowId) {
        openRow(hovered);
        return;
      }
      if (openRowId === id && !hovered && !rowHasDraft(id)) toggleRow(id);
    }, 140);
  });
}

function toggleRow(id: string): void {
  const closing = openRowId === id;
  if (closing || openRowId !== id) expandedReplyId = "";
  openRowId = closing ? "" : id;
  void islandSetReading(openRowId !== "").catch(() => {});
  feedSig = "";
  if (errorEl) errorEl.hidden = true;
  if (lastView) render(lastView);
  else renderFeed(lastRows);
}

function askCopy(pending: IslandPending): { kicker: string; title: string } {
  if (pending.kind === "choice") return { kicker: t("island.needsConfirm"), title: "" };
  if (pending.kind === "options") {
    const questions = questionsOf(pending);
    const title = questions.length === 1 ? questions[0].question : pending.detail;
    return { kicker: t("island.askOptions"), title };
  }
  if (pending.kind === "secret") return { kicker: t("island.askSecret"), title: pending.detail };
  return { kicker: t("island.askLine"), title: pending.detail };
}

function showPlanList(row: IslandFeedRow): boolean {
  return Boolean(row.plan?.length) && (row.working || row.needsYou || row.id === openRowId);
}

function appendPlanProgress(copy: HTMLElement, row: IslandFeedRow): void {
  if (showPlanList(row) || !row.plan?.length) return;
  const line = document.createElement("span");
  line.className = "island-sub";
  const doing = oneLine(doingText(row.plan), 36);
  line.textContent = doing
    ? `${planProgress(row.plan)} · ${t("plan.doingNow", { item: doing })}`
    : planProgress(row.plan);
  copy.append(line);
}

function appendPlanList(parent: HTMLElement, row: IslandFeedRow): void {
  if (!showPlanList(row) || !row.plan?.length) return;
  parent.append(renderPlanCard(row.plan, "island-plan"));
}

function appendPermission(ask: HTMLElement, pending: IslandPending): void {
  const view = permissionView(pending);
  const what = document.createElement("div");
  what.className = "island-ask-what";
  const verb = document.createElement("span");
  verb.className = "island-ask-verb";
  verb.textContent = view.verb;
  what.append(verb);
  if (view.target) {
    const target = document.createElement("span");
    target.className = "island-ask-target";
    target.textContent = view.target;
    target.title = view.target;
    what.append(target);
  }
  ask.append(what);
  if (view.note) {
    const note = document.createElement("p");
    note.className = "island-ask-note";
    note.textContent = view.note;
    ask.append(note);
  }
  if (view.lines.length === 0) return;
  const code = document.createElement("div");
  code.className = "island-ask-code";
  for (const line of view.lines) {
    const el = document.createElement("div");
    el.className = "island-code-line";
    el.dataset.kind = line.kind;
    el.textContent = `${line.kind === "add" ? "+ " : line.kind === "del" ? "- " : ""}${line.text || " "}`;
    code.append(el);
  }
  if (view.more > 0) {
    const more = document.createElement("div");
    more.className = "island-code-more";
    more.textContent = t("island.moreLines", { n: String(view.more) });
    code.append(more);
  }
  ask.append(code);
}

function fieldCaret(): { el: HTMLInputElement | HTMLTextAreaElement; start: number; end: number } | null {
  const el = document.activeElement;
  if (!(el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) return null;
  if (!actionsEl?.contains(el)) return null;
  return {
    el,
    start: el.selectionStart ?? el.value.length,
    end: el.selectionEnd ?? el.value.length,
  };
}

function restoreCaret(
  saved: { el: HTMLInputElement | HTMLTextAreaElement; start: number; end: number } | null,
): void {
  if (!saved?.el.isConnected) return;
  saved.el.focus();
  try {
    saved.el.setSelectionRange(saved.start, saved.end);
  } catch {
    // A hidden key field can reject a selection range.
  }
}

function renderAskCard(item: HTMLElement, row: IslandFeedRow, pending: IslandPending): void {
  item.classList.add("is-ask");
  const head = document.createElement("div");
  head.className = "island-ask-head";
  const avatar = document.createElement("span");
  avatar.className = "island-avatar";
  avatar.dataset.runtime = row.runtime;
  avatar.textContent = row.agent.slice(0, 1).toUpperCase();
  const copy = document.createElement("span");
  copy.className = "island-copy";
  const headline = document.createElement("span");
  headline.className = "island-headline";
  const name = document.createElement("span");
  name.className = "island-agent";
  name.textContent = row.agent;
  const title = document.createElement("span");
  title.className = "island-preview";
  title.textContent = row.preview || row.agent;
  headline.append(name, title);
  const sub = document.createElement("span");
  sub.className = "island-sub";
  sub.textContent = t("island.waitingFor", { agent: row.agent });
  copy.append(headline, sub);
  const open = row.id === openRowId;
  if (!open && row.plan?.length) {
    const progress = document.createElement("span");
    progress.className = "island-sub island-sub-quiet";
    const doing = oneLine(doingText(row.plan), 36);
    progress.textContent = doing
      ? `${planProgress(row.plan)} · ${t("plan.doingNow", { item: doing })}`
      : planProgress(row.plan);
    copy.append(progress);
  }
  const meta = document.createElement("span");
  meta.className = "island-meta";
  const when = document.createElement("span");
  when.className = "island-when";
  when.textContent = ago(row.at);
  const badge = document.createElement("span");
  badge.className = "island-badge";
  badge.dataset.runtime = row.runtime;
  badge.textContent = row.badge;
  meta.append(when, badge);
  appendState(meta, row);
  head.append(avatar, copy, meta);

  const ask = document.createElement("div");
  ask.className = "island-ask";
  ask.dataset.kind = pending.kind === "choice" ? "confirm" : "question";
  const copyText = askCopy(pending);
  const top = document.createElement("div");
  top.className = "island-ask-kicker";
  const dot = document.createElement("span");
  dot.className = "island-ask-dot";
  dot.setAttribute("aria-hidden", "true");
  top.append(dot, copyText.kicker);
  ask.append(top);
  if (pending.kind === "choice") {
    appendPermission(ask, pending);
  } else if (copyText.title) {
    const heading = document.createElement("h3");
    heading.className = "island-ask-title";
    heading.textContent = copyText.title;
    ask.append(heading);
  }
  const slot = document.createElement("div");
  slot.className = "island-ask-slot";
  ask.append(slot);

  const context = document.createElement("button");
  context.type = "button";
  context.className = "island-ask-context";
  context.setAttribute("aria-expanded", String(open));
  context.textContent = open ? t("island.hideContext") : t("island.showContext");
  context.addEventListener("click", () => toggleRow(row.id));
  item.append(head, ask, context);
  if (open) {
    item.append(renderConversation(row));
    appendPlanList(item, row);
  }
}

function renderFeed(rows: IslandFeedRow[]): void {
  if (!detailEl) return;
  lastRows = rows;
  if (openRowId && !rows.some((row) => row.id === openRowId)) openRowId = "";
  detailEl.hidden = rows.length === 0;
  const shown = rows;
  const askSig = activePending
    ? `${activePending.requestId}\u0001${activePending.kind}\u0001${activePending.detail}`
    : "";
  const resultSig = lastResult ? `${lastResult.rowId}\u0001${lastResult.text}` : "";
  const sig = `${openRowId}\u0001${askSig}\u0001${resultSig}\u0001${JSON.stringify(shown)}`;
  if (sig === feedSig && detailEl.querySelector(".island-row")) return;
  feedSig = sig;
  detailEl.dataset.shown = `feed\u0001${sig}`;
  detailEl.classList.toggle("is-reading", Boolean(openRowId));
  const oldBody = detailEl.querySelector<HTMLElement>(".island-row-body");
  const stick = !oldBody || oldBody.scrollHeight - oldBody.scrollTop - oldBody.clientHeight < 24;
  const bodyScroll = oldBody?.scrollTop ?? 0;
  const scroll = detailEl.scrollTop;
  detailEl.replaceChildren();
  for (const row of shown) {
    const item = document.createElement("article");
    item.className = "island-row";
    if (row.needsYou) item.classList.add("is-needs-you");
    if (row.working) item.classList.add("is-working");
    if (row.current) item.classList.add("is-current");
    if (row.id === openRowId) item.classList.add("is-open");
    item.dataset.id = row.id;
    if (activePending && row.needsYou) {
      renderAskCard(item, row, activePending);
      detailEl.append(item);
      const body = item.querySelector<HTMLElement>(".island-row-body");
      if (body) body.scrollTop = stick ? body.scrollHeight : bodyScroll;
      continue;
    }
    const head = document.createElement("button");
    head.type = "button";
    head.className = "island-row-head";
    const avatar = document.createElement("span");
    avatar.className = "island-avatar";
    avatar.dataset.runtime = row.runtime;
    avatar.textContent = row.agent.slice(0, 1).toUpperCase();
    const copy = document.createElement("span");
    copy.className = "island-copy";
    const headline = document.createElement("span");
    headline.className = "island-headline";
    const name = document.createElement("span");
    name.className = "island-agent";
    name.textContent = row.agent;
    const open = row.id === openRowId;
    if (!open) {
      const title = document.createElement("span");
      title.className = "island-preview";
      title.textContent = row.preview || row.agent;
      headline.append(title);
      const youLine = oneLine(row.sent, 88);
      if (youLine) {
        const you = document.createElement("span");
        you.className = "island-sub";
        you.textContent = `${t("island.you")}：${youLine}`;
        copy.append(you);
      }
      const replyLine = oneLine(row.spoken, 88);
      if (replyLine && replyLine !== row.preview) {
        const reply = document.createElement("span");
        reply.className = "island-sub island-sub-quiet";
        reply.textContent = replyLine;
        copy.append(reply);
      }
    }
    headline.prepend(name);
    copy.prepend(headline);
    appendPlanProgress(copy, row);
    const meta = document.createElement("span");
    meta.className = "island-meta";
    const when = document.createElement("span");
    when.className = "island-when";
    when.textContent = ago(row.at);
    const badge = document.createElement("span");
    badge.className = "island-badge";
    badge.dataset.runtime = row.runtime;
    badge.textContent = row.badge;
    meta.append(when, badge);
    appendState(meta, row);
    head.append(avatar, copy, meta);
    head.addEventListener("click", () => openRow(row.id));
    hoverRow(item, row.id);
    item.append(head);
    if (row.id === openRowId) {
      const panel = document.createElement("div");
      panel.className = "island-row-panel";
      appendResult(panel, row);
      const body = renderConversation(row);
      panel.append(body);
      appendPlanList(panel, row);
      const more = document.createElement("button");
      more.type = "button";
      more.className = "island-more";
      more.textContent = t("island.more");
      more.addEventListener("click", () => {
        openRowId = "";
        void islandSetReading(false).catch(() => {});
        void islandOpenSession(row.id).catch(() => {});
      });
      panel.append(more);
      item.append(panel);
      body.scrollTop = stick ? body.scrollHeight : bodyScroll;
    } else {
      appendResult(item, row);
    }
    detailEl.append(item);
  }
  detailEl.scrollTop = scroll;
  if (openRowId && openRowId !== snappedRowId) {
    snappedRowId = openRowId;
    const opened = detailEl.querySelector<HTMLElement>(
      `.island-row[data-id="${CSS.escape(openRowId)}"]`,
    );
    if (opened && !opened.classList.contains("is-ask")) {
      const top = opened.offsetTop - detailEl.offsetTop;
      const bottom = top + opened.offsetHeight;
      if (top < detailEl.scrollTop) {
        detailEl.scrollTop = top;
      } else if (bottom > detailEl.scrollTop + detailEl.clientHeight) {
        detailEl.scrollTop = Math.min(top, bottom - detailEl.clientHeight);
      }
    }
  }
}

function renderDetail(raw: string): void {
  const fromSnapshot = parseFeed(raw);
  const rows = fromSnapshot && fromSnapshot.length > 0 ? fromSnapshot : rowsFromChat();
  if (rows.length > 0) renderFeed(rows);
  else renderPlain(raw);
}

function placeActions(view: IslandView): void {
  if (!actionsEl || !detailEl) return;
  const plan = replyPlan(view.pending);
  const host = plan.target
    ? detailEl.querySelector<HTMLElement>(`.island-row[data-id="${CSS.escape(plan.target.id)}"]`)
    : null;
  const slot = plan.pending ? host?.querySelector<HTMLElement>(".island-ask-slot") : null;
  if (slot) {
    slot.append(actionsEl);
    actionsEl.classList.add("is-inline");
  } else if (host) {
    const more = host.querySelector(".island-more");
    if (more) more.before(actionsEl);
    else host.append(actionsEl);
    actionsEl.classList.add("is-inline");
  } else if (errorEl) {
    errorEl.before(actionsEl);
    actionsEl.classList.remove("is-inline");
  }
}

let lastView: IslandView | null = null;
let reportedHeight = 0;
/** Tallest size this hover has needed. It does not shrink until the island closes. */
let heightLock = 0;

function marginsOf(el: Element): number {
  const style = getComputedStyle(el);
  return (parseFloat(style.marginTop) || 0) + (parseFloat(style.marginBottom) || 0);
}

/** The window is sized from this, so the rows fit without a scrollbar. */
function reportHeight(): void {
  if (!root?.classList.contains("is-expanded")) return;
  const body = root.querySelector<HTMLElement>(".island-body");
  if (!body) return;
  let height = parseFloat(getComputedStyle(root).paddingBottom) || 0;
  const pill = root.querySelector<HTMLElement>(".island-pill");
  if (pill && pill.getClientRects().length > 0) {
    height += pill.offsetHeight + (parseFloat(getComputedStyle(body).marginTop) || 0);
  }
  for (const child of body.children) {
    if (!(child instanceof HTMLElement) || child.getClientRects().length === 0) continue;
    height += marginsOf(child) + (child === detailEl ? child.scrollHeight : child.offsetHeight);
  }
  height = Math.ceil(height);
  if (height > heightLock) heightLock = height;
  else height = heightLock;
  if (Math.abs(height - reportedHeight) < 2) return;
  reportedHeight = height;
  void islandSetContentHeight(height).catch(() => {});
}

function render(view: IslandView): void {
  const caret = fieldCaret();
  lastView = view;
  if (!root || !titleEl || !detailEl) return;
  root.hidden = !view.shown;
  if (root.classList.contains("is-expanded") !== view.expanded) {
    window.getSelection()?.removeAllRanges();
  }
  if (!view.expanded) {
    pointerInside = false;
    window.clearTimeout(rowHoverTimer);
    window.clearTimeout(scrollSettleTimer);
    snappedRowId = "";
    heightLock = 0;
    reportedHeight = 0;
    expandedReplyId = "";
    if (openRowId) {
      openRowId = "";
      feedSig = "";
    }
  }
  root.classList.toggle("is-expanded", view.expanded);
  root.classList.toggle("is-attention", view.attention);
  const toast = !view.expanded && lastResult && Date.now() < lastResult.until ? lastResult : null;
  root.classList.toggle("is-done", Boolean(toast?.ok));
  titleEl.textContent = toast ? toast.short : view.title || t("island.idle");
  activePending = view.expanded ? view.pending : null;
  renderDetail(view.detail);
  renderActions(view);
  placeActions(view);
  restoreCaret(caret);
  if (view.expanded) requestAnimationFrame(reportHeight);
  if (!view.pending && errorEl) errorEl.hidden = true;
}

function boot(): void {
  applyStaticI18n();
  detailEl?.addEventListener(
    "wheel",
    (event) => {
      if (!detailEl || !event.deltaY) return;
      lastWheelAt = Date.now();
      window.clearTimeout(rowHoverTimer);
      window.clearTimeout(scrollSettleTimer);
      scrollSettleTimer = window.setTimeout(() => {
        if (scrolling() || activePending) return;
        const hovered = rowUnderPointer();
        if (hovered === openRowId) return;
        if (!hovered && openRowId && !rowHasDraft(openRowId)) toggleRow(openRowId);
        else if (hovered) openRow(hovered);
      }, SCROLL_QUIET_MS);
      let node = event.target instanceof Element ? event.target : null;
      let blocked = false;
      while (node && node !== detailEl) {
        if (node instanceof HTMLElement && node.scrollHeight > node.clientHeight + 1) {
          const overflowY = getComputedStyle(node).overflowY;
          if (overflowY === "auto" || overflowY === "scroll") {
            const atTop = node.scrollTop <= 0;
            const atBottom = node.scrollTop + node.clientHeight >= node.scrollHeight - 1;
            if ((event.deltaY < 0 && !atTop) || (event.deltaY > 0 && !atBottom)) return;
            blocked = true;
          }
        }
        node = node.parentElement;
      }
      if (!blocked) return;
      const before = detailEl.scrollTop;
      detailEl.scrollTop += event.deltaY;
      if (detailEl.scrollTop !== before) event.preventDefault();
    },
    { passive: false },
  );
  root?.addEventListener("mouseenter", (event) => {
    notePointer(event);
    setHover(true);
  });
  root?.addEventListener("mousemove", (event) => {
    notePointer(event);
    setHover(true);
  });
  root?.addEventListener("mousedown", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    // Cancelling mousedown also cancels the click, so buttons such as
    // 「查看更多」 would never run.
    if (!target?.closest("input, textarea, button, a, .island-row-body")) {
      event.preventDefault();
    }
    setHover(true, true);
  });
  root?.addEventListener("mouseleave", () => setHover(false));
  void currentIslandView()
    .then(render)
    .catch(() => {});
  void import("@tauri-apps/api/event")
    .then(({ listen }) =>
      listen<{ sessionId?: string; status?: string }>("island-send-result", (event) => {
        showSendResult(event.payload?.status ?? "failed");
      }),
    )
    .catch(() => {});
  void import("@tauri-apps/api/webviewWindow")
    .then(({ getCurrentWebviewWindow }) =>
      getCurrentWebviewWindow().listen<IslandView>("island-view", (event) => {
        render(event.payload);
      }),
    )
    .catch(() => {});
}

boot();
