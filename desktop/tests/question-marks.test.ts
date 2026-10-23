/**
 * Question ticks: one per user turn, reply excerpt, pending only on the newest.
 *
 * Run from desktop/: node --import tsx tests/question-marks.test.ts
 */
const { currentQuestionId, questionMarks } = await import("../src/chat/question-marks.ts");

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
  console.log(`ok - ${msg}`);
}

const marks = questionMarks([
  { id: "u1", role: "user", content: "扫码登录后切不回应用\n再说明一下" },
  { id: "a1", role: "assistant", content: "扫完微信会回到授权页。\n\n```\nsecret\n```" },
  { id: "u2", role: "user", content: "   ", attachments: [{ name: "截图.png" }] },
  { id: "u3", role: "user", content: "push" },
]);

assert(marks.length === 3, "blank text still counts when a file is attached");
assert(marks[0]?.title === "扫码登录后切不回应用", "title is the first line");
assert(marks[0]?.reply.startsWith("扫完微信会回到授权页"), "reply is the answer that followed");
assert(!marks[0]?.reply.includes("secret"), "a code block is not the excerpt");
assert(marks[1]?.title === "截图.png", "a picture with no words uses its name");
assert(marks[1]?.reply === "" && marks[1]?.pending === false, "an earlier gap is not still answering");
assert(marks[2]?.pending === true, "only the newest unanswered question is still answering");

assert(questionMarks([{ id: "only", role: "user", content: "你好" }]).length === 1, "one question is still listed");

const tops = [
  { id: "q1", top: 10 },
  { id: "q2", top: 180 },
  { id: "q3", top: 360 },
];
assert(currentQuestionId(tops, { top: 0, atEnd: true }) === "q3", "the bottom of the chat marks the last question");
assert(currentQuestionId(tops, { top: 0, atEnd: false }) === "q1", "higher up, the question at the top edge is current");
assert(currentQuestionId([], { top: 0, atEnd: true }) === null, "no questions means no current mark");
