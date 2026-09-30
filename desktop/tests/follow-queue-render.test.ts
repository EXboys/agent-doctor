/**
 * Standalone check for the follow-up queue UI (src/chat/follow-queue.ts:
 * renderFollowQueue + the "insert now" handler).
 *
 * Run from desktop/:
 *   node_modules/.bin/tsx tests/follow-queue-render.test.ts
 *
 * Lives outside src/ on purpose: desktop/tsconfig.json includes only "src", so
 * this harness never enters `npm run build` (tsc) nor the vite bundle.
 *
 * No jsdom on purpose — the repo has no test deps, so this file ships a tiny
 * DOM stub covering exactly the API renderFollowQueue touches. If the module
 * starts using more of the DOM, extend FakeElement below instead of adding a
 * dependency.
 */

export {};

// follow-queue imports ../i18n, which reads localStorage/navigator at import time.
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

class FakeElement {
  tagName: string;
  className = "";
  type = "";
  hidden = false;
  children: FakeElement[] = [];
  listeners = new Map<string, Array<() => void>>();
  private ownText = "";

  constructor(tagName: string) {
    this.tagName = tagName.toUpperCase();
  }

  /** Real DOM: setting textContent drops the children. */
  get textContent(): string {
    if (this.children.length === 0) return this.ownText;
    return this.children.map((child) => child.textContent).join("");
  }

  set textContent(value: string) {
    this.ownText = value;
    this.children = [];
  }

  addEventListener(type: string, fn: () => void): void {
    const list = this.listeners.get(type) ?? [];
    list.push(fn);
    this.listeners.set(type, list);
  }

  click(): void {
    for (const fn of this.listeners.get("click") ?? []) fn();
  }

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
}

Object.defineProperty(globalThis, "document", {
  configurable: true,
  writable: true,
  value: {
    createElement: (tagName: string) => new FakeElement(tagName),
  },
});

function findAll(root: FakeElement, className: string): FakeElement[] {
  const found: FakeElement[] = [];
  const walk = (node: FakeElement): void => {
    if (node.className === className) found.push(node);
    for (const child of node.children) walk(child);
  };
  walk(root);
  return found;
}

const { enqueueFollowUp, takeFollowUps, consumeDrainIntent, setFollowQueueInsertHandler, renderFollowQueue } =
  await import("../src/chat/follow-queue.ts");
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

const host = () => new FakeElement("section");
const attachment = (name: string) => ({ id: name, name, path: `/tmp/${name}` }) as never;

/** Queue one note, render, and hand back both for assertions. */
function withItems(sessionId: string, texts: string[], canInsertNow = false) {
  for (const text of texts) {
    enqueueFollowUp(sessionId, { sessionId, text, attachments: [], mentions: [] });
  }
  const el = host();
  renderFollowQueue(el, sessionId, canInsertNow);
  return el;
}

console.log("\nfollow-queue render: host and emptiness");

check("a null host is ignored", () => {
  renderFollowQueue(null, "r-null", true);
});

check("an empty queue hides the host and renders nothing", () => {
  const el = withItems("r-empty", []);
  eq(el.hidden, true, "hidden");
  eq(el.children.length, 0, "child count");
});

check("a queue with items unhides the host", () => {
  const el = withItems("r-head", ["one"], true);
  eq(el.hidden, false, "hidden");
  takeFollowUps("r-head");
});

console.log("\nfollow-queue render: the list");

check("the header carries the queue title", () => {
  const el = withItems("r-title", ["one"]);
  eq(findAll(el, "chat-follow-queue-head").length, 1, "head count");
  eq(findAll(el, "chat-follow-queue-head")[0].textContent, t("chat.queueTitle"), "head text");
  takeFollowUps("r-title");
});

check("each note renders one row, in queue order", () => {
  const el = withItems("r-rows", ["first", "second", "third"]);
  const items = findAll(el, "chat-follow-item");
  eq(items.length, 3, "row count");
  eq(items.map((item) => item.textContent), ["first" + t("chat.queueRemove"), "second" + t("chat.queueRemove"), "third" + t("chat.queueRemove")], "row text");
  takeFollowUps("r-rows");
});

check("an attachment-only note falls back to the attach-only prompt", () => {
  enqueueFollowUp("r-attach", { sessionId: "r-attach", text: "", attachments: [attachment("a.txt")], mentions: [] });
  const el = host();
  renderFollowQueue(el, "r-attach", false);
  ok(findAll(el, "chat-follow-item")[0].textContent.includes(t("chat.attachOnlyPrompt")), "fallback text");
  takeFollowUps("r-attach");
});

check("rendering lists another session's notes only when that session is asked for", () => {
  enqueueFollowUp("r-iso-a", { sessionId: "r-iso-a", text: "note-a", attachments: [], mentions: [] });
  enqueueFollowUp("r-iso-b", { sessionId: "r-iso-b", text: "note-b", attachments: [], mentions: [] });
  const a = withItems("r-iso-a", []);
  eq(findAll(a, "chat-follow-item").length, 1, "a row count");
  ok(findAll(a, "chat-follow-item")[0].textContent.includes("note-a"), "a shows its own note");
  takeFollowUps("r-iso-a");
  takeFollowUps("r-iso-b");
});

check("re-rendering replaces the rows instead of appending duplicates", () => {
  const el = withItems("r-rerender", ["one"]);
  renderFollowQueue(el, "r-rerender", false);
  renderFollowQueue(el, "r-rerender", false);
  eq(findAll(el, "chat-follow-item").length, 1, "row count after three renders");
  takeFollowUps("r-rerender");
});

check("rendering does not drain the queue", () => {
  const el = withItems("r-keep", ["keep"]);
  ok(el.children.length > 0, "should have rendered");
  eq(takeFollowUps("r-keep").length, 1, "queue content after render");
});

console.log("\nfollow-queue render: 'insert now'");

check("no 'insert now' button while the round cannot be interrupted", () => {
  const el = withItems("r-none", ["one"], false);
  eq(findAll(el, "chat-follow-now").length, 0, "button count");
  takeFollowUps("r-none");
});

check("'insert now' appears as the second header child when the round can be interrupted", () => {
  const el = withItems("r-now", ["one"], true);
  const buttons = findAll(el, "chat-follow-now");
  eq(buttons.length, 1, "button count");
  eq(buttons[0].textContent, t("chat.queueNow"), "button text");
  const head = findAll(el, "chat-follow-queue-head")[0];
  eq(head.children.indexOf(buttons[0]), 1, "button sits after the label");
  takeFollowUps("r-now");
});

check("clicking 'insert now' sets the now intent and fires the registered handler exactly once", () => {
  let inserts = 0;
  setFollowQueueInsertHandler(() => {
    inserts += 1;
  });
  consumeDrainIntent();
  const el = withItems("r-click-now", ["one"], true);
  findAll(el, "chat-follow-now")[0].click();
  eq(consumeDrainIntent(), "now", "drain intent");
  eq(inserts, 1, "handler calls");
  eq(consumeDrainIntent(), "when-ready", "intent resets after read");
  takeFollowUps("r-click-now");
});

console.log("\nfollow-queue render: removing a note");

check("'remove' drops just that note and rebuilds the list", () => {
  const el = withItems("r-remove", ["one", "two"]);
  const buttons = findAll(el, "chat-follow-remove");
  eq(buttons.length, 2, "remove button count");
  eq(buttons[0].textContent, t("chat.queueRemove"), "button text");
  buttons[0].click();
  const rows = findAll(el, "chat-follow-item");
  eq(rows.length, 1, "row count after remove");
  ok(rows[0].textContent.includes("two"), "the other note survives");
  eq(el.hidden, false, "host stays visible");
  takeFollowUps("r-remove");
});

check("removing the last note re-renders an empty, hidden host", () => {
  const el = withItems("r-remove-last", ["only"]);
  findAll(el, "chat-follow-remove")[0].click();
  eq(findAll(el, "chat-follow-item").length, 0, "row count");
  eq(el.children.length, 0, "child count");
  eq(el.hidden, true, "hidden after emptying");
  eq(takeFollowUps("r-remove-last").length, 0, "queue is empty");
});

check("a removed note stays removed across a re-render", () => {
  const el = withItems("r-remove-sticky", ["keep", "drop"]);
  findAll(el, "chat-follow-remove")[1].click();
  renderFollowQueue(el, "r-remove-sticky", false);
  const rows = findAll(el, "chat-follow-item");
  eq(rows.length, 1, "row count");
  ok(rows[0].textContent.includes("keep"), "only the kept note renders");
  takeFollowUps("r-remove-sticky");
});

console.log("");
if (failures > 0) {
  console.log(`${failures} of ${checks} checks FAILED`);
  process.exit(1);
}
console.log(`all ${checks} checks passed`);
