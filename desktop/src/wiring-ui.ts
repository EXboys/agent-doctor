import { appState } from "./app-state";
import { isTeamEdition } from "./edition";
import { createEvotownController } from "./wiring/evotown";
import { createModeController } from "./wiring/mode";
import { createPersonalController } from "./wiring/personal";
import { createPresetsController } from "./wiring/presets";
import type { ModeStatus } from "./types";

export interface WiringUiDeps {
  setMainTab: (tab: "provider") => void;
  refresh: () => Promise<void>;
  loadSkillsInventory: () => Promise<void>;
  hideSkillsInventory: () => void;
}

export interface WiringUiApi {
  syncProviderPanelToEdition: () => void;
  loadModeStatus: () => Promise<void>;
  renderModeStatus: (status: ModeStatus) => void;
  loadEvotownStatus: () => Promise<void>;
  loadPersonalProviderStatus: () => Promise<void>;
  rewireCurrentMode: (hintEl?: HTMLElement | null) => Promise<void>;
  updateFooterCopy: (mode?: string) => void;
  updateWiringModeFootnote: (mode?: string) => void;
  refreshPresetGroupLabels: () => void;
  applyProviderPreset: (presetId: string, opts?: { forceModel?: boolean }) => void;
  showPersonalListView: () => void;
  reloadWiringLocale: () => Promise<void>;
}

export function initWiringUi(d: WiringUiDeps): WiringUiApi {
  const presets = createPresetsController();
  const mode = createModeController({
    setMainTab: d.setMainTab,
    refresh: d.refresh,
  });
  const evotown = createEvotownController({
    refresh: d.refresh,
    loadSkillsInventory: d.loadSkillsInventory,
    hideSkillsInventory: d.hideSkillsInventory,
    loadModeStatus: () => mode.loadModeStatus(),
  });
  const personal = createPersonalController({
    presets,
    refresh: d.refresh,
    loadModeStatus: () => mode.loadModeStatus(),
  });

  presets.renderPresetPicker();
  evotown.bindEvents();
  personal.bindEvents();

  return {
    syncProviderPanelToEdition: () => mode.syncProviderPanelToEdition(),
    loadModeStatus: () => mode.loadModeStatus(),
    renderModeStatus: (status) => mode.renderModeStatus(status),
    loadEvotownStatus: () => evotown.loadEvotownStatus(),
    loadPersonalProviderStatus: () => personal.loadPersonalProviderStatus(),
    rewireCurrentMode: (hintEl) => mode.rewireCurrentMode(hintEl),
    updateFooterCopy: (modeArg) => mode.updateFooterCopy(modeArg),
    updateWiringModeFootnote: (modeArg) => mode.updateWiringModeFootnote(modeArg),
    refreshPresetGroupLabels: () => presets.refreshPresetGroupLabels(),
    applyProviderPreset: (presetId, opts) => presets.applyProviderPreset(presetId, opts),
    showPersonalListView: () => personal.showPersonalListView(),
    reloadWiringLocale: async () => {
      presets.refreshPresetGroupLabels();
      mode.updateFooterCopy(isTeamEdition() ? "team" : "personal");
      mode.updateWiringModeFootnote(appState.lastModeStatus?.mode);
      if (appState.lastModeStatus) {
        mode.renderModeStatus(appState.lastModeStatus);
      }
      const lastEvotown = evotown.getLastStatus();
      if (lastEvotown) {
        evotown.renderEvotownStatus(lastEvotown, { refreshSkills: false });
      }
      if (appState.personalProvidersDoc) {
        personal.renderPersonalProviderList(appState.personalProvidersDoc);
      }
    },
  };
}
