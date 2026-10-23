export type QuestionSource = {
  id: string;
  role: string;
  content: string;
  attachments?: { name?: string }[];
};

export type QuestionMark = {
  id: string;
  title: string;
  /** Plain start of the reply. Empty when this question has none. */
  reply: string;
  /** The newest question is still waiting. */
  pending: boolean;
};

const EXCERPT = 180;

/** Which question the rail should mark. At the end of the chat, that is the last one. */
export function currentQuestionId(
  bubbles: readonly { id: string; top: number }[],
  view: { top: number; atEnd: boolean },
): string | null {
  if (bubbles.length === 0) return null;
  if (view.atEnd) return bubbles[bubbles.length - 1]!.id;
  const edge = view.top + 24;
  let current = bubbles[0]!.id;
  for (const bubble of bubbles) {
    if (bubble.top <= edge) current = bubble.id;
  }
  return current;
}

/** One mark per question that has words or a file name. Order follows the chat. */
export function questionMarks(messages: QuestionSource[]): QuestionMark[] {
  const marks: QuestionMark[] = [];
  for (let i = 0; i < messages.length; i += 1) {
    const message = messages[i];
    if (message.role !== "user") continue;
    const title = questionTitle(message);
    if (!title) continue;
    let reply = "";
    for (let j = i + 1; j < messages.length; j += 1) {
      const next = messages[j];
      if (next.role === "user") break;
      if (next.role === "assistant" && next.content.trim()) {
        reply = excerpt(next.content);
        break;
      }
    }
    marks.push({ id: message.id, title, reply, pending: false });
  }
  const last = marks[marks.length - 1];
  if (last && !last.reply) last.pending = true;
  return marks;
}

function questionTitle(message: QuestionSource): string {
  const line = message.content
    .split("\n")
    .map((part) => part.trim())
    .find(Boolean);
  if (line) return line;
  return message.attachments?.find((item) => item.name?.trim())?.name?.trim() ?? "";
}

function excerpt(text: string): string {
  const plain = text
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/!\[[^\]]*]\([^)]*\)/g, " ")
    .replace(/\[[^\]]*]\([^)]*\)/g, " ")
    .replace(/[#>*_`]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  return plain.slice(0, EXCERPT);
}
