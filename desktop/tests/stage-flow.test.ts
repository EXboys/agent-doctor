/**
 * Stage-grouped timeline & interleaving tests (Paseo / ZCode rhythm):
 * - Explore / Terminal / Changes / Browser stage grouping (threshold >= 2)
 * - Thinking inline streaming ticker & collapsed default
 * - Read-only PowerShell / Bash classification into Explore
 * - Parallel bare tool announcements deduplication
 * - Prose segment boundary preservation (stage summary stays above later tool stages)
 *
 * Run from desktop/:
 *   node --import tsx tests/stage-flow.test.ts
 */

import { parseHTML } from "linkedom";

const { document, window } = parseHTML('<html><body><div id="log"></div></body></html>');
(globalThis as any).document = document;
(globalThis as any).window = window;
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
for (const key of [
  "HTMLElement",
  "HTMLDetailsElement",
  "HTMLButtonElement",
  "HTMLInputElement",
  "HTMLTextAreaElement",
  "SVGCircleElement",
  "ShadowRoot",
  "CustomEvent",
  "MutationObserver",
]) {
  if ((window as any)[key]) (globalThis as any)[key] = (window as any)[key];
}

const { createToolRowElement, createActivityController } = await import("../src/chat/activity");
const {
  isReadOnlyTerminalCommand,
  plainRunningActivityText,
  plainRunningToolText,
  toolActivityInfo,
} = await import("../src/chat/format");
const { omitBareToolStarts, storeToolStep } = await import("../src/chat/tool-records");
const {
  appendToolRowToStageStream,
  buildStageFlowNodes,
  classifyToolRowStage,
  sealProcessContainer,
  wrapProcessRows,
} = await import("../src/chat/work-trail");
const { latestThinkingSnippet, renderThinkingBlock } = await import("../src/chat/thinking");
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

// 1. PowerShell & read-only shell classification
{
  const psInfo = toolActivityInfo("调用工具 PowerShell…\nGet-ScheduledTask -TaskName mihomo*");
  check("PowerShell classifies as terminal, not generic 工具", psInfo.kind === "terminal" && psInfo.label === "终端", psInfo);

  check("Get-ScheduledTask is a read-only Explore shell command", isReadOnlyTerminalCommand("Get-ScheduledTask -TaskName mihomo*"));
  check("schtasks /query is a read-only Explore shell command", isReadOnlyTerminalCommand("schtasks /query /fo LIST /v"));
  check("reg query is a read-only Explore shell command", isReadOnlyTerminalCommand("reg query HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run"));
  check("wevtutil qe is a read-only Explore shell command", isReadOnlyTerminalCommand("wevtutil qe Security /q:\"*[System[(EventID=4688)]]\""));
  check("Set-Content is NOT read-only", !isReadOnlyTerminalCommand("Set-Content -Path poll.ps1 -Value 'x'"));
  check("npm run build is NOT read-only", !isReadOnlyTerminalCommand("npm run build"));

  check(
    "running bar strips raw 调用工具 PowerShell… into human-readable label",
    plainRunningActivityText("tool", "调用工具 PowerShell…") === "终端",
    plainRunningActivityText("tool", "调用工具 PowerShell…"),
  );
  check(
    "running bar formats structured step cleanly",
    plainRunningToolText({ id: "p1", kind: "read", status: "running", path: "/a/watchdog.ps1" }) === "查阅 · watchdog.ps1",
  );
}

// 2. Stage grouping threshold (>= 2) and boundaries
{
  const r1 = createToolRowElement("调用工具 Read…\nwatchdog.ps1", {
    id: "r1",
    kind: "read",
    status: "done",
    path: "/work/watchdog.ps1",
  });
  const r2 = createToolRowElement("调用工具 PowerShell…\nGet-ScheduledTask", {
    id: "r2",
    kind: "terminal",
    status: "done",
    name: "PowerShell",
    title: "List enabled scheduled tasks",
    command: "Get-ScheduledTask | Where-Object State -eq Ready",
  });
  const t1 = renderThinkingBlock("先排查计划任务和启动项\n看看有没有触发器");
  const e1 = createToolRowElement("调用工具 PowerShell…\nStart-Process mihomo.exe", {
    id: "e1",
    kind: "terminal",
    status: "done",
    name: "PowerShell",
    command: "Start-Process mihomo.exe",
  });
  const e2 = createToolRowElement("调用工具 PowerShell…\nRestart-Service mihomo", {
    id: "e2",
    kind: "terminal",
    status: "failed",
    name: "PowerShell",
    command: "Restart-Service mihomo",
  });

  check("Read row classifies as explore", classifyToolRowStage(r1) === "explore");
  check("Read-only PowerShell row classifies as explore", classifyToolRowStage(r2) === "explore");
  check("Mutating PowerShell row classifies as terminal", classifyToolRowStage(e1) === "terminal");

  // Single row stays standalone
  const lone = buildStageFlowNodes([createToolRowElement("调用工具 Read…\na.md", { id: "s1", kind: "read", status: "done", path: "a.md" })], false);
  check("a single tool row stays standalone without a wrapper group", lone.length === 1 && lone[0]!.classList.contains("chat-tool-row"));

  // [Explore, Explore, Thinking, Execute, Execute] -> [ExploreGroup(2), Thinking, TerminalGroup(2)]
  const staged = buildStageFlowNodes([r1, r2, t1, e1, e2], false);
  check(
    "consecutive explore and execute tools fold around thinking in chronological order",
    staged.length === 3 &&
      staged[0]!.classList.contains("chat-stage-group") &&
      (staged[0] as HTMLElement).dataset.stage === "explore" &&
      staged[1]!.classList.contains("chat-thinking") &&
      staged[2]!.classList.contains("chat-stage-group") &&
      (staged[2] as HTMLElement).dataset.stage === "terminal",
    staged.map((n) => n.className),
  );
  const exploreLabel = staged[0]!.querySelector(".chat-stage-label")?.textContent ?? "";
  check(
    "explore group summarizes files and read-only checks without 其他 N 项",
    exploreLabel.includes("查阅 1 个文件") && exploreLabel.includes("检查 1 项") && !exploreLabel.includes("其他"),
    exploreLabel,
  );
  const terminalLabel = staged[2]!.querySelector(".chat-stage-label")?.textContent ?? "";
  check(
    "terminal group summarizes command count and failure count",
    terminalLabel.includes("执行 2 条命令") && terminalLabel.includes("1 条没成功"),
    terminalLabel,
  );
}

// 3. Live streaming stage promotion & inline ticker
{
  const stream = wrapProcessRows([], true);
  const first = createToolRowElement("调用工具 Read…\nstart-mihomo.ps1", {
    id: "l1",
    kind: "read",
    status: "done",
    path: "/work/start-mihomo.ps1",
  });
  appendToolRowToStageStream(stream, first);
  check("first live explore row stays a direct row", stream.children.length === 1 && stream.children[0]!.classList.contains("chat-tool-row"));

  const second = createToolRowElement("调用工具 Grep…\nwatchdog", {
    id: "l2",
    kind: "search",
    status: "running",
    query: "watchdog",
  });
  second.classList.add("is-live");
  appendToolRowToStageStream(stream, second);
  const group = stream.children[0] as HTMLDetailsElement;
  check(
    "second consecutive explore row promotes both into one collapsed live stage bar",
    stream.children.length === 1 && group.classList.contains("chat-stage-group") && group.classList.contains("is-live") && !group.open,
  );
  const liveLabel = group.querySelector(".chat-stage-label")?.textContent ?? "";
  const liveDetail = group.querySelector(".chat-stage-detail")?.textContent ?? "";
  check("live explore bar shows stage title + latest action ticker", liveLabel === "正在读取信息" && liveDetail.includes("搜索 watchdog"), {
    liveLabel,
    liveDetail,
  });

  sealProcessContainer(stream);
}

// 4. Thinking inline streaming summary (collapsed by default while live)
{
  const snippet = latestThinkingSnippet("先检查目录结构\n再读取 **watchdog.ps1** 确认自启逻辑\n");
  check("thinking snippet picks the latest non-empty line without markdown marks", snippet === "再读取 watchdog.ps1 确认自启逻辑", snippet);

  const liveThink = renderThinkingBlock("第一行思考\n正在比对计划任务日志", { id: "th1", live: true });
  check("live thinking block defaults to collapsed single-line ticker", !liveThink.open && liveThink.classList.contains("is-live"));
  const thinkSnippet = liveThink.querySelector(".chat-thinking-snippet")?.textContent ?? "";
  check("live thinking header carries inline snippet", thinkSnippet === "· 正在比对计划任务日志", thinkSnippet);

  const savedThink = renderThinkingBlock("思考全文", { id: "th2", durationMs: 4200 });
  const savedLabel = savedThink.querySelector(".chat-thinking-label")?.textContent ?? "";
  check("saved thinking block restores elapsed duration across repaints", savedLabel === "思考了 4 秒", savedLabel);
}

// 5. Parallel bare tool announcements deduplication
{
  const parallel = omitBareToolStarts([
    "调用工具 Read…",
    "调用工具 PowerShell…",
    { text: "调用工具 Read…\na.md", step: { id: "p_r", kind: "read", name: "Read", status: "done", path: "a.md" } },
    { text: "调用工具 PowerShell…\nls", step: { id: "p_s", kind: "terminal", name: "PowerShell", status: "done", command: "ls" } },
  ]);
  check("parallel bare starts pair with their later steps leaving zero orphan rows", parallel.length === 2, parallel);

  let seq = 0;
  const uid = () => `id_${(seq += 1)}`;
  const msgs: ChatMessage[] = [
    { id: "u1", role: "user", content: "查一下", at: 1 },
    { id: "b1", role: "tool", content: "调用工具 Read…", at: 2 },
    { id: "b2", role: "tool", content: "调用工具 PowerShell…", at: 3 },
  ];
  storeToolStep(msgs, { id: "s_r", kind: "read", name: "Read", status: "running", path: "a.md" }, null, uid);
  storeToolStep(msgs, { id: "s_p", kind: "terminal", name: "PowerShell", status: "running", command: "dir" }, null, uid);
  const tools = msgs.filter((m) => m.role === "tool");
  check(
    "storeToolStep adopts matching bare messages across parallel starts",
    tools.length === 2 && tools[0]?.step?.id === "s_r" && tools[1]?.step?.id === "s_p",
    tools,
  );
}

// 6. Prose segment boundary preservation: earlier tools stay ABOVE stage prose, later tools land BELOW
{
  const logEl = document.createElement("div");
  logEl.className = "chat-log";
  document.body.appendChild(logEl);
  let actEl: HTMLElement | null = null;
  let lifeEl: HTMLElement | null = null;
  let groupEl: HTMLDetailsElement | null = null;
  const ctrl = createActivityController({
    logEl,
    isViewingRunningSession: () => true,
    flushPendingTextSync: () => {},
    sealAssistantBubble: () => {},
    collapseResolvedPermissionsBeforeAssistant: () => {},
    getActivityEl: () => actEl,
    setActivityEl: (el) => {
      actEl = el;
    },
    getLifecycleActivityEl: () => lifeEl,
    setLifecycleActivityEl: (el) => {
      lifeEl = el;
    },
    getToolGroupEl: () => groupEl,
    setToolGroupEl: (el) => {
      groupEl = el;
    },
    rememberTool: () => {},
    rememberToolStep: () => {},
    rememberRuntimeLog: () => {},
    rememberTurnNotice: () => {},
    toolRecordsForTurn: () => [],
  });

  // Stage 1: two Explore tools
  ctrl.pushToolStep({ id: "t1", kind: "read", name: "Read", status: "done", path: "watchdog.ps1" });
  ctrl.pushToolStep({ id: "t2", kind: "read", name: "Read", status: "done", path: "start-mihomo.ps1" });

  // Stage summary prose bubble lands after Stage 1
  const prose = document.createElement("div");
  prose.className = "chat-msg chat-msg-assistant";
  prose.textContent = "已读完启动脚本，接下来检查注册表与计划任务日志。";
  logEl.appendChild(prose);

  // Stage 2: two more Explore tools after the prose
  ctrl.pushToolStep({ id: "t3", kind: "search", name: "Grep", status: "done", query: "mihomo" });
  ctrl.pushToolStep({ id: "t4", kind: "terminal", name: "PowerShell", status: "done", command: "schtasks /query" });
  ctrl.sealProcessStream();

  const kids = Array.from(logEl.children);
  check(
    "earlier tool stage stays anchored above prose and new tool stage opens below prose",
    kids.length === 3 &&
      kids[0]!.classList.contains("chat-stage-group") &&
      kids[1] === prose &&
      kids[2]!.classList.contains("chat-stage-group"),
    kids.map((k) => k.className),
  );
}

if (failures > 0) {
  console.log(`${failures} failed`);
  process.exit(1);
}
process.exit(0);
