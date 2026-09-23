import { t, type MessageKey } from "../i18n";
import {
  modelsForCustomProtocol,
  modelsForPresetId,
} from "../provider-models";
import { PROVIDER_PRESETS } from "../provider-presets";

const personalPresetEl = document.querySelector<HTMLSelectElement>("#personal-preset")!;
const personalPresetPickerEl = document.querySelector<HTMLElement>("#personal-preset-picker");
const personalProtocolEl = document.querySelector<HTMLSelectElement>("#personal-protocol")!;
const personalNameRowEl = document.querySelector<HTMLElement>("#personal-name-row")!;
const personalNameEl = document.querySelector<HTMLInputElement>("#personal-name")!;
const personalUrlEl = document.querySelector<HTMLInputElement>("#personal-url")!;
const personalModelEl = document.querySelector<HTMLInputElement>("#personal-model")!;
const personalModelSelectEl = document.querySelector<HTMLSelectElement>("#personal-model-select");
const personalModelSuggestionsEl = document.querySelector<HTMLDataListElement>(
  "#personal-model-suggestions",
)!;
const personalPresetUrlEl = document.querySelector<HTMLElement>("#personal-preset-url");
const personalAdvancedEl = document.querySelector<HTMLDetailsElement>("#personal-advanced");
const personalSaveEl = document.querySelector<HTMLButtonElement>("#personal-save")!;
const personalKeyEl = document.querySelector<HTMLInputElement>("#personal-key")!;

const PRESET_PICKER_GROUPS: Array<{ labelKey: MessageKey; ids: string[] }> = [
  {
    labelKey: "personal.groupPopular",
    ids: ["deepseek", "qwen", "glm", "minimax", "moonshot"],
  },
  {
    labelKey: "personal.groupGlobal",
    ids: ["openai", "anthropic", "gemini"],
  },
  {
    labelKey: "personal.groupHub",
    ids: ["siliconflow", "openrouter", "groq"],
  },
];

export type PresetsApi = ReturnType<typeof createPresetsController>;

export function createPresetsController() {
  function chipLabel(presetId: string): string {
    if (presetId === "custom") {
      return t("personal.presetCustom");
    }
    return PROVIDER_PRESETS[presetId]?.chip ?? PROVIDER_PRESETS[presetId]?.name ?? presetId;
  }

  function syncPresetPicker(activeId: string) {
    if (!personalPresetPickerEl) return;
    personalPresetPickerEl.querySelectorAll<HTMLButtonElement>(".provider-chip").forEach((chip) => {
      const selected = chip.dataset.presetId === activeId;
      chip.classList.toggle("is-active", selected);
      chip.setAttribute("aria-selected", selected ? "true" : "false");
    });
  }

  function renderPresetPicker() {
    if (!personalPresetPickerEl) return;
    personalPresetPickerEl.innerHTML = "";

    for (const group of PRESET_PICKER_GROUPS) {
      const groupEl = document.createElement("div");
      groupEl.className = "provider-picker-group";

      const labelEl = document.createElement("div");
      labelEl.className = "provider-picker-label";
      labelEl.dataset.i18nLabel = group.labelKey;
      labelEl.textContent = t(group.labelKey);
      groupEl.appendChild(labelEl);

      const chipsEl = document.createElement("div");
      chipsEl.className = "provider-picker-chips";
      for (const id of group.ids) {
        const chip = document.createElement("button");
        chip.type = "button";
        chip.className = "provider-chip";
        chip.dataset.presetId = id;
        chip.setAttribute("role", "option");
        chip.textContent = chipLabel(id);
        chipsEl.appendChild(chip);
      }
      groupEl.appendChild(chipsEl);
      personalPresetPickerEl.appendChild(groupEl);
    }

    const customGroup = document.createElement("div");
    customGroup.className = "provider-picker-group";
    const customChips = document.createElement("div");
    customChips.className = "provider-picker-chips";
    const customChip = document.createElement("button");
    customChip.type = "button";
    customChip.className = "provider-chip is-custom";
    customChip.dataset.presetId = "custom";
    customChip.setAttribute("role", "option");
    customChip.textContent = chipLabel("custom");
    customChips.appendChild(customChip);
    customGroup.appendChild(customChips);
    personalPresetPickerEl.appendChild(customGroup);

    syncPresetPicker(personalPresetEl.value || "deepseek");
  }

  function refreshPresetGroupLabels() {
    if (personalPresetPickerEl) {
      personalPresetPickerEl.querySelectorAll<HTMLElement>("[data-i18n-label]").forEach((el) => {
        const key = el.dataset.i18nLabel;
        if (
          key === "personal.groupPopular" ||
          key === "personal.groupGlobal" ||
          key === "personal.groupHub"
        ) {
          el.textContent = t(key);
        }
      });
      const custom = personalPresetPickerEl.querySelector<HTMLElement>(
        '.provider-chip[data-preset-id="custom"]',
      );
      if (custom) {
        custom.textContent = t("personal.presetCustom");
      }
    }
    personalPresetEl.querySelectorAll("optgroup").forEach((group) => {
      const key = group.getAttribute("data-i18n-label");
      if (key === "personal.groupOpenAI" || key === "personal.groupClaude") {
        group.label = t(key);
      }
    });
  }

  function matchPresetId(name: string, url: string, protocol?: string): string {
    const normalizedUrl = url.trim().replace(/\/+$/, "");
    for (const [id, preset] of Object.entries(PROVIDER_PRESETS)) {
      const presetUrl = preset.url.replace(/\/+$/, "");
      if (protocol && preset.protocol !== protocol) {
        continue;
      }
      if (
        normalizedUrl === presetUrl ||
        name.trim().toLowerCase() === preset.name.toLowerCase()
      ) {
        return id;
      }
    }
    return "custom";
  }

  function setModelSuggestions(models: string[]) {
    personalModelSuggestionsEl.innerHTML = "";
    for (const model of models) {
      const option = document.createElement("option");
      option.value = model;
      personalModelSuggestionsEl.appendChild(option);
    }
    if (personalModelSelectEl) {
      const current = personalModelEl.value.trim();
      personalModelSelectEl.innerHTML = "";
      for (const model of models) {
        const option = document.createElement("option");
        option.value = model;
        option.textContent = model;
        if (model === current) {
          option.selected = true;
        }
        personalModelSelectEl.appendChild(option);
      }
      if (models.length && !models.includes(current)) {
        personalModelSelectEl.value = models[0]!;
        personalModelEl.value = models[0]!;
      } else if (current && models.includes(current)) {
        personalModelSelectEl.value = current;
      }
    }
  }

  function syncModelFromSelect() {
    if (personalModelSelectEl && !personalModelSelectEl.hidden) {
      personalModelEl.value = personalModelSelectEl.value;
    }
  }

  function setPresetFormMode(isCustom: boolean, presetUrl?: string) {
    if (personalAdvancedEl) {
      // Presets: hide entirely — URL/protocol auto-routed in background.
      // Custom: keep for power users, but collapse by default.
      personalAdvancedEl.hidden = !isCustom;
      personalAdvancedEl.open = false;
    }
    if (personalModelSelectEl && personalModelEl) {
      personalModelSelectEl.hidden = isCustom;
      personalModelEl.hidden = !isCustom;
    }
    if (personalPresetUrlEl) {
      // Don't surface raw endpoint to beginners; presets already wire it.
      personalPresetUrlEl.hidden = true;
      personalPresetUrlEl.textContent = "";
      void presetUrl;
    }
    if (personalSaveEl) {
      personalSaveEl.hidden = !isCustom;
    }
  }

  function applyProviderPreset(presetId: string, { forceModel = true } = {}) {
    if (presetId === "custom" || !PROVIDER_PRESETS[presetId]) {
      personalPresetEl.value = "custom";
      syncPresetPicker("custom");
      personalNameRowEl.classList.remove("is-preset-locked");
      personalNameEl.readOnly = false;
      setModelSuggestions(modelsForCustomProtocol(personalProtocolEl.value));
      setPresetFormMode(true);
      return;
    }
    const preset = PROVIDER_PRESETS[presetId];
    personalPresetEl.value = presetId;
    syncPresetPicker(presetId);
    personalProtocolEl.value = preset.protocol;
    personalNameEl.value = preset.name;
    personalUrlEl.value = preset.url;
    setModelSuggestions(preset.models);
    if (forceModel || !personalModelEl.value.trim()) {
      personalModelEl.value = preset.models[0] ?? "";
      if (personalModelSelectEl) {
        personalModelSelectEl.value = preset.models[0] ?? "";
      }
    }
    personalNameRowEl.classList.add("is-preset-locked");
    personalNameEl.readOnly = true;
    setPresetFormMode(false, preset.url);
  }

  function focusAfterPreset(presetId: string) {
    if (presetId === "custom") {
      personalNameEl.focus();
    } else {
      personalKeyEl.focus();
    }
  }

  /** Keep preset chips in sync when protocol/url drift into custom. */
  function maybePromoteToCustomFromProtocol(): void {
    const presetId = personalPresetEl.value;
    if (presetId !== "custom" && PROVIDER_PRESETS[presetId]) {
      if (PROVIDER_PRESETS[presetId].protocol !== personalProtocolEl.value) {
        const keptName = personalNameEl.value;
        const keptUrl = personalUrlEl.value;
        const keptModel = personalModelEl.value;
        applyProviderPreset("custom");
        personalNameEl.value = keptName;
        personalUrlEl.value = keptUrl;
        personalModelEl.value = keptModel;
      }
    } else {
      setModelSuggestions(modelsForCustomProtocol(personalProtocolEl.value));
    }
  }

  function maybePromoteToCustomFromUrl(): void {
    const presetId = personalPresetEl.value;
    if (presetId !== "custom" && PROVIDER_PRESETS[presetId]) {
      const presetUrl = PROVIDER_PRESETS[presetId].url.replace(/\/+$/, "");
      const currentUrl = personalUrlEl.value.trim().replace(/\/+$/, "");
      if (currentUrl && currentUrl !== presetUrl) {
        const keptName = personalNameEl.value;
        const keptProtocol = personalProtocolEl.value;
        applyProviderPreset("custom");
        personalNameEl.value = keptName;
        personalProtocolEl.value = keptProtocol;
      }
    }
  }

  return {
    els: {
      personalPresetEl,
      personalPresetPickerEl,
      personalProtocolEl,
      personalNameEl,
      personalUrlEl,
      personalModelEl,
      personalModelSelectEl,
      personalKeyEl,
    },
    renderPresetPicker,
    refreshPresetGroupLabels,
    matchPresetId,
    setModelSuggestions,
    syncModelFromSelect,
    applyProviderPreset,
    focusAfterPreset,
    maybePromoteToCustomFromProtocol,
    maybePromoteToCustomFromUrl,
    modelsForPresetId,
    modelsForCustomProtocol,
  };
}
