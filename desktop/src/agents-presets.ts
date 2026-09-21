import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { escapeHtml } from "./format";
import { appState } from "./app-state";
import type { ProfileEntry, ProfilesDocument, UseProfileReport } from "./types";

export interface AgentsPresetsDeps {
  refresh: () => Promise<void>;
}

/**
 * Hermes scene presets (work / local). Optional UI — workspace no longer hosts this.
 * When DOM is absent, returns no-op stubs so Agents panel still boots.
 */
export function createAgentsPresets(deps: AgentsPresetsDeps) {
  const presetStatusEl = document.querySelector<HTMLElement>("#preset-status");
  const presetApplyEl = document.querySelector<HTMLButtonElement>("#preset-apply");
  const presetHintEl = document.querySelector<HTMLElement>("#preset-hint");
  const presetPickerEl = document.querySelector<HTMLElement>("#preset-picker");
  const presetTriggerEl = document.querySelector<HTMLButtonElement>("#preset-trigger");
  const presetTriggerLabelEl = document.querySelector<HTMLElement>("#preset-trigger-label");
  const presetMenuEl = document.querySelector<HTMLElement>("#preset-menu");

  const enabled = Boolean(
    presetStatusEl &&
      presetApplyEl &&
      presetHintEl &&
      presetPickerEl &&
      presetTriggerEl &&
      presetTriggerLabelEl &&
      presetMenuEl,
  );

  if (!enabled) {
    return {
      closePresetMenu: () => {
        appState.presetMenuOpen = false;
      },
      loadProfiles: async () => {
        try {
          appState.lastProfiles = await invoke<ProfilesDocument>("list_profiles_command");
        } catch {
          appState.lastProfiles = null;
        }
      },
      renderProfiles: (doc: ProfilesDocument) => {
        appState.lastProfiles = doc;
      },
      applyPreset: async () => {},
      setLoadingStatus: (_message: string) => {},
      presetPickerEl: document.createElement("div"),
    };
  }

  const statusEl = presetStatusEl!;
  const applyEl = presetApplyEl!;
  const hintEl = presetHintEl!;
  const pickerEl = presetPickerEl!;
  const triggerEl = presetTriggerEl!;
  const triggerLabelEl = presetTriggerLabelEl!;
  const menuEl = presetMenuEl!;

  function setPresetTriggerLabel(name: string | null) {
    triggerLabelEl.textContent = name ?? t("presets.noActive");
  }

  function closePresetMenu() {
    appState.presetMenuOpen = false;
    menuEl.hidden = true;
    triggerEl.setAttribute("aria-expanded", "false");
    pickerEl.classList.remove("is-open");
  }

  function openPresetMenu() {
    if (triggerEl.disabled) {
      return;
    }
    appState.presetMenuOpen = true;
    menuEl.hidden = false;
    triggerEl.setAttribute("aria-expanded", "true");
    pickerEl.classList.add("is-open");
  }

  function togglePresetMenu() {
    if (appState.presetMenuOpen) {
      closePresetMenu();
    } else {
      openPresetMenu();
    }
  }

  function presetMeta(entry: ProfileEntry | undefined): string {
    const hermes = entry?.hermes;
    if (!hermes) {
      return "";
    }
    if (hermes.provider === "ollama") {
      return t("presets.localMeta", { model: hermes.model });
    }
    return `${hermes.provider} · ${hermes.model}`;
  }

  function sortPresetNames(names: string[]): string[] {
    return [...names].sort((left, right) => {
      if (left === "local") {
        return -1;
      }
      if (right === "local") {
        return 1;
      }
      return left.localeCompare(right);
    });
  }

  function renderPresetOptions(
    names: string[],
    active: string | null,
    profiles: Record<string, ProfileEntry>,
  ) {
    if (names.length === 0) {
      menuEl.innerHTML = "";
      appState.selectedPresetName = "";
      setPresetTriggerLabel(null);
      triggerEl.disabled = true;
      closePresetMenu();
      return;
    }

    appState.selectedPresetName =
      appState.selectedPresetName && names.includes(appState.selectedPresetName)
        ? appState.selectedPresetName
        : (active ?? names[0]);
    setPresetTriggerLabel(appState.selectedPresetName);
    triggerEl.disabled = false;

    menuEl.innerHTML = names
      .map((name) => {
        const activeOption = name === appState.selectedPresetName;
        const meta = presetMeta(profiles[name]);
        return `
        <button
          type="button"
          class="picker-option ${activeOption ? "is-active" : ""}"
          role="option"
          aria-selected="${activeOption}"
          data-preset="${escapeHtml(name)}"
        >
          <span class="picker-option-body">
            <span class="picker-option-label">${escapeHtml(name)}</span>
            ${meta ? `<span class="picker-option-meta">${escapeHtml(meta)}</span>` : ""}
          </span>
          <span class="picker-option-check" aria-hidden="true">✓</span>
        </button>
      `;
      })
      .join("");
  }

  function renderProfiles(doc: ProfilesDocument) {
    appState.lastProfiles = doc;
    const names = sortPresetNames(Object.keys(doc.profiles));
    statusEl.textContent = "";

    if (names.length === 0) {
      applyEl.disabled = true;
      hintEl.textContent = t("presets.noneHint");
      renderPresetOptions([], null, doc.profiles);
      return;
    }

    renderPresetOptions(names, doc.active, doc.profiles);
    applyEl.disabled = false;
    hintEl.textContent = doc.active
      ? t("presets.active", { name: doc.active })
      : t("presets.noActive");
  }

  async function loadProfiles() {
    try {
      const doc = await invoke<ProfilesDocument>("list_profiles_command");
      renderProfiles(doc);
    } catch (error) {
      statusEl.textContent = t("presets.failed");
      hintEl.textContent = String(error);
      applyEl.disabled = true;
    }
  }

  async function applyPreset() {
    const name = appState.selectedPresetName;
    if (!name) {
      return;
    }

    closePresetMenu();

    applyEl.disabled = true;
    hintEl.textContent = t("presets.applying", { name });
    try {
      const report = await invoke<UseProfileReport>("use_profile_command", { name });
      const applied = report.applied.map((item) => item.runtime_id).join(", ");
      hintEl.textContent = applied
        ? t("presets.updated", { list: applied })
        : report.skipped.join("; ");
      await loadProfiles();
      await deps.refresh();
    } catch (error) {
      hintEl.textContent = String(error);
    } finally {
      applyEl.disabled = false;
    }
  }

  function setLoadingStatus(message: string) {
    statusEl.textContent = message;
  }

  applyEl.addEventListener("click", () => {
    void applyPreset();
  });

  triggerEl.addEventListener("click", () => {
    togglePresetMenu();
  });

  menuEl.addEventListener("click", (event) => {
    const option = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-preset]");
    const name = option?.dataset.preset;
    if (!name || !appState.lastProfiles) {
      return;
    }
    appState.selectedPresetName = name;
    renderPresetOptions(
      sortPresetNames(Object.keys(appState.lastProfiles.profiles)),
      appState.lastProfiles.active,
      appState.lastProfiles.profiles,
    );
    closePresetMenu();
  });

  return {
    closePresetMenu,
    loadProfiles,
    renderProfiles,
    applyPreset,
    setLoadingStatus,
    presetPickerEl: pickerEl,
  };
}

export type AgentsPresetsApi = ReturnType<typeof createAgentsPresets>;
