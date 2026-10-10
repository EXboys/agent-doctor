/**
 * Opening a changed file later still finds the lines that were added and removed.
 *
 * Run from desktop/:
 *   node --import tsx tests/file-changes.test.ts
 */

import { fileChangeFor, rememberFileChange, resetFileChanges } from "../src/chat/file-changes";

let failures = 0;
function check(name: string, ok: boolean, detail?: unknown): void {
  if (ok) console.log(`ok - ${name}`);
  else {
    failures += 1;
    console.log(`not ok - ${name}`, detail ?? "");
  }
}

resetFileChanges();
rememberFileChange("/proj/src/chat.css", "tool-1", {
  additions: 2,
  deletions: 1,
  addedLines: ["padding-right: 26px;"],
  deletedLines: ["svg { display: none; }"],
  lineStart: 2145,
  lineEnd: 2148,
});
rememberFileChange("/proj/src/chat.css", "tool-1", {
  additions: 2,
  deletions: 1,
  addedLines: ["padding-right: 26px;", "color: #d64545;"],
  deletedLines: ["svg { display: none; }"],
});

const again = fileChangeFor("src/chat.css");
check("a later result replaces the same step", again?.addedLines.length === 2, again);
check("relative path finds the full path", again?.deletedLines[0] === "svg { display: none; }", again);

rememberFileChange("/proj/src/chat.css", "tool-2", {
  additions: 1,
  deletions: 0,
  addedLines: ["margin-right: 8px;"],
  deletedLines: [],
});
const both = fileChangeFor("src/chat.css");
check("two steps on one file stay together", both?.additions === 3 && both.addedLines.length === 3, both);

if (failures) {
  console.log(`${failures} failed`);
  process.exit(1);
}
console.log("all passed");
