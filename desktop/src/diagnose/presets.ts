import { PROVIDER_PRESETS, PRESET_PICKER_GROUPS } from "../provider-presets";
import { modelEl, presetChipsEl, protocolEl, providerNameEl, urlEl } from "./dom";

export function applyPreset(presetId: string): void {
  const preset = PROVIDER_PRESETS[presetId];
  if (!preset) {
    return;
  }
  providerNameEl.value = preset.name;
  urlEl.value = preset.url;
  protocolEl.value = preset.protocol;
  modelEl.innerHTML = "";
  for (const model of preset.models) {
    const option = document.createElement("option");
    option.value = model;
    option.textContent = model;
    modelEl.appendChild(option);
  }
  if (preset.models[0]) {
    modelEl.value = preset.models[0];
  }
  presetChipsEl.querySelectorAll<HTMLButtonElement>(".provider-chip").forEach((chip) => {
    chip.classList.toggle("is-active", chip.dataset.presetId === presetId);
  });
}

export function renderPresetChips(): void {
  presetChipsEl.innerHTML = "";
  for (const group of PRESET_PICKER_GROUPS) {
    for (const id of group.ids) {
      const preset = PROVIDER_PRESETS[id];
      if (!preset) {
        continue;
      }
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "provider-chip";
      chip.dataset.presetId = id;
      chip.textContent = preset.chip ?? preset.name;
      chip.addEventListener("click", () => applyPreset(id));
      presetChipsEl.appendChild(chip);
    }
  }
  applyPreset("deepseek");
}
