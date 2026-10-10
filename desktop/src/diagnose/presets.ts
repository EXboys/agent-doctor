import {
  keyPlanFromUrl,
  minimaxRegionFromUrl,
  minimaxUrlForRegion,
  presetHasKeyPlan,
  urlForKeyPlan,
  type GlmKeyPlan,
  type MinimaxRegion,
  PROVIDER_PRESETS,
  PRESET_PICKER_GROUPS,
} from "../provider-presets";
import { shortModelLabel } from "../provider-models";
import { t } from "../i18n";
import {
  modelChipsEl,
  modelEl,
  presetChipsEl,
  protocolEl,
  providerNameEl,
  urlEl,
} from "./dom";

let activePresetId = "deepseek";
const glmPlanEl = document.querySelector<HTMLElement>("#diagnose-glm-plan");
const minimaxRegionEl = document.querySelector<HTMLElement>("#diagnose-minimax-region");

function markMinimaxRegion(region: MinimaxRegion) {
  minimaxRegionEl?.querySelectorAll<HTMLButtonElement>("[data-minimax-region]").forEach((chip) => {
    const selected = chip.dataset.minimaxRegion === region;
    chip.classList.toggle("is-active", selected);
    chip.setAttribute("aria-selected", selected ? "true" : "false");
  });
}

function syncMinimaxRegion() {
  if (!minimaxRegionEl) return;
  const show = activePresetId === "minimax";
  minimaxRegionEl.hidden = !show;
  if (!show) return;
  markMinimaxRegion(minimaxRegionFromUrl(urlEl.value) ?? "cn");
}

minimaxRegionEl?.addEventListener("click", (event) => {
  const chip = (event.target as HTMLElement | null)?.closest<HTMLButtonElement>(
    "[data-minimax-region]",
  );
  const region = chip?.dataset.minimaxRegion;
  if (region !== "cn" && region !== "intl") return;
  urlEl.value = minimaxUrlForRegion(region);
  markMinimaxRegion(region);
});

function markGlmPlan(plan: GlmKeyPlan) {
  glmPlanEl?.querySelectorAll<HTMLButtonElement>("[data-glm-plan]").forEach((chip) => {
    const selected = chip.dataset.glmPlan === plan;
    chip.classList.toggle("is-active", selected);
    chip.setAttribute("aria-selected", selected ? "true" : "false");
  });
}

function syncGlmPlan() {
  if (!glmPlanEl) return;
  const show = presetHasKeyPlan(activePresetId);
  glmPlanEl.hidden = !show;
  if (!show) return;
  markGlmPlan(keyPlanFromUrl(urlEl.value) ?? "payg");
}

glmPlanEl?.addEventListener("click", (event) => {
  const chip = (event.target as HTMLElement | null)?.closest<HTMLButtonElement>("[data-glm-plan]");
  const plan = chip?.dataset.glmPlan;
  if (plan !== "payg" && plan !== "coding") return;
  urlEl.value = urlForKeyPlan(activePresetId, plan, urlEl.value);
  markGlmPlan(plan);
});

function syncModelChipSelection(modelId: string): void {
  modelChipsEl.querySelectorAll<HTMLButtonElement>(".diagnose-model-chip").forEach((chip) => {
    const selected = chip.dataset.modelId === modelId;
    chip.classList.toggle("is-active", selected);
    chip.setAttribute("aria-selected", selected ? "true" : "false");
  });
}

function renderModelChips(presetId: string, models: string[], selected?: string): void {
  modelChipsEl.innerHTML = "";
  modelEl.innerHTML = "";
  const chosen = selected && models.includes(selected) ? selected : models[0] ?? "";

  for (const [index, model] of models.entries()) {
    const option = document.createElement("option");
    option.value = model;
    option.textContent = model;
    modelEl.appendChild(option);

    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = "diagnose-model-chip";
    chip.dataset.modelId = model;
    chip.setAttribute("role", "option");
    chip.title = model;
    const label = shortModelLabel(model, presetId);
    chip.innerHTML =
      index === 0
        ? `<strong>${escapeHtml(label)}</strong><span>${escapeHtml(t("diagnose.flow.modelRecommended"))}</span>`
        : `<strong>${escapeHtml(label)}</strong>`;
    chip.addEventListener("click", () => {
      modelEl.value = model;
      syncModelChipSelection(model);
    });
    modelChipsEl.appendChild(chip);
  }

  if (chosen) {
    modelEl.value = chosen;
    syncModelChipSelection(chosen);
  }
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export function applyPreset(presetId: string): void {
  const preset = PROVIDER_PRESETS[presetId];
  if (!preset) {
    return;
  }
  activePresetId = presetId;
  providerNameEl.value = preset.name;
  urlEl.value = preset.url;
  protocolEl.value = preset.protocol;
  renderModelChips(presetId, preset.models);
  presetChipsEl.querySelectorAll<HTMLButtonElement>(".provider-chip").forEach((chip) => {
    chip.classList.toggle("is-active", chip.dataset.presetId === presetId);
  });
  syncGlmPlan();
  syncMinimaxRegion();
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
  applyPreset(activePresetId);
}
