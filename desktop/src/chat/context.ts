import type { ChatAttachment, ChatSession } from "./types";
import { MAX_CONTEXT_MESSAGES } from "./types";

export type ImageReading = {
  name: string;
  text: string;
};

export function attachmentSummary(attachments: ChatAttachment[] | undefined): string {
  if (!attachments?.length) return "";
  return attachments.map((item) => `- ${item.name}`).join("\n");
}

export function imageReadingBlock(readings: ImageReading[] | undefined): string {
  const useful = (readings ?? []).filter((item) => item.text.trim());
  if (useful.length === 0) return "";
  const body = useful
    .map((item) => `--- ${item.name} ---\n${item.text.trim()}`)
    .join("\n\n");
  return (
    "The user attached pictures. The current model may not see images. " +
    "Text already read from those pictures on this computer:\n\n" +
    body
  );
}

export function buildPromptWithHistory(
  userText: string,
  attachments: ChatAttachment[],
  session: ChatSession,
  readings?: ImageReading[],
): string {
  const responseStyle =
    "Response style: answer the user directly and concisely. Lead with the result. " +
    "Use short sections or bullets only when they improve clarity. Do not narrate hidden reasoning, " +
    "routine progress, tool-selection decisions, retries, or permission flow. Do not repeat the request.";
  // Native resume already carries thread history — only send this turn.
  if (session.runtimeThreadId?.trim()) {
    const parts: string[] = [responseStyle];
    const pictureText = imageReadingBlock(readings);
    if (pictureText) {
      parts.push(pictureText);
    } else if (attachments.length > 0) {
      parts.push(
        `Attached local files for this turn (read them with your tools if needed):\n${attachmentSummary(attachments)}`,
      );
    }
    parts.push(userText);
    return parts.join("\n\n");
  }

  const prior = session.messages
    .filter((m) => m.role === "user" || m.role === "assistant")
    .filter((m) => m.content.trim() || m.attachments?.length)
    .slice(0, -1)
    .slice(-MAX_CONTEXT_MESSAGES);

  const parts: string[] = [responseStyle];
  if (prior.length > 0) {
    const transcript = prior
      .map((m) => {
        const body = m.content.trim() || "(attachments only)";
        const files = attachmentSummary(m.attachments);
        return files
          ? `${m.role === "user" ? "User" : "Assistant"}: ${body}\nAttachments:\n${files}`
          : `${m.role === "user" ? "User" : "Assistant"}: ${body}`;
      })
      .join("\n\n");
    parts.push(`Conversation so far:\n\n${transcript}`);
  }

  const pictureText = imageReadingBlock(readings);
  if (pictureText) {
    parts.push(pictureText);
  } else if (attachments.length > 0) {
    parts.push(
      `Attached local files for this turn (read them with your tools if needed):\n${attachmentSummary(attachments)}`,
    );
  }

  parts.push(`User: ${userText}\n\nAssistant:`);
  return parts.join("\n\n");
}

/** Rough token estimate — CJK denser than ASCII. */
export function estimateTokens(text: string): number {
  let cjk = 0;
  let other = 0;
  for (const ch of text) {
    if ((ch.codePointAt(0) ?? 0) > 0xff) cjk += 1;
    else other += 1;
  }
  return Math.max(1, Math.ceil(cjk / 1.5 + other / 4));
}

export function contextLimitForModel(model: string | null | undefined): number {
  const m = (model ?? "").toLowerCase();
  if (!m) return 128_000;
  // Long-context families first.
  if (m.includes("gemini")) return 1_000_000;
  if (m.includes("haiku") || m.includes("claude")) return 200_000;
  if (
    m.includes("gpt-5.6") ||
    m.includes("gpt-5.4") ||
    m.includes("gpt-4.1") ||
    m.includes("o3") ||
    m.includes("o4-mini")
  ) {
    return 128_000;
  }
  // DeepSeek / Qwen / domestic OpenAI-compatible chat models are typically ~128K.
  if (
    m.includes("deepseek") ||
    m.includes("qwen") ||
    m.includes("kimi") ||
    m.includes("moonshot") ||
    m.includes("glm") ||
    m.includes("minimax")
  ) {
    return 128_000;
  }
  return 128_000;
}

export function sessionContextText(session: ChatSession, draft = ""): string {
  const chunks: string[] = [];
  for (const message of session.messages) {
    if (message.role !== "user" && message.role !== "assistant" && message.role !== "meta") {
      continue;
    }
    if (message.content.trim()) chunks.push(message.content);
    if (message.attachments?.length) {
      chunks.push(message.attachments.map((a) => a.name).join("\n"));
    }
  }
  const trimmedDraft = draft.trim();
  if (trimmedDraft) chunks.push(trimmedDraft);
  return chunks.join("\n");
}

export function contextUsagePercent(
  session: ChatSession,
  draft = "",
  model: string | null | undefined,
): number {
  const used = estimateTokens(sessionContextText(session, draft));
  if (used <= 0) return 0;
  const limit = contextLimitForModel(model);
  const pct = Math.min(100, Math.round((used / Math.max(limit, 1)) * 100));
  // Keep a visible sliver once there is real chat content (short threads vs large windows).
  return Math.max(1, pct);
}
