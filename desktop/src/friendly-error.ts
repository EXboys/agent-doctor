import { t } from "./i18n";

export function errorDetail(error: unknown): string {
  return String(error ?? "").trim();
}

/**
 * Beginner-safe error line: plain sentence first, raw detail only as a second line.
 * Paths / English dumps must never be the only explanation.
 */
export function withErrorDetail(message: string, error: unknown): string {
  const detail = errorDetail(error);
  if (!detail || detail === "unknown" || message.includes(detail)) {
    return message;
  }
  return t("error.withDetail", { message, detail });
}
