/**
 * Standalone check for picture routing (src/chat/picture-route.ts).
 *
 * Run from desktop/:
 *   node_modules/.bin/tsx tests/picture-route.test.ts
 */

const { classifyPicture, planPictures } = await import("../src/chat/picture-route.ts");

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

function eq<T>(actual: T, expected: T, what: string): void {
  const a = JSON.stringify(actual);
  const b = JSON.stringify(expected);
  if (a !== b) throw new Error(`${what}: got ${a}, expected ${b}`);
}

const errorLog = Array.from(
  { length: 10 },
  (_, i) => `Error: cannot connect to the server at step ${i}, please check the network settings`,
).join("\n");
const chartLabels = Array.from({ length: 30 }, (_, i) => `${3200 + i * 7}.50 ${i}%`).join("\n");
const vision = { seesImages: true, formats: ["png", "jpg", "jpeg", "gif", "webp"] };
const textOnly = { seesImages: false, formats: [] };

check("wordy screenshots read as text, charts and short text as pictures", () => {
  eq(classifyPicture(errorLog), "text", "error log");
  eq(classifyPicture(chartLabels), "picture", "chart axis numbers");
  eq(classifyPicture("设置 保存"), "picture", "few words");
  eq(classifyPicture(""), "picture", "no words");
});

check("a model that sees pictures gets charts as pictures and logs as words", () => {
  const turn = planPictures(
    [
      { path: "/a/kline.png", name: "kline.png", text: "" },
      { path: "/a/log.png", name: "log.png", text: errorLog },
      { path: "/a/axis.jpg", name: "axis.jpg", text: chartLabels },
    ],
    vision,
  );
  eq(turn.sendPaths, ["/a/kline.png", "/a/axis.jpg"], "sent pictures");
  eq(turn.readings.map((r) => r.name), ["log.png", "axis.jpg"], "words carried");
  eq(turn.unseenNames, [], "nothing unseen");
});

check("unsupported formats fall back to words", () => {
  const turn = planPictures([{ path: "/a/photo.HEIC", name: "photo.HEIC", text: "receipt total 42" }], vision);
  eq(turn.sendPaths, [], "heic not sent");
  eq(turn.readings.length, 1, "words kept");
});

check("a text-only model gets words, or an honest note when there are none", () => {
  const turn = planPictures(
    [
      { path: "/a/kline.png", name: "kline.png", text: "" },
      { path: "/a/log.png", name: "log.png", text: errorLog },
    ],
    textOnly,
  );
  eq(turn.sendPaths, [], "nothing sent");
  eq(turn.readings.map((r) => r.name), ["log.png"], "words");
  eq(turn.unseenNames, ["kline.png"], "unseen chart");
});

if (failures > 0) {
  console.log(`\n${failures} failed`);
  process.exit(1);
}
console.log("\nall passed");
