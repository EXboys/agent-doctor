import { getLocale, setLocale, type Locale } from "../i18n";
import {
  applyThemePreference,
  readThemePreference,
  type ChatThemePreference,
} from "./theme";

const mainEl = () => document.querySelector<HTMLElement>("#chat-main");
const pageEl = () => document.querySelector<HTMLElement>("#chat-settings");
const openEl = () => document.querySelector<HTMLButtonElement>("#chat-settings-open");

export function isChatSettingsOpen(): boolean {
  return mainEl()?.classList.contains("is-settings") ?? false;
}

export function closeChatSettings(): void {
  mainEl()?.classList.remove("is-settings");
  const page = pageEl();
  if (page) {
    page.hidden = true;
    page.setAttribute("aria-hidden", "true");
  }
  openEl()?.classList.remove("is-on");
  openEl()?.setAttribute("aria-pressed", "false");
}

export function openChatSettings(): void {
  syncChatSettings();
  mainEl()?.classList.add("is-settings");
  const page = pageEl();
  if (page) {
    page.hidden = false;
    page.setAttribute("aria-hidden", "false");
  }
  openEl()?.classList.add("is-on");
  openEl()?.setAttribute("aria-pressed", "true");
}

function syncChatSettings(): void {
  const preference = readThemePreference();
  document.querySelectorAll<HTMLButtonElement>("[data-theme-choice]").forEach((button) => {
    const on = button.dataset.themeChoice === preference;
    button.classList.toggle("is-on", on);
    button.setAttribute("aria-checked", on ? "true" : "false");
  });
  const locale = getLocale();
  document.querySelectorAll<HTMLButtonElement>("[data-locale-choice]").forEach((button) => {
    const on = button.dataset.localeChoice === locale;
    button.classList.toggle("is-on", on);
    button.setAttribute("aria-checked", on ? "true" : "false");
  });
}

export function bindChatSettings(applyI18n: () => void): void {
  openEl()?.addEventListener("click", () => {
    if (isChatSettingsOpen()) closeChatSettings();
    else openChatSettings();
  });
  document.querySelector("#chat-settings-back")?.addEventListener("click", () => {
    closeChatSettings();
  });
  document.querySelector("#chat-settings")?.addEventListener("click", (event) => {
    const theme = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-theme-choice]");
    if (theme?.dataset.themeChoice === "light" || theme?.dataset.themeChoice === "dark" || theme?.dataset.themeChoice === "system") {
      applyThemePreference(theme.dataset.themeChoice as ChatThemePreference);
      syncChatSettings();
      return;
    }
    const locale = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-locale-choice]");
    const next = locale?.dataset.localeChoice;
    if (next === "zh" || next === "en") {
      setLocale(next as Locale);
      applyI18n();
      syncChatSettings();
    }
  });
  syncChatSettings();
}
