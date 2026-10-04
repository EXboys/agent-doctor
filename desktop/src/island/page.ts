import { parseStoreRaw } from "../chat/store";
import { STORAGE_KEY } from "../chat/types";
import { applyStaticI18n, t } from "../i18n";
import { renderMarkdown } from "../markdown";
import {
  currentIslandView,
  islandClaimKeyboard,
  islandOpenSession,
  islandSendText,
  islandSetHover,
  islandSetReading,
  resolvePermissionSession,
} from "../ipc";
import { buildIslandRows, type IslandPending, type IslandRow, type IslandView } from "./track";

const root = document.querySelector<HTMLElement>("#island");
const titleEl = document.querySelector<HTMLElement>("#island-title");
const detailEl = document.querySelector<HTMLElement>("#island-detail");
const actionsEl = document.querySelector<HTMLElement>("#island-actions");
const errorEl = document.querySelector<HTMLElement>("#island-error");

let hoverTimer = 0;
let pointerInside = false;
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
    const field = document.activeElement;
    const typing = field instanceof HTMLInputElement && field.value.trim() !== "";
    if (!typing && field instanceof HTMLElement) field.blur();
    pointerInside = false;
    window.clearTimeout(rowHoverTimer);
    void islandSetHover(false, typing).catch(() => {});
  }, 180);
}

function showSendResult(status: string): void {
  const field = actionsEl?.querySelector<HTMLInputElement>(".island-reply");
  if (status === "sent" || status === "queued") {
    if (field) {
      field.value = "";
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

async function resolve(pending: IslandPending, allow: boolean, text?: string): Promise<void> {
  if (sending || !actionsEl) return;
  sending = true;
  if (errorEl) errorEl.hidden = true;
  const controls = [...actionsEl.querySelectorAll<HTMLButtonElement | HTMLInputElement>("button, input")];
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

function appendReplyField(
  parent: HTMLElement,
  placeholder: string,
  secret: boolean,
  onSend: (text: string) => void,
): HTMLInputElement {
  const compose = document.createElement("div");
  compose.className = "island-compose";
  const field = document.createElement("input");
  field.type = secret ? "password" : "text";
  field.className = "island-reply";
  field.placeholder = placeholder;
  field.autocomplete = "off";
  field.spellcheck = false;
  field.addEventListener("pointerdown", (event) => {
    event.stopPropagation();
    void islandClaimKeyboard().finally(() => field.focus());
  });
  field.addEventListener("input", () => {
    compose.classList.toggle("is-ready", field.value.trim().length > 0);
  });
  const send = () => {
    const text = field.value.trim();
    if (!text) return;
    onSend(text);
  };
  field.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" || event.isComposing) return;
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
  const open = openRowId ? lastRows.find((row) => row.id === openRowId) ?? null : null;
  if (pending && (!open || open.id === asking?.id)) return { pending, target: asking };
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
    for (const option of question.options) {
      const card = document.createElement("button");
      card.type = "button";
      card.className = "island-choice";
      const label = document.createElement("span");
      label.className = "island-choice-label";
      label.textContent = option.label;
      card.append(label);
      if (option.description) {
        const note = document.createElement("span");
        note.className = "island-choice-note";
        note.textContent = option.description;
        card.append(note);
      }
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
    }
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
  const controls = [...actionsEl.querySelectorAll<HTMLButtonElement | HTMLInputElement>("button, input")];
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

function renderConversation(row: IslandFeedRow): HTMLElement {
  const body = document.createElement("div");
  body.className = "island-row-body";
  const messages = messagesFor(row.id);
  if (messages.length === 0) {
    if (row.sent) messages.push({ role: "user", content: row.sent });
    if (row.spoken) messages.push({ role: "assistant", content: row.spoken });
  }
  for (const message of messages) {
    if (message.role === "user") {
      const you = document.createElement("div");
      you.className = "island-msg-you";
      you.textContent = message.content;
      body.append(you);
      continue;
    }
    const reply = document.createElement("div");
    reply.className = "island-msg-reply island-md";
    reply.innerHTML = renderMarkdown(message.content);
    for (const link of reply.querySelectorAll("a")) link.removeAttribute("href");
    body.append(reply);
  }
  return body;
}

let rowHoverTimer: number | undefined;

function openRow(id: string): void {
  window.clearTimeout(rowHoverTimer);
  if (openRowId === id) return;
  toggleRow(id);
}

function hoverRow(item: HTMLElement, id: string): void {
  item.addEventListener("mouseenter", () => {
    window.clearTimeout(rowHoverTimer);
    if (openRowId === id) return;
    rowHoverTimer = window.setTimeout(() => openRow(id), 260);
  });
  item.addEventListener("mouseleave", () => window.clearTimeout(rowHoverTimer));
}

function toggleRow(id: string): void {
  openRowId = openRowId === id ? "" : id;
  void islandSetReading(openRowId !== "").catch(() => {});
  feedSig = "";
  if (errorEl) errorEl.hidden = true;
  if (lastView) render(lastView);
  else renderFeed(lastRows);
}

function askCopy(pending: IslandPending): { kicker: string; title: string } {
  if (pending.kind === "choice") {
    return { kicker: t("island.askChoice"), title: pending.detail || pending.tool || "" };
  }
  if (pending.kind === "options") {
    const questions = questionsOf(pending);
    const title = questions.length === 1 ? questions[0].question : pending.detail;
    return { kicker: t("island.askOptions"), title };
  }
  if (pending.kind === "secret") return { kicker: t("island.askSecret"), title: pending.detail };
  return { kicker: t("island.askLine"), title: pending.detail };
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
  const status = document.createElement("span");
  status.className = "island-ask-status";
  status.textContent = pending.title;
  headline.append(name, status);
  const sub = document.createElement("span");
  sub.className = "island-sub";
  sub.textContent = t("island.waitingFor", { agent: row.agent });
  copy.append(headline, sub);
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

  const { kicker, title } = askCopy(pending);
  const ask = document.createElement("div");
  ask.className = "island-ask";
  const top = document.createElement("div");
  top.className = "island-ask-kicker";
  top.textContent = kicker;
  if (pending.kind === "choice" && pending.tool) {
    const tool = document.createElement("span");
    tool.className = "island-ask-tool";
    tool.textContent = pending.tool;
    top.append(tool);
  }
  ask.append(top);
  if (title) {
    const heading = document.createElement("h3");
    heading.className = "island-ask-title";
    heading.textContent = title;
    ask.append(heading);
  }
  const slot = document.createElement("div");
  slot.className = "island-ask-slot";
  ask.append(slot);

  const context = document.createElement("button");
  context.type = "button";
  context.className = "island-ask-context";
  const open = row.id === openRowId;
  context.textContent = open ? t("island.hideContext") : t("island.showContext");
  context.addEventListener("click", () => toggleRow(row.id));
  item.append(head, ask, context);
  if (open) item.append(renderConversation(row));
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
  const sig = `${openRowId}\u0001${askSig}\u0001${JSON.stringify(shown)}`;
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
    const title = document.createElement("span");
    title.className = "island-preview";
    title.textContent = row.preview || row.agent;
    headline.append(name, title);
    copy.append(headline);
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
      const body = renderConversation(row);
      const more = document.createElement("button");
      more.type = "button";
      more.className = "island-more";
      more.textContent = t("island.more");
      more.addEventListener("click", () => {
        openRowId = "";
        void islandSetReading(false).catch(() => {});
        void islandOpenSession(row.id).catch(() => {});
      });
      item.append(body, more);
      body.scrollTop = stick ? body.scrollHeight : bodyScroll;
    }
    detailEl.append(item);
  }
  detailEl.scrollTop = scroll;
  const opened = openRowId
    ? detailEl.querySelector<HTMLElement>(`.island-row[data-id="${CSS.escape(openRowId)}"]`)
    : null;
  if (opened && !opened.classList.contains("is-ask")) {
    const top = opened.offsetTop - detailEl.offsetTop;
    const bottom = top + opened.offsetHeight;
    if (bottom > detailEl.scrollTop + detailEl.clientHeight) {
      detailEl.scrollTop = Math.min(top, bottom - detailEl.clientHeight);
    } else if (top < detailEl.scrollTop) {
      detailEl.scrollTop = top;
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

function render(view: IslandView): void {
  lastView = view;
  if (!root || !titleEl || !detailEl) return;
  root.hidden = !view.shown;
  if (root.classList.contains("is-expanded") !== view.expanded) {
    window.getSelection()?.removeAllRanges();
  }
  if (!view.expanded) {
    pointerInside = false;
    window.clearTimeout(rowHoverTimer);
    if (openRowId) {
      openRowId = "";
      feedSig = "";
    }
  }
  root.classList.toggle("is-expanded", view.expanded);
  root.classList.toggle("is-attention", view.attention);
  titleEl.textContent = view.title || t("island.idle");
  activePending = view.expanded ? view.pending : null;
  renderDetail(view.detail);
  renderActions(view);
  placeActions(view);
  if (!view.pending && errorEl) errorEl.hidden = true;
}

function boot(): void {
  applyStaticI18n();
  root?.addEventListener("mouseenter", () => setHover(true));
  root?.addEventListener("mousemove", () => setHover(true));
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
