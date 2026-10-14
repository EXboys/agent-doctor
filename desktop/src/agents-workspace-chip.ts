
import { t } from "./i18n";
import { escapeHtml } from "./format";
import { appState } from "./app-state";
import type { MainTabId, WorkspacesDocument } from "./types";
import { useWorkspace } from "./ipc";

export interface AgentsWorkspaceChipDeps {
  setMainTab: (tab: MainTabId) => void;
  loadWorkspaces: () => Promise<void>;
  addProject: () => void;
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
    const empty = names.length === 0;
    agentsWsQuickEl.disabled = false;
    agentsWsQuickEl.textContent = empty ? t("workspaces.register") : t("agents.wsQuickSwitch");
    agentsWsListEl.setAttribute("aria-label", t("nav.workspace"));
    if (empty) {
      agentsWsListEl.innerHTML = `
      <li class="ws-quick-item">
        <span class="ws-quick-item-main">
          <strong>${escapeHtml(t("workspaces.none"))}</strong>
          <span>${escapeHtml(t("workspaces.noneHint"))}</span>
        </span>
        <button type="button" class="btn-primary btn-compact" data-agents-add ${appState.workspaceBusy ? "disabled" : ""}>${escapeHtml(t("workspaces.register"))}</button>
      </li>
    `;
      return;
    }
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
    const names = Object.keys(doc.workspaces);
    const active = doc.active ?? appState.selectedWorkspaceName ?? null;
    agentsWsNameEl.textContent =
      active ?? (names.length === 0 ? t("workspaces.none") : t("workspaces.noActive"));
    agentsWsChipEl.classList.toggle("is-empty", !doc.active);
    agentsWsChipEl.classList.toggle("needs-attention", !doc.active);
    renderAgentsWorkspaceQuickList(doc);
  }

  async function applyAgentsWorkspaceQuick(name: string): Promise<void> {
    closeAgentsWsPicker();
    agentsWsNameEl.textContent = name;
    try {
      await useWorkspace({ name });
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
    const empty =
      !appState.lastWorkspaces || Object.keys(appState.lastWorkspaces.workspaces).length === 0;
    if (empty) {
      deps.addProject();
      return;
    }
    toggleAgentsWsPicker();
  });

  agentsWsManageEl.addEventListener("click", () => {
    closeAgentsWsPicker();
    deps.setMainTab("workspace");
  });

  agentsWsListEl.addEventListener("click", (event) => {
    const add = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-agents-add]");
    if (add) {
      closeAgentsWsPicker();
      deps.addProject();
      return;
    }
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
