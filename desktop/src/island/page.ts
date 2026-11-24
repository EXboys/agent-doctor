import { convertFileSrc } from "@tauri-apps/api/core";
import "../platform";
import { setAgentBrandIcon } from "../agent-brand";
import { fitTextarea } from "../fit-textarea";
import { applyOpenChatId, parseStoreRaw } from "../chat/store";
import { STORAGE_KEY } from "../chat/types";
import { applyStaticI18n, t } from "../i18n";
import { bindLocaleSync } from "../locale-sync";
import { foldPreview, stripMarkdown } from "../chat/format";
import { renderMarkdown } from "../markdown";
import { doingText, planProgress, renderPlanCard } from "../plan";
import {
  currentIslandView,
  islandClaimKeyboard,
  islandOpenSession,
  islandSendText,
  islandSetHover,
  islandSetContentHeight,
  islandSetHideWhenIdle,
  islandHideWhenIdle,
  islandPlaceChip,
  islandRestore,
  currentIslandShot,
  islandClearShot,
  islandForgetShot,
  islandShotPreview,
  islandSetReading,
  resolvePermissionSession,
  startRegionCapture,
} from "../ipc";
import { permissionView } from "./permission";
import { buildIslandRows, type IslandPending, type IslandRow, type IslandView } from "./track";

if (/\bWindows\b/.test(navigator.userAgent)) {
  document.documentElement.classList.add("is-status-chip");
}

const root = document.querySelector<HTMLElement>("#island");
const titleEl = document.querySelector<HTMLElement>("#island-title");
const detailEl = document.querySelector<HTMLElement>("#island-detail");
const scrollHintEl = document.querySelector<HTMLElement>("#island-scroll-hint");
const actionsEl = document.querySelector<HTMLElement>("#island-actions");
const errorEl = document.querySelector<HTMLElement>("#island-error");
const hideIdleEl = document.querySelector<HTMLInputElement>("#island-hide");
const backEl = document.querySelector<HTMLButtonElement>("#island-back");
const screenshotEl = document.querySelector<HTMLButtonElement>("#island-screenshot");
const shotCardEl = document.querySelector<HTMLElement>("#island-shot-card");

type ShotStatus = "" | "sent" | "queued" | "busy" | "failed";
type ShotCardState = {
  paths: string[];
  phase: "compose" | "sent";
  text: string;
  shown: string;
  sessionId: string;
  status: ShotStatus;
};
const SHOT_CAP = 4;
let shot: ShotCardState | null = null;
let shotError = "";
let shotSending = false;

const SHOT_CHIPS = [
  { label: "chat.screenshotChipDiagnose", prompt: "chat.screenshotChipDiagnosePrompt" },
  { label: "chat.screenshotChipExtract", prompt: "chat.screenshotChipExtractPrompt" },
  { label: "chat.screenshotChipExplain", prompt: "chat.screenshotChipExplainPrompt" },
] as const;

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
  errorEl.textContent = /no active ask session|already answered/i.test(raw)
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
  const next = fitTextarea(field, REPLY_MAX_HEIGHT);
  field.closest(".island-compose")?.classList.toggle("is-multiline", next > 40);
}

function sendArrowIcon(): SVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("width", "14");
  svg.setAttribute("height", "14");
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", "M8 12.8V3.6M8 3.6 4.3 7.2M8 3.6l3.7 3.6");
  path.setAttribute("fill", "none");
  path.setAttribute("stroke", "currentColor");
  path.setAttribute("stroke-width", "1.85");
  path.setAttribute("stroke-linecap", "round");
  path.setAttribute("stroke-linejoin", "round");
  svg.append(path);
  return svg;
}

function openExternalIcon(): SVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("width", "12");
  svg.setAttribute("height", "12");
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", "M4.5 11.5 11.5 4.5M6.2 4.5h5.3v5.3");
  path.setAttribute("fill", "none");
  path.setAttribute("stroke", "currentColor");
  path.setAttribute("stroke-width", "1.65");
  path.setAttribute("stroke-linecap", "round");
  path.setAttribute("stroke-linejoin", "round");
  svg.append(path);
  return svg;
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
  field.addEventListener("compositionstart", () => {
    field.dataset.composing = "1";
  });
  field.addEventListener("compositionend", () => {
    window.setTimeout(() => {
      field.dataset.composing = "0";
      if (field instanceof HTMLTextAreaElement) fitReplyField(field);
    }, 0);
  });
  field.addEventListener("input", () => {
    if (field instanceof HTMLTextAreaElement) fitReplyField(field);
    compose.classList.toggle("is-ready", field.value.trim().length > 0);
    reportHeight();
  });
  const send = () => {
    const text = field.value.trim();
    if (!text) return;
    onSend(text);
  };
  field.addEventListener("keydown", (event) => {
    if (!(event instanceof KeyboardEvent)) return;
    if (
      event.key !== "Enter" ||
      event.isComposing ||
      event.shiftKey ||
      field.dataset.composing === "1"
    ) {
      return;
    }
    event.preventDefault();
    send();
  });
  const button = document.createElement("button");
  button.type = "button";
  button.className = "island-send";
  const sendLabel = t("island.send");
  button.setAttribute("aria-label", sendLabel);
  button.title = sendLabel;
  button.append(sendArrowIcon());
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
  const parsed = parseStoreRaw(raw);
  if (!parsed) return [];
  const store = applyOpenChatId(parsed);
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
  const line = stripMarkdown(
    text
      .split("\n")
      .map((part) => part.trim())
      .find(Boolean) ?? "",
  );
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

function conversationTurns(row: IslandFeedRow): { role: "user" | "assistant"; content: string }[] {
  if (row.turns && row.turns.length > 0) return row.turns;
  const fromStore = messagesFor(row.id).filter(
    (m): m is { role: "user" | "assistant"; content: string } =>
      m.role === "user" || m.role === "assistant",
  );
  if (fromStore.length > 0) return fromStore;
  const out: { role: "user" | "assistant"; content: string }[] = [];
  if (row.sent.trim()) out.push({ role: "user", content: row.sent.trim() });
  if (row.spoken.trim()) out.push({ role: "assistant", content: row.spoken.trim() });
  return out;
}

function renderConversation(row: IslandFeedRow): HTMLElement {
  const body = document.createElement("div");
  body.className = "island-row-body";
  const focused = row.id === openRowId;
  const turns = focused ? conversationTurns(row) : [];
  if (!focused) {
    const { you, reply } = latestExchange(row);
    if (you) {
      const said = document.createElement("div");
      said.className = "island-msg-you";
      said.dataset.role = t("island.you");
      said.textContent = you;
      body.append(said);
    }
    if (reply) appendReply(body, row.id, reply, false);
    return body;
  }
  turns.forEach((turn, idx) => {
    if (turn.role === "user") {
      const said = document.createElement("div");
      said.className = "island-msg-you";
      said.dataset.role = t("island.you");
      said.textContent = turn.content;
      body.append(said);
    } else {
      appendReply(body, `${row.id}:${idx}`, turn.content, true);
    }
  });
  if (row.working && turns[turns.length - 1]?.role === "user") {
    const waiting = document.createElement("p");
    waiting.className = "island-shot-waiting";
    waiting.textContent = t("island.stateWorking");
    body.append(waiting);
  }
  return body;
}

function replyNeedsFold(reply: string): boolean {
  const plain = stripMarkdown(reply);
  const lines = plain
    .split("\n")
    .map((part) => part.trim())
    .filter(Boolean);
  return lines.length > 3 || plain.length > 80;
}

function appendReply(body: HTMLElement, rowId: string, reply: string, unfolded = false): void {
  const focused = unfolded || (openRowId !== "" && openRowId === rowId);
  const fold = !focused && replyNeedsFold(reply);
  const open = fold && expandedReplyId === rowId;
  if (fold && !open) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "island-reply-fold";
    const preview = document.createElement("span");
    preview.className = "island-reply-preview";
    preview.textContent = foldPreview(reply);
    const label = document.createElement("span");
    label.className = "island-reply-toggle";
    label.textContent = t("island.expandReply");
    button.append(preview, label);
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      setReplyOpen(rowId);
    });
    body.append(button);
    return;
  }
  const answer = document.createElement("div");
  answer.className = "island-msg-reply island-md";
  answer.innerHTML = renderMarkdown(reply);
  for (const link of answer.querySelectorAll("a")) link.removeAttribute("href");
  body.append(answer);
  if (!fold) return;
  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "island-reply-toggle";
  toggle.textContent = t("island.collapseReply");
  toggle.addEventListener("click", (event) => {
    event.stopPropagation();
    setReplyOpen("");
  });
  body.append(toggle);
}

function setReplyOpen(rowId: string): void {
  expandedReplyId = rowId;
  feedSig = "";
  if (lastView) render(lastView);
  else renderFeed(lastRows);
}

let snappedRowId = "";
let expandedReplyId = "";

function toggleRow(id: string): void {
  const closing = openRowId === id;
  if (closing || openRowId !== id) expandedReplyId = "";
  openRowId = closing ? "" : id;
  reportedHeight = 0;
  void islandSetReading(openRowId !== "").catch(() => {});
  feedSig = "";
  if (errorEl) errorEl.hidden = true;
  if (lastView) render(lastView);
  else renderFeed(lastRows);
}

function leaveFocus(): void {
  if (!openRowId) return;
  openRowId = "";
  expandedReplyId = "";
  snappedRowId = "";
  feedSig = "";
  reportedHeight = 0;
  void islandSetReading(false).catch(() => {});
  if (lastView) render(lastView);
  else renderFeed(lastRows);
}

function applyFocusChrome(): void {
  const on = Boolean(openRowId) && !shot;
  root?.classList.toggle("is-focus", on);
  if (backEl) backEl.hidden = !on;
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

function paintIslandAvatar(runtime: string, fallback: string): HTMLElement {
  const avatar = document.createElement("span");
  avatar.className = "island-avatar";
  avatar.dataset.runtime = runtime;
  setAgentBrandIcon(avatar, runtime);
  if (!avatar.querySelector(".agent-brand-img")) {
    avatar.textContent = (fallback.trim().charAt(0) || "?").toUpperCase();
  }
  return avatar;
}

function renderAskCard(item: HTMLElement, row: IslandFeedRow, pending: IslandPending): void {
  item.classList.add("is-ask");
  const head = document.createElement("div");
  head.className = "island-ask-head";
  const avatar = paintIslandAvatar(row.runtime, row.agent);
  avatar.setAttribute("aria-label", row.agent);
  const copy = document.createElement("span");
  copy.className = "island-copy";
  const headline = document.createElement("span");
  headline.className = "island-headline";
  const title = document.createElement("span");
  title.className = "island-preview";
  title.textContent = row.preview || row.agent;
  headline.append(title);
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
  const visible = openRowId ? shown.filter((row) => row.id === openRowId) : shown;
  for (const row of visible) {
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
    const head = document.createElement(openRowId ? "div" : "button");
    if (head instanceof HTMLButtonElement) head.type = "button";
    head.className = "island-row-head";
    const avatar = paintIslandAvatar(row.runtime, row.agent);
    avatar.setAttribute("aria-label", row.agent);
    const copy = document.createElement("span");
    copy.className = "island-copy";
    const headline = document.createElement("span");
    headline.className = "island-headline";
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
    copy.prepend(headline);
    appendPlanProgress(copy, row);
    const when = document.createElement("span");
    when.className = "island-when";
    when.textContent = ago(row.at);
    const meta = document.createElement("span");
    meta.className = "island-meta";
    if (open) {
      headline.append(when);
      appendState(meta, row);
      const more = document.createElement("button");
      more.type = "button";
      more.className = "island-more";
      const moreLabel = document.createElement("span");
      moreLabel.textContent = t("island.more");
      more.append(moreLabel, openExternalIcon());
      more.addEventListener("click", () => {
        openRowId = "";
        void islandSetReading(false).catch(() => {});
        void islandOpenSession(row.id).catch(() => {});
      });
      meta.append(more);
    } else {
      const badge = document.createElement("span");
      badge.className = "island-badge";
      badge.dataset.runtime = row.runtime;
      badge.textContent = row.badge;
      meta.append(when, badge);
      appendState(meta, row);
    }
    head.append(avatar, copy, meta);
    head.setAttribute("aria-expanded", String(open));
    if (head instanceof HTMLButtonElement) head.addEventListener("click", () => toggleRow(row.id));
    item.append(head);
    if (row.id === openRowId) {
      const panel = document.createElement("div");
      panel.className = "island-row-panel";
      appendResult(panel, row);
      const body = renderConversation(row);
      panel.append(body);
      appendPlanList(panel, row);
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
  } else if (errorEl) {
    errorEl.before(actionsEl);
    actionsEl.classList.remove("is-inline");
  }
}

let lastView: IslandView | null = null;
let reportedHeight = 0;

function marginsOf(el: Element): number {
  const style = getComputedStyle(el);
  return (parseFloat(style.marginTop) || 0) + (parseFloat(style.marginBottom) || 0);
}

/** Natural height of an open conversation row, before flex clamps or stretches its scroll body. */
function openRowContentHeight(row: HTMLElement): number {
  const style = getComputedStyle(row);
  let height =
    (parseFloat(style.paddingTop) || 0) +
    (parseFloat(style.paddingBottom) || 0) +
    (parseFloat(style.borderTopWidth) || 0) +
    (parseFloat(style.borderBottomWidth) || 0);
  for (const part of row.children) {
    if (!(part instanceof HTMLElement) || part.getClientRects().length === 0) continue;
    if (part.classList.contains("island-row-panel")) {
      const panelStyle = getComputedStyle(part);
      let panelHeight =
        (parseFloat(panelStyle.paddingTop) || 0) +
        (parseFloat(panelStyle.paddingBottom) || 0) +
        (parseFloat(panelStyle.borderTopWidth) || 0) +
        (parseFloat(panelStyle.borderBottomWidth) || 0);
      let visible = 0;
      for (const item of part.children) {
        if (!(item instanceof HTMLElement) || item.getClientRects().length === 0) continue;
        const inner = item.classList.contains("island-row-body")
          ? blockContentHeight(item)
          : item.offsetHeight;
        panelHeight += marginsOf(item) + inner;
        visible += 1;
      }
      const gap = parseFloat(panelStyle.rowGap) || parseFloat(panelStyle.gap) || 0;
      if (visible > 1 && gap > 0) panelHeight += gap * (visible - 1);
      height += marginsOf(part) + panelHeight;
    } else {
      height += marginsOf(part) + part.offsetHeight;
    }
  }
  return height;
}

/** Rows, not the stretched frame. A tall window must not keep the next measure tall. */
function listContentHeight(list: HTMLElement): number {
  if (!root?.classList.contains("is-focus")) return blockContentHeight(list);
  const style = getComputedStyle(list);
  let height =
    (parseFloat(style.paddingTop) || 0) +
    (parseFloat(style.paddingBottom) || 0) +
    (parseFloat(style.borderTopWidth) || 0) +
    (parseFloat(style.borderBottomWidth) || 0);
  for (const child of list.children) {
    if (!(child instanceof HTMLElement) || child.getClientRects().length === 0) continue;
    const rowHeight =
      child.classList.contains("island-row") && child.classList.contains("is-open")
        ? openRowContentHeight(child)
        : child.offsetHeight;
    height += marginsOf(child) + rowHeight;
  }
  return height;
}

/** Content height, including the gap between rows. A shrunk card must not hide the send button. */
function blockContentHeight(block: HTMLElement): number {
  const style = getComputedStyle(block);
  let height =
    (parseFloat(style.paddingTop) || 0) +
    (parseFloat(style.paddingBottom) || 0) +
    (parseFloat(style.borderTopWidth) || 0) +
    (parseFloat(style.borderBottomWidth) || 0);
  let visible = 0;
  for (const child of block.children) {
    if (!(child instanceof HTMLElement) || child.getClientRects().length === 0) continue;
    height += marginsOf(child) + child.offsetHeight;
    visible += 1;
  }
  const gap = parseFloat(style.rowGap) || parseFloat(style.gap) || 0;
  if (visible > 1 && gap > 0) height += gap * (visible - 1);
  return height;
}

/** The window is sized from this, so the rows fit without a scrollbar. */
function reportHeight(): void {
  if (!root?.classList.contains("is-expanded")) return;
  const body = root.querySelector<HTMLElement>(".island-body");
  if (!body) return;
  const style = getComputedStyle(root);
  const notch = parseFloat(style.getPropertyValue("--island-notch")) || 0;
  const topBand = Math.max(0, (parseFloat(style.paddingTop) || 0) - notch);
  let height = (parseFloat(style.paddingBottom) || 0) + topBand;
  const pill = root.querySelector<HTMLElement>(".island-pill");
  if (pill && pill.getClientRects().length > 0) {
    height += pill.offsetHeight + (parseFloat(getComputedStyle(body).marginTop) || 0);
  }
  for (const child of body.children) {
    if (!(child instanceof HTMLElement) || child.getClientRects().length === 0) continue;
    const list = child.classList.contains("island-read-frame")
      ? child.querySelector<HTMLElement>(".island-read")
      : null;
    const measured =
      list && !list.hidden
        ? listContentHeight(list)
        : child.id === "island-shot-card"
          ? blockContentHeight(child)
          : child.offsetHeight;
    height += marginsOf(child) + measured;
  }
  height = Math.ceil(height);
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
    snappedRowId = "";
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
  const tone = toast ? "" : view.tone || "";
  root.dataset.tone = tone;
  root.classList.toggle("is-done", Boolean(toast?.ok) || tone === "done");
  root.classList.toggle("is-failed", tone === "failed");
  titleEl.textContent = toast ? toast.short : view.title || t("island.idle");
  activePending = view.expanded ? view.pending : null;
  renderDetail(view.detail);
  renderActions(view);
  placeActions(view);
  restoreCaret(caret);
  if (view.expanded) {
    requestAnimationFrame(() => {
      reportHeight();
      syncScrollHint();
    });
  } else if (scrollHintEl) {
    scrollHintEl.hidden = true;
  }
  if (shotError && errorEl) {
    errorEl.hidden = false;
    errorEl.classList.remove("is-info");
    errorEl.textContent = shotError;
  } else if (!view.pending && errorEl) {
    errorEl.hidden = true;
  }
  if (shot && !toast) {
    if (shot.phase === "compose" && !view.attention) {
      titleEl.textContent = t(shot.paths.length > 1 ? "island.shotPendingMany" : "island.shotPending");
    } else if (shot.phase === "sent" && !shotReplyText()) {
      titleEl.textContent =
        shot.status === "queued"
          ? t("island.shotQueued")
          : t(shot.paths.length > 1 ? "island.shotLookingMany" : "island.shotLooking");
    }
  }
  renderShotCard();
  applyFocusChrome();
}

/** Shown only while more conversations sit below the visible edge. */
function syncScrollHint(): void {
  if (!scrollHintEl || !detailEl) return;
  if (root?.classList.contains("is-shot") || root?.classList.contains("is-focus")) {
    scrollHintEl.hidden = true;
    return;
  }
  const moreBelow =
    !detailEl.hidden &&
    detailEl.scrollHeight > detailEl.clientHeight + 8 &&
    detailEl.scrollTop + detailEl.clientHeight < detailEl.scrollHeight - 8;
  scrollHintEl.hidden = !moreBelow;
}

const CHIP_DRAG_SLOP = 5;

function bindStatusChipDrag(rootEl: HTMLElement | null): void {
  if (!rootEl || !document.documentElement.classList.contains("is-status-chip")) return;
  const root = rootEl;
  let label = "";
  let pending: { x: number; y: number; commit: boolean } | null = null;
  let pumping = false;
  let gesture: {
    pointerId: number;
    startX: number;
    startY: number;
    lastX: number;
    lastY: number;
    originX: number;
    originY: number;
    originReady: boolean;
    moved: boolean;
    done: boolean;
  } | null = null;

  function sendPlace(x: number, y: number, commit: boolean): void {
    if (!label) return;
    pending = pending?.commit === true ? pending : { x, y, commit };
    if (pumping) return;
    pumping = true;
    void (async () => {
      while (pending) {
        const next = pending;
        pending = null;
        await islandPlaceChip(label, next.x, next.y, next.commit).catch(() => {});
      }
      pumping = false;
    })();
  }

  function placeGesture(commit: boolean): void {
    const current = gesture;
    if (!current || !current.originReady) return;
    sendPlace(
      current.originX + (current.lastX - current.startX),
      current.originY + (current.lastY - current.startY),
      commit,
    );
  }

  root.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest("input, textarea, button, a, .island-row-body")) return;
    const pointerId = event.pointerId;
    gesture = {
      pointerId,
      startX: event.screenX,
      startY: event.screenY,
      lastX: event.screenX,
      lastY: event.screenY,
      originX: 0,
      originY: 0,
      originReady: false,
      moved: false,
      done: false,
    };
    root.setPointerCapture(pointerId);
    void import("@tauri-apps/api/webviewWindow")
      .then(async ({ getCurrentWebviewWindow }) => {
        const win = getCurrentWebviewWindow();
        label = win.label;
        const pos = await win.outerPosition();
        const scale = await win.scaleFactor();
        const current = gesture;
        if (!current || current.pointerId !== pointerId || scale <= 0) return;
        current.originX = pos.x / scale;
        current.originY = pos.y / scale;
        current.originReady = true;
        if (current.moved) placeGesture(current.done);
        if (current.done) gesture = null;
      })
      .catch(() => {});
  });

  root.addEventListener("pointermove", (event) => {
    const current = gesture;
    if (!current || current.pointerId !== event.pointerId) return;
    current.lastX = event.screenX;
    current.lastY = event.screenY;
    const dx = current.lastX - current.startX;
    const dy = current.lastY - current.startY;
    if (!current.moved && Math.hypot(dx, dy) < CHIP_DRAG_SLOP) return;
    current.moved = true;
    root.classList.add("is-dragging");
    placeGesture(false);
  });

  function finishDrag(event: PointerEvent, restoreOnClick: boolean): void {
    const current = gesture;
    if (!current || current.pointerId !== event.pointerId) return;
    current.lastX = event.screenX;
    current.lastY = event.screenY;
    current.done = true;
    root.classList.remove("is-dragging");
    if (!current.moved) {
      gesture = null;
      if (restoreOnClick) void islandRestore().catch(() => {});
      return;
    }
    if (current.originReady) {
      placeGesture(true);
      gesture = null;
    }
  }

  root.addEventListener("pointerup", (event) => finishDrag(event, true));
  root.addEventListener("pointercancel", (event) => finishDrag(event, false));
}

function shotTarget(): IslandFeedRow | null {
  const sessionId = shot?.sessionId ?? "";
  if (!sessionId) return lastRows.find((row) => row.current) ?? null;
  return lastRows.find((row) => row.id === sessionId) ?? null;
}

function shotDestination(): string {
  const agent = shotTarget()?.agent;
  return agent ? t("island.shotTo", { agent }) : t("island.shotToCurrent");
}

/** The send control sits beside “先不发”, so the field stays a field. */
function markShotSend(send: HTMLElement): void {
  const field = send.closest("#island-shot-card")?.querySelector<HTMLInputElement | HTMLTextAreaElement>(".island-reply");
  const ready = (field?.value.trim().length ?? 0) > 0;
  const label = shotDestination();
  send.classList.add("island-shot-send");
  send.classList.toggle("is-ready", ready);
  send.toggleAttribute("disabled", !ready);
  const text = send.querySelector("span");
  if (text && send.querySelector("svg") && text.textContent === label) return;
  const span = document.createElement("span");
  span.textContent = label;
  send.replaceChildren(sendArrowIcon(), span);
}

function shotReplyText(): string {
  if (!shot || shot.phase !== "sent") return "";
  const row = shotTarget();
  if (!row) return "";
  const asked = shot.text.trim();
  const sent = row.sent.trim();
  const ready = row.working || sent === asked || (asked.length > 24 && sent.startsWith(asked.slice(0, 24)));
  if (!ready) return "";
  return row.spoken.trim();
}

function paintShotReply(host: HTMLElement): void {
  if (!shot) return;
  const reply = shotReplyText();
  const waiting =
    shot.status === "queued"
      ? t("island.queued")
      :     shot.status === "busy"
      ? t("island.busyOther")
      : t(shot.paths.length > 1 ? "island.shotLookingMany" : "island.shotLooking");
  const sig = `${shot.status}\u0001${reply}\u0001${expandedReplyId === "shot" ? "1" : "0"}\u0001${waiting}`;
  if (host.dataset.sig === sig) return;
  host.dataset.sig = sig;
  host.replaceChildren();
  if (reply) {
    appendReply(host, "shot", reply);
    return;
  }
  const line = document.createElement("p");
  line.className = "island-shot-waiting";
  line.textContent = waiting;
  host.append(line);
}

let shotZoomedPath = "";

function shotKey(paths: string[]): string {
  return paths.join("\n");
}

function rememberShotDraft(): void {
  const field = shotCardEl?.querySelector<HTMLInputElement | HTMLTextAreaElement>(".island-reply");
  if (shot && field) shot.text = field.value;
}

function dismissShot(): void {
  shot = null;
  shotZoomedPath = "";
  shotError = "";
  if (lastView) render(lastView);
  else renderShotCard();
  void islandClearShot().catch(() => {});
}

function removeShotPath(path: string): void {
  if (!shot || shot.phase !== "compose") return;
  const next = shot.paths.filter((item) => item !== path);
  if (next.length === 0) {
    dismissShot();
    return;
  }
  shot.paths = next;
  if (shotZoomedPath === path) shotZoomedPath = "";
  void islandForgetShot(path).catch(() => {});
  reportedHeight = 0;
  if (lastView) render(lastView);
  else renderShotCard();
}

function startAnotherShot(): void {
  rememberShotDraft();
  if ((shot?.paths.length ?? 0) >= SHOT_CAP) {
    shotError = t("island.shotLimit");
    if (lastView) render(lastView);
    else renderShotCard();
    return;
  }
  pointerInside = false;
  void startRegionCapture().catch((error) => {
    showError(error);
  });
}

function sendShot(text: string, shown: string): void {
  if (!shot || shot.phase !== "compose" || shotSending || shot.paths.length === 0) return;
  const paths = [...shot.paths];
  const key = shotKey(paths);
  const sessionId = shotTarget()?.id ?? "";
  shot.text = text;
  shot.shown = shown || text;
  shot.sessionId = sessionId;
  shot.phase = "sent";
  shot.status = "sent";
  shotError = "";
  shotSending = true;
  if (lastView) render(lastView);
  else renderShotCard();
  void islandSendText(sessionId, text, paths)
    .catch(() => {
      if (!shot || shotKey(shot.paths) !== key) return;
      shot.phase = "compose";
      shot.status = "failed";
      shotError = t("island.failed");
      if (lastView) render(lastView);
      else renderShotCard();
    })
    .finally(() => {
      shotSending = false;
    });
}

function noteShotSendResult(status: string, sessionId: string): void {
  if (!shot || shot.phase !== "sent") {
    showSendResult(status);
    return;
  }
  if (sessionId) shot.sessionId = sessionId;
  if (status === "failed" || status === "busy") {
    shot.phase = "compose";
    shot.status = status === "busy" ? "busy" : "failed";
    shotError = status === "busy" ? t("island.busyOther") : t("island.failed");
  } else {
    shot.status = status === "queued" ? "queued" : "sent";
    shotError = "";
  }
  if (lastView) render(lastView);
  else renderShotCard();
}

function acceptIslandShot(path: string): void {
  const next = path.trim();
  if (!next) {
    if (!shot) return;
    shot = null;
    shotZoomedPath = "";
    shotError = "";
    if (lastView) render(lastView);
    else renderShotCard();
    return;
  }
  if (shot?.phase === "compose" && shot.paths.includes(next)) return;
  if (shot?.phase === "compose") {
    if (shot.paths.length >= SHOT_CAP) {
      shotError = t("island.shotLimit");
      setHover(true, true);
      if (lastView) render(lastView);
      else renderShotCard();
      return;
    }
    shotError = "";
    shot.paths = [...shot.paths, next];
    setHover(true, true);
    if (lastView) render(lastView);
    else renderShotCard();
    return;
  }
  shotError = "";
  shotZoomedPath = "";
  shot = { paths: [next], phase: "compose", text: "", shown: "", sessionId: "", status: "" };
  setHover(true, true);
  if (lastView) render(lastView);
  else renderShotCard();
}

function acceptIslandShots(paths: string[]): void {
  const next = paths.map((path) => path.trim()).filter(Boolean).slice(0, SHOT_CAP);
  if (next.length === 0) return;
  if (shot?.phase === "compose" && shotKey(shot.paths) === shotKey(next)) return;
  shotError = "";
  shotZoomedPath = "";
  shot = { paths: next, phase: "compose", text: shot?.text ?? "", shown: "", sessionId: "", status: "" };
  setHover(true, true);
  if (lastView) render(lastView);
  else renderShotCard();
}

function acceptIslandShotSent(payload: { path?: string; text?: string; sessionId?: string }): void {
  const path = payload.path?.trim() ?? "";
  const text = payload.text?.trim() ?? "";
  if (!path || !text) return;
  const same = Boolean(shot?.paths.includes(path));
  const paths = same && shot ? [...shot.paths] : [path];
  const shown = same && shot?.shown ? shot.shown : text;
  const sessionId = payload.sessionId?.trim() || shot?.sessionId || "";
  shot = {
    paths,
    phase: "sent",
    text,
    shown,
    sessionId,
    status: same && shot?.status ? shot.status : "sent",
  };
  setHover(true, true);
  if (lastView) render(lastView);
  else renderShotCard();
}

function mountShotPictures(parent: HTMLElement, paths: string[]): void {
  const strip = document.createElement("div");
  strip.className = "island-shot-strip";
  if (paths.length > 1) strip.classList.add("is-many");
  for (const path of paths) {
    const image = document.createElement("img");
    image.className = "island-shot-thumb";
    image.alt = "";
    image.dataset.path = path;
    try {
      image.src = convertFileSrc(path);
    } catch {
      /* The preview below still draws the picture. */
    }
    image.addEventListener("load", () => {
      reportedHeight = 0;
      reportHeight();
    });
    strip.append(image);
    void islandShotPreview(path)
      .then((data) => {
        if (!data.startsWith("data:image/") || !shot?.paths.includes(path)) return;
        if (image.isConnected && image.src !== data) image.src = data;
      })
      .catch(() => {});
  }
  parent.append(strip);
}

function toggleShotZoom(path: string): void {
  shotZoomedPath = shotZoomedPath === path ? "" : path;
  if (!shotCardEl) return;
  for (const frame of shotCardEl.querySelectorAll<HTMLElement>(".island-shot-frame")) {
    const zoomed = frame.dataset.path === shotZoomedPath;
    frame.classList.toggle("is-zoomed", zoomed);
    frame.querySelector(".island-shot-zoom")?.setAttribute("aria-label", t(zoomed ? "island.shotHide" : "island.shotZoom"));
  }
  reportedHeight = 0;
  requestAnimationFrame(() => reportHeight());
}

/** Click a picture to see it larger. 「不看了」 drops that picture. */
function ensureShotFrames(): void {
  if (!shotCardEl) return;
  for (const image of shotCardEl.querySelectorAll<HTMLImageElement>(".island-shot-thumb")) {
    const path = image.dataset.path ?? "";
    if (!path) continue;
    let frame = image.closest(".island-shot-frame");
    if (!(frame instanceof HTMLElement)) {
      const created = document.createElement("div");
      created.className = "island-shot-frame";
      created.dataset.path = path;
      const zoom = document.createElement("button");
      zoom.type = "button";
      zoom.className = "island-shot-zoom";
      zoom.setAttribute("aria-label", t("island.shotZoom"));
      image.replaceWith(created);
      zoom.append(image);
      zoom.addEventListener("click", () => toggleShotZoom(path));
      if (shot?.phase === "compose") {
        const remove = document.createElement("button");
        remove.type = "button";
        remove.className = "island-shot-remove";
        remove.textContent = t("island.shotRemove");
        remove.addEventListener("click", (event) => {
          event.stopPropagation();
          removeShotPath(path);
        });
        created.append(zoom, remove);
      } else {
        created.append(zoom);
      }
      frame = created;
    }
    const zoomed = shotZoomedPath === path;
    frame.classList.toggle("is-zoomed", zoomed);
    frame.querySelector(".island-shot-zoom")?.setAttribute("aria-label", t(zoomed ? "island.shotHide" : "island.shotZoom"));
  }
}

function buildShotCard(): void {
  if (!shotCardEl || !shot) return;
  shotCardEl.replaceChildren();
  shotCardEl.dataset.path = shotKey(shot.paths);
  shotCardEl.dataset.phase = shot.phase;
  mountShotPictures(shotCardEl, shot.paths);
  if (shot.phase === "compose") {
    const chips = document.createElement("div");
    chips.className = "island-shot-chips";
    for (const chip of SHOT_CHIPS) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "island-shot-chip";
      button.textContent = t(chip.label);
      button.addEventListener("click", () => sendShot(t(chip.prompt), t(chip.label)));
      chips.append(button);
    }
    shotCardEl.append(chips);
    const field = appendReplyField(
      shotCardEl,
      t(shot.paths.length > 1 ? "island.shotAskMany" : "island.shotAsk"),
      false,
      (text) => {
        sendShot(text, text);
      },
    );
    const send = field.parentElement?.querySelector<HTMLElement>(".island-send");
    const bar = document.createElement("div");
    bar.className = "island-shot-bar";
    const side = document.createElement("div");
    side.className = "island-shot-bar-side";
    const dismiss = document.createElement("button");
    dismiss.type = "button";
    dismiss.className = "island-shot-dismiss";
    dismiss.textContent = t("island.shotDismiss");
    dismiss.addEventListener("click", () => dismissShot());
    const add = document.createElement("button");
    add.type = "button";
    add.className = "island-shot-add";
    add.textContent = t("island.shotAdd");
    add.addEventListener("click", () => startAnotherShot());
    side.append(dismiss, add);
    if (send) {
      field.addEventListener("input", () => {
        rememberShotDraft();
        markShotSend(send);
        reportedHeight = 0;
        reportHeight();
      });
      bar.append(side, send);
    } else {
      bar.append(side);
    }
    window.requestAnimationFrame(() => {
      void islandClaimKeyboard().finally(() => field.focus());
    });
    if (shot.text) {
      field.value = shot.text;
      field.closest(".island-compose")?.classList.add("is-ready");
      if (field instanceof HTMLTextAreaElement) fitReplyField(field);
    }
    if (send) markShotSend(send);
    shotCardEl.append(bar);
    return;
  } else {
    const asked = document.createElement("div");
    asked.className = "island-msg-you";
    asked.textContent = shot.shown || shot.text;
    const reply = document.createElement("div");
    reply.className = "island-shot-reply";
    shotCardEl.append(asked, reply);
    paintShotReply(reply);
  }
  const dismiss = document.createElement("button");
  dismiss.type = "button";
  dismiss.className = "island-shot-dismiss";
  dismiss.textContent = t("island.shotHide");
  dismiss.addEventListener("click", () => dismissShot());
  shotCardEl.append(dismiss);
}

/** The picture question is the whole card. The conversation list stays out of it. */
function applyShotLayout(): void {
  const on = Boolean(shot);
  if ((root?.classList.contains("is-shot") ?? false) !== on) reportedHeight = 0;
  root?.classList.toggle("is-shot", on);
  const frame = detailEl?.closest(".island-read-frame");
  if (frame instanceof HTMLElement) frame.hidden = on;
  if (actionsEl) actionsEl.hidden = on;
  if (on && scrollHintEl) scrollHintEl.hidden = true;
  applyFocusChrome();
}

function renderShotCard(): void {
  if (!shotCardEl) return;
  applyShotLayout();
  if (!shot) {
    shotCardEl.hidden = true;
    shotCardEl.replaceChildren();
    shotCardEl.dataset.path = "";
    shotCardEl.dataset.phase = "";
    return;
  }
  shotCardEl.hidden = false;
  const built = shotCardEl.dataset.path === shotKey(shot.paths) && shotCardEl.dataset.phase === shot.phase;
  if (!built) {
    shotZoomedPath = "";
    buildShotCard();
    reportedHeight = 0;
  }
  ensureShotFrames();
  const send = shotCardEl.querySelector<HTMLElement>(".island-send");
  if (send) markShotSend(send);
  if (shot.phase === "sent") {
    const host = shotCardEl.querySelector<HTMLElement>(".island-shot-reply");
    if (host) paintShotReply(host);
    ensureShotOpen();
  }
}

function ensureShotOpen(): void {
  if (!shotCardEl || shot?.phase !== "sent") return;
  const id = shot.sessionId || shotTarget()?.id || "";
  const existing = shotCardEl.querySelector<HTMLButtonElement>(".island-more");
  if (!id) {
    existing?.remove();
    return;
  }
  if (existing) return;
  const more = document.createElement("button");
  more.type = "button";
  more.className = "island-more";
  more.textContent = t("island.more");
  more.addEventListener("click", () => {
    const sessionId = shot?.sessionId || shotTarget()?.id || "";
    if (sessionId) void islandOpenSession(sessionId);
  });
  shotCardEl.querySelector(".island-shot-dismiss")?.before(more);
}

function boot(): void {
  applyStaticI18n();
  bindLocaleSync(() => {
    applyStaticI18n();
    if (lastView) render(lastView);
  });
  scrollHintEl?.addEventListener("click", () => {
    if (!detailEl) return;
    detailEl.scrollTo({ top: detailEl.scrollHeight, behavior: "smooth" });
  });
  detailEl?.addEventListener("scroll", syncScrollHint, { passive: true });
  if (detailEl) new ResizeObserver(syncScrollHint).observe(detailEl);
  if (shotCardEl) {
    new ResizeObserver(() => {
      if (!root?.classList.contains("is-shot")) return;
      reportHeight();
    }).observe(shotCardEl);
  }
  detailEl?.addEventListener(
    "wheel",
    (event) => {
      if (!detailEl || !event.deltaY) return;
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
    if (document.documentElement.classList.contains("is-status-chip")) return;
    notePointer(event);
    setHover(true);
  });
  root?.addEventListener("mousemove", (event) => {
    if (document.documentElement.classList.contains("is-status-chip")) return;
    notePointer(event);
    setHover(true);
  });
  root?.addEventListener("mousedown", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const onControl = Boolean(
      target?.closest("input, textarea, button, a, .island-row-body, .island-shot-card"),
    );
    // Cancelling mousedown also cancels the click, so buttons such as
    // 「查看更多」 would never run.
    if (!onControl) event.preventDefault();
    // Windows moves the chip with the pointer, and a click with almost no
    // movement still returns to the conversation.
    if (document.documentElement.classList.contains("is-status-chip")) return;
    setHover(true, true);
  });
  bindStatusChipDrag(root);
  root?.addEventListener("mouseleave", () => setHover(false));
  hideIdleEl?.addEventListener("change", () => {
    void islandSetHideWhenIdle(hideIdleEl.checked).catch(() => {});
  });
  backEl?.addEventListener("click", (event) => {
    event.stopPropagation();
    leaveFocus();
  });
  screenshotEl?.addEventListener("mousedown", (event) => {
    event.stopPropagation();
  });
  screenshotEl?.addEventListener("click", (event) => {
    event.stopPropagation();
    if (shot?.phase === "compose") {
      startAnotherShot();
      return;
    }
    pointerInside = false;
    void startRegionCapture().catch((error) => {
      showError(error);
    });
  });
  void islandHideWhenIdle()
    .then((hide) => {
      if (hideIdleEl) hideIdleEl.checked = hide;
    })
    .catch(() => {});
  void currentIslandView()
    .then(render)
    .catch(() => {});
  void currentIslandShot()
    .then((paths) => {
      const list = (Array.isArray(paths) ? paths : [paths]).map((item) => String(item).trim()).filter(Boolean);
      if (list.length) acceptIslandShots(list);
    })
    .catch(() => {});
  void import("@tauri-apps/api/event")
    .then(({ listen }) =>
      listen<{ sessionId?: string; status?: string }>("island-send-result", (event) => {
        const status = event.payload?.status ?? "failed";
        const sessionId = event.payload?.sessionId ?? "";
        if (shot?.phase === "sent") noteShotSendResult(status, sessionId);
        else showSendResult(status);
      }),
    )
    .catch(() => {});
  void import("@tauri-apps/api/webviewWindow")
    .then(({ getCurrentWebviewWindow }) => {
      const islandWindow = getCurrentWebviewWindow();
      void islandWindow.listen<IslandView>("island-view", (event) => {
        render(event.payload);
      });
      void islandWindow.listen<{ path?: string }>("island-shot", (event) => {
        acceptIslandShot(event.payload?.path ?? "");
      });
      void islandWindow.listen<{ path?: string; text?: string; sessionId?: string }>(
        "island-shot-sent",
        (event) => {
          acceptIslandShotSent(event.payload ?? {});
        },
      );
      void islandWindow.listen<string>("island-shot-failed", (event) => {
        const raw = String(event.payload ?? "").trim();
        const plain =
          raw && !raw.includes("/") && !raw.includes("\\") && raw.length < 200
            ? raw
            : t("chat.screenshotFailed");
        shotError = plain;
        setHover(true, true);
        if (lastView) render(lastView);
        else if (errorEl) {
          errorEl.hidden = false;
          errorEl.textContent = plain;
        }
      });
    })
    .catch(() => {});
}

boot();
