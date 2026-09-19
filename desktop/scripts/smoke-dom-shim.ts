const memoryStore = new Map<string, string>();
(globalThis as { localStorage?: Storage }).localStorage = {
  getItem: (key: string) => memoryStore.get(key) ?? null,
  setItem: (key: string, value: string) => {
    memoryStore.set(key, String(value));
  },
  removeItem: (key: string) => {
    memoryStore.delete(key);
  },
  clear: () => memoryStore.clear(),
  key: (index: number) => [...memoryStore.keys()][index] ?? null,
  get length() {
    return memoryStore.size;
  },
} as Storage;

(globalThis as { document?: Document }).document = {
  documentElement: { lang: "" },
  querySelectorAll: () => [],
  createElement: (tag: string) => {
    const attrs: Record<string, string> = {};
    const children: unknown[] = [];
    return {
      tagName: tag.toUpperCase(),
      className: "",
      textContent: "",
      hidden: false,
      title: "",
      style: {},
      dataset: {},
      children,
      setAttribute(name: string, value: string) {
        attrs[name] = value;
      },
      getAttribute(name: string) {
        return attrs[name] ?? null;
      },
      append(...nodes: unknown[]) {
        children.push(...nodes);
      },
      appendChild(node: unknown) {
        children.push(node);
        return node;
      },
      replaceChildren(...nodes: unknown[]) {
        children.length = 0;
        children.push(...nodes);
      },
      addEventListener() {},
      removeEventListener() {},
      classList: {
        add() {},
        remove() {},
        toggle(_token?: string, _force?: boolean) {},
        contains: () => false,
      },
    };
  },
} as unknown as Document;
