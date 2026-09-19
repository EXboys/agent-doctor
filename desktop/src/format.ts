import { getLocale, t } from "./i18n";

export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export function formatTime(date: Date): string {
  const locale = getLocale() === "zh" ? "zh-CN" : "en-US";
  return date.toLocaleTimeString(locale, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function formatRate(value: number | null | undefined): string {
  if (value == null || Number.isNaN(value)) return t("skills.na");
  return `${Math.round(value * 100)}%`;
}

export function formatCount(value: number | null | undefined): string {
  if (value == null) return t("skills.na");
  return String(value);
}
