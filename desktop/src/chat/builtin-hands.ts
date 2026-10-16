import { t } from "../i18n";
import { applyBrowserTools, applyComputerUse } from "./settings-page";
import { readBrowserToolsEnabled, readComputerUseEnabled } from "./hands-pref";

export type BuiltinHandId = "browser" | "computer";

export type BuiltinHand = {
  id: BuiltinHandId;
  title: string;
  line: string;
};

export function builtinHands(query: string): BuiltinHand[] {
  const items: BuiltinHand[] = [
    {
      id: "browser",
      title: t("chat.settingsBrowser"),
      line: t("chat.handBrowserLine"),
    },
    {
      id: "computer",
      title: t("chat.settingsComputer"),
      line: t("chat.handComputerLine"),
    },
  ];
  const q = query.trim().toLowerCase();
  if (!q) return items;
  return items.filter((item) => `${item.title} ${item.line}`.toLowerCase().includes(q));
}

function handIcon(id: BuiltinHandId): string {
  if (id === "browser") {
    return `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="8" stroke="currentColor" stroke-width="1.7"/><path d="M4 12h16M12 4c2.2 2.4 3.3 5.1 3.3 8s-1.1 5.6-3.3 8c-2.2-2.4-3.3-5.1-3.3-8s1.1-5.6 3.3-8z" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/></svg>`;
  }
  return `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden="true"><rect x="3" y="4" width="18" height="12" rx="2" stroke="currentColor" stroke-width="1.7"/><path d="M8 20h8M12 16v4" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>`;
}

function handOn(id: BuiltinHandId): boolean {
  return id === "browser" ? readBrowserToolsEnabled() : readComputerUseEnabled();
}

function setHand(id: BuiltinHandId, on: boolean): void {
  if (id === "browser") applyBrowserTools(on);
  else applyComputerUse(on);
}

/** A built-in hand with the same switch as 设置 → 对话. */
export function builtinHandRow(hand: BuiltinHand, surface: "page" | "menu"): HTMLElement {
  const row = document.createElement("div");
  row.className = surface === "page" ? "chat-res-row is-hand" : "chat-slash-hand";
  row.dataset.kind = "hand";
  row.dataset.id = hand.id;
  if (surface === "page") {
    row.addEventListener("click", (event) => event.stopPropagation());
  }

  if (surface === "page") {
    const icon = document.createElement("span");
    icon.className = "chat-res-icon";
    icon.dataset.tone = "hand";
    icon.innerHTML = handIcon(hand.id);

    const body = document.createElement("span");
    body.className = "chat-res-body";
    const title = document.createElement("span");
    title.className = "chat-res-title";
    title.textContent = hand.title;
    const desc = document.createElement("span");
    desc.className = "chat-res-desc";
    desc.textContent = hand.line;
    body.append(title, desc);
    row.append(icon, body);
  } else {
    const icon = document.createElement("span");
    icon.className = "chat-slash-hand-icon";
    icon.innerHTML = handIcon(hand.id);
    const copy = document.createElement("span");
    copy.className = "chat-slash-hand-copy";
    copy.textContent = hand.title;
    row.append(icon, copy);
  }

  const input = document.createElement("input");
  input.type = "checkbox";
  input.className = "chat-settings-switch";
  input.checked = handOn(hand.id);
  input.setAttribute("aria-label", hand.title);
  input.addEventListener("click", (event) => event.stopPropagation());
  input.addEventListener("change", () => {
    setHand(hand.id, input.checked);
    document.querySelectorAll<HTMLInputElement>(`[data-kind="hand"][data-id="${hand.id}"] .chat-settings-switch`).forEach((other) => {
      if (other !== input) other.checked = input.checked;
    });
  });
  row.append(input);
  return row;
}
