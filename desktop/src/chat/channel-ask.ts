import {
  resolvePermissionSession,
  teamupsChannelAsk,
  type TeamupsChannelQuestion,
} from "../ipc";
import { formatPermissionDetail } from "./format";
import { publishPhoneChats } from "./phone-chats";
import {
  channelOptionPicks,
  parseAskFormQuestions,
  questionShowsTextInput,
  type QuestionFormQuestion,
} from "./question-form-core";
import { selectedAnswer, splitMultiSelectAnswer } from "./question-card-answer";

type AskMode = "choice" | "line" | "options";

type PendingAsk = {
  jobId: string;
  chatSessionId: string;
  requestId: string;
  backendSessionId: string;
  mode: AskMode;
  options: string[];
  question: string;
  questions: QuestionFormQuestion[];
};

const jobsBySession = new Map<string, string>();
const chatBySession = new Map<string, string>();
const pendingByRequest = new Map<string, PendingAsk>();

export function beginChannelJob(jobId: string, sessionId: string, chat = ""): void {
  jobsBySession.set(sessionId, jobId);
  const key = chat.trim();
  if (key) chatBySession.set(sessionId, key);
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

export function endChannelJob(sessionId: string, jobId?: string): void {
  const current = jobsBySession.get(sessionId);
  if (!current) return;
  // A newer sentence in this chat already replaced the map. Leave that one alone.
  if (jobId && current !== jobId) return;
  jobsBySession.delete(sessionId);
  chatBySession.delete(sessionId);
  const jobIdToClear = current;
  for (const [requestId, ask] of pendingByRequest) {
    if (ask.jobId === jobIdToClear) pendingByRequest.delete(requestId);
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

/** The phone site renders this exact shape; extra model fields stay local. */
function channelQuestionPayload(questions: QuestionFormQuestion[]): TeamupsChannelQuestion[] {
  return questions.map((question) => ({
    question: question.question,
    options: question.options.map((option) =>
      option.description ? { label: option.label, description: option.description } : { label: option.label },
    ),
    multiSelect: question.multiSelect,
  }));
}

export function noteChannelAsk(input: ChannelAskInput): void {
  const jobId = jobsBySession.get(input.chatSessionId);
  if (!jobId) return;
  const chat = chatBySession.get(input.chatSessionId) ?? "";
  if (input.inputMode === "secret") {
    void teamupsChannelAsk(
      jobId,
      input.requestId,
      "这一步要在电脑上填，飞书里填不安全。",
      false,
      undefined,
      chat,
    ).catch(() => {});
    return;
  }
  const questions = parseAskFormQuestions(input.inputJson, input.toolName);
  const question = questions[0];
  const options = question?.options.map((option) => option.label) ?? [];
  const summary = usefulSummary(input.detail, input.toolName);
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
    input.inputMode === "options" ? channelQuestionPayload(questions) : undefined,
    chat,
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
  const settled = await Promise.allSettled(
    matches.map(async (ask) => {
      await resolvePermissionSession({
        sessionId: ask.backendSessionId,
        requestId: ask.requestId,
        allow,
      });
      pendingByRequest.delete(ask.requestId);
    }),
  );
  const taken = settled.filter((result) => result.status === "fulfilled").length;
  if (taken === 0) {
    // Every send failed: report the failure instead of a stuck "already
    // confirmed"; the asks stay pending so the reply can simply be repeated.
    const failed = settled.find(
      (result): result is PromiseRejectedResult => result.status === "rejected",
    );
    throw failed?.reason ?? new Error("permission send failed");
  }
  publishPhoneChats();
  return taken;
}

/** Resolve one typed answer; only a delivered answer clears the pending ask. */
async function sendChannelAnswer(current: PendingAsk, text: string): Promise<boolean> {
  try {
    await resolvePermissionSession({
      sessionId: current.backendSessionId,
      requestId: current.requestId,
      allow: true,
      text,
    });
    pendingByRequest.delete(current.requestId);
    publishPhoneChats();
    return true;
  } catch {
    return false;
  }
}

function rePrompt(current: PendingAsk, text: string, withQuestions: boolean): void {
  void teamupsChannelAsk(
    current.jobId,
    current.requestId,
    text,
    true,
    withQuestions ? channelQuestionPayload(current.questions) : undefined,
    chatBySession.get(current.chatSessionId) ?? "",
  ).catch(() => {});
}

/** A plain-text reply against a single structured question, if it resolves. */
function freeTextOptionAnswer(
  raw: string,
  questions: QuestionFormQuestion[],
): Record<string, string> | null {
  if (questions.length !== 1) return null;
  const question = questions[0];
  const picks = channelOptionPicks(raw, question);
  if (picks.selected.length === 0 && (picks.custom === null || !questionShowsTextInput(question))) {
    return null;
  }
  const value = [
    ...picks.selected,
    ...(picks.custom !== null && (question.multiSelect || picks.selected.length === 0)
      ? [picks.custom]
      : []),
  ].join(", ");
  return { [question.question]: value };
}

export async function applyChannelAnswer(requestId: string, text: string): Promise<void> {
  const current = pendingByRequest.get(requestId);
  if (!current) return;
  const raw = text.trim();
  if (!raw) return;
  if (current.mode === "choice") {
    if (choiceVerdict(raw) === null) {
      rePrompt(current, "没看懂。回复 1 允许，回复 2 拒绝。", false);
      return;
    }
    await takePendingChoices(current.chatSessionId, raw);
    return;
  }
  if (current.mode === "options") {
    const answers = structuredAnswers(raw, current.questions) ?? freeTextOptionAnswer(raw, current.questions);
    if (!answers) {
      rePrompt(
        current,
        current.questions.length > 1
          ? `有 ${current.questions.length} 件事要选。请在手机上逐项选择；飞书暂时请到电脑上处理。`
          : optionPrompt(
              current.question,
              current.options,
              current.questions[0]?.multiSelect === true,
            ),
        true,
      );
      return;
    }
    if (!(await sendChannelAnswer(current, JSON.stringify({ answers })))) {
      rePrompt(current, "没送出去，请再发一次。", true);
    }
    return;
  }
  if (!(await sendChannelAnswer(current, raw))) {
    rePrompt(current, `没送出去，请再发一次。\n\n请直接回复：\n${current.question}`, false);
  }
}

function optionPrompt(question: string, options: string[], multiSelect = false): string {
  const lines = options.map((label, index) => `${index + 1}. ${label}`);
  const hint = multiSelect
    ? "手机上可以选择多项后统一确认。飞书暂时请到电脑上处理。"
    : "回复序号、选项原文，或直接回复你的答案。";
  return `${clip(question)}\n\n${lines.join("\n")}\n\n${hint}`;
}

/**
 * The phone site's structured reply: `{"answers": {question: "a, b"}}`,
 * keyed by question text or header. Every question must carry an answer;
 * option labels must match exactly; free text counts only where the question
 * allows a typed answer (Paseo's allowOther / multiSelect semantics).
 */
export function structuredAnswers(
  raw: string,
  questions: QuestionFormQuestion[],
): Record<string, string> | null {
  if (questions.length === 0 || !raw.startsWith("{")) return null;
  try {
    const value = JSON.parse(raw) as { answers?: unknown };
    if (!value.answers || typeof value.answers !== "object") return null;
    const source = value.answers as Record<string, unknown>;
    const answers: Record<string, string> = {};
    for (const question of questions) {
      const answer = question.header !== question.question
        ? source[question.question] ?? source[question.header]
        : source[question.question];
      const labels = question.options.map((option) => option.label);
      const allowed = new Set(labels);

      if (Array.isArray(answer)) {
        const picked = answer
          .filter((item): item is string => typeof item === "string")
          .map((item) => item.trim())
          .filter(Boolean);
        if (
          picked.length === 0 ||
          (!question.multiSelect && picked.length !== 1) ||
          picked.some((item) => !allowed.has(item))
        ) {
          return null;
        }
        answers[question.question] = picked.join(", ");
        continue;
      }
      if (typeof answer !== "string") return null;
      const text = answer.trim();
      if (!text) {
        if (question.allowEmpty && question.options.length === 0) {
          answers[question.question] = "";
          continue;
        }
        return null;
      }
      const { selected, custom } = question.multiSelect
        ? splitMultiSelectAnswer(text, labels)
        : selectedAnswer(text, labels, false);
      if (custom !== null && !questionShowsTextInput(question)) return null;
      answers[question.question] = [
        ...selected,
        ...(custom !== null && (question.multiSelect || selected.length === 0) ? [custom] : []),
      ].join(", ");
    }
    return answers;
  } catch {
    return null;
  }
}

export function usefulSummary(detail: string, toolName: string): string {
  const summary = formatPermissionDetail(detail).summary.trim();
  if (summary && summary !== "{}" && summary !== "[]" && !/^[[{]/.test(summary)) return clip(summary);
  const hint = jsonDetailHint(detail);
  const name = toolName.trim();
  if (hint) return clip(name ? `${name}：${hint}` : hint);
  if (name && name !== "{}" && !/^[[{]/.test(name)) return clip(name);
  return "这一步需要你允许后才会继续。";
}

const DETAIL_HINT_KEYS = [
  "query",
  "url",
  "path",
  "file",
  "pattern",
  "keyword",
  "topic",
  "command",
];

/**
 * Permission payloads for search/fetch tools are bare JSON like
 * `{"query":…}` or `{"url":…}`, which reads as nothing on the phone.
 * Pull out the first human-worth field so the ask names what it wants.
 */
function jsonDetailHint(detail: string): string {
  const unfenced = detail
    .trim()
    .replace(/^```(?:json)?\s*/i, "")
    .replace(/\s*```$/i, "")
    .trim();
  if (!unfenced.startsWith("{") || !unfenced.endsWith("}")) return "";
  try {
    const value = JSON.parse(unfenced) as Record<string, unknown>;
    for (const key of DETAIL_HINT_KEYS) {
      const item = value[key];
      if (typeof item === "string" && item.trim()) {
        return item.trim().replace(/\s+/g, " ");
      }
    }
    return "";
  } catch {
    return "";
  }
}

function clip(value: string): string {
  const text = value.replace(/\s+/g, " ").trim();
  return text.length > 400 ? `${text.slice(0, 400)}…` : text;
}
