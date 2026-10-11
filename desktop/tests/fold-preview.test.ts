/**
 * A folded reply keeps paragraph breaks instead of one jammed line.
 *
 * Run from desktop/:
 *   node --import tsx tests/fold-preview.test.ts
 */

globalThis.localStorage = { getItem: () => null, setItem: () => {}, removeItem: () => {} };
const { foldPreview } = await import("../src/chat/format.ts");

let failures = 0;
function check(name: string, ok: boolean, detail?: unknown): void {
  if (ok) console.log(`ok - ${name}`);
  else {
    failures += 1;
    console.log(`not ok - ${name}`, detail ?? "");
  }
}

const text = foldPreview(
  "完成。文件已放好。\n\n## 设计要点\n\n米黄底，深棕字。\n\n- 正文 19px\n- 标题 26px",
);
check(
  "paragraphs stay on their own lines",
  text === "完成。文件已放好。\n设计要点\n米黄底，深棕字。\n正文 19px\n标题 26px",
  text,
);

if (failures) {
  console.log(`${failures} failed`);
  process.exit(1);
}
