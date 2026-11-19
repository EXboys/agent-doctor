import { open } from "@tauri-apps/plugin-dialog";

import { withErrorDetail } from "../friendly-error";
import { t } from "../i18n";
import type { WorkspaceDoc } from "../ask-resources";
import { parentDirectoryForPicker } from "./session-workspace";
import {
  connectLinkedHost,
  initWorkspace,
  linkedInitWorkspace,
  linkedListDir,
  listLinkedHostStatus,
  listRemoteHostRows,
} from "../ipc";
import type { LinkDirEntry, LinkHostStatus, RemoteHostRow } from "../types";

export type RegisterProjectDeps = {
  getWorkspaceDoc: () => WorkspaceDoc | null;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  loadAskResources: () => Promise<void>;
  renderSessionList: () => void;
  syncSessionWorkspaceUi: () => void;
  startNewSession: (workspaceName?: string | null) => void;
};

function remoteKey(hostId: string, name: string): string {
  return `${hostId}::${name}`;
}

async function pickComputer(
  hosts: RemoteHostRow[],
  statuses: LinkHostStatus[],
): Promise<"local" | string | null> {
  if (hosts.length === 0) return "local";

  return await new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.className = "chat-host-pick";
    overlay.innerHTML = `
      <div class="chat-host-pick-card" role="dialog" aria-modal="true">
        <p class="chat-host-pick-title"></p>
        <div class="chat-host-pick-list"></div>
        <button type="button" class="btn-ghost chat-host-pick-cancel"></button>
      </div>
    `;
    const title = overlay.querySelector(".chat-host-pick-title")!;
    title.textContent = t("remote.pickComputer");
    const list = overlay.querySelector(".chat-host-pick-list")!;
    const cancel = overlay.querySelector<HTMLButtonElement>(".chat-host-pick-cancel")!;
    cancel.textContent = t("resources.cancel");

    const finish = (value: "local" | string | null) => {
      overlay.remove();
      resolve(value);
    };

    const addBtn = (label: string, value: "local" | string, hint = "") => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "chat-host-pick-item";
      button.innerHTML = `<strong></strong><span></span>`;
      button.querySelector("strong")!.textContent = label;
      button.querySelector("span")!.textContent = hint;
      button.addEventListener("click", () => finish(value));
      list.append(button);
    };

    addBtn(t("remote.thisComputer"), "local");
    for (const host of hosts) {
      const status = statuses.find((s) => s.hostId === host.host_id);
      const name = host.display_name?.trim() || status?.displayName || host.host_id;
      const hint = status?.connected
        ? status.computerName || host.target
        : host.target;
      addBtn(name, host.host_id, hint);
    }
    cancel.addEventListener("click", () => finish(null));
    overlay.addEventListener("click", (event) => {
      if (event.target === overlay) finish(null);
    });
    document.body.append(overlay);
  });
}

async function ensureConnected(
  hostId: string,
  setStatus: RegisterProjectDeps["setStatus"],
): Promise<boolean> {
  const statuses = await listLinkedHostStatus().catch(() => [] as LinkHostStatus[]);
  if (statuses.some((s) => s.hostId === hostId && s.connected)) {
    return true;
  }
  setStatus(t("remote.connectClientRunning"), "muted");
  const report = await connectLinkedHost({ id: hostId });
  if (!report.ok) {
    setStatus(report.message || t("remote.connectClientFail"), "error");
    return false;
  }
  setStatus(t("remote.connectClientOk", { name: report.computerName || hostId }), "ok");
  return true;
}

async function browseRemoteFolder(
  hostId: string,
  setStatus: RegisterProjectDeps["setStatus"],
): Promise<string | null> {
  let current = "";
  while (true) {
    let entries: LinkDirEntry[];
    try {
      entries = await linkedListDir({ hostId, path: current || "~" });
    } catch (error) {
      setStatus(withErrorDetail(t("remote.browseRemoteFailed"), error), "error");
      return null;
    }
    const folders = entries.filter((e) => e.is_dir);
    const picked = await new Promise<string | "up" | "use" | null>((resolve) => {
      const overlay = document.createElement("div");
      overlay.className = "chat-host-pick";
      overlay.innerHTML = `
        <div class="chat-host-pick-card" role="dialog" aria-modal="true">
          <p class="chat-host-pick-title"></p>
          <p class="chat-host-pick-path"></p>
          <div class="chat-host-pick-list"></div>
          <div class="chat-host-pick-actions"></div>
        </div>
      `;
      overlay.querySelector(".chat-host-pick-title")!.textContent = t("remote.browseRemote");
      overlay.querySelector(".chat-host-pick-path")!.textContent = current || "~";
      const list = overlay.querySelector(".chat-host-pick-list")!;
      const actions = overlay.querySelector(".chat-host-pick-actions")!;

      const close = (value: string | "up" | "use" | null) => {
        overlay.remove();
        resolve(value);
      };

      if (folders.length === 0) {
        const empty = document.createElement("p");
        empty.className = "chat-host-pick-empty";
        empty.textContent = t("remote.browseRemoteEmpty");
        list.append(empty);
      }
      for (const folder of folders) {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "chat-host-pick-item";
        button.textContent = folder.name;
        button.addEventListener("click", () => close(folder.relative_path));
        list.append(button);
      }

      const up = document.createElement("button");
      up.type = "button";
      up.className = "btn-ghost";
      up.textContent = t("remote.openParent");
      up.addEventListener("click", () => close("up"));
      const use = document.createElement("button");
      use.type = "button";
      use.className = "btn-primary";
      use.textContent = t("remote.useThisFolder");
      use.addEventListener("click", () => close("use"));
      const cancel = document.createElement("button");
      cancel.type = "button";
      cancel.className = "btn-ghost";
      cancel.textContent = t("resources.cancel");
      cancel.addEventListener("click", () => close(null));
      actions.append(up, use, cancel);
      document.body.append(overlay);
    });

    if (picked == null) return null;
    if (picked === "use") {
      if (!current) {
        // Resolve ~ by listing once more for absolute home when user confirms root of browse.
        try {
          const homeEntries = await linkedListDir({ hostId, path: "~" });
          const sample = homeEntries[0]?.relative_path ?? "";
          if (sample.includes("/")) {
            return sample.replace(/\/[^/]+$/, "") || sample;
          }
        } catch {
          // fall through
        }
        setStatus(t("remote.browseRemoteEmpty"), "warn");
        continue;
      }
      return current;
    }
    if (picked === "up") {
      if (!current || current === "~") {
        current = "";
      } else {
        const trimmed = current.replace(/\/+$/, "");
        const idx = trimmed.lastIndexOf("/");
        current = idx > 0 ? trimmed.slice(0, idx) : "";
      }
      continue;
    }
    current = picked;
  }
}

/** Register a folder as a project (same flow as main window workspace tab). */
export async function registerProjectFromAsk(deps: RegisterProjectDeps): Promise<void> {
  const doc = deps.getWorkspaceDoc();
  const defaultName = doc?.active?.trim() || "agent-doctor";
  const defaultEntry = doc?.workspaces[defaultName] ?? doc?.workspaces["agent-doctor"];
  const defaultPath = defaultEntry?.path
    ? parentDirectoryForPicker(defaultEntry.path)
    : undefined;

  const hosts = await listRemoteHostRows().catch(() => [] as RemoteHostRow[]);
  const statuses = await listLinkedHostStatus().catch(() => [] as LinkHostStatus[]);
  const computer = await pickComputer(hosts, statuses);
  if (computer == null) return;

  if (computer !== "local") {
    const ok = await ensureConnected(computer, deps.setStatus);
    if (!ok) return;
    const path = await browseRemoteFolder(computer, deps.setStatus);
    if (!path?.trim()) return;
    deps.setStatus(t("chat.addProjectRegistering"), "muted");
    try {
      const report = await linkedInitWorkspace({
        hostId: computer,
        path: path.trim(),
        name: null,
      });
      const status = statuses.find((s) => s.hostId === computer);
      const hostLabel =
        status?.displayName ||
        hosts.find((h) => h.host_id === computer)?.display_name ||
        computer;
      const key = remoteKey(computer, report.name);
      // Stash into the in-memory doc shape used by the sidebar until next reload merges it.
      if (doc) {
        doc.workspaces[key] = {
          path: report.path,
          title: report.name,
          hostId: computer,
          hostLabel,
        };
      }
      await deps.loadAskResources();
      deps.renderSessionList();
      deps.syncSessionWorkspaceUi();
      deps.startNewSession(key);
      deps.setStatus(t("chat.addProjectDone", { name: report.name }), "ok");
    } catch (error) {
      deps.setStatus(withErrorDetail(t("chat.addProjectFailed"), error), "error");
    }
    return;
  }

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
