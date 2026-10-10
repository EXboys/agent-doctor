/**
 * Structured tool steps: how a step becomes a row, and how start + result
 * are saved as one tool message (src/chat/format.ts, src/chat/tool-records.ts).
 *
 * Run from desktop/:
 *   node --import tsx tests/tool-steps.test.ts
 */

export {};

const storage = new Map<string, string>([["agent-doctor.locale", "zh"]]);
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  writable: true,
  value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
    removeItem: (key: string) => storage.delete(key),
  },
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  writable: true,
  value: { language: "zh-CN" },
});

const { stepActivityInfo, mergeToolStep } = await import("../src/chat/format");
const { storeToolStep } = await import("../src/chat/tool-records");
type ChatMessage = import("../src/chat/types").ChatMessage;

let failures = 0;
function check(name: string, ok: boolean, detail?: unknown): void {
  if (ok) {
    console.log(`ok - ${name}`);
  } else {
    failures += 1;
    console.log(`not ok - ${name}`, detail ?? "");
  }
}

const write = stepActivityInfo({
  id: "a",
  kind: "write",
  status: "running",
  path: "/tmp/test_demo.txt",
  command: "cat > /tmp/test_demo.txt << 'EOF'",
  additions: 2,
  added_lines: ["hello", "world"],
});
check("a terminal write shows the file, not the command", write.kind === "write" && write.target === "test_demo.txt", write);
check("the file stays clickable", write.path === "/tmp/test_demo.txt");

const read = stepActivityInfo({ id: "r", kind: "read", status: "done", path: "src/send.ts", line_start: 330, line_end: 409 });
check("a read names its lines", read.target.includes("send.ts") && read.target.includes("330"), read);

const shell = stepActivityInfo({ id: "s", kind: "terminal", status: "running", title: "查看改动", command: "git status" });
check("a command shows its plain description first", shell.target === "查看改动", shell);

const merged = mergeToolStep(
  { id: "s", kind: "terminal", status: "running", command: "git status" },
  { id: "s", status: "done", output: "clean" },
);
check(
  "a result keeps what the start said",
  merged.kind === "terminal" && merged.command === "git status" && merged.status === "done" && merged.output === "clean",
  merged,
);

let n = 0;
const newId = () => `m${(n += 1)}`;
const messages: ChatMessage[] = [
  { id: "u", role: "user", content: "改一下", at: 1 },
  { id: "t0", role: "tool", content: "调用工具 Bash…\ngit status", at: 2 },
];
storeToolStep(messages, { id: "s", kind: "terminal", status: "running", command: "git status" }, null, newId);
check("the first step takes over the status line saved for it", messages.length === 2 && messages[1]?.step?.id === "s", messages);

storeToolStep(messages, { id: "s", status: "done", output: "x".repeat(4000) }, null, newId);
const saved = messages[1]?.step;
check("its result merges into the same message", messages.length === 2 && saved?.status === "done" && saved?.command === "git status", saved);
check("saved output is trimmed", (saved?.output?.length ?? 0) < 2000, saved?.output?.length);

messages.push({ id: "a1", role: "assistant", content: "", at: 3 });
storeToolStep(messages, { id: "e", kind: "edit", status: "running", path: "a.ts", additions: 1 }, "a1", newId);
check(
  "a new step lands before the reply that is still streaming",
  messages.length === 4 && messages[2]?.step?.id === "e" && messages[3]?.id === "a1",
  messages.map((m) => m.id),
);

check("a result for an unknown step is ignored", !storeToolStep(messages, { id: "zzz", status: "done" }, null, newId));

if (failures > 0) {
  console.log(`${failures} failed`);
  process.exit(1);
}
