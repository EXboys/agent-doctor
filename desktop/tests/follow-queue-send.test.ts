/**
 * Standalone check for the follow-up queue wiring in the send controller
 * (src/chat/send.ts): queue while a round runs, drain when the round ends,
 * Stop must not send the queue.
 *
 * Run from desktop/:
 *   node_modules/.bin/tsx tests/follow-queue-send.test.ts
 *
 * Lives outside src/ on purpose: desktop/tsconfig.json includes only "src", so
 * this harness never enters `npm run build` (tsc) nor the vite bundle.
 *
 * There is no test framework and no jsdom in this repo, so this file stubs the
 * few globals the modules touch at import time and fakes the Tauri bridge:
 * send.ts calls invoke() through ../ipc, and @tauri-apps/api/core forwards to
 * window.__TAURI_INTERNALS__.invoke. Faking that layer lets a whole round run
 * for real (start_prompt_session -> finally -> drain) with no app and no engine.
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

// Fake Tauri bridge. `start_prompt_session_command` resolves like a finished
// round (no streaming events), so send.ts runs its settle-and-drain tail.
type InvokeCall = { cmd: string; args: Record<string, unknown> };
const invokes: InvokeCall[] = [];
let onRoundStart: (() => Promise<void> | void) | null = null;
let onReadImages: (() => Promise<void> | void) | null = null;
let cancelReply = false;
const timers: number[] = [];

Object.defineProperty(globalThis, "window", {
  configurable: true,
  writable: true,
  value: {
    // Never actually schedule: send.ts has 2.5s fallback timers that would
    // keep the node process alive.
    setTimeout: (fn: () => void, ms: number) => {
      timers.push(ms);
      void fn;
      return timers.length;
    },
    clearTimeout: () => {},
    confirm: () => true,
    __TAURI_INTERNALS__: {
      invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
        invokes.push({ cmd, args });
        if (cmd === "cancel_prompt_session_command") return cancelReply;
        if (cmd === "read_image_texts_command") {
          if (onReadImages) await onReadImages();
          return { readings: [], available: true };
        }
        if (cmd === "start_prompt_session_command") {
          const first = invokes.filter((call) => call.cmd === cmd).length === 1;
          if (first && onRoundStart) await onRoundStart();
          return { cwd: "/tmp/round", status: "succeeded", duration_ms: 7, runtime_thread_id: null };
        }
        return null;
      },
    },
  },
});

const { createSendController } = await import("../src/chat/send.ts");
const { t } = await import("../src/i18n.ts");
const { followUpsFor, takeFollowUps, consumeDrainIntent, insertFollowUpsNow, holdFollowUps } =
  await import("../src/chat/follow-queue.ts");

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

function newController(sessionId: string, opts?: { readImages?: boolean }) {
  const session = { id: sessionId, runtimeThreadId: null as string | null };
  const store = { activeId: sessionId } as never;
  const promptEl = { value: "" } as HTMLTextAreaElement;
  const state = {
    busy: false,
    busyGen: 1,
    runningChatSessionId: sessionId as string | null,
    statuses: [] as Array<{ text: string; tone?: string }>,
    attachments: [] as never[],
    refreshed: 0,
    displayedCwd: [] as string[],
    quickRepliesAllowed: 0,
  };
  const deps = {
    promptEl,
    elevatedEl: { checked: false } as HTMLInputElement,
    askResources: { mountedSkills: [], enabledMcps: [], selectedMentions: [], clearMentions: () => {} } as never,
    mentionMenu: { hideMentionMenu: () => {} } as never,
    getStore: () => store,
    getBusy: () => state.busy,
    getBusyGen: () => state.busyGen,
    getRunningChatSessionId: () => state.runningChatSessionId,
    getPendingAttachments: () => state.attachments,
    setPendingAttachments: (items: never[]) => {
      state.attachments = items;
    },
    resolveSendWorkspace: () => ({
      cwd: "/tmp/ws",
      workspaceName: "default",
    }),
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
    selectedRuntime: () => "claude-code",
    activeSession: () => session as never,
    ensureListener: async () => {},
    clearQuickReplies: () => {},
    allowQuickRepliesAgain: () => {
      state.quickRepliesAllowed += 1;
    },
    setBusy: (next: boolean) => {
      state.busy = next;
    },
    pushActivity: () => {},
    persistMessage: () => ({ id: "m-1" }) as never,
    appendBubble: () => ({}) as never,
    autoResizePrompt: () => {},
    renderPendingAttachments: () => {},
    buildPromptWithHistory: (text: string) => text,
    setDisplayedCwd: (cwd: string) => {
      state.displayedCwd.push(cwd);
    },
    sessionById: () => session as never,
    runTargetSession: () => session as never,
    touchSession: () => {},
    saveStore: () => {},
    applyVerifyMcpFooter: () => {},
    applyVerifyEvidenceFromAssistant: () => {},
    reportVerifyMcpIfNeeded: () => {},
    expireLivePermissionCards: () => {},
    settleRunRouting: () => {},
    renderSessionList: () => {},
    readImageTextEnabled: () => opts?.readImages === true,
    refreshComposer: () => {
      state.refreshed += 1;
    },
  };
  const api = createSendController(deps as never);
  return { api, state, promptEl };
}

console.log("\nfollow-queue wiring: typing while a round is running");

await check("Enter during a round queues the note instead of sending it", async () => {
  invokes.length = 0;
  const { api, state, promptEl } = newController("w-busy");
  state.busy = true;
  promptEl.value = "next note";
  state.attachments = [{ id: "a1", name: "a.txt", path: "/tmp/a.txt" }] as never[];

  await api.sendAsk();

  eq(roundCalls().length, 0, "no round should start");
  const queued = followUpsFor("w-busy");
  eq(queued.length, 1, "queued count");
  eq(queued[0].text, "next note", "queued text");
  eq(queued[0].attachments.length, 1, "queued attachments");
  eq(promptEl.value, "", "composer should be cleared");
  eq(state.attachments.length, 0, "pending attachments should be cleared");
  eq(state.statuses.at(-1), { text: t("chat.queuedStatus"), tone: "muted" }, "status");
  eq(state.refreshed, 1, "composer refresh");
  takeFollowUps("w-busy");
});

await check("Enter during a round with an empty composer warns instead of queueing", async () => {
  invokes.length = 0;
  const { api, state, promptEl } = newController("w-empty");
  state.busy = true;
  promptEl.value = "   ";

  await api.sendAsk();

  eq(followUpsFor("w-empty").length, 0, "queued count");
  eq(state.statuses.at(-1)?.text, t("chat.emptyPrompt"), "status");
});

await check("Enter while another session is running does not queue into this one", async () => {
  invokes.length = 0;
  const { api, state, promptEl } = newController("w-other");
  state.busy = true;
  state.runningChatSessionId = "w-somewhere-else";
  promptEl.value = "note for the busy session";

  await api.sendAsk();

  eq(followUpsFor("w-other").length, 0, "queued count");
  eq(state.statuses.at(-1)?.text, t("chat.otherSessionRunning"), "status");
  eq(promptEl.value, "note for the busy session", "composer should keep the text");
});

await check("a draft handed to a busy controller is queued on its own session", async () => {
  invokes.length = 0;
  const { api, state } = newController("w-draft-active");
  state.busy = true;

  await api.sendAsk({
    draft: { id: "d1", sessionId: "w-draft-target", text: "voice note", attachments: [], mentions: [] },
  });

  eq(followUpsFor("w-draft-active").length, 0, "active session queue");
  eq(followUpsFor("w-draft-target").map((item) => item.text), ["voice note"], "draft session queue");
  eq(roundCalls().length, 0, "no round should start");
  takeFollowUps("w-draft-target");
});

console.log("\nfollow-queue wiring: the round ends and the queue runs");

await check("a note queued mid-round is sent after the round finishes", async () => {
  invokes.length = 0;
  const { api, state, promptEl } = newController("w-drain");
  promptEl.value = "first task";
  onRoundStart = async () => {
    // What the user's Enter does while the round is streaming.
    promptEl.value = "second task";
    await api.sendAsk();
  };

  await api.sendAsk();
  onRoundStart = null;

  const rounds = roundCalls();
  eq(rounds.length, 2, "round count");
  const first = String(rounds[0].args.prompt);
  const second = String(rounds[1].args.prompt);
  ok(first.includes("first task"), `first prompt should carry the first task, got ${first}`);
  ok(second.includes("second task"), `second prompt should carry the queued note, got ${second}`);
  ok(!second.includes("first task"), "the finished task must not be repeated");
  eq(followUpsFor("w-drain").length, 0, "queue should be empty after the drain");
  eq(state.statuses.filter((entry) => entry.text === t("chat.queuedStatus")).length, 1, "one queued status");
});

await check("several notes queued mid-round collapse into one steer follow-up", async () => {
  invokes.length = 0;
  const { api, promptEl } = newController("w-merge");
  promptEl.value = "first task";
  onRoundStart = async () => {
    for (const text of ["earlier note", "latest note"]) {
      promptEl.value = text;
      await api.sendAsk();
    }
  };

  await api.sendAsk();
  onRoundStart = null;

  const rounds = roundCalls();
  eq(rounds.length, 2, "round count");
  const second = String(rounds[1].args.prompt);
  ok(second.includes("earlier note"), `earlier note should be carried, got ${second}`);
  ok(second.includes("latest note"), "latest note should be carried");
  ok(
    second.indexOf("earlier note") < second.indexOf("latest note"),
    "the latest note must come last",
  );
  eq(followUpsFor("w-merge").length, 0, "queue should be empty after the drain");
});

await check("Stop during a round leaves the queue unsent", async () => {
  invokes.length = 0;
  cancelReply = false;
  const { api, state, promptEl } = newController("w-hold");
  promptEl.value = "first task";
  onRoundStart = async () => {
    // Pressing Stop: holdFollowUps runs before the first await inside cancelAsk.
    void api.cancelAsk();
    promptEl.value = "note typed before Stop";
    await api.sendAsk();
  };

  await api.sendAsk();
  onRoundStart = null;

  eq(roundCalls().length, 1, "round count");
  eq(followUpsFor("w-hold").map((item) => item.text), ["note typed before Stop"], "queue survives Stop");
  eq(consumeDrainIntent(), "when-ready", "the hold must be consumed by the round");
  state.statuses.length = 0;
  takeFollowUps("w-hold");
});

await check("Stop while pictures are being read does not start the round", async () => {
  invokes.length = 0;
  const { api, state, promptEl } = newController("w-ocr-stop", { readImages: true });
  promptEl.value = "what is this";
  state.attachments = [{ id: "img", name: "shot.png", path: "/tmp/shot.png", kind: "image" }] as never[];
  onReadImages = () => {
    state.busy = false;
  };

  await api.sendAsk();
  onReadImages = null;

  eq(roundCalls().length, 0, "round should not start after Stop");
  eq(state.busy, false, "busy stays released");
  eq(
    invokes.some((call) => call.cmd === "read_image_texts_command"),
    true,
    "picture text should be requested",
  );
});

await check("Stop asks the engine to cancel and holds the queue", async () => {
  invokes.length = 0;
  consumeDrainIntent();
  cancelReply = true;
  const { api, state } = newController("w-cancel");
  state.busy = true;

  await api.cancelAsk();
  cancelReply = false;

  eq(cancelCalls().length, 1, "cancel command count");
  eq(state.statuses.at(-1)?.text, t("chat.cancelling"), "status");
  eq(consumeDrainIntent(), "hold", "Stop must hold the queue");
});

await check("a Stop the engine does not confirm still releases the UI", async () => {
  invokes.length = 0;
  consumeDrainIntent();
  cancelReply = false;
  const { api, state } = newController("w-cancel-unconfirmed");
  state.busy = true;

  await api.cancelAsk();

  eq(state.statuses.map((entry) => entry.text), [t("chat.cancelling"), t("chat.forceStopped")], "statuses");
  eq(state.busy, false, "busy should be released");
  eq(consumeDrainIntent(), "hold", "Stop must hold the queue");
});

await check("'insert now' wins over an earlier Stop", async () => {
  invokes.length = 0;
  holdFollowUps();
  insertFollowUpsNow();
  const { api, promptEl } = newController("w-now");
  promptEl.value = "first task";
  onRoundStart = () => {
    promptEl.value = "inserted note";
    return api.sendAsk();
  };

  await api.sendAsk();
  onRoundStart = null;

  eq(roundCalls().length, 2, "round count");
  ok(String(roundCalls()[1].args.prompt).includes("inserted note"), "the inserted note should be sent");
  eq(followUpsFor("w-now").length, 0, "queue should be empty after the drain");
});

console.log("\nfollow-queue wiring: quick replies come back on a real send");

await check("a real send lets quick replies show again", async () => {
  invokes.length = 0;
  const { api, state, promptEl } = newController("w-quick");
  promptEl.value = "a task";

  await api.sendAsk();

  eq(roundCalls().length, 1, "round count");
  eq(state.quickRepliesAllowed, 1, "allowQuickRepliesAgain calls");
});

await check("queueing a note is not a send and leaves quick replies alone", async () => {
  invokes.length = 0;
  const { api, state, promptEl } = newController("w-quick-busy");
  state.busy = true;
  promptEl.value = "next note";

  await api.sendAsk();

  eq(state.quickRepliesAllowed, 0, "allowQuickRepliesAgain calls");
  takeFollowUps("w-quick-busy");
});

console.log("");
if (failures > 0) {
  console.log(`${failures} of ${checks} checks FAILED`);
  process.exit(1);
}
console.log(`all ${checks} checks passed`);
