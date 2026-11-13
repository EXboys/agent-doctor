/**
 * Heavy-session render benchmark: synthetic super-long session tail (2000
 * tool rows, giant pasted user message, 200k-char thinking block) through
 * the REAL render functions, timed per stage.
 * Run: cd desktop && node --import tsx scripts/bench-heavy-render.ts
 */
import { parseHTML } from "linkedom";

const { document, window } = parseHTML("<html><body><div id=\"log\"></div></body></html>");
(globalThis as any).document = document;
(globalThis as any).window = window;
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { language: "zh-CN" },
});
(globalThis as any).localStorage = {
  getItem: () => null,
  setItem: () => undefined,
  removeItem: () => undefined,
};
// linkedom exposes element classes on its window; surface the ones the
// render path touches.
for (const key of [
  "HTMLElement", "HTMLDetailsElement", "HTMLButtonElement",
  "HTMLInputElement", "HTMLTextAreaElement", "SVGCircleElement",
  "ShadowRoot", "CustomEvent", "MutationObserver",
]) {
  if ((window as any)[key]) (globalThis as any)[key] = (window as any)[key];
}

function ms(label: string, fn: () => unknown): unknown {
  const t0 = performance.now();
  const out = fn();
  console.log(`${label}: ${(performance.now() - t0).toFixed(0)} ms`);
  return out;
}

const { renderToolHistoryGroup, createToolRowElement } = await import("../src/chat/activity");
const { renderThinkingBlock } = await import("../src/chat/thinking");

// --- synthetic tail turn: 2000 tool rows -----------------------------------
const TOOL_RUN_ROWS = 2000;
const toolRecords: Array<{ text: string; step: any }> = [];
for (let i = 0; i < TOOL_RUN_ROWS; i += 1) {
  const output = `step ${i} done\nexit 0\n` + "行".repeat(300 + ((i * 7919) % 2000));
  toolRecords.push({
    text: `调用工具 Bash…\nnpm test --series=${i}\n${output.slice(0, 200)}`,
    step: {
      id: `t${i}`,
      kind: "terminal",
      status: "done",
      name: "Bash",
      command: `npm test --series=${i}`,
      output,
    },
  });
}

// stage 1: record->group render (mounts last 40 rows only)
const g1 = ms("renderToolHistoryGroup: 2000 records, mount last page (40)", () =>
  renderToolHistoryGroup(toolRecords),
) as HTMLDetailsElement;

// stage 2: deep exploration - mount ALL pages like the inner pager does
ms("mount ALL 2000 rows (simulating pager reveals)", () => {
  const list = g1.querySelector(".chat-tool-list")!;
  for (const record of toolRecords) {
    const row = createToolRowElement(record.text, record.step);
    list.appendChild(row);
  }
});

// stage 3: giant pasted user message (~2MB) through the user-bubble path
{
  const paste = ["2026-11-11 10:00:01 INFO build start"];
  for (let i = 0; i < 50_000; i += 1) {
    paste.push(`2026-11-11 10:${String(Math.floor(i / 60) % 60).padStart(2, "0")}:${String(i % 60).padStart(2, "0")} WARN module-${i % 97} something happened ${i}`);
  }
  const giant = paste.join("\n");
  console.log(`giant paste: ${(giant.length / 1e6).toFixed(1)}M chars`);
  ms("user bubble textContent (2MB paste, is-folded)", () => {
    const bubble = document.createElement("div");
    bubble.className = "chat-bubble user is-folded";
    bubble.textContent = giant;
  });
  ms("user bubble 4000-char slice (beta.18 behavior)", () => {
    const bubble = document.createElement("div");
    bubble.className = "chat-bubble user";
    bubble.textContent = giant.slice(0, 3600) + "\n…\n" + giant.slice(-400);
  });
}

// stage 4: deep thinking block (200k chars)
{
  const think = "推理步骤 ".repeat(100_000).slice(0, 200_000);
  ms("renderThinkingBlock(200k chars, closed)", () =>
    renderThinkingBlock(think, { id: "think-1" }),
  );
}

console.log("bench done");
