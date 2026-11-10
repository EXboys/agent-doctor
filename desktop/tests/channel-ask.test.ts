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

const { beginChannelJob, channelSessionForJob, choiceVerdict, endChannelJob, structuredAnswers, usefulSummary } =
  await import("../src/chat/channel-ask");
const { parseAskFormQuestions } = await import("../src/chat/question-form-core");

assert.equal(choiceVerdict("允许"), true);
assert.equal(choiceVerdict("2"), false);

// The phone renders the ask text as one card. A bare tool name tells the
// reader nothing; the payload's own query/url has to come along.
assert.equal(usefulSummary(JSON.stringify({ query: "Seedance 2.5 pricing" }), "WebSearch"), "WebSearch：Seedance 2.5 pricing");
assert.equal(usefulSummary(JSON.stringify({ url: "https://example.com/pricing" }), "WebFetch"), "WebFetch：https://example.com/pricing");
assert.equal(usefulSummary(JSON.stringify({ path: "docs/plan.md" }), "Read"), "Read：docs/plan.md");
assert.equal(usefulSummary(JSON.stringify({ command: "npm test", description: "跑一遍测试" }), "Bash"), "跑一遍测试");
assert.equal(usefulSummary(JSON.stringify({ size: 3 }), "Inspect"), "Inspect");
assert.equal(usefulSummary("rm -rf /tmp/build", "Bash").length > 0, true);

// The questions go through the shared parser, the same one the desktop card
// uses, so both sides read one inputJson the same way. AskUserQuestion asks
// always accept a typed answer alongside the options.
const questions = parseAskFormQuestions(
  JSON.stringify({
    questions: [
      {
        question: "要检查哪些部分？",
        header: "检查项",
        options: [{ label: "网络" }, { label: "配置" }, { label: "日志" }],
        multiSelect: true,
      },
      {
        question: "完成后怎么处理？",
        options: [{ label: "只报告" }, { label: "直接修复" }],
        multiSelect: false,
      },
    ],
  }),
  "AskUserQuestion",
);

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

// The site may key answers by header instead of question text.
assert.deepEqual(
  structuredAnswers(
    JSON.stringify({
      answers: {
        检查项: "网络",
        "完成后怎么处理？": "只报告",
      },
    }),
    questions,
  ),
  {
    "要检查哪些部分？": "网络",
    "完成后怎么处理？": "只报告",
  },
);

// A label containing ", " stays one selection when it is an exact option.
const commaLabel = parseAskFormQuestions(
  JSON.stringify({
    questions: [
      {
        question: "怎么清理？",
        options: [{ label: "Delete both" }, { label: "Delete both, fix code" }],
        multiSelect: true,
      },
    ],
  }),
  "AskUserQuestion",
);
assert.deepEqual(
  structuredAnswers(
    JSON.stringify({ answers: { "怎么清理？": "Delete both, fix code" } }),
    commaLabel,
  ),
  { "怎么清理？": "Delete both, fix code" },
);

// A typed answer stands in for a single-select option (allowOther is forced
// for AskUserQuestion) and appends to a multi-select. A missing question
// still fails the whole form.
assert.equal(
  structuredAnswers(
    JSON.stringify({ answers: { "完成后怎么处理？": "先不动，等我确认" } }),
    questions,
  ),
  null,
);
assert.deepEqual(
  structuredAnswers(
    JSON.stringify({
      answers: { "要检查哪些部分？": "网络", "完成后怎么处理？": "先不动，等我确认" },
    }),
    questions,
  ),
  { "要检查哪些部分？": "网络", "完成后怎么处理？": "先不动，等我确认" },
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
  "array items must be exact labels",
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
  "every question must carry an answer",
);

beginChannelJob("job-a", "chat-a");
beginChannelJob("job-b", "chat-b");
endChannelJob("chat-a", "job-old");
assert.equal(channelSessionForJob("job-a"), "chat-a", "an older sentence must not clear the chat that replaced it");
assert.equal(channelSessionForJob("job-b"), "chat-b", "ending one chat leaves the other running");
endChannelJob("chat-a", "job-a");
assert.equal(channelSessionForJob("job-a"), null);
assert.equal(channelSessionForJob("job-b"), "chat-b");

// A send whose invoke returned while a newer turn runs on detached must not
// end the channel job: the phone's stop still has to find this session. The
// mapping may only be cleared by a real turn end (chat/send.ts).
beginChannelJob("job-live", "chat-live", "phone:x");
assert.equal(channelSessionForJob("job-live"), "chat-live", "a detached turn keeps its job stoppable");
endChannelJob("chat-live", "job-live");
assert.equal(channelSessionForJob("job-live"), null, "the real turn end clears the mapping");

console.log("channel ask tests passed");
