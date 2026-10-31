import {
  resolvePermissionSession,
  teamupsChannelAsk,
  type TeamupsChannelQuestion,
} from "../ipc";
import { formatPermissionDetail } from "./format";
import { publishPhoneChats } from "./phone-chats";

type AskMode = "choice" | "line" | "options";

type PendingAsk = {
  jobId: string;
  chatSessionId: string;
  requestId: string;
  backendSessionId: string;
  mode: AskMode;
  options: string[];
  question: string;
  questions: TeamupsChannelQuestion[];
};

const jobsBySession = new Map<string, string>();
const pendingByRequest = new Map<string, PendingAsk>();

export function beginChannelJob(jobId: string, sessionId: string): void {
  jobsBySession.set(sessionId, jobId);
}

export function channelSessionForJob(jobId: string): string | null {
  for (const [sessionId, id] of jobsBySession) {
    if (id === jobId) return sessionId;
  }
  return null;
}

export function channelSessionWaiting(sessionId: string): boolean {
  for (const ask of pendingByRequest.values()) {
    if (ask.chatSessionId === sessionId) return true;
  }
  return false;
}

export function endChannelJob(sessionId: string): void {
  const jobId = jobsBySession.get(sessionId);
  jobsBySession.delete(sessionId);
  if (!jobId) return;
  for (const [requestId, ask] of pendingByRequest) {
    if (ask.jobId === jobId) pendingByRequest.delete(requestId);
  }
}

type ChannelAskInput = {
  chatSessionId: string;
  requestId: string;
  backendSessionId: string;
  toolName: string;
  detail: string;
  inputMode: "choice" | "line" | "secret" | "options";
  inputJson?: string;
};

export function noteChannelAsk(input: ChannelAskInput): void {
  const jobId = jobsBySession.get(input.chatSessionId);
  if (!jobId) return;
  if (input.inputMode === "secret") {
    void teamupsChannelAsk(
      jobId,
      input.requestId,
      "这一步要在电脑上填，飞书里填不安全。",
      false,
    ).catch(() => {});
    return;
  }
  const questions = parseQuestions(input.inputJson);
  const question = questions[0];
  const options = question?.options.map((option) => option.label) ?? [];
  const summary = clip(formatPermissionDetail(input.detail).summary || input.detail || input.toolName);
  const prompt = questions.length > 1
    ? `有 ${questions.length} 件事要选。请在手机上逐项选择；飞书暂时请到电脑上处理。`
    : question
      ? optionPrompt(question.question, options, question.multiSelect)
    : input.inputMode === "line"
      ? `请直接回复：\n${summary}`
      : `要确认这一步：\n${summary}\n\n回复 1 允许\n回复 2 拒绝`;
  pendingByRequest.set(input.requestId, {
    jobId,
    chatSessionId: input.chatSessionId,
    requestId: input.requestId,
    backendSessionId: input.backendSessionId,
    mode: input.inputMode === "options" ? "options" : input.inputMode === "line" ? "line" : "choice",
    options,
    question: question?.question || summary,
    questions,
  });
  void teamupsChannelAsk(
    jobId,
    input.requestId,
    prompt,
    true,
    input.inputMode === "options" ? questions : undefined,
  ).catch(() => {});
  publishPhoneChats();
}

/** 1 / 允许, or 2 / 拒绝. Anything else is a normal sentence. */
export function choiceVerdict(text: string): boolean | null {
  const raw = text.trim();
  if (/^(1|允许|好|可以|是)$/.test(raw)) return true;
  if (/^(2|拒绝|不|不行|否)$/.test(raw)) return false;
  return null;
}

export function pendingChoiceCount(sessionId: string): number {
  let count = 0;
  for (const ask of pendingByRequest.values()) {
    if (ask.chatSessionId === sessionId && ask.mode === "choice") count += 1;
  }
  return count;
}

/** One Feishu reply covers every still-open 允许/拒绝 for this chat. */
export async function takePendingChoices(sessionId: string, text: string): Promise<number> {
  const allow = choiceVerdict(text);
  if (allow === null) return 0;
  const matches = [...pendingByRequest.values()].filter(
    (ask) => ask.chatSessionId === sessionId && ask.mode === "choice",
  );
  if (matches.length === 0) return 0;
  for (const ask of matches) pendingByRequest.delete(ask.requestId);
  await Promise.all(
    matches.map((ask) =>
      resolvePermissionSession({
        sessionId: ask.backendSessionId,
        requestId: ask.requestId,
        allow,
      }),
    ),
  );
  publishPhoneChats();
  return matches.length;
}

export async function applyChannelAnswer(requestId: string, text: string): Promise<void> {
  const current = pendingByRequest.get(requestId);
  if (!current) return;
  const raw = text.trim();
  if (!raw) return;
  if (current.mode === "choice") {
    if (choiceVerdict(raw) === null) {
      void teamupsChannelAsk(current.jobId, current.requestId, "没看懂。回复 1 允许，回复 2 拒绝。", true).catch(() => {});
      return;
    }
    await takePendingChoices(current.chatSessionId, raw);
    return;
  }
  if (current.mode === "options") {
    const answers = structuredAnswers(raw, current.questions);
    if (answers) {
      pendingByRequest.delete(requestId);
      await resolvePermissionSession({
        sessionId: current.backendSessionId,
        requestId: current.requestId,
        allow: true,
        text: JSON.stringify({ answers }),
      });
      publishPhoneChats();
      return;
    }
    const picked = pickOption(raw, current.options);
    if (!picked) {
      void teamupsChannelAsk(
        current.jobId,
        current.requestId,
        current.questions.length > 1
          ? `有 ${current.questions.length} 件事要选。请在手机上逐项选择；飞书暂时请到电脑上处理。`
          : optionPrompt(
              current.question,
              current.options,
              current.questions[0]?.multiSelect === true,
            ),
        true,
        current.questions,
      ).catch(() => {});
      return;
    }
    pendingByRequest.delete(requestId);
    await resolvePermissionSession({
      sessionId: current.backendSessionId,
      requestId: current.requestId,
      allow: true,
      text: JSON.stringify({ answers: { [current.question]: picked } }),
    });
    publishPhoneChats();
    return;
  }
  pendingByRequest.delete(requestId);
  await resolvePermissionSession({
    sessionId: current.backendSessionId,
    requestId: current.requestId,
    allow: true,
    text: raw,
  });
  publishPhoneChats();
}

function optionPrompt(question: string, options: string[], multiSelect = false): string {
  const lines = options.map((label, index) => `${index + 1}. ${label}`);
  const hint = multiSelect
    ? "手机上可以选择多项后统一确认。飞书暂时请到电脑上处理。"
    : "回复序号，或直接回复选项。";
  return `${clip(question)}\n\n${lines.join("\n")}\n\n${hint}`;
}

function pickOption(raw: string, options: string[]): string | null {
  const index = Number(raw);
  if (Number.isInteger(index) && index >= 1 && index <= options.length) return options[index - 1];
  return options.find((label) => label === raw) ?? null;
}

export function structuredAnswers(
  raw: string,
  questions: TeamupsChannelQuestion[],
): Record<string, string> | null {
  if (questions.length === 0 || !raw.startsWith("{")) return null;
  try {
    const value = JSON.parse(raw) as { answers?: unknown };
    if (!value.answers || typeof value.answers !== "object") return null;
    const source = value.answers as Record<string, unknown>;
    const answers: Record<string, string> = {};
    for (const question of questions) {
      const answer = source[question.question];
      const picked = Array.isArray(answer)
        ? answer
            .filter((item): item is string => typeof item === "string")
            .map((item) => item.trim())
            .filter(Boolean)
        : typeof answer === "string" && answer.trim()
          ? question.multiSelect
            ? answer.split(",").map((item) => item.trim()).filter(Boolean)
            : [answer.trim()]
          : [];
      const allowed = new Set(question.options.map((option) => option.label));
      if (
        picked.length === 0 ||
        (!question.multiSelect && picked.length !== 1) ||
        picked.some((item) => !allowed.has(item))
      ) {
        return null;
      }
      answers[question.question] = picked.join(", ");
    }
    return answers;
  } catch {
    return null;
  }
}

function parseQuestions(raw: string | undefined): TeamupsChannelQuestion[] {
  if (!raw?.trim()) return [];
  try {
    const value = JSON.parse(raw) as { questions?: unknown };
    if (!Array.isArray(value.questions)) return [];
    const questions: TeamupsChannelQuestion[] = [];
    for (const item of value.questions) {
      if (!item || typeof item !== "object") continue;
      const record = item as {
        question?: unknown;
        options?: unknown;
        multiSelect?: unknown;
      };
      const question = typeof record.question === "string" ? record.question.trim() : "";
      if (!question) continue;
      const options: { label: string; description?: string }[] = [];
      if (Array.isArray(record.options)) {
        for (const option of record.options) {
          if (!option || typeof option !== "object") continue;
          const optionRecord = option as { label?: unknown; description?: unknown };
          const label =
            typeof optionRecord.label === "string" ? optionRecord.label.trim() : "";
          if (!label) continue;
          const description =
            typeof optionRecord.description === "string"
              ? optionRecord.description.trim()
              : "";
          options.push(description ? { label, description } : { label });
        }
      }
      if (options.length > 0) {
        questions.push({
          question,
          options,
          multiSelect: record.multiSelect === true,
        });
      }
    }
    return questions;
  } catch {
    return [];
  }
}

function clip(value: string): string {
  const text = value.replace(/\s+/g, " ").trim();
  return text.length > 400 ? `${text.slice(0, 400)}…` : text;
}
