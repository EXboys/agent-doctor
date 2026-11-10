// Question-answer core ported from Paseo (Apache-2.0, github.com/getpaseo/paseo)
// packages/app/src/components/question-form-card-core.ts, kept
// function-for-function so upstream fixes land here as verbatim diffs. The two
// functions at the bottom are Paseo's provider-adapter logic (claude/agent.ts
// normalizeClaudeAskUserQuestionRequestInput + resolveClaudeAskUserQuestionAnswers)
// folded into one local adapter, since our backend wire needs answers keyed by
// full question text rather than Paseo's header keys.

import { selectedAnswer } from "./question-card-answer";

export interface QuestionOption {
  label: string;
  description?: string;
  preview?: string;
}

export interface QuestionFormQuestion {
  question: string;
  header: string;
  options: QuestionOption[];
  multiSelect: boolean;
  allowOther: boolean;
  allowEmpty: boolean;
  placeholder?: string;
  dismissLabel?: string;
  /** Local extension: the backend flagged this question as holding a secret. */
  isSecret?: boolean;
}

export type QuestionSelections = Record<number, ReadonlySet<number>>;
export type QuestionOtherTexts = Record<number, string>;

function readOptionalString(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

function readNonEmptyString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

export function questionShowsTextInput(question: QuestionFormQuestion): boolean {
  return question.options.length === 0 || question.allowOther;
}

export function isQuestionAnswered(
  question: QuestionFormQuestion,
  qIndex: number,
  selections: QuestionSelections,
  otherTexts: QuestionOtherTexts,
): boolean {
  const selected = selections[qIndex];
  if (selected && selected.size > 0) {
    return true;
  }

  if (!questionShowsTextInput(question)) {
    return false;
  }

  const otherText = otherTexts[qIndex]?.trim();
  if (otherText && otherText.length > 0) {
    return true;
  }

  return question.allowEmpty;
}

export function areQuestionsAnswered(
  questions: QuestionFormQuestion[] | null,
  selections: QuestionSelections,
  otherTexts: QuestionOtherTexts,
): boolean {
  return (
    questions?.every((question, qIndex) =>
      isQuestionAnswered(question, qIndex, selections, otherTexts),
    ) ?? false
  );
}

export function buildQuestionFormAnswers(
  questions: QuestionFormQuestion[],
  selections: QuestionSelections,
  otherTexts: QuestionOtherTexts,
): Record<string, string> {
  const answers: Record<string, string> = {};
  for (let i = 0; i < questions.length; i++) {
    const q = questions[i];
    const selected = selections[i];
    const otherText = otherTexts[i]?.trim();
    const labels = selected ? Array.from(selected).map((idx) => q.options[idx].label) : [];

    if (questionShowsTextInput(q)) {
      if (otherText && otherText.length > 0) {
        // Multi-select keeps the checked options and appends the custom answer, the way
        // Claude Code's own AskUserQuestion UI does. Single-select replaces the option.
        answers[q.header] = q.multiSelect ? [...labels, otherText].join(", ") : otherText;
        continue;
      }
      if (q.allowEmpty && q.options.length === 0) {
        answers[q.header] = "";
        continue;
      }
    }

    if (labels.length > 0) {
      answers[q.header] = labels.join(", ");
    }
  }
  return answers;
}

export function shouldSubmitEmptyOnDismiss(questions: QuestionFormQuestion[]): boolean {
  return (
    questions.length > 0 &&
    questions.every((question) => question.allowEmpty && question.options.length === 0)
  );
}

export function resolveDismissLabel(
  questions: QuestionFormQuestion[],
  fallbackLabel = "Dismiss",
): string {
  return questions.find((question) => question.dismissLabel)?.dismissLabel ?? fallbackLabel;
}

/**
 * Re-key answers onto full question text: look each question's answer up by
 * full text first, then by header, both trimmed — so callers may hand us
 * either keying and padded strings (Paseo's claude adapter contract). Empty
 * answers drop out, matching readNonEmptyString upstream.
 */
export function resolveQuestionAnswers(
  questions: QuestionFormQuestion[],
  answers: Record<string, string>,
): Record<string, string> {
  const normalized: Record<string, string> = {};
  for (const question of questions) {
    const questionText = question.question.trim();
    if (!questionText) continue;
    const header = question.header.trim();
    const answer =
      readNonEmptyString(answers[questionText]) ??
      (header ? readNonEmptyString(answers[header]) : null);
    if (answer !== null) {
      normalized[questionText] = answer;
    }
  }
  return normalized;
}

/**
 * Local adapter replacing the two per-consumer parsers. Backends speak raw
 * tool input (`{questions:[{question, options, multiSelect, isOther?, …}]}`),
 * not Paseo's validated transport, so: bad items are skipped, `header`
 * defaults to the question text (keeping our answers keyed by full text on
 * the wire), `isOther` maps onto allowOther (Paseo keeps that alias for
 * Codex), and AskUserQuestion questions are always allowOther — Claude Code's
 * schema says "Other" is host-provided, never a model-supplied option.
 */
export function parseAskFormQuestions(
  raw: string | undefined,
  toolName = "",
): QuestionFormQuestion[] {
  if (!raw?.trim()) return [];
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return [];
  }
  const questions = (value as { questions?: unknown } | null)?.questions;
  if (!Array.isArray(questions)) return [];
  const forceAllowOther = toolName.trim() === "AskUserQuestion";
  const parsed: QuestionFormQuestion[] = [];
  for (const item of questions) {
    if (!item || typeof item !== "object" || Array.isArray(item)) continue;
    const record = item as Record<string, unknown>;
    const question = typeof record.question === "string" ? record.question.trim() : "";
    if (!question) continue;
    const options: QuestionOption[] = [];
    if (Array.isArray(record.options)) {
      for (const option of record.options) {
        if (!option || typeof option !== "object") continue;
        const optionRecord = option as Record<string, unknown>;
        const label = typeof optionRecord.label === "string" ? optionRecord.label.trim() : "";
        if (!label) continue;
        const description =
          typeof optionRecord.description === "string" ? optionRecord.description.trim() : "";
        const preview = readOptionalString(optionRecord, "preview");
        options.push({
          label,
          ...(description ? { description } : {}),
          ...(preview ? { preview } : {}),
        });
      }
    }
    parsed.push({
      question,
      header: readOptionalString(record, "header") ?? question,
      options,
      multiSelect: record.multiSelect === true,
      allowOther: record.allowOther === true || record.isOther === true || forceAllowOther,
      allowEmpty: record.allowEmpty === true,
      ...(readOptionalString(record, "placeholder")
        ? { placeholder: readOptionalString(record, "placeholder") }
        : {}),
      ...(readOptionalString(record, "dismissLabel")
        ? { dismissLabel: readOptionalString(record, "dismissLabel") }
        : {}),
      ...(record.isSecret === true ? { isSecret: true } : {}),
    });
  }
  return parsed;
}

/**
 * The phone replies one question with plain text. Resolve it like the desktop
 * card would have: indices win first ("回复序号" is the channel prompt), then
 * exact labels via selectedAnswer; a multi-select list of indices or labels
 * collects every hit, and what remains is the typed answer when the question
 * allows free text.
 */
export function channelOptionPicks(
  raw: string,
  question: QuestionFormQuestion,
): { selected: string[]; custom: string | null } {
  const labels = question.options.map((option) => option.label);
  const indices = raw
    .split(/[,，、\s]+/)
    .filter(Boolean)
    .map((token) => Number(token));
  if (
    indices.length > 0 &&
    indices.every((index) => Number.isInteger(index) && index >= 1 && index <= labels.length)
  ) {
    return { selected: indices.map((index) => labels[index - 1]), custom: null };
  }
  const split = selectedAnswer(raw, labels, question.multiSelect);
  if (split.selected.length > 0 || split.custom !== null) return split;
  return { selected: [], custom: null };
}
