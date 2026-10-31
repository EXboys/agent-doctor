/**
 * A plan written in the reply still becomes the corner checklist.
 * Run from desktop/: node --import tsx tests/plan-from-reply.test.ts
 */
export {};

const storage = new Map<string, string>([["agent-doctor.locale", "zh"]]);
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
    removeItem: (key: string) => storage.delete(key),
  },
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { language: "zh-CN" },
});

const { planStepsFromMessages } = await import("../src/plan.ts");

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
  console.log(`ok - ${msg}`);
}

const messages = [
  { role: "user", content: "先列一个三步计划" },
  {
    role: "assistant",
    content: "三步计划：\n1. 列出当前文件夹里的内容\n2. 数一下一共有几个条目\n3. 用一句话告诉你数量和概况",
  },
  { role: "assistant", content: "第三步：桌面文件夹里共有 29 个条目。" },
];

const done = planStepsFromMessages(messages, true);
assert(done.length === 3 && done.every((item) => item.state === "done"), "a finished reply marks every step done");
assert(done[0]?.text === "列出当前文件夹里的内容", "the first step is the first numbered line");

const live = planStepsFromMessages(messages.slice(0, 2), false);
assert(live[0]?.state === "doing" && live[1]?.state === "pending", "before the work starts, the first step is current");

const noted = planStepsFromMessages(messages, false);
assert(noted[0]?.state === "done" && noted[2]?.state === "doing", "第3步 makes the third step current");

assert(
  planStepsFromMessages([{ role: "assistant", content: "1. 买菜\n2. 做饭" }], true).length === 0,
  "a numbered list that is not called a plan stays out of the corner",
);
