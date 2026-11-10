import assert from "node:assert/strict";

// Ported from Paseo (Apache-2.0) question-form-card-core.test.ts and
// question-card-answer.test.ts, plus the local adapter cases.

const {
  areQuestionsAnswered,
  buildQuestionFormAnswers,
  channelOptionPicks,
  parseAskFormQuestions,
  questionShowsTextInput,
  resolveDismissLabel,
  resolveQuestionAnswers,
  shouldSubmitEmptyOnDismiss,
} = await import("../src/chat/question-form-core");
const { selectedAnswer, splitMultiSelectAnswer } = await import("../src/chat/question-card-answer");

const labels = ["Delete both", "Delete both, fix code", "Email the user"];

assert.deepEqual(splitMultiSelectAnswer("Email the user, Delete both", labels), {
  selected: ["Email the user", "Delete both"],
  custom: null,
});
assert.deepEqual(splitMultiSelectAnswer("Delete both, fix code, Email the user", labels), {
  selected: ["Delete both, fix code", "Email the user"],
  custom: null,
});
assert.deepEqual(splitMultiSelectAnswer("Delete both, Also check the logs", labels), {
  selected: ["Delete both"],
  custom: "Also check the logs",
});
assert.deepEqual(splitMultiSelectAnswer("Only typed text", labels), {
  selected: [],
  custom: "Only typed text",
});

assert.deepEqual(selectedAnswer("Codex", ["Claude Code", "Codex"], false), {
  selected: ["Codex"],
  custom: null,
});
assert.deepEqual(selectedAnswer("OpenCode", ["Claude Code", "Codex"], false), {
  selected: [],
  custom: "OpenCode",
});

// Optional input prompts are skippable empty answers.
const optional = parseAskFormQuestions(
  JSON.stringify({
    questions: [
      {
        question: "Optional comment?",
        header: "Response",
        options: [],
        multiSelect: false,
        placeholder: "Optional comment (press Enter to skip)...",
        allowEmpty: true,
        dismissLabel: "Skip",
      },
    ],
  }),
);
assert.equal(areQuestionsAnswered(optional, {}, {}), true);
assert.deepEqual(buildQuestionFormAnswers(optional, {}, {}), { Response: "" });
assert.equal(shouldSubmitEmptyOnDismiss(optional), true);
assert.equal(resolveDismissLabel(optional, "先不答"), "Skip");

// Option-only questions need a selection.
const pickOne = parseAskFormQuestions(
  JSON.stringify({
    questions: [
      { question: "Pick one", header: "Response", options: [{ label: "A" }, { label: "B" }] },
    ],
  }),
);
assert.equal(questionShowsTextInput(pickOne[0]), false);
assert.equal(areQuestionsAnswered(pickOne, {}, { 0: "freeform" }), false);
assert.equal(areQuestionsAnswered(pickOne, { 0: new Set([1]) }, {}), true);
assert.deepEqual(buildQuestionFormAnswers(pickOne, { 0: new Set([1]) }, {}), { Response: "B" });

// Multi-select keeps checked options and appends the typed answer.
const fruits = parseAskFormQuestions(
  JSON.stringify({
    questions: [
      {
        question: "Which fruits do you like?",
        header: "Fruits",
        options: [{ label: "Apple" }, { label: "Banana" }, { label: "Cherry" }],
        multiSelect: true,
        allowOther: true,
      },
    ],
  }),
);
assert.deepEqual(buildQuestionFormAnswers(fruits, { 0: new Set([0, 2]) }, { 0: " durian " }), {
  Fruits: "Apple, Cherry, durian",
});
assert.deepEqual(buildQuestionFormAnswers(fruits, { 0: new Set([0, 2]) }, {}), {
  Fruits: "Apple, Cherry",
});
assert.deepEqual(buildQuestionFormAnswers(fruits, { 0: new Set() }, { 0: "durian" }), {
  Fruits: "durian",
});
assert.deepEqual(buildQuestionFormAnswers(fruits, {}, { 0: "durian" }), { Fruits: "durian" });

// Single-select typed answers replace the picked option.
const provider = parseAskFormQuestions(
  JSON.stringify({
    questions: [
      {
        question: "Which provider?",
        header: "Provider",
        options: [{ label: "Claude Code" }, { label: "Codex" }],
        multiSelect: false,
        allowOther: true,
      },
    ],
  }),
);
assert.deepEqual(buildQuestionFormAnswers(provider, { 0: new Set([1]) }, { 0: "OpenCode" }), {
  Provider: "OpenCode",
});
assert.deepEqual(buildQuestionFormAnswers(provider, { 0: new Set([1]) }, {}), {
  Provider: "Codex",
});

// isOther is the Codex spelling of allowOther.
const isOther = parseAskFormQuestions(
  JSON.stringify({
    questions: [
      { question: "Pick or type", header: "Response", options: [{ label: "A" }], isOther: true },
    ],
  }),
);
assert.equal(questionShowsTextInput(isOther[0]), true);
assert.equal(areQuestionsAnswered(isOther, {}, { 0: "custom" }), true);
assert.deepEqual(buildQuestionFormAnswers(isOther, {}, { 0: "custom" }), { Response: "custom" });

// Option previews survive the parse so options can be compared.
const layout = parseAskFormQuestions(
  JSON.stringify({
    questions: [
      {
        question: "Which layout?",
        header: "Layout",
        options: [
          {
            label: "Tabs",
            description: "One tab per section",
            preview: "<Tabs>\n  <Tab />\n</Tabs>",
          },
          { label: "Single page" },
        ],
      },
    ],
  }),
);
assert.deepEqual(layout[0].options, [
  { label: "Tabs", description: "One tab per section", preview: "<Tabs>\n  <Tab />\n</Tabs>" },
  { label: "Single page" },
]);

// Local adapter: header defaults to the question text, labels are trimmed,
// AskUserQuestion questions always take a typed answer, isSecret is kept.
const adapted = parseAskFormQuestions(
  JSON.stringify({
    questions: [
      {
        question: " 用哪种登录？ ",
        options: [{ label: " 浏览器 " }, { label: "客户端" }],
        isSecret: true,
      },
      { question: "补充说明", options: [], isOther: true },
    ],
  }),
  "AskUserQuestion",
);
assert.equal(adapted[0].header, "用哪种登录？");
assert.equal(adapted[0].question, "用哪种登录？");
assert.deepEqual(adapted[0].options, [{ label: "浏览器" }, { label: "客户端" }]);
assert.equal(adapted[0].allowOther, true, "AskUserQuestion always allows a typed answer");
assert.equal(adapted[0].isSecret, true);
assert.equal(adapted[1].allowOther, true, "isOther maps onto allowOther");
assert.equal(adapted[1].header, "补充说明");
const notForced = parseAskFormQuestions(
  JSON.stringify({
    questions: [{ question: "选一个", options: [{ label: "甲" }, { label: "乙" }] }],
  }),
  "Bash",
);
assert.equal(notForced[0].allowOther, false);

// Answers re-key onto full question text: text first, header fallback, padded tolerated.
assert.deepEqual(
  resolveQuestionAnswers(adapted, { "用哪种登录？": "浏览器", 补充说明: "无" }),
  { "用哪种登录？": "浏览器", 补充说明: "无" },
);
assert.deepEqual(resolveQuestionAnswers(adapted, { 用哪种登录: "浏览器" }), {});
assert.deepEqual(
  resolveQuestionAnswers(
    parseAskFormQuestions(
      JSON.stringify({
        questions: [{ question: "用哪种登录？", header: "登录", options: [{ label: "浏览器" }] }],
      }),
    ),
    { 登录: "浏览器" },
  ),
  { "用哪种登录？": "浏览器" },
);
// Answer keys are exact (question text first, header fallback); values pass
// through verbatim — trim only decides emptiness, like Paseo's
// readNonEmptyString.
assert.deepEqual(
  resolveQuestionAnswers(
    parseAskFormQuestions(
      JSON.stringify({
        questions: [{ question: "用哪种登录？", header: "登录", options: [{ label: "浏览器" }] }],
      }),
    ),
    { 登录: " 浏览器 " },
  ),
  { "用哪种登录？": " 浏览器 " },
);
assert.deepEqual(
  resolveQuestionAnswers(
    parseAskFormQuestions(
      JSON.stringify({
        questions: [{ question: "用哪种登录？", header: "登录", options: [{ label: "浏览器" }] }],
      }),
    ),
    { 登录: "   " },
  ),
  {},
);

// Phone free-text replies resolve indices first, then labels, then typed text.
const phoneQuestion = parseAskFormQuestions(
  JSON.stringify({
    questions: [
      {
        question: "要检查哪些部分？",
        options: [{ label: "网络" }, { label: "配置" }, { label: "日志" }],
        multiSelect: true,
        allowOther: true,
      },
    ],
  }),
  "AskUserQuestion",
)[0];
assert.deepEqual(channelOptionPicks("1, 3", phoneQuestion), { selected: ["网络", "日志"], custom: null });
assert.deepEqual(channelOptionPicks("网络, 日志", phoneQuestion), { selected: ["网络", "日志"], custom: null });
assert.deepEqual(channelOptionPicks("网络, 还有数据库", phoneQuestion), {
  selected: ["网络"],
  custom: "还有数据库",
});
const singlePick = parseAskFormQuestions(
  JSON.stringify({
    questions: [
      {
        question: "完成后怎么处理？",
        options: [{ label: "只报告" }, { label: "直接修复" }],
        multiSelect: false,
      },
    ],
  }),
  "Bash",
)[0];
assert.deepEqual(channelOptionPicks("2", singlePick), { selected: ["直接修复"], custom: null });
assert.deepEqual(channelOptionPicks("只报告", singlePick), { selected: ["只报告"], custom: null });
assert.deepEqual(channelOptionPicks("先别动", singlePick), { selected: [], custom: "先别动" });

console.log("question form core tests passed");
