/**
 * Chat fence coloring: plain text stays plain, named languages get tokens.
 *
 * Run from desktop/:
 *   node_modules/.bin/tsx tests/code-highlight.test.ts
 */

const { highlightCode, isPlainFenceLanguage, verticalWheelStaysInCodeBlock } = await import(
  "../src/chat/code-highlight.ts"
);

let failures = 0;

function check(name: string, fn: () => void): void {
  try {
    fn();
    console.log(`  ok   ${name}`);
  } catch (error) {
    failures += 1;
    console.log(`  FAIL ${name}\n         ${(error as Error).message}`);
  }
}

function ok(condition: boolean, what: string): void {
  if (!condition) throw new Error(what);
}

check("blank and text fences stay plain", () => {
  ok(isPlainFenceLanguage(""), "blank");
  ok(isPlainFenceLanguage("text"), "text");
  ok(isPlainFenceLanguage("Plain"), "plain");
  ok(isPlainFenceLanguage("txt"), "txt");
  ok(!isPlainFenceLanguage("python"), "python");
  ok(!isPlainFenceLanguage("ts"), "ts");
});

check("a named language colors keywords and strings", () => {
  const html = highlightCode('const name = "ada"', "ts");
  ok(html.includes('class="syntax-keyword"'), html);
  ok(html.includes('class="syntax-string"'), html);
  ok(!html.includes("<script"), html);
});

check("an upward wheel leaves a code box that is already at the top", () => {
  ok(!verticalWheelStaysInCodeBlock(-40, 0, 200), "at top");
  ok(!verticalWheelStaysInCodeBlock(-40, 0, 0), "no vertical room");
  ok(verticalWheelStaysInCodeBlock(-40, 20, 200), "still inside");
  ok(!verticalWheelStaysInCodeBlock(40, 199, 200), "at bottom");
});

check("markup in source stays escaped", () => {
  const html = highlightCode("const x = '<b>hi</b>'", "js");
  ok(html.includes("&lt;b&gt;"), html);
  ok(!html.includes("<b>"), html);
});

if (failures > 0) {
  console.error(`${failures} failed`);
  process.exit(1);
}
console.log("code highlight ok");
