/**
 * Switching, opening, and deleting chats (src/chat/sessions.ts): which chat is
 * shown, whether a working chat keeps its reply when you leave it and picks the
 * stream back up when you return, and what the sidebar marks as open.
 *
 * Run from desktop/:
 *   node --import tsx tests/switch-session.test.ts
 *
 * No jsdom: FakeElement covers only what createSessionsController touches. The
 * workspace doc is null, so the sidebar renders as a flat list of rows.
 * sessions.ts pulls in agent-brand.ts, which needs Vite's import.meta.glob;
 * vite-shims.mjs stands in for it.
 */

export {};

await import("./vite-shims.mjs");

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
Object.defineProperty(globalThis, "CSS", {
  configurable: true,
  writable: true,
  value: { escape: (value: string) => value },
});

class FakeElement {
  tagName: string;
  className = "";
  type = "";
  title = "";
  innerHTML = "";
  hidden = false;
  scrollTop = 0;
  scrollHeight = 0;
  dataset: Record<string, string> = {};
  style: Record<string, string> = {};
  children: FakeElement[] = [];
  attrs = new Map<string, string>();
  focused = 0;
  private ownText = "";

  constructor(tagName: string) {
    this.tagName = tagName.toUpperCase();
  }

  get textContent(): string {
    if (this.children.length === 0) return this.ownText;
    return this.children.map((child) => child.textContent).join("");
  }

  set textContent(value: string) {
    this.ownText = value;
    this.children = [];
  }

  get classList() {
    const read = () => new Set(this.className.split(/\s+/).filter(Boolean));
    const write = (set: Set<string>) => {
      this.className = [...set].join(" ");
    };
    return {
      add: (...names: string[]) => {
        const set = read();
        names.forEach((n) => set.add(n));
        write(set);
      },
      remove: (...names: string[]) => {
        const set = read();
        names.forEach((n) => set.delete(n));
        write(set);
      },
      toggle: (name: string, force?: boolean) => {
        const set = read();
        const on = force ?? !set.has(name);
        if (on) set.add(name);
        else set.delete(name);
        write(set);
        return on;
      },
      contains: (name: string) => read().has(name),
    };
  }

  setAttribute(name: string, value: string): void {
    this.attrs.set(name, value);
  }

  getAttribute(name: string): string | null {
    return this.attrs.get(name) ?? null;
  }

  addEventListener(): void {}

  appendChild(child: FakeElement): FakeElement {
    this.ownText = "";
    this.children.push(child);
    return child;
  }

  append(...items: FakeElement[]): void {
    for (const item of items) this.appendChild(item);
  }

  replaceChildren(...items: FakeElement[]): void {
    this.ownText = "";
    this.children = [];
    for (const item of items) this.appendChild(item);
  }

  remove(): void {}

  querySelector(): null {
    return null;
  }

  querySelectorAll(): FakeElement[] {
    return [];
  }

  closest(): null {
    return null;
  }

  focus(): void {
    this.focused += 1;
  }

  scrollIntoView(): void {}

  getBoundingClientRect() {
    return { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 };
  }
}

Object.defineProperty(globalThis, "document", {
  configurable: true,
  writable: true,
  value: {
    createElement: (tagName: string) => new FakeElement(tagName),
    querySelector: () => null,
    querySelectorAll: () => [],
    addEventListener: () => {},
    body: new FakeElement("body"),
  },
});

let confirmReply = true;
let confirmAsked = 0;
Object.defineProperty(globalThis, "window", {
  configurable: true,
  writable: true,
  value: {
    addEventListener: () => {},
    setTimeout: () => 0,
    clearTimeout: () => {},
    innerWidth: 1200,
    innerHeight: 800,
    confirm: () => {
      confirmAsked += 1;
      return confirmReply;
    },
    __TAURI_INTERNALS__: {
      invoke: async (cmd: string) => (cmd === "list_installed_ask_agents_command" ? [] : null),
    },
  },
});

const { createSessionsController } = await import("../src/chat/sessions.ts");
const { beginLiveRun, endLiveRun } = await import("../src/chat/live-runs.ts");
const { runtimeDisplayName } = await import("../src/chat/runtime.ts");
const { MAX_SESSIONS } = await import("../src/chat/types.ts");
const { t } = await import("../src/i18n.ts");

let failures = 0;
let checks = 0;

function check(name: string, fn: () => void): void {
  checks += 1;
  try {
    fn();
    console.log(`  ok   ${name}`);
  } catch (error) {
    failures += 1;
    console.log(`  FAIL ${name}\n         ${(error as Error).message}`);
  }
}

function eq<T>(actual: T, expected: T, what: string): void {
  const a = JSON.stringify(actual);
  const b = JSON.stringify(expected);
  if (a !== b) throw new Error(`${what}: got ${a}, expected ${b}`);
}

function ok(condition: boolean, what: string): void {
  if (!condition) throw new Error(what);
}

type FakeSession = {
  id: string;
  title: string;
  runtime: string;
  createdAt: number;
  updatedAt: number;
  messages: Array<{ id: string; role: string; content: string; at: number }>;
  runtimeThreadId: string | null;
  workspaceName: string | null;
};

function makeSession(id: string, runtime = "claude-code", title = `chat ${id}`): FakeSession {
  return {
    id,
    title,
    runtime,
    createdAt: 1,
    updatedAt: 1,
    messages: [{ id: `${id}-m`, role: "user", content: "hi", at: 1 }],
    runtimeThreadId: null,
    workspaceName: null,
  };
}

function newController(opts: {
  sessions: FakeSession[];
  activeId?: string;
  assistantMessageId?: string | null;
  assistantRaw?: string;
  pendingPermissions?: unknown[];
  /** What restoreRunningView loads back from the working chat's live run. */
  liveReply?: { id: string; raw: string };
}) {
  let store = { activeId: opts.activeId ?? opts.sessions[0].id, sessions: opts.sessions };
  const sessionListEl = new FakeElement("div");
  const titleEl = new FakeElement("h1");
  const promptEl = new FakeElement("textarea");
  const logEl = new FakeElement("div");
  const unseen = new Set<string>();
  const calls: string[] = [];
  const state = {
    saves: 0,
    statuses: [] as Array<{ text: string; tone?: string }>,
    runtimes: [] as string[],
    assistantMessageId: opts.assistantMessageId ?? null,
    assistantRaw: opts.assistantRaw ?? "",
    assistantBubble: null as unknown,
    updated: [] as Array<{ id: string; content: string; persist?: boolean }>,
    bubbles: [] as Array<{ kind: string; text: string }>,
    activity: [] as string[],
  };
  const note = (name: string) => () => {
    calls.push(name);
  };
  const deps = {
    sessionListEl,
    titleEl,
    promptEl,
    logEl,
    getStore: () => store,
    setStore: (next: typeof store) => {
      store = next;
      calls.push("setStore");
    },
    getBusy: () => false,
    getRunningChatSessionId: () => null,
    getPendingPermissionBatch: () => opts.pendingPermissions ?? [],
    getUnseenCompletedSessionIds: () => unseen,
    getCurrentRuntime: () => "claude-code",
    selectedRuntime: () => "claude-code",
    sessionTitle: (session: FakeSession) => session.title || "New chat",
    activeSession: () => store.sessions.find((s) => s.id === store.activeId) ?? store.sessions[0],
    saveStore: () => {
      state.saves += 1;
    },
    setCurrentRuntime: (runtime: string) => {
      state.runtimes.push(runtime);
    },
    setStatus: (text: string, tone?: string) => {
      state.statuses.push({ text, tone });
    },
    syncComposerUi: note("syncComposerUi"),
    updateContextMeter: note("updateContextMeter"),
    closeContextPopover: note("closeContextPopover"),
    hideDecisionDock: note("hideDecisionDock"),
    flushPendingTextSync: note("flushPendingTextSync"),
    scheduleStorePersist: note("scheduleStorePersist"),
    updateAssistantMessage: (id: string, content: string, o?: { persist?: boolean }) => {
      state.updated.push({ id, content, persist: o?.persist });
    },
    setAssistantMarkdown: note("setAssistantMarkdown"),
    syncAssistantCopyButton: note("syncAssistantCopyButton"),
    appendBubble: (kind: string, text: string) => {
      state.bubbles.push({ kind, text });
      return new FakeElement("div");
    },
    pushActivity: (_phase: string, message: string) => {
      state.activity.push(message);
    },
    schedulePaintLivePermissionBatch: note("schedulePaintLivePermissionBatch"),
    renderActiveMessages: note("renderActiveMessages"),
    renderPendingAttachments: note("renderPendingAttachments"),
    isViewingRunningSession: () => beginRunIds.has(store.activeId),
    getAssistantBubble: () => state.assistantBubble,
    setAssistantBubble: (el: unknown) => {
      state.assistantBubble = el;
    },
    getAssistantMessageId: () => state.assistantMessageId,
    setAssistantMessageId: (id: string | null) => {
      state.assistantMessageId = id;
    },
    getAssistantRaw: () => state.assistantRaw,
    setAssistantRaw: (raw: string) => {
      state.assistantRaw = raw;
    },
    getPendingText: () => "",
    setPendingText: () => {},
    getTurnHadAssistantText: () => false,
    setTurnHadAssistantText: () => {},
    getActivityEl: () => null,
    setActivityEl: () => {},
    getLifecycleActivityEl: () => null,
    setLifecycleActivityEl: () => {},
    getToolGroupEl: () => null,
    setToolGroupEl: () => {},
    setPendingAttachments: () => {},
    touchSession: () => {},
    isComposerLocked: () => false,
    getWorkspaceDoc: () => null,
    syncSessionWorkspaceUi: note("syncSessionWorkspaceUi"),
    captureRunningView: note("captureRunningView"),
    restoreRunningView: () => {
      calls.push("restoreRunningView");
      if (opts.liveReply) {
        state.assistantMessageId = opts.liveReply.id;
        state.assistantRaw = opts.liveReply.raw;
      }
    },
    addProject: () => {},
    removeProject: () => {},
    syncAskChrome: () => {},
  };
  const api = createSessionsController(deps as never);
  return { api, state, calls, unseen, sessionListEl, titleEl, promptEl, getStore: () => store };
}

/** Chats this test marked as working; mirrors live-runs for isViewingRunningSession. */
const beginRunIds = new Set<string>();
function startRun(id: string): void {
  beginRunIds.add(id);
  beginLiveRun(id);
}
function stopRuns(): void {
  for (const id of beginRunIds) endLiveRun(id);
  beginRunIds.clear();
}

function rows(listEl: FakeElement): Array<{ id: string; className: string }> {
  return listEl.children
    .filter((child) => child.dataset.sessionId)
    .map((child) => ({ id: child.dataset.sessionId, className: child.className }));
}

function openRowId(listEl: FakeElement): string | undefined {
  return rows(listEl).find((row) => row.className.split(" ").includes("is-active"))?.id;
}

console.log("\nswitch: moving between chats");

check("switching to an idle chat shows it and clears the last reply", () => {
  const { api, state, calls, getStore, sessionListEl, titleEl, promptEl } = newController({
    sessions: [makeSession("a"), makeSession("b", "codex")],
    assistantMessageId: "old-m",
    assistantRaw: "old reply",
  });

  api.switchSession("b");

  eq(getStore().activeId, "b", "open chat");
  ok(state.saves >= 1, "choice is saved");
  eq(state.runtimes.at(-1), "codex", "agent follows the chat");
  eq(state.assistantMessageId, null, "old reply id cleared");
  eq(state.assistantRaw, "", "old reply text cleared");
  ok(calls.includes("renderActiveMessages"), "messages redrawn");
  eq(state.statuses.at(-1)?.text, "", "status cleared when nothing else runs");
  eq(titleEl.textContent, "chat b", "title");
  eq(openRowId(sessionListEl), "b", "sidebar marks the new chat as open");
  eq(promptEl.focused, 1, "composer focused");
});

check("leaving a working chat keeps its half-written reply", () => {
  startRun("a");
  const { api, state, calls, sessionListEl } = newController({
    sessions: [makeSession("a"), makeSession("b")],
    assistantMessageId: "m-live",
    assistantRaw: "half a reply",
  });

  api.switchSession("b");

  ok(calls.includes("captureRunningView"), "running view captured");
  ok(calls.includes("flushPendingTextSync"), "pending text flushed");
  eq(state.updated, [{ id: "m-live", content: "half a reply", persist: false }], "reply written back");
  ok(calls.includes("scheduleStorePersist"), "reply saved later");
  eq(state.assistantMessageId, "m-live", "reply id kept so the stream can resume");
  eq(state.statuses.at(-1), { text: t("chat.otherSessionRunningHint"), tone: "muted" }, "status");
  const a = rows(sessionListEl).find((row) => row.id === "a");
  ok(Boolean(a?.className.includes("is-running")), "sidebar still shows the left chat as working");
  stopRuns();
});

check("coming back to a working chat picks the stream back up", () => {
  startRun("a");
  const { api, state, calls, getStore } = newController({
    sessions: [makeSession("a", "codex"), makeSession("b")],
    activeId: "b",
    liveReply: { id: "m-live", raw: "half a reply" },
  });

  api.switchSession("a");

  eq(getStore().activeId, "a", "open chat");
  ok(calls.includes("restoreRunningView"), "running view restored");
  eq(state.bubbles, [{ kind: "assistant", text: "half a reply" }], "reply bubble drawn again");
  ok(state.assistantBubble !== null, "stream writes into the new bubble");
  eq(
    state.statuses.at(-1),
    { text: t("chat.running", { runtime: runtimeDisplayName("codex") }), tone: "muted" },
    "status",
  );
  ok(state.activity.includes(t("chat.typing")), "typing activity shown");
  stopRuns();
});

check("coming back to a chat waiting on a choice asks for the choice", () => {
  startRun("a");
  const { api, state, calls } = newController({
    sessions: [makeSession("a"), makeSession("b")],
    activeId: "b",
    pendingPermissions: [{ requestId: "r1" }],
  });

  api.switchSession("a");

  eq(state.statuses.at(-1), { text: t("chat.needYourChoice"), tone: "warn" }, "status");
  ok(calls.includes("schedulePaintLivePermissionBatch"), "choice cards drawn");
  stopRuns();
});

check("clicking the chat that is already open changes nothing", () => {
  const { api, state, calls, getStore } = newController({
    sessions: [makeSession("a"), makeSession("b")],
  });

  api.switchSession("a");

  eq(getStore().activeId, "a", "open chat");
  eq(state.saves, 0, "nothing saved");
  ok(!calls.includes("renderActiveMessages"), "messages not redrawn");
});

check("an unknown chat id is ignored", () => {
  const { api, state, getStore } = newController({ sessions: [makeSession("a")] });

  api.switchSession("gone");

  eq(getStore().activeId, "a", "open chat");
  eq(state.saves, 0, "nothing saved");
});

check("opening a chat that finished elsewhere clears its done mark", () => {
  const { api, unseen } = newController({ sessions: [makeSession("a"), makeSession("b")] });
  unseen.add("b");

  api.switchSession("b");

  ok(!unseen.has("b"), "done mark cleared");
});

check("opening a chat from the island moves it to the top and opens it", () => {
  const { api, getStore, sessionListEl } = newController({
    sessions: [makeSession("a"), makeSession("b"), makeSession("c")],
  });

  api.openSessionFromIsland("c");

  eq(getStore().sessions.map((s) => s.id), ["c", "a", "b"], "order");
  eq(getStore().activeId, "c", "open chat");
  eq(openRowId(sessionListEl), "c", "sidebar marks it as open");
});

console.log("\nswitch: new and deleted chats");

check("a new chat goes on top and is the one shown", () => {
  const { api, state, getStore, sessionListEl } = newController({ sessions: [makeSession("a")] });

  api.startNewSession();

  const first = getStore().sessions[0];
  eq(getStore().sessions.length, 2, "count");
  eq(getStore().activeId, first.id, "new chat is open");
  eq(first.messages.length, 0, "starts empty");
  eq(openRowId(sessionListEl), first.id, "sidebar marks it as open");
  eq(state.statuses.at(-1), { text: t("chat.newSessionReady"), tone: "ok" }, "status");
});

check("a new chat while another works says the other one keeps going", () => {
  startRun("a");
  const { api, state, calls } = newController({ sessions: [makeSession("a")] });

  api.startNewSession();

  ok(calls.includes("captureRunningView"), "working chat's view captured");
  eq(state.statuses.at(-1), { text: t("chat.otherSessionRunningHint"), tone: "muted" }, "status");
  stopRuns();
});

check("the chat list stays capped", () => {
  const many = Array.from({ length: MAX_SESSIONS }, (_, i) => makeSession(`s${i}`));
  const { api, getStore } = newController({ sessions: many });

  api.startNewSession();

  eq(getStore().sessions.length, MAX_SESSIONS, "count");
  ok(!getStore().sessions.some((s) => s.id === `s${MAX_SESSIONS - 1}`), "oldest dropped");
});

check("a working chat cannot be deleted, and says why", () => {
  startRun("a");
  confirmAsked = 0;
  const { api, state, getStore } = newController({ sessions: [makeSession("a"), makeSession("b")] });

  api.deleteSession("a");

  eq(getStore().sessions.map((s) => s.id), ["a", "b"], "nothing deleted");
  eq(confirmAsked, 0, "no confirm prompt");
  eq(state.statuses.at(-1), { text: t("chat.cannotDeleteRunning"), tone: "warn" }, "status");
  stopRuns();
});

check("saying no to delete keeps the chat", () => {
  confirmReply = false;
  const { api, getStore } = newController({ sessions: [makeSession("a"), makeSession("b")] });

  api.deleteSession("a");
  confirmReply = true;

  eq(getStore().sessions.map((s) => s.id), ["a", "b"], "nothing deleted");
});

check("deleting the open chat opens the next one", () => {
  const { api, getStore, titleEl, sessionListEl } = newController({
    sessions: [makeSession("a"), makeSession("b")],
  });

  api.deleteSession("a");

  eq(getStore().sessions.map((s) => s.id), ["b"], "remaining");
  eq(getStore().activeId, "b", "open chat");
  eq(titleEl.textContent, "chat b", "title");
  eq(openRowId(sessionListEl), "b", "sidebar marks it as open");
});

check("deleting the last chat leaves one fresh empty chat", () => {
  const { api, calls, getStore } = newController({ sessions: [makeSession("a")] });

  api.deleteSession("a");

  ok(calls.includes("setStore"), "store replaced");
  eq(getStore().sessions.length, 1, "one chat left");
  ok(getStore().sessions[0].id !== "a", "it is a new chat");
  eq(getStore().sessions[0].messages.length, 0, "and it is empty");
  eq(getStore().activeId, getStore().sessions[0].id, "and it is open");
});

console.log("");
if (failures > 0) {
  console.log(`${failures} of ${checks} checks FAILED`);
  process.exit(1);
}
console.log(`all ${checks} checks passed`);
