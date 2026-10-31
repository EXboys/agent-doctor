/**
 * One Feishu conversation stays one desktop chat. Later sentences queue.
 * Run from desktop/: node --import tsx tests/channel-session.test.ts
 */

import {
  advanceChannelCursor,
  assistantForChannelTurn,
  channelDelta,
  channelTurnTrace,
  settleChannelAnswer,
  channelConfirmAction,
  enqueueChannelJob,
  notifyChannelSessionIdle,
  setChannelSessionIdle,
  takeNextChannelJob,
  unshiftChannelJob,
} from "../src/chat/channel-lane";
import { channelKeyOf, placeChannelSession } from "../src/chat/channel-route";

function assert(cond: boolean, message: string): void {
  if (!cond) throw new Error(message);
}

const same = channelKeyOf("");
const other = channelKeyOf("oc_other");
assert(same === "feishu", "a job without a conversation shares one chat");
assert(other === "oc_other", "another Feishu conversation is another key");

type Session = { id: string; channelKey?: string | null; title: string };
const first = placeChannelSession<Session>([], "oc_a", () => ({ id: "s1", title: "打开项目" }));
assert(first.created && first.session.channelKey === "oc_a", "first sentence opens a chat");
const again = placeChannelSession(first.sessions, "oc_a", () => ({ id: "s2", title: "下一句" }));
assert(!again.created && again.session.id === "s1", "the same conversation stays in that chat");
const next = placeChannelSession(again.sessions, "oc_b", () => ({ id: "s3", title: "另一件" }));
assert(next.created && next.session.id === "s3", "another conversation opens another chat");
assert(next.sessions[1]?.id === "s1", "the first chat is still there");

const sessionId = "s1";
let idle = "";
setChannelSessionIdle((id) => {
  idle = id;
});
notifyChannelSessionIdle(sessionId);
assert(idle === sessionId, "a finished chat can start its next sentence");
enqueueChannelJob({ id: "j1", sessionId, text: "第一句", noticed: true });
enqueueChannelJob({ id: "j2", sessionId, text: "第二句", noticed: true });
unshiftChannelJob({ id: "j0", sessionId, text: "先做这句", noticed: true });
assert(takeNextChannelJob(sessionId)?.id === "j0", "a sentence that lost the race stays first");
assert(takeNextChannelJob(sessionId)?.id === "j1", "then the first queued sentence");
assert(takeNextChannelJob(sessionId)?.id === "j2", "then the next sentence");
assert(takeNextChannelJob(sessionId) === undefined, "the queue is empty after that");

const messages = [
  { role: "user", content: "第一句", at: 10 },
  { role: "assistant", content: "先做完这句", at: 11 },
  { role: "user", content: "第二句", at: 20 },
  { role: "assistant", content: "接着做完", at: 21 },
];
assert(
  assistantForChannelTurn(messages, 9, "第一句") === "先做完这句",
  "the reply stays with the sentence that asked",
);
assert(
  assistantForChannelTurn(messages, 19, "第二句") === "接着做完",
  "the next sentence gets its own reply",
);
assert(assistantForChannelTurn(messages, 19, "第一句") === "", "an old sentence is not the new reply");

const live = [
  { role: "user", content: "看一下桌面", at: 30 },
  { role: "thinking", content: "先看屏幕上有什么。", at: 31 },
  {
    role: "tool",
    content: "调用工具 screen_snapshot…",
    at: 32,
    step: {
      name: "screen_snapshot",
      status: "done",
      command: "",
      output: "三个窗口",
    },
  },
  { role: "assistant", content: "桌面上有三个窗口。", at: 33 },
];
const traced = channelTurnTrace(live, 29, "看一下桌面");
assert(traced.thinking === "先看屏幕上有什么。", "thinking from this sentence is included");
assert(traced.tools.length === 1 && traced.tools[0]?.name === "screen_snapshot", "the tool run is included");
assert(traced.tools[0]?.output === "三个窗口", "the tool result is included");
assert(traced.files.length === 0, "a look at the screen is not a file change");
const edited = channelTurnTrace(
  [
    { role: "user", content: "改一下", at: 40 },
    {
      role: "tool",
      content: "调用工具 edit…",
      at: 41,
      step: {
        kind: "edit",
        path: "/proj/src/app.ts",
        additions: 2,
        deletions: 1,
        added_lines: ["const next = 1;", "return next;"],
        deleted_lines: ["return 0;"],
      },
    },
    {
      role: "tool",
      content: "调用工具 write…",
      at: 42,
      step: {
        kind: "write",
        path: "/proj/notes.md",
        additions: 1,
        added_lines: ["# 记录"],
      },
    },
  ],
  39,
  "改一下",
);
assert(
  edited.files.length === 2 &&
    edited.files[0]?.kind === "change" &&
    edited.files[0]?.name === "app.ts" &&
    edited.files[0]?.addedLines[0] === "const next = 1;" &&
    edited.files[1]?.kind === "output" &&
    edited.files[1]?.name === "notes.md",
  "edits and written files travel with the turn",
);
assert(
  traced.tools[0]?.status === "done",
  "a finished tool says it is done",
);
assert(
  channelTurnTrace(live, 29, "另一句").tools.length === 0 &&
    channelTurnTrace(live, 29, "另一句").thinking === "",
  "another sentence does not borrow this stream",
);
assert(
  channelTurnTrace(
    [
      { role: "user", content: "看一下桌面", at: 30 },
      { role: "tool", content: "调用工具 screen_snapshot…\n全屏", at: 31 },
    ],
    29,
    "看一下桌面",
  ).tools[0]?.output === "全屏",
  "a tool announced as text still carries what it did",
);

const blank = { text: "", thinking: "", tools: [] as { id: string; name: string; status: "done"; detail: string; output: string }[] };
const opening = channelDelta(blank, { text: "你好", thinking: "先看", tools: [] });
assert(opening?.text === "你好" && opening.thinking === "先看" && !opening.resetText, "the first piece is the whole start");
const cursor = advanceChannelCursor(blank, opening!);
const second = channelDelta(cursor, {
  text: "你好世界",
  thinking: "先看屏幕",
  tools: [{ id: "tool-1", name: "看屏幕", status: "done", detail: "", output: "三个窗口" }],
});
assert(second?.text === "世界" && second.thinking === "屏幕", "later pieces are only the new tail");
assert(second?.tools.length === 1 && second.tools[0]?.id === "tool-1", "a new tool is one row, not the whole list");
assert(channelDelta(advanceChannelCursor(cursor, second!), {
  text: "你好世界",
  thinking: "先看屏幕",
  tools: [{ id: "tool-1", name: "看屏幕", status: "done", detail: "", output: "三个窗口" }],
}) === null, "nothing is sent when the sentence did not grow");

let growing = "";
const frames = ["先看一眼", "先看一眼。", "先看一眼。\n\n桌面上有这些项目。"];
const settled = await settleChannelAnswer(
  () => growing,
  {
    flush: () => {
      if (frames.length) growing = frames.shift() ?? growing;
    },
    wait: async () => {},
  },
);
assert(settled === "先看一眼。\n\n桌面上有这些项目。", "the last body is sent after it stops growing");

assert(
  channelConfirmAction({ confirmToken: true, pendingChoices: 2, busy: true }) === "confirm",
  "one reply confirms every step still waiting",
);
assert(
  channelConfirmAction({ confirmToken: true, pendingChoices: 0, busy: true }) === "repeat",
  "a second 1 is not a new sentence",
);
assert(
  channelConfirmAction({ confirmToken: false, pendingChoices: 0, busy: true }) === "queue",
  "a real sentence still waits its turn",
);
assert(
  channelConfirmAction({ confirmToken: false, pendingChoices: 0, busy: false }) === "run",
  "an idle chat starts now",
);

console.log("channel-session ok");
