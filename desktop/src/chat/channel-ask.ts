import { resolvePermissionSession, teamupsChannelAsk } from "../ipc";
import { formatPermissionDetail } from "./format";

type AskMode = "choice" | "line" | "options";

type PendingAsk = {
  jobId: string;
  chatSessionId: string;
  requestId: string;
  backendSessionId: string;
  mode: AskMode;
  options: string[];
  question: string;
};

const jobsBySession = new Map<string, string>();
const pendingByRequest = new Map<string, PendingAsk>();

export function beginChannelJob(jobId: string, sessionId: string): void {
  jobsBySession.set(sessionId, jobId);
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
  if (input.inputMode === "options" && questions.length > 1) {
    void teamupsChannelAsk(jobId, input.requestId, "有好几件事要选，请在电脑上点。", false).catch(() => {});
    return;
  }
  const question = questions[0];
  const options = question?.options ?? [];
  const summary = clip(formatPermissionDetail(input.detail).summary || input.detail || input.toolName);
  const prompt = question
    ? optionPrompt(question.question, options)
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
  });
  void teamupsChannelAsk(jobId, input.requestId, prompt, true).catch(() => {});
}

export async function applyChannelAnswer(requestId: string, text: string): Promise<void> {
  const current = pendingByRequest.get(requestId);
  if (!current) return;
  const raw = text.trim();
  if (!raw) return;
  if (current.mode === "choice") {
    const allow = /^(1|允许|好|可以|是)$/.test(raw);
    const deny = /^(2|拒绝|不|不行|否)$/.test(raw);
    if (!allow && !deny) {
      void teamupsChannelAsk(current.jobId, current.requestId, "没看懂。回复 1 允许，回复 2 拒绝。", true).catch(() => {});
      return;
    }
    pendingByRequest.delete(requestId);
    await resolvePermissionSession({
      sessionId: current.backendSessionId,
      requestId: current.requestId,
      allow,
    });
    return;
  }
  if (current.mode === "options") {
    const picked = pickOption(raw, current.options);
    if (!picked) {
      void teamupsChannelAsk(
        current.jobId,
        current.requestId,
        optionPrompt(current.question, current.options),
        true,
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
    return;
  }
  pendingByRequest.delete(requestId);
  await resolvePermissionSession({
    sessionId: current.backendSessionId,
    requestId: current.requestId,
    allow: true,
    text: raw,
  });
}

function optionPrompt(question: string, options: string[]): string {
  const lines = options.map((label, index) => `${index + 1}. ${label}`);
  return `${clip(question)}\n\n${lines.join("\n")}\n\n回复序号，或直接回复选项。`;
}

function pickOption(raw: string, options: string[]): string | null {
  const index = Number(raw);
  if (Number.isInteger(index) && index >= 1 && index <= options.length) return options[index - 1];
  return options.find((label) => label === raw) ?? null;
}

function parseQuestions(raw: string | undefined): { question: string; options: string[] }[] {
  if (!raw?.trim()) return [];
  try {
    const value = JSON.parse(raw) as { questions?: unknown };
    if (!Array.isArray(value.questions)) return [];
    const questions: { question: string; options: string[] }[] = [];
    for (const item of value.questions) {
      if (!item || typeof item !== "object") continue;
      const record = item as { question?: unknown; options?: unknown };
      const question = typeof record.question === "string" ? record.question.trim() : "";
      if (!question) continue;
      const options: string[] = [];
      if (Array.isArray(record.options)) {
        for (const option of record.options) {
          if (!option || typeof option !== "object") continue;
          const label = (option as { label?: unknown }).label;
          if (typeof label === "string" && label.trim()) options.push(label.trim());
        }
      }
      questions.push({ question, options });
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
