/**
 * Removed lines and new lines sit in one list, in the order a person would read the file.
 *
 * Run from desktop/:
 *   node --import tsx tests/file-diff.test.ts
 */

import { inlineFileDiff, interleaveLines } from "../src/chat/file-diff";

let failures = 0;
function check(name: string, ok: boolean, detail?: unknown): void {
  if (ok) console.log(`ok - ${name}`);
  else {
    failures += 1;
    console.log(`not ok - ${name}`, detail ?? "");
  }
}

const mixed = interleaveLines(
  ["keep", "old line", "tail"],
  ["keep", "new line", "tail"],
);
check(
  "a changed line sits between the lines that stayed",
  mixed.map((line) => `${line.kind}:${line.text}`).join("|") ===
    "same:keep|delete:old line|add:new line|same:tail",
  mixed,
);

const file = inlineFileDiff(
  ["intro", "keep", "new line", "tail", "end"].join("\n"),
  ["keep", "old line", "tail"],
  ["keep", "new line", "tail"],
);
check(
  "the removed line is put back inside the file",
  file?.map((line) => `${line.kind}:${line.text}`).join("|") ===
    "same:intro|same:keep|delete:old line|add:new line|same:tail|same:end",
  file,
);

check(
  "a new file is all added lines",
  inlineFileDiff("hello\n", [], ["hello"])?.every((line) => line.kind === "add") === true,
);

const split = inlineFileDiff("A\nB\nC\nD\n", ["old"], ["A", "B", "D"]);
check(
  "a file is shown once when the new lines are not one block",
  split?.filter((line) => line.text === "A").length === 1 &&
    split.some((line) => line.kind === "delete" && line.text === "old") &&
    split.filter((line) => line.kind !== "delete").map((line) => line.text).join("|") === "A|B|C|D",
  split,
);

if (failures) {
  console.log(`${failures} failed`);
  process.exit(1);
}
