/**
 * Run from desktop/: node_modules/.bin/tsx tests/ask-resources-browser-intent.test.ts
 */

const storage = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  writable: true,
  value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => {
      storage.set(key, value);
    },
    removeItem: (key: string) => {
      storage.delete(key);
    },
  },
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  writable: true,
  value: { language: "zh" },
});

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
}

async function main() {
  const { promptRequestsBrowserMcp } = await import("../src/ask-resources");

  // Chinese open-a-site intents need the browser.
  const positives: string[] = [
    "打开百度，搜索agent doctor",
    "打开 https://www.baidu.com/s?wd=agent%20doctor",
    "帮我访问 github.com 上的仓库",
    "打开这个网页看看",
    "open https://www.baidu.com",
    "visit baidu.com",
    "用浏览器打开那个网站",
  ];
  for (const text of positives) {
    assert(promptRequestsBrowserMcp(text), `should request browser: ${text}`);
  }

  // 打开/搜索 without a site hint stays non-browser.
  const negatives: string[] = [
    "打开这个项目文件夹里的文件",
    "打开设置",
    "搜索一下本地配置里的 model 字段",
    "你好",
    "打开微信",
  ];
  for (const text of negatives) {
    assert(!promptRequestsBrowserMcp(text), `should not request browser: ${text}`);
  }

  console.log("ask-resources-browser-intent ok");
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
