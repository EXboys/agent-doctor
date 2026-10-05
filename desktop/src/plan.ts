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

/** Replace the one live checklist in the chat log. */
export function paintChatPlan(log: HTMLElement, plan: AgentPlan | null | undefined): void {
  const existing = log.querySelector<HTMLElement>(".chat-plan");
  if (!plan?.items.length) {
    existing?.remove();
    return;
  }
  const card = renderPlanCard(plan.items, "chat-plan");
  if (existing) existing.replaceWith(card);
  else log.append(card);
  const nearBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 80;
  if (nearBottom) log.scrollTop = log.scrollHeight;
}
