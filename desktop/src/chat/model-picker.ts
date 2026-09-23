import { invoke } from "@tauri-apps/api/core";
import type { AskRuntime } from "../ask-resources";
import { isPersonalEdition } from "../edition";
import { withErrorDetail } from "../friendly-error";
import { t } from "../i18n";
import { modelsForProviderUrl, providerChipForUrl } from "../provider-models";
import type {
  PersonalProviderListItem,
  PersonalProviderStatus,
  PersonalProvidersDocument,
} from "../types";
import { runtimeDisplayName } from "./runtime";

export type ModelPickerEls = {
  modelBtnEl: HTMLButtonElement;
  modelLabelEl: HTMLElement;
  modelMenuEl: HTMLElement;
  modelWrapEl: HTMLElement | null;
  composerBoxEl: HTMLElement;
  composerEl: HTMLElement;
};

export type ModelPickerDeps = ModelPickerEls & {
  isComposerLocked: () => boolean;
  selectedRuntime: () => AskRuntime;
  getWiredProvider: () => PersonalProviderListItem | null;
  setWiredProvider: (provider: PersonalProviderListItem | null) => void;
  getModelMenuOpen: () => boolean;
  setModelMenuOpen: (open: boolean) => void;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  updateContextMeter: () => void;
};

export type ModelPickerApi = ReturnType<typeof createModelPickerController>;

export function createModelPickerController(deps: ModelPickerDeps) {
  function closeModelMenu(): void {
    deps.setModelMenuOpen(false);
    deps.modelMenuEl.hidden = true;
    deps.modelBtnEl.classList.remove("is-open");
    deps.modelBtnEl.setAttribute("aria-expanded", "false");
    deps.modelWrapEl?.classList.remove("is-open");
    deps.composerBoxEl.classList.remove("is-model-open");
    deps.composerEl.classList.remove("is-model-open");
    deps.modelMenuEl.style.left = "";
    deps.modelMenuEl.style.right = "";
    deps.modelMenuEl.style.top = "";
    deps.modelMenuEl.style.bottom = "";
    deps.modelMenuEl.style.width = "";
    deps.modelMenuEl.style.minWidth = "";
    deps.modelMenuEl.style.position = "";
    deps.modelMenuEl.style.zIndex = "";
  }

  function positionModelMenu(): void {
    const rect = deps.modelBtnEl.getBoundingClientRect();
    const gap = 8;
    const minWidth = Math.max(rect.width, 200);
    const maxWidth = Math.min(280, window.innerWidth - 24);
    const width = Math.min(Math.max(minWidth, rect.width), maxWidth);
    let left = rect.left;
    if (left + width > window.innerWidth - 12) {
      left = Math.max(12, window.innerWidth - 12 - width);
    }
    // Anchor just above the model button (original interaction).
    deps.modelMenuEl.style.position = "fixed";
    deps.modelMenuEl.style.left = `${Math.round(left)}px`;
    deps.modelMenuEl.style.right = "auto";
    deps.modelMenuEl.style.width = `${Math.round(width)}px`;
    deps.modelMenuEl.style.minWidth = `${Math.round(width)}px`;
    deps.modelMenuEl.style.bottom = `${Math.round(window.innerHeight - rect.top + gap)}px`;
    deps.modelMenuEl.style.top = "auto";
    deps.modelMenuEl.style.zIndex = "120";
  }

  function renderModelPickerLabel(): void {
    const runtimeName = runtimeDisplayName(deps.selectedRuntime());
    const wiredProvider = deps.getWiredProvider();
    if (wiredProvider) {
      const chip = providerChipForUrl(wiredProvider.url, wiredProvider.name);
      const model = wiredProvider.model.trim() || "—";
      deps.modelLabelEl.textContent = `${chip} · ${model}`;
      deps.modelBtnEl.disabled = deps.isComposerLocked();
      deps.modelBtnEl.title = t("chat.modelPickHint");
      deps.modelBtnEl.setAttribute("aria-label", deps.modelLabelEl.textContent);
      return;
    }
    // Personal: keep clickable so the menu can say “go wire a provider”.
    // Team / locked: show runtime name only — model is chosen on the Agents page.
    if (isPersonalEdition()) {
      deps.modelLabelEl.textContent = t("chat.modelPickLabel");
      deps.modelBtnEl.disabled = deps.isComposerLocked();
      deps.modelBtnEl.title = t("chat.modelNeedProvider");
    } else {
      deps.modelLabelEl.textContent = runtimeName;
      deps.modelBtnEl.disabled = true;
      deps.modelBtnEl.title = `${runtimeName} — ${t("chat.runtimeLockedHint")}`;
    }
    deps.modelBtnEl.setAttribute("aria-label", deps.modelLabelEl.textContent);
  }

  function renderModelMenu(): void {
    deps.modelMenuEl.replaceChildren();
    const wiredProvider = deps.getWiredProvider();
    if (!wiredProvider) {
      const hint = document.createElement("div");
      hint.className = "chat-model-menu-hint";
      hint.textContent = t("chat.modelNeedProvider");
      deps.modelMenuEl.appendChild(hint);
      return;
    }
    const models = modelsForProviderUrl(wiredProvider.url, wiredProvider.model);
    if (models.length === 0) {
      const hint = document.createElement("div");
      hint.className = "chat-model-menu-hint";
      hint.textContent = wiredProvider.model || t("chat.modelNeedProvider");
      deps.modelMenuEl.appendChild(hint);
      return;
    }
    for (const model of models) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = `chat-model-option${model === wiredProvider.model ? " is-active" : ""}`;
      btn.role = "option";
      btn.textContent = model;
      btn.addEventListener("click", () => {
        void switchWiredModel(model);
      });
      deps.modelMenuEl.appendChild(btn);
    }
  }

  function openModelMenu(): void {
    if (deps.modelBtnEl.disabled || deps.isComposerLocked()) return;
    renderModelMenu();
    deps.setModelMenuOpen(true);
    deps.modelMenuEl.hidden = false;
    deps.modelBtnEl.classList.add("is-open");
    deps.modelBtnEl.setAttribute("aria-expanded", "true");
    deps.modelWrapEl?.classList.add("is-open");
    deps.composerBoxEl.classList.add("is-model-open");
    deps.composerEl.classList.add("is-model-open");
    positionModelMenu();
  }

  async function refreshWiredProvider(): Promise<void> {
    if (!isPersonalEdition()) {
      deps.setWiredProvider(null);
      renderModelPickerLabel();
      return;
    }
    try {
      const [status, doc] = await Promise.all([
        invoke<PersonalProviderStatus>("get_personal_provider_status_command"),
        invoke<PersonalProvidersDocument>("list_personal_providers_command"),
      ]);
      const active =
        doc.providers.find((p) => p.active) ||
        (status.active_id
          ? doc.providers.find((p) => p.id === status.active_id)
          : undefined) ||
        null;
      deps.setWiredProvider(
        active
          ? {
              ...active,
              model: active.model || status.model || "",
              name: active.name || status.active_name || active.name,
            }
          : status.configured && status.active_id
            ? {
                id: status.active_id,
                name: status.active_name || "Provider",
                url: status.gateway_url || "",
                model: status.model || "",
                protocol: status.protocol || "openai",
                api_key_hint: status.api_key_hint || "",
                active: true,
              }
            : null,
      );
    } catch (error) {
      console.warn("Ask: failed to load personal provider", error);
      deps.setWiredProvider(null);
    }
    renderModelPickerLabel();
    if (deps.getModelMenuOpen()) renderModelMenu();
  }

  async function switchWiredModel(model: string): Promise<void> {
    const wiredProvider = deps.getWiredProvider();
    if (!wiredProvider || deps.isComposerLocked()) return;
    const next = model.trim();
    if (!next || next === wiredProvider.model) {
      closeModelMenu();
      return;
    }
    closeModelMenu();
    // Ask reads the active provider model from store at send time — skip full
    // activate/mode-switch so the picker stays snappy.
    const previous = wiredProvider.model;
    deps.setWiredProvider({ ...wiredProvider, model: next });
    renderModelPickerLabel();
    deps.setStatus(t("chat.modelSwitching"), "muted");
    try {
      const current = deps.getWiredProvider()!;
      await invoke<PersonalProvidersDocument>("upsert_personal_provider_command", {
        id: current.id,
        name: current.name,
        url: current.url,
        key: "",
        model: next,
        protocol: current.protocol || "openai",
        activate: false,
      });
      deps.setStatus(t("chat.modelSwitched", { model: next }), "ok");
    } catch (error) {
      deps.setWiredProvider({ ...wiredProvider, model: previous });
      renderModelPickerLabel();
      deps.setStatus(withErrorDetail(t("chat.modelSwitchFailed"), error), "error");
    } finally {
      deps.modelBtnEl.disabled = deps.isComposerLocked() || !deps.getWiredProvider();
    }
  }

  function updateRuntimeLabel(): void {
    renderModelPickerLabel();
    deps.updateContextMeter();
  }

  return {
    closeModelMenu,
    positionModelMenu,
    renderModelPickerLabel,
    renderModelMenu,
    openModelMenu,
    refreshWiredProvider,
    switchWiredModel,
    updateRuntimeLabel,
  };
}
