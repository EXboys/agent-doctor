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
const { createBubblesController } = await import("../src/chat/bubbles");
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

  // Stage 2: two more Explore tools after the prose (turn still running)
  ctrl.pushToolStep({ id: "t3", kind: "search", name: "Grep", status: "done", query: "mihomo" });
  ctrl.pushToolStep({ id: "t4", kind: "terminal", name: "PowerShell", status: "done", command: "schtasks /query" });

  const liveKids = Array.from(logEl.children);
  check(
    "while running, earlier tool stage stays anchored above prose and new live stage opens below prose",
    liveKids.length === 3 &&
      liveKids[0]!.classList.contains("chat-stage-group") &&
      liveKids[1] === prose &&
      liveKids[2]!.classList.contains("chat-work-stream"),
    liveKids.map((k) => k.className),
  );

  // Final result arrives and turn completes -> entire pre-result process folds into one `.chat-work`
  const finalAnswer = document.createElement("div");
  finalAnswer.className = "chat-msg chat-msg-assistant";
  finalAnswer.textContent = "结论：计划任务触发器导致 mihomo 重启。";
  logEl.appendChild(finalAnswer);
  ctrl.sealProcessStream();

  const settledKids = Array.from(logEl.children);
  const workModule = settledKids[0] as HTMLDetailsElement | undefined;
  check(
    "after result returns, all execution stages + intermediate prose collapse into one closed .chat-work module",
    settledKids.length === 2 &&
      Boolean(workModule?.classList.contains("chat-work")) &&
      workModule?.open === false &&
      settledKids[1] === finalAnswer,
    settledKids.map((k) => k.className),
  );
  const workBodyKids = Array.from(workModule?.querySelector(".chat-work-body")?.children ?? []);
  check(
    "expanding .chat-work reveals chronological stage groups interleaved with stage summary prose",
    workBodyKids.length === 3 &&
      workBodyKids[0]!.classList.contains("chat-stage-group") &&
      workBodyKids[1]!.classList.contains("chat-work-prose") &&
      workBodyKids[2]!.classList.contains("chat-stage-group"),
    workBodyKids.map((k) => k.className),
  );
}

// 7. Completed repaint via createBubblesController: single tool & multi-stage turns fold into .chat-work
{
  const logEl = document.createElement("div");
  logEl.className = "chat-log";
  const titleEl = document.createElement("div");
  let busy = false;
  let completing = false;
  const session: import("../src/chat/types").ChatSession = {
    id: "s1",
    title: "测试折叠",
    runtime: "doctor",
    createdAt: 1,
    updatedAt: 10,
    messages: [
      { id: "u1", role: "user", content: "检查项目配置", at: 1 },
      { id: "th1", role: "thinking", content: "先读取配置文件", at: 2, durationMs: 3000 },
      {
        id: "tl1",
        role: "tool",
        content: "调用工具 Read…\nconfig.toml",
        at: 3,
        step: { id: "st1", kind: "read", name: "Read", status: "done", path: "config.toml" },
      },
      { id: "a_mid", role: "assistant", content: "已读完配置，接下来运行检查命令。", at: 4 },
      {
        id: "tl2",
        role: "tool",
        content: "调用工具 Bash…\ncargo check",
        at: 5,
        step: { id: "st2", kind: "terminal", name: "Bash", status: "done", command: "cargo check" },
      },
      {
        id: "tl3",
        role: "tool",
        content: "调用工具 Bash…\ncargo test",
        at: 6,
        step: { id: "st3", kind: "terminal", name: "Bash", status: "done", command: "cargo test" },
      },
      { id: "a_final", role: "assistant", content: "检查完成，配置正常。", at: 7 },
    ],
  };
  let assistantBubble: HTMLElement | null = null;
  let assistantMessageId: string | null = null;
  let assistantRaw = "";
  let pendingText = "";
  let turnHadText = false;
  let activityEl: HTMLElement | null = null;
  let toolGroupEl: HTMLDetailsElement | null = null;
  let lifecycleEl: HTMLElement | null = null;

  const bubbles = createBubblesController({
    logEl,
    titleEl,
    getBusy: () => busy,
    getStore: () => ({ activeId: session.id, sessions: [session] }),
    isViewingRunningSession: () => busy,
    isTurnCompleting: () => completing,
    runTargetSession: () => session,
    activeSession: () => session,
    selectedRuntime: () => "doctor",
    sessionTitle: (s) => s.title,
    touchSession: () => {},
    saveStore: () => {},
    flushStorePersist: () => {},
    scheduleStorePersist: () => {},
    scheduleSessionListRender: () => {},
    renderSessionList: () => {},
    updateContextMeter: () => {},
    pushActivity: () => {},
    settleActivity: () => {},
    finishToolGroup: () => {},
    dismissLifecycleActivity: () => {},
    collapseResolvedPermissionsBeforeAssistant: () => {},
    renderPermissionCard: () => document.createElement("div"),
    renderPermissionGroup: () => document.createElement("details"),
    getPendingPermissionBatch: () => [],
    getAssistantBubble: () => assistantBubble,
    setAssistantBubble: (el) => {
      assistantBubble = el;
    },
    getAssistantMessageId: () => assistantMessageId,
    setAssistantMessageId: (id) => {
      assistantMessageId = id;
    },
    getAssistantRaw: () => assistantRaw,
    setAssistantRaw: (raw) => {
      assistantRaw = raw;
    },
    getPendingText: () => pendingText,
    setPendingText: (text) => {
      pendingText = text;
    },
    getTurnHadAssistantText: () => turnHadText,
    setTurnHadAssistantText: (v) => {
      turnHadText = v;
    },
    getActivityEl: () => activityEl,
    setActivityEl: (el) => {
      activityEl = el;
    },
    getToolGroupEl: () => toolGroupEl,
    setToolGroupEl: (el) => {
      toolGroupEl = el;
    },
    getLifecycleActivityEl: () => lifecycleEl,
    setLifecycleActivityEl: (el) => {
      lifecycleEl = el;
    },
    addProject: () => {},
    openProvider: () => {},
    draftPrompt: () => {},
  });

  // Completed repaint (same path stream.ts runs on `completed` with isTurnCompleting = true)
  busy = true;
  completing = true;
  bubbles.renderActiveMessages();
  busy = false;
  completing = false;

  const kids = Array.from(logEl.children);
  const fold = kids[1] as HTMLDetailsElement | undefined;
  const label = fold?.querySelector(".chat-work-label")?.textContent ?? "";
  check(
    "completed repaint renders [user, closed .chat-work, final assistant]",
    kids.length === 3 &&
      kids[0]!.classList.contains("chat-msg-user") &&
      Boolean(fold?.classList.contains("chat-work")) &&
      fold?.open === false &&
      kids[2]!.classList.contains("chat-msg-assistant"),
    kids.map((k) => k.className),
  );
  check(
    "collapsed .chat-work header summarizes read and terminal counts",
    label.includes("查阅 1 个文件") && label.includes("执行 2 条命令"),
    label,
  );
  const inner = Array.from(fold?.querySelector(".chat-work-body")?.children ?? []);
  check(
    "inside .chat-work-body: thinking, single read row, stage summary prose, and grouped terminal stage",
    inner.length === 4 &&
      inner[0]!.classList.contains("chat-thinking") &&
      inner[1]!.classList.contains("chat-tool-row") &&
      inner[2]!.classList.contains("chat-work-prose") &&
      inner[3]!.classList.contains("chat-stage-group"),
    inner.map((n) => n.className),
  );
}

if (failures > 0) {
  console.log(`${failures} failed`);
  process.exit(1);
}
process.exit(0);
