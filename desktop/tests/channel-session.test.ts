/**
 * One Feishu conversation stays one desktop chat. Later sentences queue.
 * Run from desktop/: node --import tsx tests/channel-session.test.ts
 */

import {
  assistantForChannelTurn,
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

console.log("channel-session ok");
