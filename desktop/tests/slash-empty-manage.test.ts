/**
 * Run from desktop/: node --import tsx tests/slash-empty-manage.test.ts
 * Empty slash tabs for skills and knowledge offer a way into their manage pages.
 */
const storage = new Map<string, string>();
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
  value: { language: "zh" },
});

type Node = {
  tagName: string;
  className: string;
  textContent: string;
  hidden: boolean;
  type: string;
  children: Node[];
  childElementCount: number;
  parent: Node | null;
  listeners: Record<string, Array<(event: Eventish) => void>>;
  append: (...nodes: Node[]) => void;
  appendChild: (node: Node) => Node;
  replaceChildren: (...nodes: Node[]) => void;
  addEventListener: (type: string, fn: (event: Eventish) => void) => void;
  setAttribute: () => void;
  classList: { add: () => void; remove: () => void; toggle: () => boolean; contains: () => boolean };
  querySelector: (selector: string) => Node | null;
  click: () => void;
};

type Eventish = { preventDefault: () => void; stopPropagation: () => void };

function el(tag: string): Node {
  const node: Node = {
    tagName: tag.toUpperCase(),
    className: "",
    textContent: "",
    hidden: false,
    type: "",
    children: [],
    get childElementCount() {
      return node.children.length;
    },
    parent: null,
    listeners: {},
    append(...nodes: Node[]) {
      for (const child of nodes) {
        child.parent = node;
        node.children.push(child);
      }
    },
    appendChild(child: Node) {
      child.parent = node;
      node.children.push(child);
      return child;
    },
    replaceChildren(...nodes: Node[]) {
      node.children.forEach((child) => {
        child.parent = null;
      });
      node.children.length = 0;
      node.append(...nodes);
    },
    addEventListener(type: string, fn: (event: Eventish) => void) {
      (node.listeners[type] ??= []).push(fn);
    },
    setAttribute() {},
    classList: {
      add() {},
      remove() {},
      toggle() {
        return false;
      },
      contains: () => false,
    },
    querySelector(selector: string) {
      const className = selector.startsWith(".") ? selector.slice(1) : "";
      const walk = (current: Node): Node | null => {
        if (className && current.className.split(/\s+/).includes(className)) return current;
        for (const child of current.children) {
          const found = walk(child);
          if (found) return found;
        }
        return null;
      };
      for (const child of node.children) {
        const found = walk(child);
        if (found) return found;
      }
      return null;
    },
    click() {
      const event = { preventDefault() {}, stopPropagation() {} };
      for (const fn of node.listeners.click ?? []) fn(event);
    },
  };
  return node;
}

Object.defineProperty(globalThis, "document", {
  configurable: true,
  writable: true,
  value: {
    documentElement: { lang: "zh" },
    createElement: (tag: string) => el(tag),
    querySelectorAll: () => [],
  },
});

const { setLocale, t } = await import("../src/i18n");
const { AskMentionMenuController } = await import("../src/ask-resources");

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
  console.log(`ok - ${msg}`);
}

setLocale("zh");

const prompt = el("textarea");
let value = "";
let caret = 0;
Object.defineProperty(prompt, "value", {
  get: () => value,
  set: (next: string) => {
    value = next;
  },
});
Object.defineProperty(prompt, "selectionStart", { get: () => caret });
prompt.setSelectionRange = (start: number) => {
  caret = start;
};
(prompt as Node & { focus: () => void; setSelectionRange: (start: number) => void }).focus = () => {};

const menu = el("div");
const opened: string[] = [];
const mentionMenu = new AskMentionMenuController(
  { promptEl: prompt as unknown as HTMLTextAreaElement, mentionMenuEl: menu as unknown as HTMLElement },
  () => [],
  () => {},
  () => {},
  undefined,
  undefined,
  (kind) => opened.push(kind),
);

value = "/";
caret = 1;
mentionMenu.renderMentionMenu();
const skillButton = menu.querySelector(".chat-empty-next");
assert(skillButton?.textContent === t("chat.slashManageSkills"), "skills empty offers add");
skillButton?.click();
assert(opened.join() === "skill", "skills button opens the skills page");
assert(value === "", "slash token is cleared");
assert(menu.hidden === true, "picker closes");

value = "/";
caret = 1;
mentionMenu.setSlashTab("knowledge");
const knowledgeButton = menu.querySelector(".chat-empty-next");
assert(knowledgeButton?.textContent === t("chat.slashManageKnowledge"), "knowledge empty offers add");
knowledgeButton?.click();
assert(opened.join() === "skill,knowledge", "knowledge button opens the knowledge page");

value = "/zzzz";
caret = 5;
mentionMenu.setSlashTab("mcp");
assert(menu.querySelector(".chat-empty-next") === null, "tools empty stays a sentence");
