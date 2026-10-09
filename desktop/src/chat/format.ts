import { t } from "../i18n";

/** One sentence for a command the user should allow without reading a shell line. */
export function plainPermissionSummary(raw: string): string | null {
  const lower = raw.toLowerCase();
  if (
    lower.includes("rm -f") ||
    lower.includes("rm -rf") ||
    lower.includes("rm -fr") ||
    lower.includes("safer approach") ||
    /\brm\b[^\n]{0,40}-[a-z]*f/.test(lower)
  ) {
    return t("chat.permissionTempFiles");
  }
  if (
    lower.includes("ffmpeg") ||
    lower.includes("剪视频") ||
    /\.(mp4|mov|mkv|m4v|avi)\b/.test(lower)
  ) {
    return t("chat.permissionVideo");
  }
  return null;
}

/** Prefer a short human summary; keep the full command for the expandable row. */
export function formatPermissionDetail(raw: string): { summary: string; full: string } {
  const full = raw.trim();
  if (!full) {
    return { summary: "", full: "" };
  }
  const plain = plainPermissionSummary(full);
  if (plain) {
    return { summary: plain, full };
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

/** Short name on the first line; command or path on the following lines. */
export function splitToolActivity(text: string): { summary: string; detail: string } {
  const stripped = text
    .replace(/^(?:调用工具|call(?:ing)? tool)\s*/i, "")
    .replace(/[….\s]+$/g, "")
    .trim();
  const breakAt = stripped.indexOf("\n");
  const head = (breakAt === -1 ? stripped : stripped.slice(0, breakAt)).trim();
  const detail = breakAt === -1 ? "" : stripped.slice(breakAt + 1).trim();
  const aliases = head.split("__").filter(Boolean);
  const summary = (aliases.length > 1 ? aliases[aliases.length - 1] : head) || text.trim();
  return { summary, detail };
}

export function cleanToolLabel(text: string): string {
  return splitToolActivity(text).summary;
}

export function toolSignature(text: string): string {
  const { summary, detail } = splitToolActivity(text);
  return `${summary}\n${detail}`.toLocaleLowerCase();
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
    (lower.includes("cua-driver") &&
      (lower.includes("is available") ||
        lower.includes("update with") ||
        lower.includes("release notes") ||
        lower.includes("mcp launched") ||
        lower.includes("tcc") ||
        lower.includes("auto-launching") ||
        lower.includes("proxying mcp"))) ||
    lower.includes("openclaw gateway run") ||
    lower.includes("openclaw gateway status") ||
    lower.startsWith("gateway target:") ||
    lower.startsWith("source: local loopback") ||
    lower.startsWith("bind: loopback") ||
    (lower.startsWith("config:") && lower.includes("openclaw.json"))
  );
}

/** A short closing question that is explicitly asking to continue. */
export function looksLikeChoiceQuestion(text: string): boolean {
  const trimmed = text.trim();
  if (!trimmed) return false;
  const plain = trimmed.replace(/\s+/g, " ");
  const match = plain.match(/[^。！？!?]{0,40}[？?]\s*$/);
  if (!match) return false;
  const ask = match[0].trim();
  if (
    /有什么(可以|需要|能)?(帮|帮忙|做)/.test(ask) ||
    /需要我(帮忙|帮你)/.test(ask) ||
    /随时(找我|叫我|告诉我)/.test(ask)
  ) {
    return false;
  }
  const lower = ask.toLowerCase();
  return (
    /(要不要继续|还要继续|继续吗|要我继续|需要我继续|确认继续|接着做吗)/.test(ask) ||
    /(shall i continue|should i continue|want me to continue|proceed\?)/i.test(lower)
  );
}

/** One-line surfaces (the island list) show the words, not the marks. */
export function stripMarkdown(text: string): string {
  return text
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/!\[[^\]]*]\([^)]*\)/g, "")
    .replace(/\[([^\]]+)]\([^)]*\)/g, "$1")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/__([^_]+)__/g, "$1")
    .replace(/(^|[\s])\*([^*\n]+)\*(?=$|[\s])/g, "$1$2")
    .replace(/`/g, "")
    .replace(/\*\*/g, "")
    .replace(/^\s*[-*+]\s+/gm, "")
    .replace(/\s+/g, " ")
    .trim();
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
