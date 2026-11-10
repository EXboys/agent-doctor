/**
 * Standalone check for the reply-rating controls (src/chat/feedback.ts:
 * syncFeedbackControls visibility + rated state, and the actions row's
 * hidden recomputation that must account for feedback alone).
 *
 * Run from desktop/:
 *   node_modules/.bin/tsx tests/feedback-controls.test.ts
 *
 * Lives outside src/ on purpose (never enters tsc/vite). No jsdom on purpose —
 * this file ships a tiny DOM stub covering exactly the API syncFeedbackControls
 * touches; extend FakeElement instead of adding a dependency.
 */

export {};

// feedback.ts imports ../i18n, which reads localStorage/navigator at import time.
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
  hidden = false;
  dataset: Record<string, string> = {};
  children: FakeElement[] = [];
  private classes = new Set<string>();

  constructor(tagName: string, className = "") {
    this.tagName = tagName.toUpperCase();
    this.className = className;
    this.classes = new Set(className.split(/\s+/).filter(Boolean));
  }

  get classList() {
    const classes = this.classes;
    const refresh = () => {
      this.className = Array.from(classes).join(" ");
    };
    return {
      add: (name: string) => {
        classes.add(name);
        refresh();
      },
      remove: (name: string) => {
        classes.delete(name);
        refresh();
      },
      toggle: (name: string, force?: boolean) => {
        const on = force ?? !classes.has(name);
        if (on) classes.add(name);
        else classes.delete(name);
        refresh();
        return on;
      },
      contains: (name: string) => classes.has(name),
    };
  }

  append(...items: FakeElement[]): void {
    this.children.push(...items);
  }

  appendChild(child: FakeElement): FakeElement {
    this.children.push(child);
    return child;
  }

  private hasClass(token: string): boolean {
    return this.classes.has(token);
  }

  /** Covers the selector shapes syncFeedbackControls uses: `:scope > A > B`
      chains, bare classes, and `tag.class` compounds. */
  querySelector<T = FakeElement>(selector: string): T | null {
    const parts = selector
      .split(">")
      .map((part) => part.trim().replace(/^:scope\s*/, "").replace(/^\./, ""))
      .filter(Boolean);
    const matchesToken = (node: FakeElement, token: string): boolean => {
      if (!token.includes(".")) {
        return node.hasClass(token) || node.tagName.toLowerCase() === token;
      }
      const [tag, ...classes] = token.split(".");
      return (
        (!tag || node.tagName.toLowerCase() === tag) &&
        classes.every((name) => node.hasClass(name))
      );
    };
    const walk = (nodes: FakeElement[], depth: number): FakeElement | null => {
      for (const node of nodes) {
        if (!matchesToken(node, parts[depth]!)) continue;
        if (depth === parts.length - 1) return node;
        const deeper = walk(node.children, depth + 1);
        if (deeper) return deeper;
      }
      return null;
    };
    return (walk(this.children, 0) as unknown as T) ?? null;
  }

  querySelectorAll<T = FakeElement>(selector: string): T[] {
    const token = selector.replace(/^\./, "");
    const found: FakeElement[] = [];
    for (const child of this.children) {
      if (child.hasClass(token)) found.push(child);
      found.push(...child.querySelectorAll<FakeElement>(selector));
    }
    return found as unknown as T[];
  }
}

const { syncFeedbackControls } = await import("../src/chat/feedback");

function buildRow() {
  const wrap = new FakeElement("div", "chat-msg chat-msg-assistant");
  const bubble = new FakeElement("div", "chat-bubble assistant chat-md");
  const actions = new FakeElement("div", "chat-msg-actions");
  actions.hidden = true;
  const copy = new FakeElement("button", "chat-msg-copy");
  const time = new FakeElement("time", "chat-msg-time");
  const box = new FakeElement("div", "chat-feedback");
  box.hidden = true;
  const up = new FakeElement("button", "chat-feedback-btn");
  up.dataset.feedback = "up";
  const down = new FakeElement("button", "chat-feedback-btn");
  down.dataset.feedback = "down";
  box.append(up, down);
  actions.append(copy, time, box);
  wrap.append(bubble, actions);
  return { wrap, actions, copy, time, box, up, down };
}

let failed = 0;
function check(label: string, condition: boolean): void {
  if (!condition) {
    failed += 1;
    console.error(`fail - ${label}`);
  } else {
    console.log(`ok   ${label}`);
  }
}

{
  const { wrap, actions, box, up, down } = buildRow();
  syncFeedbackControls(wrap as unknown as HTMLElement, { owner: true, streaming: false });
  check("owner + settled reply shows the pair", box.hidden === false);
  check("feedback alone reveals the actions row", actions.hidden === false);
  check("neither side is highlighted", !up.classList.contains("is-rated") && !down.classList.contains("is-rated"));

  syncFeedbackControls(wrap as unknown as HTMLElement, {
    owner: true,
    streaming: false,
    rated: "down",
  });
  check("rated side is highlighted", down.classList.contains("is-rated"));
  check("other side stays plain", !up.classList.contains("is-rated"));
}

{
  const { wrap, box, up, down } = buildRow();
  syncFeedbackControls(wrap as unknown as HTMLElement, { owner: false, streaming: false });
  check("non-owner fragment hides the pair", box.hidden === true);

  syncFeedbackControls(wrap as unknown as HTMLElement, { owner: true, streaming: true });
  check("streaming owner hides the pair too", box.hidden === true);
  check("streaming state highlights nothing", !up.classList.contains("is-rated") && !down.classList.contains("is-rated"));
}

{
  const { wrap, actions, copy, time, box } = buildRow();
  copy.hidden = true;
  time.hidden = true;
  syncFeedbackControls(wrap as unknown as HTMLElement, { owner: true, streaming: false });
  check("feedback alone reveals the actions row", actions.hidden === false);
  void box;
}

{
  const { wrap, actions, copy, time } = buildRow();
  copy.hidden = true;
  time.hidden = true;
  syncFeedbackControls(wrap as unknown as HTMLElement, { owner: false, streaming: false });
  check("row stays hidden when neither copy nor feedback shows", actions.hidden === true);
}

if (failed > 0) {
  console.error(`feedback-controls: ${failed} failure(s)`);
  process.exit(1);
}
console.log("feedback-controls ok");
