import { invoke } from "@tauri-apps/api/core";
import { appState } from "../app-state";
import { isPersonalEdition, isTeamEdition, productEdition } from "../edition";
import { withErrorDetail } from "../friendly-error";
import { t } from "../i18n";
import type { ModeStatus, ModeSwitchReport } from "../types";
import {
  formatModeSwitchDetail,
  formatModeSwitchHint,
  wantsBrowserMcp,
} from "./format";

const modeMetaEl = document.querySelector<HTMLElement>("#mode-meta")!;
const modeHintEl = document.querySelector<HTMLElement>("#mode-hint")!;
const wiringModeFootnoteEl = document.querySelector<HTMLElement>("#wiring-mode-footnote")!;
const footerCopyEl = document.querySelector<HTMLElement>("#footer-copy")!;
const providerPanels = Array.from(document.querySelectorAll<HTMLElement>("[data-provider-panel]"));
const personalApplyEl = document.querySelector<HTMLButtonElement>("#personal-apply")!;
const personalVerifyEl = document.querySelector<HTMLButtonElement>("#personal-verify")!;
const personalSaveEl = document.querySelector<HTMLButtonElement>("#personal-save")!;
const evotownConnectEl = document.querySelector<HTMLButtonElement>("#evotown-connect")!;
const evotownResyncEl = document.querySelector<HTMLButtonElement>("#evotown-resync")!;

export type ModeDeps = {
  setMainTab: (tab: "provider") => void;
  refresh: () => Promise<void>;
};

export type ModeApi = ReturnType<typeof createModeController>;

export function createModeController(deps: ModeDeps) {
  let modeSwitchInFlight = false;

  function updateFooterCopy(mode?: string): void {
    const activeMode = mode ?? appState.lastModeStatus?.mode ?? productEdition();
    footerCopyEl.textContent = activeMode === "team" ? t("app.footerTeam") : t("app.footer");
  }

  function updateWiringModeFootnote(_mode?: string): void {
    // Pathway toggle removed — Browser MCP always writes with provider apply.
    if (!wiringModeFootnoteEl.classList.contains("is-busy")) {
      wiringModeFootnoteEl.textContent = "";
      wiringModeFootnoteEl.removeAttribute("title");
    }
  }

  /** Provider panel is locked to the build edition — no Personal/Team switch tabs. */
  function syncProviderPanelToEdition() {
    const tab = isTeamEdition() ? "evotown" : "personal";
    for (const panel of providerPanels) {
      const active = panel.dataset.providerPanel === tab;
      panel.classList.toggle("is-active", active);
      panel.hidden = !active;
    }
  }

  function modeDisplayName(mode: string): string {
    if (mode === "personal") return t("mode.personal");
    if (mode === "team") return t("mode.team");
    return t("mode.unset");
  }

  function renderModeStatus(status: ModeStatus) {
    appState.lastModeStatus = status;
    const meta = status.active_gateway_url
      ? t("mode.meta", {
          label: status.active_label || modeDisplayName(status.mode),
          url: status.active_gateway_url,
          key: status.active_key_hint || "—",
        })
      : t("mode.metaEmpty");
    modeMetaEl.textContent = meta;
    syncProviderPanelToEdition();
    updateWiringModeFootnote(status.mode);
    updateFooterCopy(isTeamEdition() ? "team" : "personal");
  }

  async function loadModeStatus() {
    try {
      const status = await invoke<ModeStatus>("get_mode_status_command");
      renderModeStatus(status);
    } catch (error) {
      modeMetaEl.textContent = String(error);
      syncProviderPanelToEdition();
    }
  }

  function showModeHint(text: string, detail?: string) {
    // Keep sr-only #mode-hint for a11y, but also surface on the visible footnote —
    // otherwise wiring actions look "stuck" while busy.
    modeHintEl.hidden = !text;
    modeHintEl.textContent = text;
    if (text) {
      wiringModeFootnoteEl.textContent = text;
      wiringModeFootnoteEl.title = detail || text;
    }
  }

  function setModeSwitchBusy(busy: boolean) {
    modeSwitchInFlight = busy;
    if (isPersonalEdition()) {
      personalApplyEl.disabled = busy;
      personalVerifyEl.disabled = busy;
      personalSaveEl.disabled = busy;
    } else {
      evotownConnectEl.disabled = busy;
      evotownResyncEl.disabled = busy;
    }
    wiringModeFootnoteEl.classList.toggle("is-busy", busy);
  }

  async function rewireCurrentMode(hintEl?: HTMLElement | null) {
    if (modeSwitchInFlight) return;
    const locked = productEdition();
    const mode = appState.lastModeStatus?.mode;
    if (mode !== locked) {
      // Edition package only rewires its own path; jump to wiring to configure.
      deps.setMainTab("provider");
      syncProviderPanelToEdition();
      showModeHint(
        isTeamEdition() ? t("mode.teamNotReady") : t("mode.personalNotReady"),
      );
      if (hintEl) {
        hintEl.hidden = false;
        hintEl.textContent = t("runtime.installWireNext");
      }
      return;
    }
    setModeSwitchBusy(true);
    if (hintEl) {
      hintEl.hidden = false;
      hintEl.textContent = t("mode.switching");
    }
    try {
      const report = await invoke<ModeSwitchReport>("rewire_current_mode_command", {
        withBrowserMcp: wantsBrowserMcp(),
      });
      await loadModeStatus();
      const message = t("mode.switchOk", {
        message: `${t("mode.rewireOk")} · ${formatModeSwitchHint(report)}`,
      });
      showModeHint(message, formatModeSwitchDetail(report));
      if (hintEl) {
        hintEl.textContent = message;
      }
      void deps.refresh();
    } catch (error) {
      const message = withErrorDetail(t("mode.rewireFailed"), error);
      showModeHint(message);
      if (hintEl) {
        hintEl.hidden = false;
        hintEl.textContent = message;
      }
    } finally {
      setModeSwitchBusy(false);
    }
  }

  return {
    syncProviderPanelToEdition,
    loadModeStatus,
    renderModeStatus,
    rewireCurrentMode,
    updateFooterCopy,
    updateWiringModeFootnote,
    showModeHint,
  };
}
