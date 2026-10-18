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

const { repeatRuns, stepActivityInfo, mergeToolStep, summarizeToolActivities } = await import("../src/chat/format");
const { earlierToolRange, laterToolRange, storeToolStep } = await import("../src/chat/tool-records");
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

const look = stepActivityInfo({
  id: "c",
  kind: "other",
  status: "done",
  name: "mcp__computer__screen_snapshot",
});
check("a computer look says what was seen", look.label === "看了一眼屏幕" && look.target === "", look);

const looked = stepActivityInfo({
  id: "c2",
  kind: "other",
  status: "done",
  name: "screen_snapshot",
  output: "front app: 微信\nwindow: 微信\n",
});
check("a look names the app it saw", looked.label === "看了一眼「微信」", looked);

const opened = stepActivityInfo({
  id: "o",
  kind: "other",
  status: "done",
  name: "mcp__computer__screen_open",
  title: "微信",
});
check("opening an app names the app", opened.label === "打开了「微信」" && opened.target === "", opened);

const settings = stepActivityInfo({
  id: "settings",
  kind: "other",
  status: "done",
  name: "screen_open",
  output: '{\n  "bundleId": "com.apple.systempreferences",\n  "opened": "System Settings",\n  "path": "/System/Applications/System Settings.app"\n}',
});
check("system settings is named in plain words", settings.label === "打开了「系统设置」", settings);

const finder = stepActivityInfo({
  id: "finder",
  kind: "other",
  status: "done",
  name: "screen_open",
  title: "Finder",
});
check("finder is named in plain words", finder.label === "打开了「访达」", finder);

const clicked = stepActivityInfo({
  id: "k",
  kind: "other",
  status: "done",
  name: "screen_click",
  title: "发送",
});
check("a click names the button", clicked.label === "点了「发送」", clicked);

const dumped = stepActivityInfo({
  id: "dump",
  kind: "other",
  status: "done",
  name: "screen_snapshot",
  title: "计算器\\n@e1 text 789×45 12 34",
  output: "front app: 计算器\n@e1 text 789×45 12 34\n@e3 button 全部清除 315 582\n",
});
check("a look keeps the app and hides element rows", dumped.label === "看了一眼「计算器」", dumped);

const clickRef = stepActivityInfo({
  id: "ref",
  kind: "other",
  status: "done",
  name: "screen_click",
  title: "@e3 button 全部清除 315 582",
});
check("a click keeps the button name and drops the element code", clickRef.label === "点了「全部清除」", clickRef);

const spot = stepActivityInfo({
  id: "p",
  kind: "other",
  status: "failed",
  name: "screen_click",
  output: "这一下没有点到。画面没有变化。",
});
check("a missed click says so", spot.label === "在画面上点了一下 · 这一下没有点到", spot);

const typed = stepActivityInfo({
  id: "y",
  kind: "other",
  status: "done",
  name: "screen_press",
  title: "command+a",
});
check("a key press uses a plain name", typed.label === "按下了全选", typed);

const runs = repeatRuns(["点一下", "点一下", "点一下", "打开应用\n微信", "打开应用\n微信"]);
check("identical rows fold into one count", runs.join(",") === "3,0,0,2,0", runs);

const bareRead = stepActivityInfo({ id: "rd", kind: "read", status: "done", name: "Read" });
check("a read without a file does not repeat the English name", bareRead.label === "查阅" && bareRead.target === "", bareRead);

const summary = summarizeToolActivities([
  stepActivityInfo({ id: "f", kind: "read", status: "done", path: "notes.md" }),
  look,
]);
check("the group names the look instead of other", summary.includes("看了一眼屏幕") && !summary.includes("其他"), summary);

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

const longTask: ChatMessage[] = [{ id: "u2", role: "user", content: "做完", at: 10 }];
for (let i = 0; i < 50; i += 1) {
  storeToolStep(longTask, { id: `t${i}`, kind: "terminal", status: "done", command: `echo ${i}` }, null, newId);
}
const kept = longTask.filter((message) => message.role === "tool");
check("a long task keeps every step so scrolling can load it", kept.length === 50, kept.length);
check("the first step is still there", kept[0]?.step?.id === "t0" && kept[49]?.step?.id === "t49");
const earlier = earlierToolRange(60);
check("scrolling up loads the previous page", earlier?.from === 20 && earlier?.to === 60, earlier);
check("the top of the list has nothing older", earlierToolRange(0) === null);
const later = laterToolRange(39, 100);
check("scrolling back down loads the next page", later?.from === 40 && later?.to === 80, later);
check("the newest page has nothing after it", laterToolRange(49, 50) === null);
check("the previous reply is untouched", messages.filter((message) => message.role === "tool").length === 2);

if (failures > 0) {
  console.log(`${failures} failed`);
  process.exit(1);
}
