import { isPersonalEdition } from "../edition";
import { getLocale, t } from "../i18n";
import {
  computerUseAccess,
  type ComputerUseAccess,
  islandEnabled,
  islandHideWhenIdle,
  islandSetEnabled,
  islandSetHideWhenIdle,
  teamupsAccountStatus,
  voiceCloudReady,
  wireBrowserMcp,
} from "../ipc";
import { openUrl } from "@tauri-apps/plugin-opener";
import { publishLocale } from "../locale-sync";
import { checkForAppUpdates, readAppVersion } from "../updater";
import { isMac, openMacAccessibility } from "./computer-grant";
import { readAutoApproveDefault, writeAutoApproveDefault } from "./confirm-pref";
import { readImageTextEnabled, setReadImageTextEnabled } from "./image-text";
import { bindKnowledgeSyncSettings, refreshVisibleKnowledgeSync } from "./knowledge-sync";
import {
  readBrowserToolsEnabled,
  readComputerUseEnabled,
  writeBrowserToolsEnabled,
  writeComputerUseEnabled,
} from "./hands-pref";
import { readKnowledgeSyncEnabled, writeKnowledgeSyncEnabled } from "./knowledge-sync-pref";
import { readVoiceRoute, writeVoiceRoute, type VoiceRoute } from "./voice/route";
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
function markChoice(attr: "settingsTheme" | "settingsLocale", value: string): void {
  const name = attr === "settingsTheme" ? "settings-theme" : "settings-locale";
  document.querySelectorAll<HTMLButtonElement>(`[data-${name}]`).forEach((button) => {
    const on = button.dataset[attr] === value;
    button.classList.toggle("is-on", on);
    button.setAttribute("aria-selected", on ? "true" : "false");
  });
}

const SECTIONS = ["appearance", "conversation", "language", "channel", "about"] as const;
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

export function openChatSettings(
  section?: SettingsSection,
  focus?: "knowledge" | "browser" | "computer",
): void {
  if (section) {
    activeSection = section;
    const search = searchEl();
    if (search) search.value = "";
  }
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
  if (section === "conversation") {
    const focusId =
      focus === "browser"
        ? "#chat-settings-browser"
        : focus === "computer"
          ? "#chat-settings-computer"
          : "#chat-settings-knowledge-sync";
    document.querySelector<HTMLInputElement>(focusId)?.focus();
  }
  else searchEl()?.focus();
}

function sectionTitle(section: SettingsSection): string {
  if (section === "language") return t("chat.settingsLanguage");
  if (section === "conversation") return t("chat.settingsConversation");
  if (section === "channel") return t("chat.settingsChannels");
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
const autoApproveDefaultEl = () => document.querySelector<HTMLInputElement>("#chat-settings-auto-approve");
const readImageEl = () => document.querySelector<HTMLInputElement>("#chat-settings-read-image");
const browserEl = () => document.querySelector<HTMLInputElement>("#chat-settings-browser");
const browserHintEl = () => document.querySelector<HTMLElement>("#chat-settings-browser-hint");
const computerEl = () => document.querySelector<HTMLInputElement>("#chat-settings-computer");
const computerHintEl = () => document.querySelector<HTMLElement>("#chat-settings-computer-hint");
let offeredScreenGrant = false;
const knowledgeSyncEl = () => document.querySelector<HTMLInputElement>("#chat-settings-knowledge-sync");
const knowledgeSyncRowEl = () => document.querySelector<HTMLElement>("#chat-settings-knowledge-sync-row");
const voiceHintEl = () => document.querySelector<HTMLElement>("#chat-settings-voice-hint");
let voiceLoggedOut = false;

export function refreshConfirmCopy(): void {
  const input = autoApproveDefaultEl();
  if (!input) return;
  input.checked = readAutoApproveDefault();
  input.setAttribute("aria-label", t("chat.settingsAutoApproveSwitch"));
}

function refreshReadImage(): void {
  const input = readImageEl();
  if (!input) return;
  input.checked = readImageTextEnabled();
  input.setAttribute("aria-label", t("chat.readImageText"));
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

function markVoice(route: VoiceRoute, cloudEnabled: boolean): void {
  document.querySelectorAll<HTMLButtonElement>("[data-settings-voice]").forEach((button) => {
    const value = button.dataset.settingsVoice;
    const on = value === route;
    button.classList.toggle("is-on", on);
    button.setAttribute("aria-selected", on ? "true" : "false");
    button.disabled = value === "cloud" && !cloudEnabled;
    button.textContent =
      value === "cloud" ? t("chat.settingsVoiceCloud") : t("chat.settingsVoiceLocal");
  });
  const hint = voiceHintEl();
  if (!hint) return;
  if (voiceLoggedOut) hint.textContent = t("chat.settingsVoiceLoggedOut");
  else if (!cloudEnabled && route !== "cloud") {
    hint.textContent = t("chat.settingsVoiceSignedOut");
  } else if (route === "cloud") hint.textContent = t("chat.settingsVoiceCloudHint");
  else hint.textContent = t("chat.settingsVoiceLocalHint");
}

function syncVoiceChoice(): void {
  void Promise.all([teamupsAccountStatus().catch(() => null), voiceCloudReady().catch(() => false)]).then(
    ([account, ready]) => {
      const signedIn = account?.signed_in === true;
      if (account && !account.signed_in && readVoiceRoute() === "cloud") {
        writeVoiceRoute("local");
        voiceLoggedOut = true;
      } else if (signedIn) {
        voiceLoggedOut = false;
      }
      const cloudEnabled = signedIn && ready;
      if (!cloudEnabled && readVoiceRoute() === "cloud") {
        writeVoiceRoute("local");
      }
      const route = readVoiceRoute();
      if (signedIn && !ready && !voiceLoggedOut) {
        const hint = voiceHintEl();
        markVoice(route, false);
        if (hint) hint.textContent = t("chat.settingsVoiceUnready");
        return;
      }
      markVoice(route, cloudEnabled);
    },
  );
}

function syncKnowledgeSwitch(): void {
  const row = knowledgeSyncRowEl();
  const input = knowledgeSyncEl();
  if (row) row.hidden = !isPersonalEdition();
  if (!input) return;
  input.checked = readKnowledgeSyncEnabled();
  input.setAttribute("aria-label", t("chat.settingsKnowledgeSyncSwitch"));
}

export function applyBrowserTools(on: boolean): void {
  writeBrowserToolsEnabled(on);
  syncBrowserSwitch();
  if (!on) return;
  void wireBrowserMcp()
    .then((report) => {
      if (!browserHintEl() || !readBrowserToolsEnabled()) return;
      const failed = report.results.find((item) => !item.ok);
      if (failed) browserHintEl()!.textContent = t("chat.settingsBrowserFailed");
    })
    .catch(() => {
      if (browserHintEl() && readBrowserToolsEnabled()) {
        browserHintEl()!.textContent = t("chat.settingsBrowserFailed");
      }
    });
}

export function applyComputerUse(on: boolean): void {
  writeComputerUseEnabled(on);
  if (!on) {
    offeredScreenGrant = false;
    void syncComputerSwitch();
    return;
  }
  void syncComputerSwitch().then((access) => {
    if (!readComputerUseEnabled()) return;
    openComputerGrant(access);
  });
}

function syncBrowserSwitch(): void {
  const input = browserEl();
  const hint = browserHintEl();
  if (input) {
    input.checked = readBrowserToolsEnabled();
    input.setAttribute("aria-label", t("chat.settingsBrowserSwitch"));
  }
  if (hint) {
    hint.textContent = input?.checked ? t("chat.settingsBrowserReady") : t("chat.settingsBrowserHint");
  }
}

function openComputerGrant(access: ComputerUseAccess | null): void {
  if (access?.trusted) return;
  if (!access || access.step === "accessibility" || (isMac() && access.step !== "screen")) {
    const hint = computerHintEl();
    if (hint) hint.textContent = t("chat.settingsComputerGuideAccessibility");
    void openMacAccessibility().catch(() => {
      if (computerHintEl()) computerHintEl()!.textContent = t("chat.handsSystemFailed");
    });
    return;
  }
  const url = access.settings_url;
  if (!url) return;
  if (access.step === "screen") offeredScreenGrant = true;
  void openUrl(url).catch(() => {
    const hint = computerHintEl();
    if (hint) hint.textContent = t("chat.handsSystemFailed");
  });
}

function computerHint(access: ComputerUseAccess | null, on: boolean): string {
  if (!on) {
    if (access?.platform === "windows") return t("chat.settingsComputerHintWindows");
    if (access?.platform === "macos") return t("chat.settingsComputerHintMac");
    return t("chat.settingsComputerHint");
  }
  if (!access) return isMac() ? t("chat.settingsComputerGuideAccessibility") : t("chat.settingsComputerHint");
  if (access.step === "accessibility") return t("chat.settingsComputerGuideAccessibility");
  if (access.step === "screen") return t("chat.settingsComputerGuideScreen");
  if (access.platform === "windows") return t("chat.settingsComputerReadyWindows");
  if (!access.trusted) return t("chat.settingsComputerGuideAccessibility");
  return t("chat.settingsComputerReady");
}

async function syncComputerSwitch(): Promise<ComputerUseAccess | null> {
  const input = computerEl();
  const hint = computerHintEl();
  if (input) {
    input.checked = readComputerUseEnabled();
    input.setAttribute("aria-label", t("chat.settingsComputerSwitch"));
  }
  let access: ComputerUseAccess | null = null;
  try {
    access = await computerUseAccess();
  } catch {
    access = null;
  }
  if (hint) hint.textContent = computerHint(access, Boolean(input?.checked));
  return access;
}

function syncChatSettings(): void {
  const search = searchEl();
  if (search) search.placeholder = t("chat.settingsSearch");
  markChoice("settingsTheme", readThemePreference());
  markChoice("settingsLocale", getLocale());
  refreshConfirmCopy();
  refreshReadImage();
  syncBrowserSwitch();
  void syncComputerSwitch();
  syncKnowledgeSwitch();
  syncVoiceChoice();
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
    if (next === "appearance" || next === "conversation" || next === "language" || next === "channel" || next === "about") showSection(next);
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
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-settings-voice]");
    const next = button?.dataset.settingsVoice;
    if (button?.disabled) return;
    if (next === "local" || next === "cloud") {
      writeVoiceRoute(next);
      voiceLoggedOut = false;
      syncVoiceChoice();
    }
  });
  browserEl()?.addEventListener("change", () => {
    const input = browserEl();
    if (!input) return;
    applyBrowserTools(input.checked);
  });
  computerEl()?.addEventListener("change", () => {
    const input = computerEl();
    if (!input) return;
    applyComputerUse(input.checked);
  });
  window.addEventListener("focus", () => {
    if (!isChatSettingsOpen() || !readComputerUseEnabled()) return;
    void syncComputerSwitch().then((access) => {
      if (access?.step === "screen" && !offeredScreenGrant) openComputerGrant(access);
    });
  });
  autoApproveDefaultEl()?.addEventListener("change", () => {
    const input = autoApproveDefaultEl();
    if (!input) return;
    writeAutoApproveDefault(input.checked);
  });
  readImageEl()?.addEventListener("change", () => {
    const input = readImageEl();
    if (!input) return;
    setReadImageTextEnabled(input.checked);
  });
  bindKnowledgeSyncSettings(() => openChatSettings("conversation"));
  knowledgeSyncEl()?.addEventListener("change", () => {
    const input = knowledgeSyncEl();
    if (!input) return;
    writeKnowledgeSyncEnabled(input.checked);
    refreshVisibleKnowledgeSync();
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
