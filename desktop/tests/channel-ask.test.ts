import assert from "node:assert/strict";

Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  value: {
    getItem: () => null,
    setItem: () => {},
    removeItem: () => {},
  },
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { language: "zh" },
});

const { choiceVerdict, structuredAnswers } = await import("../src/chat/channel-ask");

assert.equal(choiceVerdict("允许"), true);
assert.equal(choiceVerdict("2"), false);

const questions = [
  {
    question: "要检查哪些部分？",
    options: [{ label: "网络" }, { label: "配置" }, { label: "日志" }],
    multiSelect: true,
  },
  {
    question: "完成后怎么处理？",
    options: [{ label: "只报告" }, { label: "直接修复" }],
    multiSelect: false,
  },
];

assert.deepEqual(
  structuredAnswers(
    JSON.stringify({
      answers: {
        "要检查哪些部分？": ["网络", "日志"],
        "完成后怎么处理？": ["只报告"],
      },
    }),
    questions,
  ),
  {
    "要检查哪些部分？": "网络, 日志",
    "完成后怎么处理？": "只报告",
  },
);

assert.equal(
  structuredAnswers(
    JSON.stringify({
      answers: {
        "要检查哪些部分？": ["网络", "不存在的选项"],
        "完成后怎么处理？": ["只报告"],
      },
    }),
    questions,
  ),
  null,
);

assert.equal(
  structuredAnswers(
    JSON.stringify({
      answers: {
        "要检查哪些部分？": ["网络"],
      },
    }),
    questions,
  ),
  null,
);

console.log("channel ask tests passed");
