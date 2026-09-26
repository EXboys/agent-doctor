/** Prefer a short human summary; keep the full command for the expandable row. */
export function formatPermissionDetail(raw: string): { summary: string; full: string } {
  const full = raw.trim();
  if (!full) {
    return { summary: "", full: "" };
  }
  const unfenced = full
    .replace(/^```(?:json)?\s*/i, "")
    .replace(/\s*```$/i, "")
    .trim();
  if (unfenced.startsWith("{") && unfenced.endsWith("}")) {
    try {
      const value = JSON.parse(unfenced) as Record<string, unknown>;
      const description =
        typeof value.description === "string" ? value.description.trim() : "";
      const command = typeof value.command === "string" ? value.command.trim() : "";
      if (description || command) {
        return {
          summary:
            description ||
            (command.replace(/\s+/g, " ").length > 96
              ? `${command.replace(/\s+/g, " ").slice(0, 96)}…`
              : command.replace(/\s+/g, " ")),
          full: command || unfenced,
        };
      }
    } catch {
      // fall through
    }
  }
  const oneLine = full.replace(/\s+/g, " ");
  return {
    summary: oneLine.length > 96 ? `${oneLine.slice(0, 96)}…` : oneLine,
    full,
  };
}

export function looksLikeToolPayloadJson(text: string): boolean {
  const trimmed = text.trim();
  const unfenced = trimmed
    .replace(/^```(?:json)?\s*/i, "")
    .replace(/\s*```$/i, "")
    .trim();
  if (!unfenced.startsWith("{") || !unfenced.endsWith("}")) {
    return false;
  }
  try {
    const value = JSON.parse(unfenced) as Record<string, unknown>;
    if (!value || typeof value !== "object" || Array.isArray(value)) {
      return false;
    }
    const hasToolShape =
      typeof value.command === "string" ||
      typeof value.description === "string" ||
      typeof value.tool === "string" ||
      typeof value.name === "string";
    return hasToolShape && !("role" in value) && !("content" in value);
  } catch {
    return false;
  }
}

export function activityKind(phase: string): "tool" | "think" | "write" | "info" | "error" {
  if (phase === "tool" || phase === "command" || phase === "permission") return "tool";
  if (phase === "thinking" || phase === "reasoning") return "think";
  if (phase === "writing" || phase === "streaming") return "write";
  if (phase === "error") return "error";
  return "info";
}

/** Lifecycle chatter that belongs in the header live pill, not the transcript. */
export function isQuietPhase(phase: string): boolean {
  return phase === "writing" || phase === "streaming" || phase === "info" || phase === "done";
}

export function cleanToolLabel(text: string): string {
  const cleaned = text
    .replace(/^(?:调用工具|call(?:ing)? tool)\s*/i, "")
    .replace(/[….\s]+$/g, "")
    .trim();
  const aliases = cleaned.split("__").filter(Boolean);
  return aliases.length > 1 ? aliases[aliases.length - 1] : cleaned || text;
}

export function toolSignature(text: string): string {
  return cleanToolLabel(text).toLocaleLowerCase();
}

export function isQuietStderr(line: string): boolean {
  const text = line.trim();
  const lower = text.toLowerCase();
  return (
    /^session_id:/i.test(text) ||
    /^resume this session/i.test(text) ||
    /resumed session/i.test(text) ||
    lower.includes("unrecognized_model") ||
    lower.includes("unrecognized model") ||
    lower.startsWith("[agent/embedded]") ||
    lower.includes("preserved orphaned user message") ||
    lower.includes("network connection was interrupted") ||
    lower.includes("transient same-model retry") ||
    lower.startsWith("[secrets]") ||
    lower.includes("secrets.resolve unavailable") ||
    lower.includes("resolved command secrets locally") ||
    lower.includes("openclaw gateway run") ||
    lower.includes("openclaw gateway status") ||
    lower.startsWith("gateway target:") ||
    lower.startsWith("source: local loopback") ||
    lower.startsWith("bind: loopback") ||
    (lower.startsWith("config:") && lower.includes("openclaw.json"))
  );
}

/** Only yes/no confirmations — not open greetings like「有什么可以帮你的吗？」. */
export function looksLikeChoiceQuestion(text: string): boolean {
  const trimmed = text.trim();
  if (!trimmed) return false;
  const plain = trimmed.replace(/\s+/g, " ");
  const lower = plain.toLowerCase();
  if (
    /有什么(可以|需要|能)?(帮|帮忙|做)/.test(plain) ||
    /需要我(做|帮忙|帮你)/.test(plain) ||
    /随时(找我|叫我|告诉我)/.test(plain) ||
    /how can i help|anything (i can|you need)|what can i (do|help)/i.test(lower)
  ) {
    return false;
  }
  const tail = plain.slice(-120);
  return (
    /(要不要|要我|是否|继续吗|可以吗|好吗|行吗|确认一下|选一个|选哪|选哪个)/.test(tail) ||
    /(shall i|should i|want me to|would you like me to|continue\?|proceed\?)/i.test(tail)
  );
}

export function preferPlainSummary(summary: string): string {
  const trimmed = summary.trim();
  if (!trimmed) return "";
  if (trimmed.startsWith("{")) {
    try {
      const parsed = JSON.parse(trimmed) as { result?: unknown };
      if (typeof parsed.result === "string" && parsed.result.trim()) {
        return parsed.result.trim();
      }
    } catch {
      for (const line of trimmed.split("\n").reverse()) {
        try {
          const parsed = JSON.parse(line) as { type?: string; result?: unknown; is_error?: boolean };
          if (
            parsed.type === "result" &&
            !parsed.is_error &&
            typeof parsed.result === "string" &&
            parsed.result.trim()
          ) {
            return parsed.result.trim();
          }
        } catch {
          /* continue */
        }
      }
    }
    return "";
  }
  if (/^claude-code (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  if (/^codex (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  if (/^hermes (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  if (/^openclaw (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  if (/^deepseek-harness (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  return trimmed;
}

export function shortCwdLabel(cwd: string): string {
  const trimmed = cwd.trim();
  if (!trimmed || trimmed === "—") return "—";
  const parts = trimmed.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] || trimmed;
}
