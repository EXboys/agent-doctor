
import type { AskRuntime } from "../ask-resources";
import { isPersonalEdition } from "../edition";
import { t } from "../i18n";
import { modelsForProviderUrl, providerChipForUrl } from "../provider-models";
import type {
  PersonalProviderListItem,
} from "../types";
import { runtimeDisplayName } from "./runtime";
import type { ChatSession } from "./types";
import { getPersonalProviderStatus, listPersonalProviders } from "../ipc";

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
  getProviders: () => PersonalProviderListItem[];
  setProviders: (providers: PersonalProviderListItem[]) => void;
  activeSession: () => ChatSession | null;
  pinSessionModel: (provider: PersonalProviderListItem, model: string) => void;
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
    const minWidth = Math.max(rect.width, 220);
    const maxWidth = Math.min(340, window.innerWidth - 24);
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

  function chosenModel(): { provider: PersonalProviderListItem; model: string } | null {
    const wired = deps.getWiredProvider();
    const session = deps.activeSession();
    const pinned = session?.providerId
      ? deps.getProviders().find((item) => item.id === session.providerId)
      : undefined;
    const provider = pinned || wired;
    if (!provider) return null;
    const model = (pinned ? session?.model || provider.model : provider.model).trim();
    return { provider, model: model || "—" };
  }

  function renderModelPickerLabel(): void {
    const runtimeName = runtimeDisplayName(deps.selectedRuntime());
    const chosen = chosenModel();
    if (chosen) {
      deps.modelLabelEl.textContent = chosen.model;
      deps.modelBtnEl.disabled = deps.isComposerLocked();
      deps.modelBtnEl.title = t("chat.modelPickHint");
      deps.modelBtnEl.setAttribute("aria-label", chosen.model);
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
    const providers = deps.getProviders();
    const wired = deps.getWiredProvider();
    if (providers.length === 0 && !wired) {
      const hint = document.createElement("div");
      hint.className = "chat-model-menu-hint";
      hint.textContent = t("chat.modelNeedProvider");
      deps.modelMenuEl.appendChild(hint);
      return;
    }
    const chosen = chosenModel();
    const rows = providers.length > 0 ? providers : wired ? [wired] : [];
    for (const provider of rows) {
      const models = modelsForProviderUrl(provider.url, provider.model);
      if (models.length === 0) continue;
      const group = document.createElement("div");
      group.className = "chat-model-group";
      const label = document.createElement("div");
      label.className = "chat-model-group-label";
      label.textContent = provider.name.trim() || providerChipForUrl(provider.url, provider.name);
      if (provider.active) {
        const mark = document.createElement("span");
        mark.className = "chat-model-default";
        mark.textContent = t("chat.modelDefault");
        label.appendChild(mark);
      }
      group.appendChild(label);
      for (const model of models) {
        const btn = document.createElement("button");
        btn.type = "button";
        const active = chosen?.provider.id === provider.id && chosen.model === model;
        btn.className = `chat-model-option${active ? " is-active" : ""}`;
        btn.role = "option";
        btn.setAttribute("aria-selected", active ? "true" : "false");
        btn.textContent = model;
        btn.addEventListener("click", () => {
          selectSessionModel(provider, model);
        });
        group.appendChild(btn);
      }
      deps.modelMenuEl.appendChild(group);
    }
    if (!deps.modelMenuEl.childElementCount) {
      const hint = document.createElement("div");
      hint.className = "chat-model-menu-hint";
      hint.textContent = t("chat.modelNeedProvider");
      deps.modelMenuEl.appendChild(hint);
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
        getPersonalProviderStatus(),
        listPersonalProviders(),
      ]);
      deps.setProviders(doc.providers);
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
      deps.setProviders([]);
      deps.setWiredProvider(null);
    }
    renderModelPickerLabel();
    if (deps.getModelMenuOpen()) renderModelMenu();
  }

  function selectSessionModel(provider: PersonalProviderListItem, model: string): void {
    if (deps.isComposerLocked()) return;
    const next = model.trim();
    if (!next) {
      closeModelMenu();
      return;
    }
    const current = chosenModel();
    closeModelMenu();
    if (current?.provider.id === provider.id && current.model === next) return;
    deps.pinSessionModel(provider, next);
    renderModelPickerLabel();
    deps.updateContextMeter();
    const name = providerChipForUrl(provider.url, provider.name);
    deps.setStatus(t("chat.modelSwitched", { model: `${name} · ${next}` }), "ok");
    deps.modelBtnEl.disabled = deps.isComposerLocked() || !chosenModel();
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
    switchWiredModel: async (model: string) => {
      const wired = deps.getWiredProvider();
      if (!wired) return;
      selectSessionModel(wired, model);
    },
    updateRuntimeLabel,
  };
}
