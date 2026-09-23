import { invoke } from "@tauri-apps/api/core";
import { currentMonitor, getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { AskResourcesController, type WorkspaceDoc } from "../ask-resources";
import { t } from "../i18n";
import { shortCwdLabel } from "./format";

/** Matches `.chat-shell.is-resources-open` grid first column. */
const RESOURCES_PANEL_WIDTH_PX = 320;
const ASK_WINDOW_MIN_WIDTH_PX = 720;

export type ShellUiEls = {
  shellEl: HTMLElement;
  cwdEl: HTMLElement;
  workspaceSelectEl: HTMLSelectElement;
  workspaceActivateEl: HTMLButtonElement;
  workspaceHintEl: HTMLElement;
};

export type ShellUiDeps = ShellUiEls & {
  askResources: AskResourcesController;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  autoResizePrompt: () => void;
  setWorkspaceCwd: (cwd: string | null) => void;
  getWorkspaceDoc: () => WorkspaceDoc | null;
  setWorkspaceDoc: (doc: WorkspaceDoc | null) => void;
  setDisplayedCwd: (cwd: string) => void;
};

export type ShellUiApi = ReturnType<typeof createShellUiController>;

export function createShellUiController(deps: ShellUiDeps) {
  /** Remember width before opening Skills/MCP so close restores, not blindly -320. */
  let askWidthBeforeResources: number | null = null;

  async function adaptAskWindowForResources(open: boolean): Promise<void> {
    try {
      const win = getCurrentWindow();
      const size = await win.innerSize();
      const factor = await win.scaleFactor();
      const logicalW = size.width / factor;
      const logicalH = size.height / factor;

      let nextW: number;
      if (open) {
        askWidthBeforeResources = logicalW;
        nextW = logicalW + RESOURCES_PANEL_WIDTH_PX;
        const monitor = await currentMonitor();
        if (monitor) {
          const maxW = monitor.size.width / monitor.scaleFactor - 24;
          nextW = Math.min(nextW, maxW);
        }
      } else {
        nextW = askWidthBeforeResources ?? logicalW - RESOURCES_PANEL_WIDTH_PX;
        askWidthBeforeResources = null;
      }
      nextW = Math.max(ASK_WINDOW_MIN_WIDTH_PX, nextW);
      if (Math.abs(nextW - logicalW) < 1) return;
      await win.setSize(new LogicalSize(nextW, logicalH));
    } catch {
      // Browser / non-Tauri preview — CSS adaptation still applies.
    }
  }

  function toggleResourcesPanel(): void {
    const willOpen = !deps.shellEl.classList.contains("is-resources-open");
    deps.askResources.toggleResourcesPanel();
    void adaptAskWindowForResources(willOpen).finally(() => {
      // After layout width settles, re-measure the prompt (placeholder may wrap).
      requestAnimationFrame(() => deps.autoResizePrompt());
    });
  }

  function syncWorkspaceActivateButton(doc: WorkspaceDoc | null = deps.getWorkspaceDoc()): void {
    const selected = deps.workspaceSelectEl.value.trim();
    const isCurrent = Boolean(doc?.active && selected && doc.active === selected);
    deps.workspaceActivateEl.disabled =
      !selected || isCurrent || !doc || Object.keys(doc.workspaces).length === 0;
    deps.workspaceActivateEl.classList.toggle("is-current", isCurrent);
    deps.workspaceActivateEl.textContent = isCurrent
      ? t("ask.workspaceCurrent")
      : t("ask.workspaceActivate");
  }

  function renderWorkspaceSwitcher(doc: WorkspaceDoc): void {
    const names = Object.keys(doc.workspaces).sort();
    deps.workspaceSelectEl.innerHTML = "";
    if (names.length === 0) {
      const opt = document.createElement("option");
      opt.value = "";
      opt.textContent = t("ask.workspaceEmpty");
      deps.workspaceSelectEl.appendChild(opt);
      deps.workspaceSelectEl.disabled = true;
      syncWorkspaceActivateButton(doc);
      return;
    }

    deps.workspaceSelectEl.disabled = false;
    for (const name of names) {
      const opt = document.createElement("option");
      opt.value = name;
      const path = doc.workspaces[name]?.path ?? "";
      opt.textContent = path ? `${name} · ${shortCwdLabel(path)}` : name;
      if (name === doc.active) {
        opt.selected = true;
      }
      deps.workspaceSelectEl.appendChild(opt);
    }
    if (!doc.active && names[0]) {
      deps.workspaceSelectEl.value = names[0];
    }
    syncWorkspaceActivateButton(doc);
  }

  async function loadAskResources(): Promise<void> {
    const result = await deps.askResources.loadAskResources({
      setStatus: deps.setStatus,
      renderWorkspaceSwitcher,
      cwdEl: deps.cwdEl,
      workspaceHintEl: deps.workspaceHintEl,
      setDisplayedCwd: deps.setDisplayedCwd,
    });
    deps.setWorkspaceCwd(result.workspaceCwd);
    deps.setWorkspaceDoc(result.workspaceDoc);
  }

  async function activateSelectedWorkspace(): Promise<void> {
    const name = deps.workspaceSelectEl.value.trim();
    if (!name) {
      deps.setStatus(t("ask.workspaceEmpty"), "warn");
      return;
    }
    deps.workspaceActivateEl.disabled = true;
    deps.setStatus(t("ask.workspaceActivating", { name }), "muted");
    try {
      await invoke("use_workspace_command", { name });
      deps.setStatus(t("ask.workspaceActivated", { name }), "ok");
      await loadAskResources();
    } catch (error) {
      deps.setStatus(String(error), "error");
      syncWorkspaceActivateButton();
    }
  }

  async function openMainWorkspace(): Promise<void> {
    try {
      await invoke("focus_main_tab_command", { tab: "workspace" });
    } catch (error) {
      deps.setStatus(String(error), "error");
    }
  }

  async function openMainResources(): Promise<void> {
    try {
      await invoke("open_resources_window_command", { section: "catalog" });
    } catch (error) {
      deps.setStatus(String(error), "error");
    }
  }

  return {
    toggleResourcesPanel,
    syncWorkspaceActivateButton,
    renderWorkspaceSwitcher,
    loadAskResources,
    activateSelectedWorkspace,
    openMainWorkspace,
    openMainResources,
    updateResourcesSummary: () => deps.askResources.updateResourcesSummary(),
  };
}
