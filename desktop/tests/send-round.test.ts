/**
 * What a send leaves behind (src/chat/send.ts): the saved agent thread, the
 * auto-approve switch each agent gets, the chat a background send writes into,
 * and what the person sees when a round fails or Stop is pressed.
 *
 * The follow-up queue and Stop-holds-the-queue cases live in
 * follow-queue-send.test.ts. This file covers the rest of sendAsk / cancelAsk.
 *
 * Run from desktop/:
 *   node --import tsx tests/send-round.test.ts
 */

export {};

const storage = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  writable: true,
  value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => {
      storage.set(key, value);
    },
    removeItem: (key: string) => {
      storage.delete(key);
    },
  },
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  writable: true,
  value: { language: "en" },
});
Object.defineProperty(globalThis, "document", {
  configurable: true,
  writable: true,
  value: { createElement: () => ({}), querySelector: () => null },
});

type Report = {
  cwd: string;
  status: "succeeded" | "failed" | "cancelled" | "timed_out";
  duration_ms: number;
  runtime_thread_id: string | null;
  summary?: string;
};
type InvokeCall = { cmd: string; args: Record<string, unknown> };

const invokes: InvokeCall[] = [];
const timers: Array<{ fn: () => void; ms: number }> = [];
let confirmReply = true;
let confirmAsked = 0;
let startReply: () => Promise<Report> = async () => okReport();
let cancelReply: () => Promise<boolean> = async () => true;
let liveRuns: Array<{ client_run_id: string; session_id: string | null }> = [];

function okReport(extra: Partial<Report> = {}): Report {
  return { cwd: "/tmp/round", status: "succeeded", duration_ms: 7, runtime_thread_id: null, ...extra };
}

Object.defineProperty(globalThis, "window", {
  configurable: true,
  writable: true,
  value: {
    setTimeout: (fn: () => void, ms: number) => {
      timers.push({ fn, ms });
      return timers.length;
    },
    clearTimeout: () => {},
    confirm: () => {
      confirmAsked += 1;
      return confirmReply;
    },
    __TAURI_INTERNALS__: {
      invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
        invokes.push({ cmd, args });
        if (cmd === "start_prompt_session_command") return startReply();
        if (cmd === "cancel_prompt_session_command") return cancelReply();
        if (cmd === "list_prompt_runs_command") return liveRuns;
        if (cmd === "replay_prompt_asks_command") return 0;
        return null;
      },
    },
  },
});

const { createSendController } = await import("../src/chat/send.ts");
const { MAX_PARALLEL_RUNS } = await import("../src/chat/live-runs.ts");
const { consumeDrainIntent } = await import("../src/chat/follow-queue.ts");
const { t } = await import("../src/i18n.ts");

let failures = 0;
let checks = 0;

function check(name: string, fn: () => void | Promise<void>): Promise<void> {
  checks += 1;
  return Promise.resolve()
    .then(fn)
    .then(() => {
      console.log(`  ok   ${name}`);
    })
    .catch((error: unknown) => {
      failures += 1;
      console.log(`  FAIL ${name}\n         ${(error as Error).message}`);
    });
}

function eq<T>(actual: T, expected: T, what: string): void {
  const a = JSON.stringify(actual);
  const b = JSON.stringify(expected);
  if (a !== b) throw new Error(`${what}: got ${a}, expected ${b}`);
}

function ok(condition: boolean, what: string): void {
  if (!condition) throw new Error(what);
}

const roundCalls = () => invokes.filter((call) => call.cmd === "start_prompt_session_command");
const cancelCalls = () => invokes.filter((call) => call.cmd === "cancel_prompt_session_command");

type FakeSession = {
  id: string;
  title: string;
  runtime: string;
  messages: Array<{ role: string; content: string }>;
  runtimeThreadId: string | null;
  providerTag?: string | null;
  interrupted?: { status: string; at: number } | null;
  autoApprove?: boolean;
};

function makeSession(id: string, extra: Partial<FakeSession> = {}): FakeSession {
  return { id, title: "", runtime: "claude-code", messages: [], runtimeThreadId: null, ...extra };
}

function reset(): void {
  invokes.length = 0;
  timers.length = 0;
  confirmReply = true;
  confirmAsked = 0;
  startReply = async () => okReport();
  cancelReply = async () => true;
  liveRuns = [];
  consumeDrainIntent();
}

function newController(opts: {
  sessions: FakeSession[];
  activeId?: string;
  runtime?: string;
  elevated?: boolean;
  providerTag?: string;
  runningCount?: number;
}) {
  const sessions = opts.sessions;
  if (opts.elevated) {
    for (const session of sessions) session.autoApprove = true;
  }
  const store = { activeId: opts.activeId ?? sessions[0].id, sessions };
  const promptEl = { value: "" } as HTMLTextAreaElement;
  const state = {
    busy: false,
    busyGen: 1,
    statuses: [] as Array<{ text: string; tone?: string }>,
    bubbles: [] as Array<{ kind: string; text: string }>,
    persisted: [] as Array<{ role: string; content: string }>,
    displayedCwd: [] as string[],
    expired: 0,
    saves: 0,
  };
  const byId = (id: string | null | undefined) => sessions.find((session) => session.id === id);
  const deps = {
    promptEl,
    elevatedEl: { checked: opts.elevated === true } as HTMLInputElement,
    askResources: { mountedSkills: [], enabledMcps: [], selectedMentions: [], clearMentions: () => {} } as never,
    mentionMenu: { hideMentionMenu: () => {} } as never,
    getStore: () => store as never,
    getBusy: () => state.busy,
    getBusyGen: () => state.busyGen,
    runningCount: () => opts.runningCount ?? 0,
    getRunningChatSessionId: () => (state.busy ? store.activeId : null),
    getPendingAttachments: () => [],
    setPendingAttachments: () => {},
    resolveSendWorkspace: () => ({ cwd: "/tmp/ws", workspaceName: "default" }),
    getVerifyMcpTurn: () => false,
    setVerifyMcpTurn: () => {},
    getVerifySawBrowserNavigate: () => false,
    setVerifySawBrowserNavigate: () => {},
    getVerifyMcpReported: () => true,
    setVerifyMcpReported: () => {},
    getVerifyTurnText: () => "",
    setVerifyTurnText: () => {},
    getAssistantBubble: () => null,
    setAssistantBubble: () => {},
    getAssistantMessageId: () => null,
    setAssistantMessageId: () => {},
    getAssistantRaw: () => "",
    setAssistantRaw: () => {},
    getPendingText: () => "",
    setPendingText: () => {},
    getTurnHadAssistantText: () => false,
    setTurnHadAssistantText: () => {},
    setStatus: (text: string, tone?: string) => {
      state.statuses.push({ text, tone });
    },
    selectedRuntime: () => opts.runtime ?? "claude-code",
    activeSession: () => byId(store.activeId) as never,
    ensureListener: async () => {},
    clearQuickReplies: () => {},
    allowQuickRepliesAgain: () => {},
    setBusy: (next: boolean) => {
      state.busy = next;
      if (next) state.busyGen += 1;
    },
    pushActivity: () => {},
    persistMessage: (role: string, content: string) => {
      state.persisted.push({ role, content });
      return { id: `m-${state.persisted.length}` } as never;
    },
    appendBubble: (kind: string, text: string) => {
      state.bubbles.push({ kind, text });
      return {} as never;
    },
    autoResizePrompt: () => {},
    renderPendingAttachments: () => {},
    buildPromptWithHistory: (text: string) => text,
    setDisplayedCwd: (cwd: string) => {
      state.displayedCwd.push(cwd);
    },
    sessionById: (id: string | null | undefined) => byId(id) as never,
    runTargetSession: () => byId(store.activeId) as never,
    touchSession: () => {},
    saveStore: () => {
      state.saves += 1;
    },
    applyVerifyMcpFooter: () => {},
    applyVerifyEvidenceFromAssistant: () => {},
    reportVerifyMcpIfNeeded: () => {},
    expireLivePermissionCards: () => {
      state.expired += 1;
    },
    settleRunRouting: () => {},
    renderSessionList: () => {},
    readImageTextEnabled: () => false,
    refreshComposer: () => {},
    providerTag: () => opts.providerTag ?? "",
  };
  const api = createSendController(deps as never);
  return { api, state, store, promptEl };
}

console.log("\nsend: what a finished round keeps");

await check("a finished round keeps the agent's thread, and the next send resumes it", async () => {
  reset();
  const thread = "11111111-2222-4333-8444-555555555555";
  const session = makeSession("s-keep");
  const { api, state, promptEl } = newController({ sessions: [session], providerTag: "p1" });
  startReply = async () => okReport({ runtime_thread_id: ` ${thread} `, cwd: "/tmp/project" });
  promptEl.value = "first";

  await api.sendAsk();

  eq(session.runtimeThreadId, thread, "saved thread");
  eq(session.providerTag, "p1", "provider the thread belongs to");
  eq(session.interrupted, null, "a finished round is not marked interrupted");
  eq(state.displayedCwd, ["/tmp/project"], "shown folder");
  eq(state.busy, false, "busy released");
  eq(state.statuses.at(-1)?.text, "", "status cleared after success");
  eq(roundCalls()[0].args.resumeThreadId, null, "the first round starts fresh");

  promptEl.value = "second";
  await api.sendAsk();
  eq(roundCalls()[1].args.resumeThreadId, thread, "the next round resumes the saved thread");
});

await check("a ZCode session id is not passed to Claude", async () => {
  reset();
  const session = makeSession("s-zcode-on-claude", {
    runtime: "claude-code",
    runtimeThreadId: "sess_1fe74b61-77d7-466f-a6dd-f9246c751058",
  });
  const { api, promptEl } = newController({ sessions: [session], runtime: "claude-code" });
  promptEl.value = "again";

  await api.sendAsk();

  eq(roundCalls()[0].args.resumeThreadId, null, "sess_ is not a Claude session");
  eq(session.runtimeThreadId, null, "the mismatched id is dropped");
});

await check("ZCode keeps its own session id", async () => {
  reset();
  const session = makeSession("s-zcode", {
    runtime: "zcode",
    runtimeThreadId: "sess_1fe74b61-77d7-466f-a6dd-f9246c751058",
  });
  const { api, promptEl } = newController({ sessions: [session], runtime: "zcode" });
  promptEl.value = "again";

  await api.sendAsk();

  eq(
    roundCalls()[0].args.resumeThreadId,
    "sess_1fe74b61-77d7-466f-a6dd-f9246c751058",
    "ZCode resumes sess_",
  );
});

await check("after switching provider, the next send starts a fresh thread", async () => {
  reset();
  const session = makeSession("s-provider", { runtimeThreadId: "th-old", providerTag: "p-old" });
  const { api, promptEl } = newController({ sessions: [session], providerTag: "p-new" });
  promptEl.value = "hello";

  await api.sendAsk();

  eq(roundCalls()[0].args.resumeThreadId, null, "old provider's thread must not be resumed");
});

await check("a thread that cannot be resumed is dropped, so the next send starts clean", async () => {
  reset();
  const session = makeSession("s-gone", {
    runtimeThreadId: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
    providerTag: "p1",
  });
  const { api, promptEl } = newController({ sessions: [session], providerTag: "p1" });
  startReply = async () => okReport({ status: "failed" });
  promptEl.value = "hello";

  await api.sendAsk();

  eq(
    roundCalls()[0].args.resumeThreadId,
    "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
    "the round tried to resume",
  );
  eq(session.runtimeThreadId, null, "dead thread dropped");
  eq(session.interrupted?.status, "failed", "chat marked as not finished");
});

console.log("\nsend: auto-approve and limits");

await check("auto-approve turns on Claude's own switch", async () => {
  reset();
  const { api, promptEl } = newController({ sessions: [makeSession("s-claude")], elevated: true });
  promptEl.value = "do it";

  await api.sendAsk();

  eq(confirmAsked, 0, "the chat switch is already on, so send does not ask again");
  eq(roundCalls()[0].args.dangerouslySkipPermissions, true, "Claude skip-permissions");
  eq(roundCalls()[0].args.fullAuto, false, "Claude does not use full-auto");
});

await check("auto-approve turns on Codex's full-auto instead", async () => {
  reset();
  const { api, promptEl } = newController({
    sessions: [makeSession("s-codex", { runtime: "codex" })],
    runtime: "codex",
    elevated: true,
  });
  promptEl.value = "do it";

  await api.sendAsk();

  eq(roundCalls()[0].args.fullAuto, true, "Codex full-auto");
  eq(roundCalls()[0].args.dangerouslySkipPermissions, false, "Codex has no skip-permissions");
});

await check("auto-approve stays off when the switch is off", async () => {
  reset();
  const { api, promptEl } = newController({ sessions: [makeSession("s-plain")] });
  promptEl.value = "do it";

  await api.sendAsk();

  eq(confirmAsked, 0, "no prompt without auto-approve");
  eq(roundCalls()[0].args.dangerouslySkipPermissions, false, "skip-permissions off");
  eq(roundCalls()[0].args.fullAuto, false, "full-auto off");
});

await check("a chat with auto-approve off does not borrow another chat's switch", async () => {
  reset();
  const quiet = makeSession("s-quiet");
  const loud = makeSession("s-loud", { autoApprove: true });
  const { api, promptEl } = newController({ sessions: [quiet, loud], elevated: false });
  promptEl.value = "do it";

  await api.sendAsk();

  eq(roundCalls()[0].args.dangerouslySkipPermissions, false, "this chat stays off");
});

await check("too many chats working: nothing is sent and the reason is shown", async () => {
  reset();
  const { api, state, promptEl } = newController({
    sessions: [makeSession("s-full")],
    runningCount: MAX_PARALLEL_RUNS,
  });
  promptEl.value = "one more";

  await api.sendAsk();

  eq(roundCalls().length, 0, "no round");
  eq(promptEl.value, "one more", "text kept");
  eq(
    state.statuses.at(-1),
    { text: t("chat.tooManyRunning", { n: String(MAX_PARALLEL_RUNS) }), tone: "warn" },
    "status",
  );
});

console.log("\nsend: a chat that is not the open one");

await check("a send for another chat writes into that chat, not the open one", async () => {
  reset();
  const open = makeSession("s-open");
  const background = makeSession("s-bg", { runtime: "codex" });
  const { api, state } = newController({ sessions: [open, background], activeId: "s-open" });

  await api.sendAsk({
    draft: { id: "d1", sessionId: "s-bg", text: "background task\nsecond line", attachments: [], mentions: [] },
  });

  eq(background.messages.map((m) => [m.role, m.content]), [["user", "background task\nsecond line"]], "message lands in the background chat");
  eq(background.title, "background task", "title seeded from the first line");
  eq(open.messages.length, 0, "open chat untouched");
  eq(state.persisted.length, 0, "nothing persisted into the open chat");
  eq(state.bubbles.length, 0, "no bubble drawn in the open chat");
  eq(roundCalls()[0].args.runtime, "codex", "round uses the background chat's agent");
  eq(roundCalls()[0].args.clientRunId, "s-bg", "round is bound to the background chat");
  eq(state.displayedCwd.length, 0, "open chat's folder line untouched");
});

console.log("\nsend: when the round fails");

await check("another window is answering: the text goes back in the box", async () => {
  reset();
  startReply = async () => {
    throw new Error("prompt session busy in another window");
  };
  const { api, state, promptEl } = newController({ sessions: [makeSession("s-other-window")] });
  promptEl.value = "my question";

  await api.sendAsk();

  eq(promptEl.value, "my question", "text restored");
  eq(state.statuses.at(-1), { text: t("chat.otherWindowRunning"), tone: "warn" }, "status");
  ok(state.bubbles.some((b) => b.kind === "meta" && b.text === t("chat.otherWindowRunning")), "note in the chat");
  eq(cancelCalls().length, 0, "the other window's round is not stopped");
  eq(state.busy, false, "busy released");
});

await check("the engine says this chat is already running: stop it and say so", async () => {
  reset();
  startReply = async () => {
    throw new Error("session already running");
  };
  const { api, state, promptEl } = newController({ sessions: [makeSession("s-stuck")] });
  promptEl.value = "again";

  await api.sendAsk();

  eq(cancelCalls().map((c) => c.args.clientRunId), ["s-stuck"], "stops this chat's run");
  eq(state.statuses.at(-1), { text: t("chat.forceStopped"), tone: "warn" }, "status");
  eq(state.busy, false, "busy released");
});

await check("the page reloaded while this chat was still working: pick it back up, do not stop it", async () => {
  reset();
  liveRuns = [{ client_run_id: "s-reloaded", session_id: "ps-9" }];
  startReply = async () => {
    throw new Error("another ask session is already running");
  };
  const { api, state, promptEl } = newController({ sessions: [makeSession("s-reloaded")] });
  promptEl.value = "next step";

  await api.sendAsk();

  eq(cancelCalls().length, 0, "the running turn is left alone");
  eq(state.statuses.at(-1), { text: t("chat.resumedRunBusy"), tone: "warn" }, "status");
  eq(promptEl.value, "next step", "text back in the box");
  eq(state.busy, true, "the chat still shows it is working");
});

await check("after a reload the page takes over turns that are still running", async () => {
  reset();
  liveRuns = [
    { client_run_id: "s-live", session_id: "ps-3" },
    { client_run_id: "s-gone", session_id: null },
  ];
  const { api, state } = newController({ sessions: [makeSession("s-live")] });

  const n = await api.adoptLiveRuns();

  eq(n, 1, "only a chat this page knows");
  eq(state.busy, true, "shown as working");
  ok(invokes.some((c) => c.cmd === "replay_prompt_asks_command" && c.args.clientRunId === "s-live"), "open questions asked again");
});

await check("the project folder is gone: say so in plain words", async () => {
  reset();
  startReply = async () => {
    throw new Error("session cwd does not exist: /Users/me/old-project");
  };
  const { api, state, promptEl } = newController({ sessions: [makeSession("s-cwd")] });
  promptEl.value = "hello";

  await api.sendAsk();

  eq(state.statuses.at(-1), { text: t("chat.failedCwd"), tone: "error" }, "status");
  ok(state.bubbles.some((b) => b.kind === "meta" && b.text === t("chat.failedCwd")), "note in the chat");
  eq(state.busy, false, "busy released");
});

await check("an unknown failure releases the chat and leaves a note", async () => {
  reset();
  startReply = async () => {
    throw new Error("boom");
  };
  const { api, state, promptEl } = newController({ sessions: [makeSession("s-boom")] });
  promptEl.value = "hello";

  await api.sendAsk();

  const last = state.statuses.at(-1);
  eq(last?.tone, "error", "error tone");
  ok(Boolean(last?.text.startsWith(t("chat.failed"))), `status should start with the plain failure line, got ${last?.text}`);
  ok(state.bubbles.some((b) => b.kind === "meta"), "note in the chat");
  eq(state.busy, false, "busy released");
  eq(state.expired, 1, "waiting approvals are expired");
});

console.log("\ncancel: what Stop does");

await check("Stop asks the engine to stop the open chat", async () => {
  reset();
  const { api, state } = newController({
    sessions: [makeSession("s-a"), makeSession("s-b")],
    activeId: "s-b",
  });
  state.busy = true;

  await api.cancelAsk();

  eq(cancelCalls().map((c) => c.args.clientRunId), ["s-b"], "cancel targets the open chat");
  consumeDrainIntent();
});

await check("Stop the engine confirms waits for the round, then frees the chat if it never ends", async () => {
  reset();
  const { api, state } = newController({ sessions: [makeSession("s-wait")] });
  state.busy = true;

  await api.cancelAsk();

  eq(state.busy, true, "still busy until the round ends");
  eq(timers.map((x) => x.ms), [2500], "one fallback timer");
  timers[0].fn();
  eq(state.busy, false, "fallback frees the chat");
  eq(state.statuses.at(-1), { text: t("chat.forceStopped"), tone: "warn" }, "status");
  consumeDrainIntent();
});

await check("the fallback leaves a newer round alone", async () => {
  reset();
  const { api, state } = newController({ sessions: [makeSession("s-newer")] });
  state.busy = true;

  await api.cancelAsk();
  state.busyGen += 1;
  timers[0].fn();

  eq(state.busy, true, "newer round keeps running");
  consumeDrainIntent();
});

await check("Stop when the engine cannot be reached frees the chat with a plain reason", async () => {
  reset();
  cancelReply = async () => {
    throw new Error("ipc closed");
  };
  const { api, state } = newController({ sessions: [makeSession("s-unreachable")] });
  state.busy = true;

  await api.cancelAsk();

  const last = state.statuses.at(-1);
  eq(state.busy, false, "busy released");
  eq(last?.tone, "error", "error tone");
  ok(Boolean(last?.text.startsWith(t("chat.cancelFailed"))), `status should start with the plain line, got ${last?.text}`);
  eq(state.expired, 1, "waiting approvals are expired");
  consumeDrainIntent();
});

console.log("");
if (failures > 0) {
  console.log(`${failures} of ${checks} checks FAILED`);
  process.exit(1);
}
console.log(`all ${checks} checks passed`);
