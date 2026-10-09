import { getLocale, setLocale, t, type Locale } from "../i18n";
import { closeKnowledgePage, closeResourcesMainPage, isKnowledgePageOpen } from "./overlay-pages";
import {
  applyThemePreference,
  readThemePreference,
  type ChatThemePreference,
} from "./theme";

const shellEl = () => document.querySelector<HTMLElement>("#chat-shell");
const mainEl = () => document.querySelector<HTMLElement>("#chat-main");
const pageEl = () => document.querySelector<HTMLElement>("#chat-settings");
const openEl = () => document.querySelector<HTMLButtonElement>("#chat-settings-open");
const searchEl = () => document.querySelector<HTMLInputElement>("#chat-settings-search");
const headingEl = () => document.querySelector<HTMLElement>("#chat-settings-heading");
const emptyEl = () => document.querySelector<HTMLElement>("#chat-settings-empty");
const themeEl = () => document.querySelector<HTMLSelectElement>("#chat-settings-theme");
const localeEl = () => document.querySelector<HTMLSelectElement>("#chat-settings-locale");

const SECTIONS = ["appearance", "language"] as const;
type SettingsSection = (typeof SECTIONS)[number];

let activeSection: SettingsSection = "appearance";

export function isChatSettingsOpen(): boolean {
  return mainEl()?.classList.contains("is-settings") ?? false;
}

export function closeChatSettings(): void {
  shellEl()?.classList.remove("is-settings");
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
  if (isKnowledgePageOpen()) closeKnowledgePage();
  closeResourcesMainPage();
  syncChatSettings();
  shellEl()?.classList.add("is-settings");
  mainEl()?.classList.add("is-settings");
  const page = pageEl();
  if (page) {
    page.hidden = false;
    page.setAttribute("aria-hidden", "false");
  }
  openEl()?.classList.add("is-on");
  openEl()?.setAttribute("aria-pressed", "true");
  searchEl()?.focus();
}

function sectionTitle(section: SettingsSection): string {
  return section === "language" ? t("chat.settingsLanguage") : t("chat.settingsAppearance");
}

function sectionMatches(section: SettingsSection, query: string): boolean {
  if (!query) return true;
  const nav = document.querySelector<HTMLElement>(`[data-settings-nav="${section}"]`);
  const panel = document.querySelector<HTMLElement>(`[data-settings-panel="${section}"]`);
  const blob = `${nav?.textContent ?? ""} ${panel?.textContent ?? ""}`.toLowerCase();
  return blob.includes(query);
}

function showSection(section: SettingsSection): void {
  activeSection = section;
  const query = searchEl()?.value.trim().toLowerCase() ?? "";
  const visible = SECTIONS.filter((id) => sectionMatches(id, query));
  document.querySelectorAll<HTMLButtonElement>("[data-settings-nav]").forEach((button) => {
    const id = button.dataset.settingsNav as SettingsSection | undefined;
    const shown = id ? visible.includes(id) : false;
    button.hidden = !shown;
    const on = shown && id === activeSection;
    button.classList.toggle("is-on", on);
    if (on) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  });
  document.querySelectorAll<HTMLElement>("[data-settings-panel]").forEach((panel) => {
    const id = panel.dataset.settingsPanel as SettingsSection | undefined;
    const shown = id === activeSection && visible.includes(activeSection);
    panel.hidden = !shown;
  });
  const heading = headingEl();
  const empty = emptyEl();
  if (visible.length === 0) {
    if (heading) heading.hidden = true;
    if (empty) empty.hidden = false;
    return;
  }
  if (!visible.includes(activeSection)) {
    showSection(visible[0]);
    return;
  }
  if (heading) {
    heading.hidden = false;
    heading.textContent = sectionTitle(activeSection);
  }
  if (empty) empty.hidden = true;
}

function syncChatSettings(): void {
  const search = searchEl();
  if (search) search.placeholder = t("chat.settingsSearch");
  const theme = themeEl();
  if (theme) theme.value = readThemePreference();
  const locale = localeEl();
  if (locale) locale.value = getLocale();
  showSection(activeSection);
}

export function bindChatSettings(applyI18n: () => void): void {
  openEl()?.addEventListener("click", () => {
    if (isChatSettingsOpen()) closeChatSettings();
    else openChatSettings();
  });
  document.querySelector(".chat-settings-menu")?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-settings-nav]");
    const next = button?.dataset.settingsNav;
    if (next === "appearance" || next === "language") showSection(next);
  });
  searchEl()?.addEventListener("input", () => showSection(activeSection));
  themeEl()?.addEventListener("change", () => {
    const next = themeEl()?.value;
    if (next === "light" || next === "dark" || next === "system") {
      applyThemePreference(next as ChatThemePreference);
    }
  });
  localeEl()?.addEventListener("change", () => {
    const next = localeEl()?.value;
    if (next === "zh" || next === "en") {
      setLocale(next as Locale);
      applyI18n();
      syncChatSettings();
    }
  });
  syncChatSettings();
}
