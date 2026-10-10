import { getLocale } from "../i18n";
import { splitToolActivity } from "./format";
import type { PictureTurn } from "./picture-route";
import type { ChatAttachment, ChatSession } from "./types";
import { MAX_CONTEXT_MESSAGES } from "./types";

const STOPPED_STEP_CHARS = 160;
const STOPPED_MAX_STEPS = 20;
const STOPPED_REPLY_CHARS = 1200;

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}…` : text;
}

/**
 * What the stopped turn already did. Without it a follow-up like “continue”
 * reaches a model that never saw the tool work, so it starts over.
 */
export function stoppedTurnNote(session: ChatSession, withReply: boolean): string {
  const status = session.interrupted?.status;
  if (!status) return "";
  const userAt = session.messages.flatMap((m, i) => (m.role === "user" ? [i] : []));
  if (userAt.length < 2) return "";
  const turn = session.messages.slice(userAt[userAt.length - 2] + 1, userAt[userAt.length - 1]);

  const steps = turn
    .filter((m) => m.role === "tool" && m.content.trim())
    .map((m) => {
      const { summary, detail } = splitToolActivity(m.content);
      const name = summary.replace(/[….\s]+$/, "");
      const firstDetail = detail.split("\n")[0]?.trim() ?? "";
      return clip(firstDetail ? `${name}: ${firstDetail}` : name, STOPPED_STEP_CHARS);
    });
  const reply = withReply
    ? turn
        .filter((m) => m.role === "assistant")
        .map((m) => m.content.trim())
        .filter(Boolean)
        .join("\n\n")
    : "";
  if (steps.length === 0 && !reply) return "";

  const why =
    status === "timed_out"
      ? "it went too long without any progress and was stopped"
      : status === "cancelled"
        ? "the user stopped it"
        : "it hit an error";
  const parts = [`The previous turn did not finish: ${why}.`];
  if (steps.length > 0) {
    const kept = steps.slice(-STOPPED_MAX_STEPS);
    const skipped = steps.length - kept.length;
    const lines = kept.map((step) => `- ${step}`).join("\n");
    parts.push(
      `Steps it already took${skipped > 0 ? ` (last ${kept.length} of ${steps.length})` : ""}:\n${lines}`,
    );
  }
  if (reply) {
    const tail = reply.length > STOPPED_REPLY_CHARS ? `…${reply.slice(-STOPPED_REPLY_CHARS)}` : reply;
    parts.push(`What it had written before it stopped:\n${tail}`);
  }
  parts.push(
    "If the user asks to continue, pick up where it stopped. Check the current state first, " +
      "keep the work already done, and do not start the task over. " +
      "If a step was cut off mid-way, redo only that step. " +
      "If a long command caused the stop, run it in a way that prints progress or finishes sooner.",
  );
  return parts.join("\n\n");
}

export type ImageReading = {
  name: string;
  text: string;
};

export function attachmentSummary(attachments: ChatAttachment[] | undefined): string {
  if (!attachments?.length) return "";
  return attachments.map((item) => `- ${item.name}`).join("\n");
}

export function imageReadingBlock(
  readings: ImageReading[] | undefined,
  picturesAttached = false,
): string {
  const useful = (readings ?? []).filter((item) => item.text.trim());
  if (useful.length === 0) return "";
  const body = useful
    .map((item) => `--- ${item.name} ---\n${item.text.trim()}`)
    .join("\n\n");
  const lead = picturesAttached
    ? "Text read on this computer from the user's pictures:\n\n"
    : "The user attached pictures. The current model may not see images. " +
      "Text already read from those pictures on this computer:\n\n";
  return lead + body;
}

/** This turn's attachments as the model should understand them. */
export function turnAttachmentBlock(
  attachments: ChatAttachment[],
  pictures?: PictureTurn,
): string {
  const parts: string[] = [];
  const files = pictures ? attachments.filter((item) => item.kind !== "image") : attachments;
  if (pictures?.sentNames.length) {
    parts.push(
      `The user attached ${pictures.sentNames.length === 1 ? "a picture" : "pictures"} to this message ` +
        `(${pictures.sentNames.join(", ")}). They are included in this message: look at them directly. ` +
        "Do not search the disk for them.",
    );
  }
  const pictureText = imageReadingBlock(pictures?.readings, Boolean(pictures?.sentNames.length));
  if (pictureText) parts.push(pictureText);
  if (pictures?.unseenNames.length) {
    parts.push(
      `The user also attached pictures you cannot see, and no words were found in them ` +
        `(${pictures.unseenNames.join(", ")}). In one short sentence, tell the user you cannot see ` +
        "these pictures and suggest switching to an assistant that can see pictures. " +
        "Do not search the disk for them.",
    );
  }
  if (files.length > 0) {
    parts.push(
      "Attached local files for this turn (read them with your tools if needed):\n" +
        files.map((item) => `- ${item.name}: ${item.path}`).join("\n"),
    );
  }
  return parts.join("\n\n");
}

export function buildPromptWithHistory(
  userText: string,
  attachments: ChatAttachment[],
  session: ChatSession,
  pictures?: PictureTurn,
): string {
  const useChinese = getLocale() === "zh" || /[\u3400-\u9fff]/.test(userText);
  const responseStyle =
    "Response style: answer the user directly and concisely. Lead with the result. " +
    "Use short sections or bullets only when they improve clarity. Do not narrate hidden reasoning, " +
    "routine progress, tool-selection decisions, retries, or permission flow. Do not repeat the request." +
    (useChinese
      ? " Reply in Simplified Chinese. If this runtime exposes visible thinking or reasoning text, that visible text must also be in Simplified Chinese; do not output English thinking."
      : "");
  // Native resume already carries thread history — only send this turn.
  if (session.runtimeThreadId?.trim()) {
    const parts: string[] = [responseStyle];
    // A killed turn may not have saved its last words in the native thread.
    const stopped = stoppedTurnNote(session, true);
    if (stopped) parts.push(stopped);
    const attached = turnAttachmentBlock(attachments, pictures);
    if (attached) parts.push(attached);
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
  const stopped = stoppedTurnNote(session, false);
  if (stopped) parts.push(stopped);

  const attached = turnAttachmentBlock(attachments, pictures);
  if (attached) parts.push(attached);

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
