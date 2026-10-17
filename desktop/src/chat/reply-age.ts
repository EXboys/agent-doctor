import { getLocale, t } from "../i18n";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** Relative time under a reply, in the current language. Older than a week shows the date. */
export function replyAgeLabel(at: number, now = Date.now()): string {
  if (!Number.isFinite(at) || at <= 0) return "";
  const elapsed = Math.max(0, now - at);
  const minutes = Math.floor(elapsed / MINUTE);
  if (minutes < 1) return t("chat.replyJustNow");
  if (minutes < 60) return t("chat.replyMinutesAgo", { n: String(minutes) });
  const hours = Math.floor(elapsed / HOUR);
  if (hours < 24) return t("chat.replyHoursAgo", { n: String(hours) });
  const days = Math.floor(elapsed / DAY);
  if (days < 7) return t("chat.replyDaysAgo", { n: String(days) });
  return replyExactLabel(at);
}

/** Clock time for the hover label, in the current language. */
export function replyExactLabel(at: number): string {
  if (!Number.isFinite(at) || at <= 0) return "";
  const locale = getLocale() === "zh" ? "zh-CN" : "en";
  const date = new Date(at);
  return new Intl.DateTimeFormat(locale, {
    year: date.getFullYear() === new Date().getFullYear() ? undefined : "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

export function paintReplyAge(node: HTMLTimeElement, at: number | undefined, streaming: boolean): void {
  if (streaming || at == null || !Number.isFinite(at) || at <= 0) {
    node.hidden = true;
    node.textContent = "";
    delete node.dataset.at;
    node.removeAttribute("datetime");
    node.removeAttribute("title");
    return;
  }
  node.hidden = false;
  node.dataset.at = String(at);
  node.dateTime = new Date(at).toISOString();
  node.textContent = replyAgeLabel(at);
  node.title = replyExactLabel(at);
}

export function refreshReplyAges(root: ParentNode): void {
  root.querySelectorAll<HTMLTimeElement>("time.chat-msg-time").forEach((node) => {
    const at = Number(node.dataset.at);
    if (!node.dataset.at || !Number.isFinite(at)) return;
    const label = replyAgeLabel(at);
    if (node.textContent !== label) node.textContent = label;
    const exact = replyExactLabel(at);
    if (node.title !== exact) node.title = exact;
  });
}
