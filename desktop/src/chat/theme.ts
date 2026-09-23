import { t } from "../i18n";
import type { ChatTheme } from "./types";
import { CHAT_THEME_KEY } from "./types";

export function ensureChatThemeButton(): HTMLButtonElement {
  const existing = document.querySelector<HTMLButtonElement>("#chat-theme");
  if (existing) {
    return existing;
  }
  const top = document.querySelector<HTMLElement>(".chat-top");
  let actions = document.querySelector<HTMLElement>(".chat-top-actions");
  if (!actions) {
    actions = document.createElement("div");
    actions.className = "chat-top-actions";
    top?.appendChild(actions);
  }
  const btn = document.createElement("button");
  btn.id = "chat-theme";
  btn.type = "button";
  btn.className = "chat-theme";
  btn.innerHTML = `
    <svg class="chat-theme-icon chat-theme-icon-moon" width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path d="M21 14.3A8.4 8.4 0 0 1 9.7 3 7.2 7.2 0 1 0 21 14.3Z" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
    </svg>
    <svg class="chat-theme-icon chat-theme-icon-sun" width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="12" cy="12" r="4.2" stroke="currentColor" stroke-width="1.8"/>
      <path d="M12 2.5v2.2M12 19.3v2.2M2.5 12h2.2M19.3 12h2.2M5.05 5.05l1.56 1.56M17.39 17.39l1.56 1.56M5.05 18.95l1.56-1.56M17.39 6.61l1.56-1.56" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>
    </svg>
  `;
  actions.appendChild(btn);
  return btn;
}

export function readStoredChatTheme(): ChatTheme | null {
  try {
    const saved = window.localStorage.getItem(CHAT_THEME_KEY);
    if (saved === "light" || saved === "dark") {
      return saved;
    }
  } catch {
    /* ignore */
  }
  return null;
}

export function systemChatTheme(): ChatTheme {
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function currentChatTheme(): ChatTheme {
  return document.documentElement.dataset.theme === "dark" ? "dark" : "light";
}

export function applyChatTheme(
  theme: ChatTheme,
  themeEl: HTMLButtonElement,
  persist = true,
): void {
  document.documentElement.dataset.theme = theme;
  const nextLabel = theme === "dark" ? t("chat.themeLight") : t("chat.themeDark");
  themeEl.setAttribute("aria-label", nextLabel);
  themeEl.title = nextLabel;
  if (persist) {
    try {
      window.localStorage.setItem(CHAT_THEME_KEY, theme);
    } catch {
      /* ignore */
    }
  }
}
