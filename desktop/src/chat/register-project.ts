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

function hostPickIcon(kind: "local" | "remote"): SVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("width", "16");
  svg.setAttribute("height", "16");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("fill", "none");
  svg.setAttribute("aria-hidden", "true");
  const add = (tag: "path" | "rect", attrs: Record<string, string>) => {
    const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
    node.setAttribute("stroke", "currentColor");
    node.setAttribute("stroke-width", "2");
    node.setAttribute("stroke-linecap", "round");
    node.setAttribute("stroke-linejoin", "round");
    svg.append(node);
  };
  if (kind === "local") {
    add("rect", { x: "2", y: "3", width: "20", height: "14", rx: "2" });
    add("path", { d: "M8 21h8M12 17v4" });
  } else {
    add("rect", { x: "2", y: "2", width: "20", height: "8", rx: "2" });
    add("rect", { x: "2", y: "14", width: "20", height: "8", rx: "2" });
    add("path", { d: "M6 6h.01M6 18h.01" });
  }
  return svg;
}

function hostPickChevron(): SVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("width", "16");
  svg.setAttribute("height", "16");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("fill", "none");
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", "m9 18 6-6-6-6");
  path.setAttribute("stroke", "currentColor");
  path.setAttribute("stroke-width", "2");
  path.setAttribute("stroke-linecap", "round");
  path.setAttribute("stroke-linejoin", "round");
  svg.append(path);
  return svg;
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
      <div class="chat-host-pick-card" role="dialog" aria-modal="true" aria-labelledby="chat-host-pick-title">
        <p id="chat-host-pick-title" class="chat-host-pick-title"></p>
        <p class="chat-host-pick-hint"></p>
        <div class="chat-host-pick-list"></div>
        <button type="button" class="chat-host-pick-cancel"></button>
      </div>
    `;
    const title = overlay.querySelector(".chat-host-pick-title")!;
    title.textContent = t("remote.pickComputer");
    overlay.querySelector(".chat-host-pick-hint")!.textContent = t("remote.pickComputerHint");
    const list = overlay.querySelector(".chat-host-pick-list")!;
    const cancel = overlay.querySelector<HTMLButtonElement>(".chat-host-pick-cancel")!;
    cancel.textContent = t("resources.cancel");

    const finish = (value: "local" | string | null) => {
      document.removeEventListener("keydown", onKey, true);
      overlay.remove();
      resolve(value);
    };

    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      finish(null);
    };

    const addBtn = (label: string, value: "local" | string, hint = "", kind: "local" | "remote" = "remote") => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "chat-host-pick-item";
      const icon = document.createElement("span");
      icon.className = "chat-host-pick-icon";
      icon.append(hostPickIcon(kind));
      const copy = document.createElement("span");
      copy.className = "chat-host-pick-copy";
      const name = document.createElement("strong");
      name.textContent = label;
      copy.append(name);
      if (hint) {
        const meta = document.createElement("span");
        meta.className = "chat-host-pick-meta";
        meta.textContent = hint;
        copy.append(meta);
      }
      const chevron = document.createElement("span");
      chevron.className = "chat-host-pick-chevron";
      chevron.append(hostPickChevron());
      button.append(icon, copy, chevron);
      button.addEventListener("click", () => finish(value));
      list.append(button);
    };

    addBtn(t("remote.thisComputer"), "local", t("remote.thisComputerHint"), "local");
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
    document.addEventListener("keydown", onKey, true);
    document.body.append(overlay);
    list.querySelector<HTMLButtonElement>(".chat-host-pick-item")?.focus();
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
