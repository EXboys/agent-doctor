import { open } from "@tauri-apps/plugin-dialog";

import { withErrorDetail } from "../friendly-error";
import { t } from "../i18n";
import type { WorkspaceDoc } from "../ask-resources";
import { parentDirectoryForPicker } from "./session-workspace";
import { initWorkspace } from "../ipc";

export type RegisterProjectDeps = {
  getBusy: () => boolean;
  getWorkspaceDoc: () => WorkspaceDoc | null;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  loadAskResources: () => Promise<void>;
  renderSessionList: () => void;
  syncSessionWorkspaceUi: () => void;
  startNewSession: (workspaceName?: string | null) => void;
};

/** Register a folder as a project (same flow as main window workspace tab). */
export async function registerProjectFromAsk(deps: RegisterProjectDeps): Promise<void> {
  if (deps.getBusy()) {
    deps.setStatus(t("chat.addProjectWhileBusy"), "warn");
    return;
  }

  const doc = deps.getWorkspaceDoc();
  const defaultName = doc?.active?.trim() || "agent-doctor";
  const defaultEntry = doc?.workspaces[defaultName] ?? doc?.workspaces["agent-doctor"];
  const defaultPath = defaultEntry?.path
    ? parentDirectoryForPicker(defaultEntry.path)
    : undefined;

  let selected: string | string[] | null;
  try {
    selected = await open({
      directory: true,
      multiple: false,
      title: t("chat.addProjectPick"),
      defaultPath,
    });
  } catch (error) {
    deps.setStatus(withErrorDetail(t("chat.addProjectFailed"), error), "error");
    return;
  }

  const path = Array.isArray(selected) ? selected[0] : selected;
  if (!path?.trim()) return;

  deps.setStatus(t("chat.addProjectRegistering"), "muted");
  try {
    const report = await initWorkspace({
      path: path.trim(),
      name: null,
      // Use the folder the user picked — do not fold to an already-registered git root.
      gitRoot: false,
      // Add a row in the project list only; keep the main window default project unchanged.
      activate: false,
    });
    await deps.loadAskResources();
    deps.renderSessionList();
    deps.syncSessionWorkspaceUi();
    deps.startNewSession(report.name);
    deps.setStatus(t("chat.addProjectDone", { name: report.name }), "ok");
  } catch (error) {
    const detail = String(error ?? "");
    const existing = detail.match(/already registered as workspace '([^']+)'/i);
    if (existing?.[1]) {
      const name = existing[1];
      await deps.loadAskResources();
      deps.renderSessionList();
      deps.startNewSession(name);
      deps.setStatus(t("chat.addProjectAlready", { name }), "warn");
      return;
    }
    deps.setStatus(withErrorDetail(t("chat.addProjectFailed"), error), "error");
  }
}
