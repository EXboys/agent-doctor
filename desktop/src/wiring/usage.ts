import { escapeHtml } from "../format";
import { getLocale, t } from "../i18n";
import { tokenUsageByDay } from "../ipc";
import { isKnownRuntimeId, runtimeLabel } from "../runtime-catalog";
import type { UsageDayRow } from "../types";

type Range = "today" | "week" | "month" | "days30";

const overviewEl = document.querySelector<HTMLElement>("#usage-overview");
const todayEl = document.querySelector<HTMLElement>("#usage-today");
const weekEl = document.querySelector<HTMLElement>("#usage-week");
const monthEl = document.querySelector<HTMLElement>("#usage-month");
const miniBarsEl = document.querySelector<HTMLElement>("#usage-mini-bars");
const overviewEmptyEl = document.querySelector<HTMLElement>("#usage-overview-empty");
const overviewNoteEl = document.querySelector<HTMLElement>("#usage-overview-note");
const openEl = document.querySelector<HTMLButtonElement>("#usage-open");

const listViewEl = document.querySelector<HTMLElement>("#personal-list-view");
const detailViewEl = document.querySelector<HTMLElement>("#personal-usage-view");
const backEl = document.querySelector<HTMLButtonElement>("#usage-back");
const rangeEl = document.querySelector<HTMLElement>("#usage-range");
const totalEl = document.querySelector<HTMLElement>("#usage-total");
const totalMetaEl = document.querySelector<HTMLElement>("#usage-total-meta");
const splitEl = document.querySelector<HTMLElement>("#usage-split");
const chartEl = document.querySelector<HTMLElement>("#usage-chart");
const byServiceEl = document.querySelector<HTMLElement>("#usage-by-service");
const byModelEl = document.querySelector<HTMLElement>("#usage-by-model");
const byToolEl = document.querySelector<HTMLElement>("#usage-by-tool");
const detailEmptyEl = document.querySelector<HTMLElement>("#usage-detail-empty");

const DAY_SEC = 86_400;

const AGENT_NAMES: Record<string, string> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  hermes: "Hermes",
  openclaw: "OpenClaw",
  "deepseek-harness": "DeepSeek Harness",
};

function agentName(runtime: string): string {
  return isKnownRuntimeId(runtime) ? runtimeLabel(runtime) : AGENT_NAMES[runtime] ?? runtime;
}

function tzOffsetSec(): number {
  return -new Date().getTimezoneOffset() * 60;
}

function localDay(date: Date): number {
  return Math.floor((date.getTime() / 1000 + tzOffsetSec()) / DAY_SEC);
}

function dayStartTs(day: number): number {
  return day * DAY_SEC - tzOffsetSec();
}

function dayDate(day: number): Date {
  return new Date(dayStartTs(day) * 1000);
}

function rowTotal(row: UsageDayRow): number {
  return row.input + row.output + row.cache_read + row.cache_write;
}

export function formatTokens(tokens: number): string {
  if (!Number.isFinite(tokens) || tokens <= 0) return "0";
  return new Intl.NumberFormat(getLocale() === "zh" ? "zh-CN" : "en", {
    notation: tokens >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(Math.round(tokens));
}

function shortDate(day: number): string {
  const date = dayDate(day);
  return `${date.getMonth() + 1}/${date.getDate()}`;
}

function rangeStart(range: Range, today: number): number {
  if (range === "today") return today;
  if (range === "week") return today - 6;
  if (range === "days30") return today - 29;
  const now = new Date();
  return localDay(new Date(now.getFullYear(), now.getMonth(), 1));
}

function sumBy(rows: UsageDayRow[], key: (row: UsageDayRow) => string) {
  const map = new Map<string, { total: number; turns: number; provider: string }>();
  for (const row of rows) {
    const name = key(row);
    const entry = map.get(name) ?? { total: 0, turns: 0, provider: row.provider };
    entry.total += rowTotal(row);
    entry.turns += row.turns;
    map.set(name, entry);
  }
  return [...map.entries()]
    .map(([name, entry]) => ({ name, ...entry }))
    .sort((a, b) => b.total - a.total);
}

function dailyTotals(rows: UsageDayRow[], from: number, to: number): number[] {
  const totals = new Array<number>(to - from + 1).fill(0);
  for (const row of rows) {
    if (row.day >= from && row.day <= to) totals[row.day - from] += rowTotal(row);
  }
  return totals;
}

function renderBars(el: HTMLElement, totals: number[], from: number, today: number, labelEvery: number) {
  const peak = Math.max(...totals, 1);
  el.classList.toggle("is-dense", totals.length > 10);
  el.innerHTML = totals
    .map((total, index) => {
      const day = from + index;
      const height = total > 0 ? Math.max(6, Math.round((total / peak) * 100)) : 0;
      const clearOfLast = totals.length - 1 - index >= Math.ceil(labelEvery / 2);
      const label =
        (index % labelEvery === 0 && clearOfLast) || index === totals.length - 1
          ? shortDate(day)
          : "";
      const title = `${shortDate(day)} · ${formatTokens(total)}`;
      return `<div class="usage-bar${day === today ? " is-today" : ""}" title="${escapeHtml(title)}">
        <span class="usage-bar-track"><span class="usage-bar-fill" style="height:${height}%"></span></span>
        <span class="usage-bar-label">${escapeHtml(label)}</span>
      </div>`;
    })
    .join("");
}

function renderShareList(el: HTMLElement, items: ReturnType<typeof sumBy>, sub: (item: { provider: string }) => string) {
  const grand = items.reduce((acc, item) => acc + item.total, 0) || 1;
  el.innerHTML = items
    .slice(0, 8)
    .map((item) => {
      const pct = Math.round((item.total / grand) * 100);
      const subText = sub(item);
      return `<li class="usage-share">
        <div class="usage-share-head">
          <span class="usage-share-name">${escapeHtml(item.name)}${
            subText ? `<small>${escapeHtml(subText)}</small>` : ""
          }</span>
          <span class="usage-share-value">${escapeHtml(formatTokens(item.total))}</span>
        </div>
        <span class="usage-share-track"><span class="usage-share-fill" style="width:${pct}%"></span></span>
      </li>`;
    })
    .join("");
}

export type ServiceUsage = { today: number; month: number };

export function createUsageController(deps: { onShowList: () => void; onLoaded?: () => void }) {
  let rows: UsageDayRow[] = [];
  let range: Range = "week";
  let loading: Promise<void> | null = null;

  /** Tokens billed to one saved service. Older rows only carry its name. */
  function usageForService(id: string, name: string): ServiceUsage | null {
    const today = localDay(new Date());
    const monthStart = rangeStart("month", today);
    const mine = rows.filter((row) =>
      row.provider_id ? row.provider_id === id : row.provider === name,
    );
    if (mine.length === 0) return null;
    const result = { today: 0, month: 0 };
    for (const row of mine) {
      if (row.day === today) result.today += rowTotal(row);
      if (row.day >= monthStart) result.month += rowTotal(row);
    }
    return result.month > 0 ? result : null;
  }

  async function load(): Promise<void> {
    if (!overviewEl) return;
    if (loading) return loading;
    const today = localDay(new Date());
    const from = Math.min(today - 29, rangeStart("month", today));
    loading = tokenUsageByDay({ sinceTs: dayStartTs(from), tzOffsetSec: tzOffsetSec() })
      .then((next) => {
        rows = next;
        renderOverview();
        if (detailViewEl && !detailViewEl.hidden) renderDetail();
        deps.onLoaded?.();
      })
      .catch(() => {
        rows = [];
        renderOverview();
      })
      .finally(() => {
        loading = null;
      });
    return loading;
  }

  function totalSince(from: number): number {
    return rows.filter((row) => row.day >= from).reduce((acc, row) => acc + rowTotal(row), 0);
  }

  function renderOverview(): void {
    if (!overviewEl) return;
    const today = localDay(new Date());
    if (todayEl) todayEl.textContent = formatTokens(totalSince(today));
    if (weekEl) weekEl.textContent = formatTokens(totalSince(today - 6));
    if (monthEl) monthEl.textContent = formatTokens(totalSince(rangeStart("month", today)));
    const empty = rows.length === 0;
    overviewEl.classList.toggle("is-empty", empty);
    if (overviewEmptyEl) overviewEmptyEl.hidden = !empty;
    if (overviewNoteEl) {
      const todayRows = rows.filter((row) => row.day === today);
      const sent = todayRows.reduce((acc, row) => acc + row.input + row.cache_read + row.cache_write, 0);
      const replied = todayRows.reduce((acc, row) => acc + row.output, 0);
      // A short reply still sends the agent's instructions and tool list every time.
      const show = sent > 0 && sent > replied * 5;
      overviewNoteEl.hidden = !show;
      if (show) {
        overviewNoteEl.textContent = t("usage.mostlyPrompt", {
          reply: formatTokens(replied),
          sent: formatTokens(sent),
        });
      }
    }
    if (miniBarsEl) {
      miniBarsEl.hidden = empty;
      if (!empty) renderBars(miniBarsEl, dailyTotals(rows, today - 6, today), today - 6, today, 1);
    }
  }

  function renderDetail(): void {
    const today = localDay(new Date());
    const from = rangeStart(range, today);
    const inRange = rows.filter((row) => row.day >= from && row.day <= today);
    const sum = inRange.reduce(
      (acc, row) => {
        acc.input += row.input;
        acc.output += row.output;
        acc.cache += row.cache_read + row.cache_write;
        acc.turns += row.turns;
        return acc;
      },
      { input: 0, output: 0, cache: 0, turns: 0 },
    );
    const total = sum.input + sum.output + sum.cache;

    rangeEl?.querySelectorAll<HTMLButtonElement>("[data-usage-range]").forEach((chip) => {
      const active = chip.dataset.usageRange === range;
      chip.classList.toggle("is-active", active);
      chip.setAttribute("aria-selected", String(active));
    });
    if (totalEl) totalEl.textContent = formatTokens(total);
    if (totalMetaEl) totalMetaEl.textContent = t("usage.turns", { count: String(sum.turns) });
    if (splitEl) {
      splitEl.innerHTML = [
        [t("usage.input"), sum.input],
        [t("usage.output"), sum.output],
        [t("usage.cache"), sum.cache],
      ]
        .map(
          ([label, value]) =>
            `<div class="usage-split-item"><span>${escapeHtml(String(label))}</span><strong>${escapeHtml(
              formatTokens(Number(value)),
            )}</strong></div>`,
        )
        .join("");
    }

    const empty = inRange.length === 0;
    if (detailEmptyEl) detailEmptyEl.hidden = !empty;
    if (chartEl) {
      chartEl.hidden = empty || range === "today";
      if (!chartEl.hidden) {
        const days = today - from + 1;
        renderBars(chartEl, dailyTotals(rows, from, today), from, today, days > 10 ? 7 : 1);
      }
    }
    for (const el of [byServiceEl, byModelEl, byToolEl]) {
      el?.closest<HTMLElement>(".usage-block")?.toggleAttribute("hidden", empty);
    }
    if (byServiceEl) {
      renderShareList(
        byServiceEl,
        sumBy(inRange, (row) => row.provider || t("usage.unknownService")),
        () => "",
      );
    }
    if (byModelEl) {
      renderShareList(
        byModelEl,
        sumBy(inRange, (row) => row.model || t("usage.unknownModel")),
        (item) => item.provider,
      );
    }
    if (byToolEl) {
      renderShareList(byToolEl, sumBy(inRange, (row) => agentName(row.runtime)), () => "");
    }
  }

  function showDetail(): void {
    if (!listViewEl || !detailViewEl) return;
    listViewEl.hidden = true;
    detailViewEl.hidden = false;
    renderDetail();
    void load();
  }

  function hideDetail(): void {
    if (detailViewEl) detailViewEl.hidden = true;
  }

  function bindEvents(): void {
    openEl?.addEventListener("click", showDetail);
    backEl?.addEventListener("click", () => {
      hideDetail();
      deps.onShowList();
    });
    rangeEl?.addEventListener("click", (event) => {
      const chip = (event.target as HTMLElement | null)?.closest<HTMLButtonElement>(
        "[data-usage-range]",
      );
      const next = chip?.dataset.usageRange as Range | undefined;
      if (!next || next === range) return;
      range = next;
      renderDetail();
    });
    window.addEventListener("focus", () => void load());
  }

  return { load, bindEvents, hideDetail, renderOverview, renderDetail, usageForService };
}
