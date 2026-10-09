import { t } from "../i18n";
import type { ChatTheme } from "./types";
import { CHAT_THEME_KEY } from "./types";

export type ChatThemePreference = ChatTheme | "system";

export function ensureChatThemeButton(): HTMLButtonElement | null {
  return document.querySelector<HTMLButtonElement>("#chat-theme");
}

export function readThemePreference(): ChatThemePreference {
  try {
    const saved = window.localStorage.getItem(CHAT_THEME_KEY);
    if (saved === "light" || saved === "dark" || saved === "system") {
      return saved;
    }
  } catch {
    /* ignore */
  }
  return "system";
}

export function readStoredChatTheme(): ChatTheme | null {
  const saved = readThemePreference();
  return saved === "system" ? null : saved;
}

export function systemChatTheme(): ChatTheme {
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function currentChatTheme(): ChatTheme {
  return document.documentElement.dataset.theme === "dark" ? "dark" : "light";
}

export function applyThemePreference(preference: ChatThemePreference, persist = true): void {
  const theme = preference === "system" ? systemChatTheme() : preference;
  document.documentElement.dataset.theme = theme;
  if (persist) {
    try {
      window.localStorage.setItem(CHAT_THEME_KEY, preference);
    } catch {
      /* ignore */
    }
  }
}

let systemListener: ((event: MediaQueryListEvent) => void) | null = null;

export function watchSystemTheme(): void {
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  if (systemListener) {
    media.removeEventListener("change", systemListener);
  }
  systemListener = () => {
    if (readThemePreference() === "system") {
      document.documentElement.dataset.theme = systemChatTheme();
    }
  };
  media.addEventListener("change", systemListener);
}

export function applyChatTheme(
  theme: ChatTheme,
  themeEl: HTMLButtonElement | null,
  persist = true,
): void {
  document.documentElement.dataset.theme = theme;
  if (themeEl) {
    const nextLabel = theme === "dark" ? t("chat.themeLight") : t("chat.themeDark");
    themeEl.setAttribute("aria-label", nextLabel);
    themeEl.title = nextLabel;
  }
  if (persist) {
    try {
      window.localStorage.setItem(CHAT_THEME_KEY, theme);
    } catch {
      /* ignore */
    }
  }
}
