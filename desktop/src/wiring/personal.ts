import { invoke } from "@tauri-apps/api/core";
import { appState } from "../app-state";
import { isPersonalEdition } from "../edition";
import { escapeHtml } from "../format";
import { withErrorDetail } from "../friendly-error";
import { t } from "../i18n";
import { mergeLiveModels } from "../provider-models";
import { PROVIDER_PRESETS } from "../provider-presets";
import type {
  PersonalProviderListItem,
  PersonalProviderSetupReport,
  PersonalProvidersDocument,
  PersonalProviderStatus,
  PersonalProviderVerifyReport,
  ProviderProtocol,
} from "../types";
import type { PresetsApi } from "./presets";

const personalSectionEl = document.querySelector<HTMLElement>("#personal-section")!;
const personalListViewEl = document.querySelector<HTMLElement>("#personal-list-view")!;
const personalFormViewEl = document.querySelector<HTMLElement>("#personal-form-view")!;
const personalStatusEl = document.querySelector<HTMLElement>("#personal-status")!;
const personalConnectedEl = document.querySelector<HTMLElement>("#personal-connected")!;
const personalConnectedUrlEl = document.querySelector<HTMLElement>("#personal-connected-url");
const personalConnectedMetaEl = document.querySelector<HTMLElement>("#personal-connected-meta");
const personalAgentsFootnoteEl = document.querySelector<HTMLElement>("#personal-agents-footnote");
const personalListEl = document.querySelector<HTMLUListElement>("#personal-list")!;
const personalListHintEl = document.querySelector<HTMLElement>("#personal-list-hint")!;
const personalFormEl = document.querySelector<HTMLFormElement>("#personal-form")!;
const personalFormTitleEl = document.querySelector<HTMLElement>("#personal-form-title")!;
const personalIdEl = document.querySelector<HTMLInputElement>("#personal-id")!;
const personalNameEl = document.querySelector<HTMLInputElement>("#personal-name")!;
const personalUrlEl = document.querySelector<HTMLInputElement>("#personal-url")!;
const personalKeyEl = document.querySelector<HTMLInputElement>("#personal-key")!;
const personalModelEl = document.querySelector<HTMLInputElement>("#personal-model")!;
const personalModelSelectEl = document.querySelector<HTMLSelectElement>("#personal-model-select");
const personalProtocolEl = document.querySelector<HTMLSelectElement>("#personal-protocol")!;
const personalPresetEl = document.querySelector<HTMLSelectElement>("#personal-preset")!;
const personalAddEl = document.querySelector<HTMLButtonElement>("#personal-add")!;
const personalBackEl = document.querySelector<HTMLButtonElement>("#personal-back")!;
const personalVerifyEl = document.querySelector<HTMLButtonElement>("#personal-verify")!;
const personalSaveEl = document.querySelector<HTMLButtonElement>("#personal-save")!;
const personalApplyEl = document.querySelector<HTMLButtonElement>("#personal-apply")!;
const personalHintEl = document.querySelector<HTMLElement>("#personal-hint")!;

const RUNTIME_FOOTNOTE_LABELS: Record<string, string> = {
  hermes: "Hermes",
  openclaw: "OpenClaw",
  "claude-code": "Claude",
  codex: "Codex",
  "deepseek-harness": "DeepSeek",
};

export type PersonalDeps = {
  presets: PresetsApi;
  refresh: () => Promise<void>;
  loadModeStatus: () => Promise<void>;
};

export type PersonalApi = ReturnType<typeof createPersonalController>;

export function createPersonalController(deps: PersonalDeps) {
  const { presets } = deps;

  function renderPersonalProviderStatus(status: PersonalProviderStatus) {
    personalSectionEl.classList.toggle("is-configured", status.configured);
    // Status is shown on the active list row — keep this block hidden.
    personalConnectedEl.hidden = true;
    personalStatusEl.textContent = status.configured
      ? t("personal.configured")
      : t("personal.notConfigured");
    if (personalConnectedUrlEl) personalConnectedUrlEl.textContent = "";
    if (personalConnectedMetaEl) personalConnectedMetaEl.textContent = "";
  }

  function updatePersonalAgentsFootnote(): void {
    if (!personalAgentsFootnoteEl) return;
    const report = appState.lastReport;
    if (!report) {
      personalAgentsFootnoteEl.textContent = "";
      return;
    }
    const installed = report.runtimes
      .filter((runtime) => runtime.installed)
      .map((runtime) => RUNTIME_FOOTNOTE_LABELS[runtime.id] ?? runtime.display_name)
      .filter(Boolean);
    personalAgentsFootnoteEl.textContent =
      installed.length > 0
        ? t("personal.agentsOk", { list: installed.join("、") })
        : t("personal.agentsNone");
  }

  function renderPersonalProviderList(doc: PersonalProvidersDocument) {
    personalListEl.innerHTML = "";
    updatePersonalAgentsFootnote();
    if (doc.providers.length === 0) {
      const empty = document.createElement("li");
      empty.className = "provider-item provider-item-empty";
      empty.innerHTML = `
      <div class="provider-item-main">
        <p class="provider-item-kicker">${escapeHtml(t("personal.preset"))}</p>
        <p class="provider-item-title">${escapeHtml(t("personal.emptyTitle"))}</p>
        <p class="provider-item-desc">${escapeHtml(t("personal.emptyList"))}</p>
      </div>
    `;
      personalListEl.appendChild(empty);
      return;
    }

    const personalModeActive =
      isPersonalEdition() || appState.lastModeStatus?.mode === "personal";
    for (const item of doc.providers) {
      const routingActive = item.active && personalModeActive;
      const presetId = presets.matchPresetId(item.name, item.url, item.protocol);
      const brand =
        presetId !== "custom"
          ? PROVIDER_PRESETS[presetId]?.chip ?? PROVIDER_PRESETS[presetId]?.name ?? item.name
          : item.name.trim() || t("personal.presetCustom");
      const titleText = item.name.trim() || brand;

      const li = document.createElement("li");
      li.className = `provider-item${routingActive ? " is-active" : ""}`;
      li.dataset.providerId = item.id;

      const main = document.createElement("div");
      main.className = "provider-item-main";

      const kicker = document.createElement("p");
      kicker.className = "provider-item-kicker";
      kicker.textContent = brand;

      const title = document.createElement("p");
      title.className = "provider-item-title";
      title.textContent = titleText;
      if (routingActive) {
        const badge = document.createElement("span");
        badge.className = "provider-badge";
        badge.textContent = t("personal.activeBadge");
        title.appendChild(badge);
      }

      const meta = document.createElement("p");
      meta.className = "provider-item-meta";
      meta.textContent = item.model;

      const showUse = !item.active;
      const desc = document.createElement("p");
      desc.className = "provider-item-desc";
      desc.textContent = showUse ? t("personal.itemIdleDesc") : t("personal.itemActiveDesc");

      main.append(kicker, title, meta, desc);

      const actions = document.createElement("div");
      actions.className = "provider-item-actions";

      if (!item.active) {
        const activateBtn = document.createElement("button");
        activateBtn.type = "button";
        activateBtn.className = "btn-primary btn-compact";
        activateBtn.dataset.action = "activate-provider";
        activateBtn.dataset.providerId = item.id;
        activateBtn.textContent = t("personal.activate");
        actions.appendChild(activateBtn);
      }

      const editBtn = document.createElement("button");
      editBtn.type = "button";
      editBtn.className = "btn-secondary btn-compact";
      editBtn.dataset.action = "edit-provider";
      editBtn.dataset.providerId = item.id;
      editBtn.textContent = t("personal.edit");
      actions.appendChild(editBtn);

      const deleteBtn = document.createElement("button");
      deleteBtn.type = "button";
      deleteBtn.className = "btn-ghost btn-compact";
      deleteBtn.dataset.action = "delete-provider";
      deleteBtn.dataset.providerId = item.id;
      deleteBtn.textContent = t("personal.delete");
      actions.appendChild(deleteBtn);

      li.append(main, actions);
      personalListEl.appendChild(li);
    }
  }

  async function loadPersonalProviderStatus() {
    try {
      const [status, doc] = await Promise.all([
        invoke<PersonalProviderStatus>("get_personal_provider_status_command"),
        invoke<PersonalProvidersDocument>("list_personal_providers_command"),
      ]);
      appState.personalProvidersDoc = doc;
      renderPersonalProviderStatus(status);
      renderPersonalProviderList(doc);
    } catch (error) {
      personalStatusEl.textContent = withErrorDetail(t("personal.applyFailed"), error);
    }
  }

  function showPersonalListView() {
    personalListViewEl.hidden = false;
    personalFormViewEl.hidden = true;
  }

  function showPersonalFormView(mode: "add" | "edit") {
    personalListViewEl.hidden = true;
    personalFormViewEl.hidden = false;
    personalFormTitleEl.textContent =
      mode === "edit" ? t("personal.formEdit") : t("personal.formAdd");
    personalHintEl.textContent = "";
  }

  function resetPersonalForm() {
    personalIdEl.value = "";
    personalNameEl.value = "";
    personalUrlEl.value = "";
    personalKeyEl.value = "";
    personalModelEl.value = "";
    personalProtocolEl.value = "openai";
    personalKeyEl.placeholder = "sk-…";
    presets.applyProviderPreset("deepseek");
  }

  function fillPersonalForm(item: PersonalProviderListItem) {
    personalIdEl.value = item.id;
    personalNameEl.value = item.name;
    personalUrlEl.value = item.url;
    personalModelEl.value = item.model;
    personalProtocolEl.value = item.protocol === "anthropic" ? "anthropic" : "openai";
    personalKeyEl.value = "";
    personalKeyEl.placeholder = t("personal.keyKeepHint");
    const presetId = presets.matchPresetId(item.name, item.url, item.protocol);
    if (presetId === "custom") {
      presets.applyProviderPreset("custom");
      personalNameEl.value = item.name;
      personalUrlEl.value = item.url;
      personalProtocolEl.value = item.protocol === "anthropic" ? "anthropic" : "openai";
      personalModelEl.value = item.model;
    } else {
      presets.applyProviderPreset(presetId, { forceModel: false });
      personalNameEl.value = item.name;
      personalUrlEl.value = item.url;
      personalModelEl.value = item.model;
      if (personalModelSelectEl) {
        if (![...personalModelSelectEl.options].some((o) => o.value === item.model)) {
          const option = document.createElement("option");
          option.value = item.model;
          option.textContent = item.model;
          personalModelSelectEl.appendChild(option);
        }
        personalModelSelectEl.value = item.model;
      }
    }
  }

  function personalFormValues(requireKey: boolean): {
    id: string | null;
    name: string;
    url: string;
    key: string;
    model: string;
    protocol: ProviderProtocol;
  } | null {
    presets.syncModelFromSelect();
    const id = personalIdEl.value.trim() || null;
    const name = personalNameEl.value.trim();
    const url = personalUrlEl.value.trim();
    const key = personalKeyEl.value.trim();
    const model = personalModelEl.value.trim();
    const protocol: ProviderProtocol =
      personalProtocolEl.value === "anthropic" ? "anthropic" : "openai";
    if (!name || !url || !model || (requireKey && !key && !id)) {
      personalHintEl.textContent = t("personal.missingFields");
      return null;
    }
    return { id, name, url, key, model, protocol };
  }

  function setPersonalBusy(busy: boolean) {
    personalVerifyEl.disabled = busy;
    personalSaveEl.disabled = busy;
    personalApplyEl.disabled = busy;
    personalAddEl.disabled = busy;
    personalBackEl.disabled = busy;
  }

  async function verifyPersonalProvider() {
    const values = personalFormValues(true);
    if (!values) {
      return;
    }
    if (!values.key) {
      personalHintEl.textContent = t("personal.missingFields");
      return;
    }
    setPersonalBusy(true);
    personalHintEl.textContent = t("personal.verifying");
    try {
      const report = await invoke<PersonalProviderVerifyReport>("verify_personal_provider_command", {
        url: values.url,
        key: values.key,
        protocol: values.protocol,
      });
      if (report.ok) {
        const presetId = personalPresetEl.value;
        const base =
          presetId !== "custom" && PROVIDER_PRESETS[presetId]
            ? presets.modelsForPresetId(presetId)
            : presets.modelsForCustomProtocol(personalProtocolEl.value);
        if (report.models_sample.length > 0) {
          presets.setModelSuggestions(
            mergeLiveModels(base, report.models_sample, personalModelEl.value),
          );
        }
        const sample =
          report.models_sample.length > 0
            ? ` (${report.models_sample.slice(0, 3).join(", ")})`
            : "";
        personalHintEl.textContent = t("personal.verifyOk", {
          message: `${report.message}${sample}`,
        });
      } else {
        personalHintEl.textContent = t("personal.verifyFailed", { error: report.message });
      }
    } catch (error) {
      personalHintEl.textContent = withErrorDetail(t("personal.verifyFailed"), error);
    } finally {
      setPersonalBusy(false);
    }
  }

  async function upsertPersonalProvider(activate: boolean) {
    const editing = Boolean(personalIdEl.value.trim());
    const values = personalFormValues(!editing);
    if (!values) {
      return;
    }
    setPersonalBusy(true);
    personalHintEl.textContent = activate ? t("personal.applying") : t("personal.saving");
    try {
      if (activate) {
        // Save first without activate, then activate for a proper setup report.
        const doc = await invoke<PersonalProvidersDocument>("upsert_personal_provider_command", {
          id: values.id,
          name: values.name,
          url: values.url,
          key: values.key,
          model: values.model,
          protocol: values.protocol,
          activate: false,
        });
        const targetId =
          values.id ??
          doc.providers.find((p) => p.name === values.name && p.url === values.url)?.id ??
          doc.providers[doc.providers.length - 1]?.id;
        if (!targetId) {
          throw new Error("saved provider id missing");
        }
        const report = await invoke<PersonalProviderSetupReport>(
          "activate_personal_provider_command",
          { id: targetId },
        );
        personalKeyEl.value = "";
        await loadPersonalProviderStatus();
        await deps.loadModeStatus();
        await deps.refresh();
        personalListHintEl.textContent = t("personal.applyOk", {
          name: report.provider_name ?? values.name,
        });
        resetPersonalForm();
        showPersonalListView();
      } else {
        await invoke<PersonalProvidersDocument>("upsert_personal_provider_command", {
          id: values.id,
          name: values.name,
          url: values.url,
          key: values.key,
          model: values.model,
          protocol: values.protocol,
          activate: false,
        });
        personalKeyEl.value = "";
        await loadPersonalProviderStatus();
        await deps.loadModeStatus();
        personalListHintEl.textContent = t("personal.saveOk", { name: values.name });
        resetPersonalForm();
        showPersonalListView();
      }
    } catch (error) {
      personalHintEl.textContent = withErrorDetail(t("personal.applyFailed"), error);
    } finally {
      setPersonalBusy(false);
    }
  }

  async function activateProviderById(id: string) {
    setPersonalBusy(true);
    personalListHintEl.textContent = t("personal.applying");
    try {
      const report = await invoke<PersonalProviderSetupReport>("activate_personal_provider_command", {
        id,
      });
      await loadPersonalProviderStatus();
      await deps.loadModeStatus();
      await deps.refresh();
      personalListHintEl.textContent = t("personal.applyOk", {
        name: report.provider_name ?? id,
      });
    } catch (error) {
      personalListHintEl.textContent = withErrorDetail(t("personal.applyFailed"), error);
    } finally {
      setPersonalBusy(false);
    }
  }

  async function deleteProviderById(id: string) {
    setPersonalBusy(true);
    try {
      const doc = await invoke<PersonalProvidersDocument>("delete_personal_provider_command", {
        id,
      });
      appState.personalProvidersDoc = doc;
      await loadPersonalProviderStatus();
      personalListHintEl.textContent = t("personal.deleteOk");
      showPersonalListView();
    } catch (error) {
      personalListHintEl.textContent = withErrorDetail(t("personal.applyFailed"), error);
    } finally {
      setPersonalBusy(false);
    }
  }

  function bindEvents() {
    personalAddEl.addEventListener("click", () => {
      resetPersonalForm();
      showPersonalFormView("add");
    });

    personalBackEl.addEventListener("click", () => {
      resetPersonalForm();
      showPersonalListView();
    });

    personalModelSelectEl?.addEventListener("change", () => {
      presets.syncModelFromSelect();
    });

    personalFormEl.addEventListener("submit", (event) => {
      event.preventDefault();
      void upsertPersonalProvider(true);
    });

    personalSaveEl.addEventListener("click", () => {
      void upsertPersonalProvider(false);
    });

    personalVerifyEl.addEventListener("click", () => {
      void verifyPersonalProvider();
    });

    personalPresetEl.addEventListener("change", () => {
      presets.applyProviderPreset(personalPresetEl.value, { forceModel: true });
      presets.focusAfterPreset(personalPresetEl.value);
    });

    presets.els.personalPresetPickerEl?.addEventListener("click", (event) => {
      const target = event.target as HTMLElement | null;
      const chip = target?.closest<HTMLButtonElement>(".provider-chip");
      if (!chip?.dataset.presetId) return;
      const presetId = chip.dataset.presetId;
      presets.applyProviderPreset(presetId, { forceModel: true });
      presets.focusAfterPreset(presetId);
    });

    personalProtocolEl.addEventListener("change", () => {
      presets.maybePromoteToCustomFromProtocol();
    });

    personalUrlEl.addEventListener("change", () => {
      presets.maybePromoteToCustomFromUrl();
    });

    personalListEl.addEventListener("click", (event) => {
      const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-action]");
      const action = button?.dataset.action;
      const id = button?.dataset.providerId;
      if (!action || !id) {
        return;
      }
      if (action === "activate-provider") {
        void activateProviderById(id);
        return;
      }
      if (action === "edit-provider") {
        const item = appState.personalProvidersDoc?.providers.find((p) => p.id === id);
        if (item) {
          fillPersonalForm(item);
          showPersonalFormView("edit");
          personalHintEl.textContent = t("personal.keyKeepHint");
        }
        return;
      }
      if (action === "delete-provider") {
        void deleteProviderById(id);
      }
    });
  }

  return {
    loadPersonalProviderStatus,
    renderPersonalProviderList,
    showPersonalListView,
    bindEvents,
  };
}
