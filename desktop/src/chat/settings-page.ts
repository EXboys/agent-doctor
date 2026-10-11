import { getLocale, t } from "../i18n";
import { islandEnabled, islandHideWhenIdle, islandSetEnabled, islandSetHideWhenIdle } from "../ipc";
import { publishLocale } from "../locale-sync";
import { checkForAppUpdates, readAppVersion } from "../updater";
import { confirmHintKey, readConfirmMode, writeConfirmMode, type ConfirmMode } from "./confirm-pref";
import { elevatedEl } from "../chat-dom";
import { chatState } from "../chat-state";
import {
  closeKnowledgePage,
  closeResourcesMainPage,
  closeSchedulePage,
  isKnowledgePageOpen,
} from "./overlay-pages";
import {
  applyThemePreference,
  readThemePreference,
  type ChatThemePreference,
} from "./theme";

const shellEl = () => document.querySelector<HTMLElement>("#chat-shell");
const mainEl = () => document.querySelector<HTMLElement>("#chat-main");
const pageEl = () => document.querySelector<HTMLElement>("#chat-settings");
const openEl = () => document.querySelector<HTMLButtonElement>("#chat-settings-open");
const backEl = () => document.querySelector<HTMLButtonElement>("#chat-settings-back");
const searchEl = () => document.querySelector<HTMLInputElement>("#chat-settings-search");
const headingEl = () => document.querySelector<HTMLElement>("#chat-settings-heading");
const emptyEl = () => document.querySelector<HTMLElement>("#chat-settings-empty");
function markChoice(attr: "settingsTheme" | "settingsLocale" | "settingsConfirm", value: string): void {
  const name = attr === "settingsTheme" ? "settings-theme" : attr === "settingsLocale" ? "settings-locale" : "settings-confirm";
  document.querySelectorAll<HTMLButtonElement>(`[data-${name}]`).forEach((button) => {
    const on = button.dataset[attr] === value;
    button.classList.toggle("is-on", on);
    button.setAttribute("aria-selected", on ? "true" : "false");
  });
}

const SECTIONS = ["appearance", "conversation", "language", "about"] as const;
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
  closeSchedulePage();
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
  if (section === "language") return t("chat.settingsLanguage");
  if (section === "conversation") return t("chat.settingsConversation");
  if (section === "about") return t("chat.settingsAbout");
  return t("chat.settingsAppearance");
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

const islandEl = () => document.querySelector<HTMLInputElement>("#chat-settings-island");
const islandIdleEl = () => document.querySelector<HTMLInputElement>("#chat-settings-island-idle");
const versionEl = () => document.querySelector<HTMLElement>("#chat-settings-version");
const confirmHintEl = () => document.querySelector<HTMLElement>("#chat-settings-confirm-hint");

export function refreshConfirmCopy(): void {
  applyConfirmMode(readConfirmMode(), false);
}

function applyConfirmMode(mode: ConfirmMode, approveNow: boolean): void {
  const hint = confirmHintEl();
  if (hint) hint.textContent = t(confirmHintKey(mode));
  elevatedEl.checked = mode === "auto";
  if (approveNow && mode === "auto") {
    void chatState.permissions?.approvePendingChoices();
  }
}

function syncIslandSwitch(): void {
  const input = islandEl();
  const idle = islandIdleEl();
  if (input) input.setAttribute("aria-label", t("chat.settingsIsland"));
  if (idle) idle.setAttribute("aria-label", t("chat.settingsHideIdle"));
  void Promise.all([islandEnabled(), islandHideWhenIdle()])
    .then(([on, hide]) => {
      if (input) input.checked = on;
      if (idle) {
        idle.checked = hide;
        idle.disabled = !on;
      }
    })
    .catch(() => {
      if (input) input.checked = true;
      if (idle) idle.disabled = false;
    });
}

function syncAbout(): void {
  const version = versionEl();
  if (!version) return;
  void readAppVersion().then((value) => {
    version.textContent = t("update.version", { version: value });
  });
}

function syncChatSettings(): void {
  const search = searchEl();
  if (search) search.placeholder = t("chat.settingsSearch");
  markChoice("settingsTheme", readThemePreference());
  markChoice("settingsLocale", getLocale());
  const confirmMode = readConfirmMode();
  markChoice("settingsConfirm", confirmMode);
  applyConfirmMode(confirmMode, false);
  syncIslandSwitch();
  syncAbout();
  showSection(activeSection);
}

export function bindChatSettings(applyI18n: () => void): void {
  openEl()?.addEventListener("click", () => {
    if (isChatSettingsOpen()) closeChatSettings();
    else openChatSettings();
  });
  backEl()?.addEventListener("click", () => closeChatSettings());
  document.querySelector(".chat-settings-menu")?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-settings-nav]");
    const next = button?.dataset.settingsNav;
    if (next === "appearance" || next === "conversation" || next === "language" || next === "about") showSection(next);
  });
  searchEl()?.addEventListener("input", () => showSection(activeSection));
  islandEl()?.addEventListener("change", () => {
    const input = islandEl();
    if (!input) return;
    const on = input.checked;
    const idle = islandIdleEl();
    if (idle) idle.disabled = !on;
    void islandSetEnabled(on).catch(() => {
      input.checked = !on;
      if (idle) idle.disabled = on;
    });
  });
  islandIdleEl()?.addEventListener("change", () => {
    const input = islandIdleEl();
    if (!input) return;
    const hide = input.checked;
    void islandSetHideWhenIdle(hide).catch(() => {
      input.checked = !hide;
    });
  });
  document.querySelector("[data-settings-panel='conversation']")?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-settings-confirm]");
    const next = button?.dataset.settingsConfirm;
    if (next === "ask" || next === "session" || next === "auto") {
      writeConfirmMode(next);
      markChoice("settingsConfirm", next);
      applyConfirmMode(next, true);
    }
  });
  document.querySelector("#chat-settings-update")?.addEventListener("click", () => {
    void checkForAppUpdates({ interactive: true });
  });
  document.querySelector("[data-settings-panel='appearance']")?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-settings-theme]");
    const next = button?.dataset.settingsTheme;
    if (next === "light" || next === "dark" || next === "system") {
      applyThemePreference(next as ChatThemePreference);
      markChoice("settingsTheme", next);
    }
  });
  document.querySelector("[data-settings-panel='language']")?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-settings-locale]");
    const next = button?.dataset.settingsLocale;
    if (next === "zh" || next === "en") {
      void publishLocale(next).then(() => {
        applyI18n();
        syncChatSettings();
      });
    }
  });
  syncChatSettings();
}
