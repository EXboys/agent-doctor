// Ported from Paseo (Apache-2.0, github.com/getpaseo/paseo)
// packages/server/src/server/agent/providers/question-card-answer.ts — kept
// function-for-function so upstream fixes can land here as verbatim diffs.

/**
 * The shared question card sends a multi-select answer as one string: the checked option
 * labels in click order, then any typed answer, joined with ", ". Consume exact option
 * labels from the front, longest first so a label containing ", " wins over its prefix;
 * any remaining text is the typed answer.
 */
export function splitMultiSelectAnswer(
  answer: string,
  labels: string[],
): { selected: string[]; custom: string | null } {
  let remaining = answer;
  const selected: string[] = [];
  while (remaining.length > 0) {
    const label = labels
      .filter((candidate) => !selected.includes(candidate))
      .sort((left, right) => right.length - left.length)
      .find((candidate) => remaining === candidate || remaining.startsWith(`${candidate}, `));
    if (!label) break;
    selected.push(label);
    remaining = remaining === label ? "" : remaining.slice(label.length + 2);
  }
  return { selected, custom: remaining || null };
}

/**
 * One phone/desktop reply resolved against one question's options, the way
 * Paseo's OMP adapter does: a single-select reply is either an exact label or
 * free text; a multi-select reply goes through splitMultiSelectAnswer.
 */
export function selectedAnswer(
  answer: string,
  labels: string[],
  multi: boolean,
): { selected: string[]; custom: string | null } {
  if (!multi)
    return labels.includes(answer)
      ? { selected: [answer], custom: null }
      : { selected: [], custom: answer };
  return splitMultiSelectAnswer(answer, labels);
}
