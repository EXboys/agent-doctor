/** One file, with removed and added lines sitting together. Color marks which is which. */

export type DiffKind = "same" | "add" | "delete";

export type DiffLine = {
  kind: DiffKind;
  text: string;
};

/** Put the old lines and the new lines in one list, keeping lines that did not change in place. */
export function interleaveLines(deleted: string[], added: string[]): DiffLine[] {
  const n = deleted.length;
  const m = added.length;
  if (n === 0) return added.map((text) => ({ kind: "add", text }));
  if (m === 0) return deleted.map((text) => ({ kind: "delete", text }));
  if (n * m > 80_000) {
    return [
      ...deleted.map((text) => ({ kind: "delete" as const, text })),
      ...added.map((text) => ({ kind: "add" as const, text })),
    ];
  }
  const next = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i -= 1) {
    for (let j = m - 1; j >= 0; j -= 1) {
      next[i][j] =
        deleted[i] === added[j] ? next[i + 1][j + 1] + 1 : Math.max(next[i + 1][j], next[i][j + 1]);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (deleted[i] === added[j]) {
      out.push({ kind: "same", text: deleted[i] });
      i += 1;
      j += 1;
    } else if (next[i + 1][j] >= next[i][j + 1]) {
      out.push({ kind: "delete", text: deleted[i] });
      i += 1;
    } else {
      out.push({ kind: "add", text: added[j] });
      j += 1;
    }
  }
  while (i < n) out.push({ kind: "delete", text: deleted[i++] });
  while (j < m) out.push({ kind: "add", text: added[j++] });
  return out;
}

function sameText(a: string, b: string): boolean {
  return a.replace(/\s+$/, "") === b.replace(/\s+$/, "");
}

type Run = { fileAt: number; addedFrom: number; addedTo: number };

/** Pieces of the new text that still sit in the file, in order. One edit can be several pieces. */
function successiveRuns(lines: string[], added: string[]): Run[] {
  const runs: Run[] = [];
  let fileFrom = 0;
  let index = 0;
  while (index < added.length) {
    let anchor = index;
    while (anchor < added.length && added[anchor].trim().length < 3) anchor += 1;
    if (anchor >= added.length) break;
    let at = -1;
    for (let i = fileFrom; i < lines.length; i += 1) {
      if (sameText(lines[i], added[anchor])) {
        at = i;
        break;
      }
    }
    if (at < 0) {
      index = anchor + 1;
      continue;
    }
    let fileAt = at;
    let addedFrom = anchor;
    while (
      addedFrom > index &&
      fileAt > fileFrom &&
      sameText(lines[fileAt - 1], added[addedFrom - 1])
    ) {
      fileAt -= 1;
      addedFrom -= 1;
    }
    let addedTo = anchor + 1;
    let fileTo = at + 1;
    while (addedTo < added.length && fileTo < lines.length && sameText(lines[fileTo], added[addedTo])) {
      addedTo += 1;
      fileTo += 1;
    }
    runs.push({ fileAt, addedFrom, addedTo });
    fileFrom = fileTo;
    index = addedTo;
  }
  return runs;
}

function alongAdded(deleted: string[], added: string[]): { kinds: DiffKind[]; deletesBefore: string[][] } {
  const kinds: DiffKind[] = [];
  const deletesBefore: string[][] = Array.from({ length: added.length + 1 }, () => []);
  let index = 0;
  for (const line of interleaveLines(deleted, added)) {
    if (line.kind === "delete") deletesBefore[Math.min(index, added.length)].push(line.text);
    else {
      kinds.push(line.kind);
      index += 1;
    }
  }
  return { kinds, deletesBefore };
}

/**
 * The open file, once. Removed lines are put back beside the lines that replaced them.
 * New lines that are already in the file are not copied into a second block.
 */
export function inlineFileDiff(content: string, deleted: string[], added: string[]): DiffLine[] | null {
  if (deleted.length === 0 && added.length === 0) return null;
  const lines = content.split("\n");
  if (lines.length && lines[lines.length - 1] === "") lines.pop();
  const { kinds, deletesBefore } = alongAdded(deleted, added);
  const runs = added.length ? successiveRuns(lines, added) : [];
  if (added.length > 0 && runs.length === 0) {
    return [
      ...deleted.map((text) => ({ kind: "delete" as const, text })),
      ...lines.map((text) => ({ kind: "same" as const, text })),
    ];
  }
  const out: DiffLine[] = [];
  let cursor = 0;
  let coveredUntil = 0;
  const pushDeletes = (from: number, to: number) => {
    for (let index = from; index < to; index += 1) {
      for (const text of deletesBefore[index] ?? []) out.push({ kind: "delete", text });
    }
  };
  for (const run of runs) {
    for (let i = cursor; i < run.fileAt; i += 1) out.push({ kind: "same", text: lines[i] });
    pushDeletes(coveredUntil, run.addedFrom + 1);
    for (let addedIndex = run.addedFrom; addedIndex < run.addedTo; addedIndex += 1) {
      if (addedIndex > run.addedFrom) pushDeletes(addedIndex, addedIndex + 1);
      const fileIndex = run.fileAt + (addedIndex - run.addedFrom);
      out.push({ kind: kinds[addedIndex] === "add" ? "add" : "same", text: lines[fileIndex] ?? added[addedIndex] });
    }
    cursor = run.fileAt + (run.addedTo - run.addedFrom);
    coveredUntil = run.addedTo;
  }
  for (let i = cursor; i < lines.length; i += 1) out.push({ kind: "same", text: lines[i] });
  pushDeletes(coveredUntil, deletesBefore.length);
  return out;
}
