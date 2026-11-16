/**
 * Run from desktop/: node_modules/.bin/tsx tests/channel-images.test.ts
 */
import {
  collectChannelReplyImages,
  extractChannelReplyImagePaths,
} from "../src/chat/channel-images";

function assert(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
}

async function main(): Promise<void> {
  // 1. Standalone CJK path line + prose inline path + backticked relative path
  const prose = [
    "界面图已经生成：",
    "",
    "/Users/airlu/Desktop/手机端选Agent-效果图.png",
    "",
    "另外对比图保存在 ~/Desktop/对比-2.webp，细节放在 `screenshots/detail.jpg`。",
    "外链图片 ![remote](https://example.com/demo.png) 和代码 `src/app.ts` 不算本机回图。",
  ].join("\n");

  const paths = extractChannelReplyImagePaths(prose, [
    { path: "out/chart.png", kind: "output" },
    { path: "src/index.ts", kind: "output" },
    { path: "assets/existing.png", kind: "change" },
  ]);
  assert(
    JSON.stringify(paths) ===
      JSON.stringify([
        "screenshots/detail.jpg",
        "/Users/airlu/Desktop/手机端选Agent-效果图.png",
        "~/Desktop/对比-2.webp",
        "out/chart.png",
      ]),
    `unexpected extracted paths: ${JSON.stringify(paths)}`,
  );

  // 2. Markdown image syntax with spaces / angle brackets / title / file:// URL
  const md = [
    "![UI](</Users/airlu/Desktop/My UI Shot.png>)",
    '![Chart](reports/equity.png "Equity Curve")',
    "[打开原图](file:///Users/airlu/Desktop/shot.gif)",
  ].join("\n");
  const mdPaths = extractChannelReplyImagePaths(md);
  assert(
    JSON.stringify(mdPaths) ===
      JSON.stringify([
        "/Users/airlu/Desktop/My UI Shot.png",
        "reports/equity.png",
        "file:///Users/airlu/Desktop/shot.gif",
      ]),
    `unexpected markdown paths: ${JSON.stringify(mdPaths)}`,
  );

  // 3. collectChannelReplyImages skips missing files, deduplicates identical payloads, and caps at 3
  const disk = new Map<string, { name: string; mime: string; data: string }>([
    ["/ok/one.png", { name: "one.png", mime: "image/png", data: "AAAA" }],
    ["/ok/one-copy.png", { name: "one-copy.png", mime: "image/png", data: "AAAA" }],
    ["rel/two.jpg", { name: "two.jpg", mime: "image/jpeg", data: "BBBB" }],
    ["/ok/three.webp", { name: "three.webp", mime: "image/webp", data: "CCCC" }],
    ["/ok/four.gif", { name: "four.gif", mime: "image/gif", data: "DDDD" }],
  ]);
  const seenCwds: (string | null | undefined)[] = [];
  const loaded = await collectChannelReplyImages({
    text: [
      "`/missing/example.png`",
      "`/ok/one.png`",
      "`/ok/one-copy.png`",
      "`rel/two.jpg`",
      "`/ok/three.webp`",
      "`/ok/four.gif`",
    ].join("\n"),
    steps: [{ kind: "text", text: "先看 `/ok/one.png`" }],
    cwd: "/Users/airlu/project",
    readImage: async (path, cwd) => {
      seenCwds.push(cwd);
      const found = disk.get(path);
      if (!found) throw new Error("not found");
      return found;
    },
  });
  assert(
    JSON.stringify(loaded.map((item) => item.name)) ===
      JSON.stringify(["one.png", "two.jpg", "three.webp"]),
    `unexpected loaded images: ${JSON.stringify(loaded)}`,
  );
  assert(
    seenCwds.every((cwd) => cwd === "/Users/airlu/project"),
    "cwd is forwarded to readImage",
  );

  console.log("channel-images ok");
}

void main();
