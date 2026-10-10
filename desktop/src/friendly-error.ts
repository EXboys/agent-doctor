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

/** TeamUps browser-login failures: one plain sentence, no HTML or English dumps. */
export function teamupsLoginFailure(error: unknown): string {
  const lower = errorDetail(error).toLowerCase();
  if (/\(404|\(405|\(501|html page/.test(lower)) {
    return t("resources.mallLoginUnavailable");
  }
  if (/\(5\d\d/.test(lower)) {
    return t("resources.mallLoginServerDown");
  }
  if (
    /error sending request|dns|timed?\s*out|timeout|connection|unreachable|certificate|tls/.test(
      lower,
    )
  ) {
    return t("resources.mallLoginNetwork");
  }
  return t("resources.mallLoginFailed");
}

/** TeamUps mall install — hide English pack-id errors from beginners. */
export function teamupsMallInstallFailure(error: unknown): string {
  const lower = errorDetail(error).toLowerCase();
  if (/pack id|pack slug required|needs a pack/.test(lower)) {
    return t("resources.mallInstallNeedsUpdate");
  }
  if (/401|403|license|rejected/.test(lower)) {
    return t("resources.mallInstallNeedLogin");
  }
  if (/produced no skills|nothing was downloaded|sha256 mismatch|invalid skill zip/.test(lower)) {
    return t("resources.mallInstallBrokenPackage");
  }
  if (/manifest failed \(404|\"detail\":\"not found\"|teamups install failed/i.test(lower)) {
    return t("resources.mallInstallWrongServer");
  }
  if (
    /error sending request|dns|timed?\s*out|timeout|connection|unreachable|certificate|tls/.test(
      lower,
    )
  ) {
    return t("resources.mallInstallNetwork");
  }
  return t("resources.mallInstallFailed");
}

export type ProviderFailureKind =
  | "key"
  | "url"
  | "model"
  | "missing"
  | "balance"
  | "glm_plan"
  | "rate_limit"
  | "unknown";

export type LlmAccountBlock = "balance" | "rate_limit";

/** Provider said the key works but the account cannot answer: no credit, or a temporary limit. */
export function explainLlmAccountBlock(error: unknown): LlmAccountBlock | null {
  const raw = errorDetail(error);
  if (!raw) return null;
  const lower = raw.toLowerCase();
  const balance =
    /"code"\s*:\s*"?1113\b/.test(lower) ||
    /余额不足|无可用资源包|请充值|欠费/.test(raw) ||
    /insufficient[_\s-]*(quota|balance|credit)|exceeded your current quota|billing[_\s-]*hard[_\s-]*limit|account.?balance|no available resource/.test(
      lower,
    );
  if (balance) return "balance";
  if (
    /\b429\b|too many requests|rate[_\s-]*limit|请求过于频繁|请求频率|限流/.test(lower)
  ) {
    return "rate_limit";
  }
  return null;
}

export function accountBlockMessage(kind: LlmAccountBlock): string {
  return kind === "balance"
    ? t("diagnose.flow.accountBalance")
    : t("diagnose.flow.accountRateLimit");
}

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
  const account = explainLlmAccountBlock(raw);

  if (isGlmCodingPlanOnPayg(raw, lower)) {
    return {
      kind: "glm_plan",
      message: t("provider.fail.glmPlan"),
      next: t("provider.fail.glmPlanNext"),
    };
  }

  if (account === "balance") {
    return {
      kind: "balance",
      message: t("provider.fail.balance"),
      next: t("provider.fail.balanceNext"),
    };
  }

  // Verify already tried MiniMax's China and international hosts before giving up.
  if (
    /minimaxi?\.(cn|com|io)/.test(lower) &&
    (status === 401 || /http 401|\b1004\b|\b2049\b|invalid api key/.test(lower))
  ) {
    return {
      kind: "key",
      message: t("provider.fail.minimaxKey"),
      next: t("provider.fail.minimaxKeyNext"),
    };
  }

  if (account === "rate_limit" || status === 429) {
    return {
      kind: "rate_limit",
      message: t("provider.fail.rateLimit"),
      next: t("provider.fail.rateLimitNext"),
    };
  }

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

export type ChatFailureKind =
  | "glm_codex_url"
  | "glm_coding_plan"
  | "geo_blocked"
  | "macos_denied"
  | "account_balance"
  | "rate_limit"
  | "provider_key"
  | "provider_url"
  | "provider_model"
  | "network"
  | "unknown";

export type ChatFailureAction = "provider" | "repair" | "native";

export type ChatFailureExplain = {
  kind: ChatFailureKind;
  message: string;
  next: string;
  actions: ChatFailureAction[];
};

/** A coding-plan key on the pay-as-you-go address. GLM says "no balance"; Qwen and Kimi say the key is invalid. */
function isGlmCodingPlanOnPayg(raw: string, lower: string): boolean {
  const glmPayg =
    /open\.bigmodel\.cn|api\.z\.ai/.test(lower) &&
    /\/api\/paas\/v4/.test(lower) &&
    !/\/api\/coding\//.test(lower);
  if (glmPayg) {
    return (
      /"code"\s*:\s*"?1113\b/.test(lower) ||
      /"code"\s*:\s*"?1315\b/.test(lower) ||
      /编程套餐|coding plan/.test(raw)
    );
  }
  const qwenPayg = /dashscope/.test(lower) && !/coding[-.]dashscope/.test(lower);
  const kimiPayg = /api\.moonshot\.(cn|ai)/.test(lower);
  if (!qwenPayg && !kimiPayg) return false;
  return /\b401\b|\b403\b|invalid api-key|invalid access token|unauthorized|authentication/.test(
    lower,
  );
}

function isGlmCodexResponses404(lower: string): boolean {
  return (
    /paas\/v4\/responses/.test(lower) ||
    (/open\.bigmodel\.cn|api\.z\.ai/.test(lower) &&
      (/unexpected status 404|404 not found|status 404/.test(lower) ||
        /\/responses/.test(lower))) ||
    /codex needs zhipu responses|responses_base_url|chat-completions host.*codex/i.test(lower)
  );
}

function isMacosFileDenied(lower: string): boolean {
  return (
    /operation not permitted/.test(lower) &&
    (/config error|loading default config|os error 1/.test(lower) || /eperm/.test(lower))
  );
}

function isGeoBlockedService(lower: string): boolean {
  return (
    (/api\.anthropic\.com|auth\.openai\.com/.test(lower) &&
      (/403|forbidden|not available in your country|supported countries/.test(lower) ||
        /unable to connect to anthropic|failed to connect to api\.anthropic/.test(lower))) ||
    /not logged in.*codex|codex.*not logged in/.test(lower)
  );
}

/** Classify Codex / Ask stderr and API dumps for in-chat beginner copy. */
export function explainChatFailure(raw: string): ChatFailureExplain | null {
  const text = raw.trim();
  if (!text) return null;
  const lower = text.toLowerCase();
  const account = explainLlmAccountBlock(text);

  if (isGlmCodingPlanOnPayg(text, lower)) {
    return {
      kind: "glm_coding_plan",
      message: t("chat.fail.glmPlan.message"),
      next: t("chat.fail.glmPlan.next"),
      actions: ["provider"],
    };
  }

  if (account === "balance") {
    return {
      kind: "account_balance",
      message: t("chat.fail.balance.message"),
      next: t("chat.fail.balance.next"),
      actions: ["provider"],
    };
  }

  if (account === "rate_limit") {
    return {
      kind: "rate_limit",
      message: t("chat.fail.rateLimit.message"),
      next: t("chat.fail.rateLimit.next"),
      actions: [],
    };
  }

  if (isGlmCodexResponses404(lower)) {
    return {
      kind: "glm_codex_url",
      message: t("chat.fail.glmCodex.message"),
      next: t("chat.fail.glmCodex.next"),
      actions: ["repair", "provider", "native"],
    };
  }

  if (isGeoBlockedService(lower)) {
    return {
      kind: "geo_blocked",
      message: t("chat.fail.geo.message"),
      next: t("chat.fail.geo.next"),
      actions: ["provider", "native"],
    };
  }

  if (isMacosFileDenied(lower)) {
    return {
      kind: "macos_denied",
      message: t("chat.fail.macosDenied.message"),
      next: t("chat.fail.macosDenied.next"),
      actions: [],
    };
  }

  const provider = explainProviderFailure(text);
  const actions: ChatFailureAction[] = ["provider"];
  if (provider.kind === "url" || provider.kind === "unknown") {
    actions.push("repair");
  }
  actions.push("native");

  let kind: ChatFailureKind = "unknown";
  if (provider.kind === "glm_plan") kind = "glm_coding_plan";
  else if (provider.kind === "key" || provider.kind === "missing") kind = "provider_key";
  else if (provider.kind === "url") kind = "provider_url";
  else if (provider.kind === "model") kind = "provider_model";
  else if (
    /reconnecting|network|timeout|connection|unreachable|dns|certificate|tls/.test(lower)
  ) {
    kind = "network";
  } else if (
    /unexpected status|401|403|404|500|502|503|not found|forbidden/.test(lower)
  ) {
    kind = "unknown";
  } else {
    return null;
  }

  return {
    kind,
    message: provider.message,
    next: provider.next,
    actions: [...new Set(actions)],
  };
}

export function formatChatFailureLine(explain: ChatFailureExplain): string {
  return t("provider.fail.combined", {
    message: explain.message,
    next: explain.next,
  })
    .replace(/\s+/g, " ")
    .trim();
}
