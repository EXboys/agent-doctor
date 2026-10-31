import { t } from "./i18n";

/** Shared checklist. The session stores it; the chat and the notch both draw it. */
export type PlanStepState = "pending" | "doing" | "done";

export type PlanStep = {
  text: string;
  state: PlanStepState;
};

export type AgentPlan = {
  items: PlanStep[];
  at: number;
};

const STATES = new Set<PlanStepState>(["pending", "doing", "done"]);
const MAX_STEPS = 24;
const MAX_TEXT = 180;

export function normalizePlanItems(raw: unknown): PlanStep[] {
  if (!Array.isArray(raw)) return [];
  const items: PlanStep[] = [];
  for (const entry of raw) {
    if (!entry || typeof entry !== "object") continue;
    const record = entry as { text?: unknown; state?: unknown };
    const text = typeof record.text === "string" ? record.text.trim() : "";
    const state = record.state;
    if (!text || typeof state !== "string" || !STATES.has(state as PlanStepState)) continue;
    items.push({ text: text.slice(0, MAX_TEXT), state: state as PlanStepState });
    if (items.length >= MAX_STEPS) break;
  }
  return items;
}

export function normalizePlan(raw: unknown): AgentPlan | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const record = raw as { items?: unknown; at?: unknown };
  const items = normalizePlanItems(record.items);
  if (!items.length) return undefined;
  const at = typeof record.at === "number" && Number.isFinite(record.at) ? record.at : Date.now();
  return { items, at };
}

export function planProgress(items: PlanStep[]): string {
  const done = items.filter((item) => item.state === "done").length;
  return t("plan.progress", { done: String(done), total: String(items.length) });
}

export function planShort(items: PlanStep[]): string {
  const done = items.filter((item) => item.state === "done").length;
  return t("plan.short", { done: String(done), total: String(items.length) });
}

export function doingText(items: PlanStep[]): string {
  return items.find((item) => item.state === "doing")?.text ?? "";
}

const PLAN_ITEM = /^(\d{1,2})[.、)]\s+(\S.{0,160})$/;

/**
 * A numbered plan written in the reply, when the model did not send a checklist.
 * Only a short list that the reply itself calls a plan. Ordinary numbered notes stay in the text.
 */
export function planStepsFromMessages(
  messages: readonly { role: string; content: string }[],
  finished: boolean,
): PlanStep[] {
  let start = 0;
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (messages[i]?.role === "user") {
      start = i + 1;
      break;
    }
  }
  const reply = messages
    .slice(start)
    .filter((message) => message.role === "assistant")
    .map((message) => message.content)
    .join("\n");
  const lines = reply.split("\n").map((line) => line.trim()).filter(Boolean);
  let intro = -1;
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i] ?? "";
    if (line.length <= 40 && /计划|\bplan\b/i.test(line)) {
      intro = i;
      break;
    }
  }
  if (intro < 0) return [];
  const texts: string[] = [];
  for (let i = intro + 1; i < lines.length && texts.length < 8; i += 1) {
    const match = (lines[i] ?? "").match(PLAN_ITEM);
    if (!match?.[2]) {
      if (texts.length) break;
      continue;
    }
    texts.push(match[2].trim());
  }
  if (texts.length < 2) return [];
  let doing = 0;
  if (!finished) {
    const mentions = [...reply.matchAll(/第\s*([1-9一二三四五六七八九])\s*步/g)];
    const raw = mentions.length ? mentions[mentions.length - 1]?.[1] ?? "1" : "1";
    const fromDigit = Number(raw);
    const step = fromDigit || "一二三四五六七八九".indexOf(raw) + 1 || 1;
    doing = Math.min(texts.length, Math.max(1, step)) - 1;
  }
  return texts.map((text, index) => ({
    text,
    state: finished || index < doing ? "done" : index === doing ? "doing" : "pending",
  }));
}

function stateLabel(state: PlanStepState): string {
  if (state === "done") return t("plan.done");
  if (state === "doing") return t("plan.doing");
  return t("plan.pending");
}

/** One card. `rootClass` is `chat-plan` or `island-plan`. */
export function renderPlanCard(items: PlanStep[], rootClass: string): HTMLElement {
  const root = document.createElement("section");
  root.className = rootClass;
  root.setAttribute("aria-label", t("plan.title"));

  const head = document.createElement("div");
  head.className = "plan-head";
  const title = document.createElement("span");
  title.className = "plan-title";
  title.textContent = t("plan.title");
  const progress = document.createElement("span");
  progress.className = "plan-progress";
  progress.textContent = planProgress(items);
  head.append(title, progress);

  const list = document.createElement("ol");
  list.className = "plan-list";
  for (const item of items) {
    const row = document.createElement("li");
    row.className = "plan-item";
    row.dataset.state = item.state;
    row.setAttribute("aria-label", `${stateLabel(item.state)} ${item.text}`);
    const mark = document.createElement("span");
    mark.className = "plan-mark";
    mark.setAttribute("aria-hidden", "true");
    if (item.state === "done") mark.textContent = "✓";
    const text = document.createElement("span");
    text.className = "plan-text";
    text.textContent = item.text;
    row.append(mark, text);
    list.append(row);
  }

  root.append(head, list);
  return root;
}

function focusStep(items: PlanStep[]): PlanStep {
  return (
    items.find((item) => item.state === "doing") ??
    [...items].reverse().find((item) => item.state === "done") ??
    items[0]!
  );
}

function planDock(log: HTMLElement): HTMLElement | null {
  return (
    log.closest(".chat-main")?.querySelector<HTMLElement>("#chat-plan-dock") ??
    document.querySelector<HTMLElement>("#chat-plan-dock")
  );
}

/** The checklist stays in the top-right of the chat. It is absent when this turn has no plan. */
export function paintChatPlan(log: HTMLElement, plan: AgentPlan | null | undefined): void {
  log.querySelector(".chat-plan")?.remove();
  const dock = planDock(log);
  if (!dock) return;
  if (!plan?.items.length) {
    dock.hidden = true;
    dock.classList.remove("is-open", "is-done-folded");
    dock.replaceChildren();
    return;
  }
  const open = dock.classList.contains("is-open");
  const doneFolded = dock.classList.contains("is-done-folded");
  dock.hidden = false;
  dock.replaceChildren();
  dock.append(renderPlanDock(plan.items));
  dock.classList.toggle("is-open", open);
  dock.classList.toggle("is-done-folded", doneFolded);
  dock.querySelector(".chat-plan-pill")?.setAttribute("aria-expanded", open ? "true" : "false");
  dock.querySelector(".chat-plan-done-toggle")?.setAttribute("aria-expanded", doneFolded ? "false" : "true");
}

function renderPlanDock(items: PlanStep[]): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "chat-plan-dock-body";

  const focus = focusStep(items);
  const pill = document.createElement("button");
  pill.type = "button";
  pill.className = "chat-plan-pill";
  pill.setAttribute("aria-expanded", "false");
  const pillMark = document.createElement("span");
  pillMark.className = "plan-mark";
  pillMark.dataset.state = focus.state;
  pillMark.setAttribute("aria-hidden", "true");
  if (focus.state === "done") pillMark.textContent = "✓";
  const pillText = document.createElement("span");
  pillText.className = "chat-plan-pill-text";
  pillText.textContent = focus.text;
  pill.append(pillMark, pillText);
  pill.addEventListener("click", () => {
    const dock = pill.closest<HTMLElement>("#chat-plan-dock");
    if (!dock) return;
    dock.classList.add("is-open");
    pill.setAttribute("aria-expanded", "true");
  });

  const panel = document.createElement("section");
  panel.className = "chat-plan-panel";
  panel.setAttribute("aria-label", t("plan.dockTitle"));

  const head = document.createElement("div");
  head.className = "chat-plan-panel-head";
  const title = document.createElement("span");
  title.className = "chat-plan-panel-title";
  title.textContent = t("plan.dockTitle");
  const progress = document.createElement("span");
  progress.className = "chat-plan-panel-count";
  progress.textContent = planShort(items);
  const fold = document.createElement("button");
  fold.type = "button";
  fold.className = "chat-plan-fold";
  fold.textContent = t("plan.fold");
  fold.addEventListener("click", () => {
    const dock = fold.closest<HTMLElement>("#chat-plan-dock");
    dock?.classList.remove("is-open");
    pill.setAttribute("aria-expanded", "false");
  });
  head.append(title, progress, fold);

  const done = items.filter((item) => item.state === "done");
  const parts: HTMLElement[] = [head];
  if (done.length) {
    const doneBtn = document.createElement("button");
    doneBtn.type = "button";
    doneBtn.className = "chat-plan-done-toggle";
    doneBtn.textContent = t("plan.doneCount", { count: String(done.length) });
    doneBtn.addEventListener("click", () => {
      const dock = doneBtn.closest<HTMLElement>("#chat-plan-dock");
      if (!dock) return;
      const folded = dock.classList.toggle("is-done-folded");
      doneBtn.setAttribute("aria-expanded", folded ? "false" : "true");
    });
    parts.push(doneBtn);
  }

  const list = document.createElement("ol");
  list.className = "plan-list";
  for (const item of items) {
    const row = document.createElement("li");
    row.className = "plan-item";
    row.dataset.state = item.state;
    row.setAttribute("aria-label", `${stateLabel(item.state)} ${item.text}`);
    const text = document.createElement("span");
    text.className = "plan-text";
    text.textContent = item.text;
    const mark = document.createElement("span");
    mark.className = "plan-mark";
    mark.setAttribute("aria-hidden", "true");
    if (item.state === "done") mark.textContent = "✓";
    row.append(text, mark);
    list.append(row);
  }
  parts.push(list);
  panel.append(...parts);
  wrap.append(pill, panel);
  return wrap;
}
