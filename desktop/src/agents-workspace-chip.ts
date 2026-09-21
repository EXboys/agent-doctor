import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { escapeHtml } from "./format";
import { appState } from "./app-state";
import type { MainTabId, WorkspacesDocument } from "./types";

export interface AgentsWorkspaceChipDeps {
  setMainTab: (tab: MainTabId) => void;
  loadWorkspaces: () => Promise<void>;
  refreshRuntimeCardActions: (card: HTMLElement, runtimeId: string) => void;
  getActiveRuntimeId: () => string | null;
  getRuntimesEl: () => HTMLElement;
}

export function createAgentsWorkspaceChip(deps: AgentsWorkspaceChipDeps) {
  const agentsWsChipEl = document.querySelector<HTMLElement>("#agents-ws-chip")!;
  const agentsWsNameEl = document.querySelector<HTMLElement>("#agents-ws-name")!;
  const agentsWsQuickEl = document.querySelector<HTMLButtonElement>("#agents-ws-quick")!;
  const agentsWsManageEl = document.querySelector<HTMLButtonElement>("#agents-ws-manage")!;
  const agentsWsPickerEl = document.querySelector<HTMLElement>("#agents-ws-picker")!;
  const agentsWsListEl = document.querySelector<HTMLUListElement>("#agents-ws-list")!;

  function closeAgentsWsPicker(): void {
    appState.agentsWsPickerOpen = false;
    agentsWsPickerEl.hidden = true;
  }

  function renderAgentsWorkspaceQuickList(doc: WorkspacesDocument): void {
    const names = Object.keys(doc.workspaces).sort();
    if (names.length === 0) {
      agentsWsListEl.innerHTML = `
      <li class="ws-quick-item">
        <span class="ws-quick-item-main">
          <strong>${escapeHtml(t("workspaces.none"))}</strong>
        </span>
      </li>
    `;
      agentsWsQuickEl.disabled = true;
      return;
    }

    agentsWsQuickEl.disabled = false;
    const active = doc.active ?? appState.selectedWorkspaceName;
    agentsWsListEl.innerHTML = names
      .map((name) => {
        const entry = doc.workspaces[name];
        const isActive = name === active;
        const meta = entry?.path ?? "";
        return `
        <li class="ws-quick-item ${isActive ? "is-active" : ""}">
          <span class="ws-quick-item-main">
            <strong>${escapeHtml(name)}</strong>
            ${meta ? `<span>${escapeHtml(meta)}</span>` : ""}
          </span>
          ${
            isActive
              ? `<span class="badge ok">${escapeHtml(t("agents.wsActiveBadge"))}</span>`
              : `<button type="button" class="btn-secondary btn-compact" data-agents-ws="${escapeHtml(name)}">${escapeHtml(t("agents.wsUse"))}</button>`
          }
        </li>
      `;
      })
      .join("");
  }

  function toggleAgentsWsPicker(): void {
    if (appState.agentsWsPickerOpen) {
      closeAgentsWsPicker();
      return;
    }
    if (appState.lastWorkspaces) {
      renderAgentsWorkspaceQuickList(appState.lastWorkspaces);
    }
    appState.agentsWsPickerOpen = true;
    agentsWsPickerEl.hidden = false;
  }

  function updateAgentsWorkspaceChip(doc: WorkspacesDocument): void {
    const active = doc.active ?? appState.selectedWorkspaceName ?? null;
    agentsWsNameEl.textContent = active ?? t("workspaces.noActive");
    agentsWsChipEl.classList.toggle("is-empty", !doc.active);
    agentsWsChipEl.classList.toggle("needs-attention", !doc.active);
    renderAgentsWorkspaceQuickList(doc);
  }

  async function applyAgentsWorkspaceQuick(name: string): Promise<void> {
    closeAgentsWsPicker();
    agentsWsNameEl.textContent = name;
    try {
      await invoke("use_workspace_command", { name });
      await deps.loadWorkspaces();
      // Refresh CTA now that workspace is active.
      const activeRuntimeId = deps.getActiveRuntimeId();
      if (activeRuntimeId && appState.lastReport) {
        const card = deps
          .getRuntimesEl()
          .querySelector<HTMLElement>(`[data-runtime="${activeRuntimeId}"]`);
        if (card) {
          deps.refreshRuntimeCardActions(card, activeRuntimeId);
        }
      }
    } catch {
      await deps.loadWorkspaces();
    }
  }

  agentsWsQuickEl.addEventListener("click", () => {
    toggleAgentsWsPicker();
  });

  agentsWsManageEl.addEventListener("click", () => {
    closeAgentsWsPicker();
    deps.setMainTab("workspace");
  });

  agentsWsListEl.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-agents-ws]");
    const name = button?.dataset.agentsWs;
    if (name) {
      void applyAgentsWorkspaceQuick(name);
    }
  });

  return {
    agentsWsChipEl,
    agentsWsPickerEl,
    closeAgentsWsPicker,
    toggleAgentsWsPicker,
    updateAgentsWorkspaceChip,
    renderAgentsWorkspaceQuickList,
    applyAgentsWorkspaceQuick,
  };
}

export type AgentsWorkspaceChipApi = ReturnType<typeof createAgentsWorkspaceChip>;
