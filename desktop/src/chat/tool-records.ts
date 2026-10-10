import { mergeToolStep, toolStepText } from "./format";
import type { ChatMessage, ToolStep } from "./types";

const SAVED_OUTPUT_CHARS = 1_500;
const SAVED_LINES = 80;

/** The saved copy keeps the row and a taste of the output, not the whole log. */
function slim(step: ToolStep): ToolStep {
  const out = { ...step };
  if (out.output && out.output.length > SAVED_OUTPUT_CHARS) {
    out.output = `${out.output.slice(0, SAVED_OUTPUT_CHARS)}\n…`;
  }
  if (out.added_lines) out.added_lines = out.added_lines.slice(0, SAVED_LINES);
  if (out.deleted_lines) out.deleted_lines = out.deleted_lines.slice(0, SAVED_LINES);
  return out;
}

/**
 * Put a step on this turn's tool messages. A result merges into the message with the
 * same id; a new step takes over the plain status message just saved for it.
 */
export function storeToolStep(
  messages: ChatMessage[],
  step: ToolStep,
  streamingAssistantId: string | null,
  newId: () => string,
): boolean {
  let userAt = -1;
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (messages[i]?.role === "user") {
      userAt = i;
      break;
    }
  }
  for (let i = messages.length - 1; i > userAt; i -= 1) {
    const message = messages[i];
    if (message?.role === "tool" && message.step?.id === step.id) {
      message.step = slim(mergeToolStep(message.step, step));
      return true;
    }
  }
  if (!step.kind) return false;
  const tailAt = messages.length - 1;
  const tail = messages[tailAt];
  const anchorAt = tail?.role === "assistant" ? tailAt - 1 : tailAt;
  const anchor = messages[anchorAt];
  if (anchorAt > userAt && anchor?.role === "tool" && !anchor.step) {
    anchor.step = slim(step);
    return true;
  }
  const message: ChatMessage = {
    id: newId(),
    role: "tool",
    content: toolStepText(step),
    at: Date.now(),
    step: slim(step),
  };
  if (tail?.role === "assistant" && tail.id === streamingAssistantId) {
    messages.splice(tailAt, 0, message);
  } else {
    messages.push(message);
  }
  return true;
}
