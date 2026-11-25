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

const { deskChatKey, mapTranscriptRows } = await import("../src/chat/desk-transcripts");

assert.equal(deskChatKey("sess-1"), "desk:sess-1");

// user -> me, assistant -> pc with jobId; tool/thinking/meta rows stay local.
const rows = mapTranscriptRows([
  { id: "u1", role: "user", content: "帮我看下这段报错", at: 1 },
  { id: "t1", role: "tool", content: "tool output", at: 2 },
  { id: "th1", role: "thinking", content: "思考中", at: 3 },
  { id: "m1", role: "meta", content: "raw log", at: 4 },
  { id: "a1", role: "assistant", content: "是配置错了。", at: 5 },
  { id: "a2", role: "assistant", content: "   ", at: 6 },
] as never[]);
assert.deepEqual(
  rows,
  [
    { key: "m:u1", kind: "me", text: "帮我看下这段报错" },
    { key: "p:a1", kind: "pc", jobId: "a1", text: "是配置错了。" },
  ],
  JSON.stringify(rows),
);

// Long rows are clipped so one runaway answer cannot stuff the line.
const long = "啊".repeat(9000);
const clipped = mapTranscriptRows([
  { id: "u2", role: "user", content: long, at: 1 },
  { id: "a3", role: "assistant", content: long, at: 2 },
] as never[]);
assert.equal(clipped[0].text.length, 4000);
assert.equal(clipped[1].text.length, 8000);

// Only the last 200 rows ride along: the archive tail, not the whole project log.
const many = Array.from({ length: 260 }, (_, index) => ({
  id: `u${index}`,
  role: index % 2 ? "assistant" : "user",
  content: `row ${index}`,
  at: index,
})) as never[];
assert.equal(mapTranscriptRows(many).length, 200);
assert.equal(mapTranscriptRows(many)[0].text, "row 60");

console.log("desk transcripts ok");
