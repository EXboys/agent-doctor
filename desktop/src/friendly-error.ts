import { t, type MessageKey } from "./i18n";

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

export type ProviderFailureKind = "key" | "url" | "model" | "missing" | "unknown";

export type ProviderFailureExplain = {
  kind: ProviderFailureKind;
  /** One plain sentence naming the problem. */
  message: string;
  /** What to tap / fill next. */
  next: string;
};

function stripProviderNoise(raw: string): string {
  return raw
    .replace(/^provider connectivity check failed:\s*/i, "")
    .replace(/^Anthropic endpoint (rejected key|error):\s*/i, "")
    .replace(/^Anthropic request failed:\s*/i, "")
    .trim();
}

/**
 * Classify personal-provider verify/apply failures into beginner copy.
 * Prefer statusCode when present; otherwise match common English dumps.
 */
export function explainProviderFailure(
  error: unknown,
  opts?: { statusCode?: number | null },
): ProviderFailureExplain {
  const status = opts?.statusCode ?? null;
  const raw = stripProviderNoise(errorDetail(error));
  const lower = raw.toLowerCase();

  if (
    status === 401 ||
    status === 403 ||
    /unauthorized|forbidden|invalid.?api.?key|incorrect.?api.?key|authentication|auth.?fail|rejected key|401|403/.test(
      lower,
    )
  ) {
    return {
      kind: "key",
      message: t("provider.fail.key"),
      next: t("provider.fail.keyNext"),
    };
  }

  if (
    /model.?not.?found|invalid.?model|unknown.?model|does not exist.*model|no such model|model_not_found/.test(
      lower,
    )
  ) {
    return {
      kind: "model",
      message: t("provider.fail.model"),
      next: t("provider.fail.modelNext"),
    };
  }

  if (/api key must not be empty|key must not be empty|missing.*key/.test(lower)) {
    return {
      kind: "missing",
      message: t("provider.fail.missingKey"),
      next: t("provider.fail.missingKeyNext"),
    };
  }

  if (/model must not be empty|missing.*model/.test(lower)) {
    return {
      kind: "missing",
      message: t("provider.fail.missingModel"),
      next: t("provider.fail.missingModelNext"),
    };
  }

  if (
    status === 404 ||
    /enotfound|getaddrinfo|name.?resolution|econnrefused|connection.?refused|timed?\s*out|timeout|network|unreachable|ssl|certificate|tls|dns|no models endpoint|request failed|failed to fetch|http 404|404/.test(
      lower,
    )
  ) {
    return {
      kind: "url",
      message: t("provider.fail.url"),
      next: t("provider.fail.urlNext"),
    };
  }

  return {
    kind: "unknown",
    message: t("provider.fail.unknown"),
    next: t("provider.fail.unknownNext"),
  };
}

/** One calm card line: problem + what to do. Never dumps URL/JSON by default. */
export function formatProviderFailure(
  error: unknown,
  opts?: { statusCode?: number | null },
): string {
  const explained = explainProviderFailure(error, opts);
  return t("provider.fail.combined", {
    message: explained.message,
    next: explained.next,
  }).replace(/\s+/g, " ").trim();
}

/** Generic beginner failure: fixed message + classified next step; no raw dumps. */
export function withProviderFailure(messageKey: MessageKey, error: unknown): string {
  const explained = explainProviderFailure(error);
  const message = t(messageKey);
  // Prefer the classified one-card line; fall back to messageKey + next if kind is unknown
  // and the key already names a different action (e.g. save failed).
  if (messageKey === "personal.verifyFailed" || messageKey === "diagnose.flow.verifyFailed") {
    return formatProviderFailure(error);
  }
  return `${message} ${explained.next}`.replace(/\s+/g, " ").trim();
}
