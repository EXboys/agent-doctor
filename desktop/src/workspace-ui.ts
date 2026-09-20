import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { t } from "./i18n";
import { escapeHtml } from "./format";
import { appState } from "./app-state";
import type {
  RemoteDoctorReport,
  RemoteHostsDocument,
  RemoteProjectRow,
  RemoteProbeCheck,
  WorkspaceCheck,
  WorkspaceDoctorReport,
  WorkspaceFixReport,
  WorkspacesDocument,
} from "./types";

export interface WorkspaceUiDeps {
  onWorkspacesChanged: (doc: WorkspacesDocument) => void;
}

let deps!: WorkspaceUiDeps;

const workspaceStatusEl = document.querySelector<HTMLElement>("#workspace-status")!;
const workspaceListEl = document.querySelector<HTMLUListElement>("#workspace-list")!;
const workspaceChecksEl = document.querySelector<HTMLUListElement>("#workspace-checks")!;
const workspaceHintEl = document.querySelector<HTMLElement>("#workspace-hint")!;
const workspaceRegisterEl = document.querySelector<HTMLButtonElement>("#workspace-register")!;
const remoteStatusEl = document.querySelector<HTMLElement>("#remote-status")!;
const remoteListEl = document.querySelector<HTMLUListElement>("#remote-list")!;
const remoteChecksEl = document.querySelector<HTMLUListElement>("#remote-checks")!;
const remoteHintEl = document.querySelector<HTMLElement>("#remote-hint")!;
const remoteRefreshEl = document.querySelector<HTMLButtonElement>("#remote-refresh")!;
const remoteBootstrapFormEl = document.querySelector<HTMLFormElement>("#remote-bootstrap-form")!;
const remoteHostFormEl = document.querySelector<HTMLFormElement>("#remote-host-form")!;
const remoteProjectFormEl = document.querySelector<HTMLFormElement>("#remote-project-form")!;
const remoteBootstrapIdEl = document.querySelector<HTMLInputElement>("#remote-bootstrap-id")!;
const remoteBootstrapHostnameEl = document.querySelector<HTMLInputElement>("#remote-bootstrap-hostname")!;
const remoteBootstrapUserEl = document.querySelector<HTMLInputElement>("#remote-bootstrap-user")!;
const remoteBootstrapPortEl = document.querySelector<HTMLInputElement>("#remote-bootstrap-port")!;
const remoteBootstrapPasswordEl = document.querySelector<HTMLInputElement>("#remote-bootstrap-password")!;
const remoteHostIdEl = document.querySelector<HTMLInputElement>("#remote-host-id")!;
const remoteSshHostEl = document.querySelector<HTMLInputElement>("#remote-ssh-host")!;
const remoteProjectHostEl = document.querySelector<HTMLSelectElement>("#remote-project-host")!;
const remoteProjectNameEl = document.querySelector<HTMLInputElement>("#remote-project-name")!;
const remoteProjectPathEl = document.querySelector<HTMLInputElement>("#remote-project-path")!;
let remoteBusy = false;

function renderWorkspaceManageList(doc: WorkspacesDocument): void {
  const names = Object.keys(doc.workspaces).sort();
  appState.selectedWorkspaceName = doc.active ?? appState.selectedWorkspaceName;

  if (names.length === 0) {
    workspaceListEl.innerHTML = `
      <li class="ws-manage-item">
        <div class="ws-manage-main">
          <strong>${escapeHtml(t("workspaces.none"))}</strong>
          <span>${escapeHtml(t("workspaces.noneHint"))}</span>
        </div>
      </li>
    `;
    return;
  }

  workspaceListEl.innerHTML = names
    .map((name) => {
      const entry = doc.workspaces[name];
      const path = entry?.path ?? "";
      const isActive = name === doc.active;
      const pathLabel = isActive
        ? `${path}${path ? " · " : ""}${t("agents.wsActiveBadge")}`
        : path;
      const right = isActive
        ? `
          <div class="ws-manage-right">
            <span class="badge ok">${escapeHtml(t("agents.wsActiveBadge"))}</span>
            <div class="ws-manage-actions">
              <button type="button" class="btn-ghost btn-compact" data-workspace-action="doctor" data-workspace="${escapeHtml(name)}" ${appState.workspaceBusy ? "disabled" : ""}>${escapeHtml(t("workspaces.doctor"))}</button>
              <button type="button" class="btn-ghost btn-compact" data-workspace-action="fix" data-workspace="${escapeHtml(name)}" ${appState.workspaceBusy ? "disabled" : ""}>${escapeHtml(t("workspaces.fix"))}</button>
            </div>
          </div>
        `
        : `
          <div class="ws-manage-right">
            <button type="button" class="btn-secondary btn-compact" data-workspace-action="use" data-workspace="${escapeHtml(name)}" ${appState.workspaceBusy ? "disabled" : ""}>${escapeHtml(t("agents.wsUse"))}</button>
          </div>
        `;
      return `
        <li class="ws-manage-item ${isActive ? "is-active" : ""}">
          <div class="ws-manage-main">
            <strong>${escapeHtml(name)}</strong>
            ${pathLabel ? `<span>${escapeHtml(pathLabel)}</span>` : ""}
          </div>
          ${right}
        </li>
      `;
    })
    .join("");
}

function renderWorkspaces(doc: WorkspacesDocument) {
  appState.lastWorkspaces = doc;
  deps.onWorkspacesChanged(doc);
  renderWorkspaceManageList(doc);

  if (Object.keys(doc.workspaces).length === 0) {
    workspaceStatusEl.textContent = t("workspaces.none");
    workspaceHintEl.textContent = t("workspaces.noneHint");
    return;
  }

  workspaceStatusEl.textContent = "";
  workspaceHintEl.textContent = "";
}

async function loadWorkspaces() {
  try {
    const doc = await invoke<WorkspacesDocument>("list_workspaces_command");
    renderWorkspaces(doc);
  } catch (error) {
    workspaceStatusEl.textContent = t("workspaces.failed");
    workspaceHintEl.textContent = String(error);
    workspaceListEl.innerHTML = "";
  }
}

async function applyWorkspace(name: string) {
  if (!name || appState.workspaceBusy) {
    return;
  }

  appState.workspaceBusy = true;
  appState.selectedWorkspaceName = name;
  renderWorkspaceManageList(appState.lastWorkspaces ?? { active: null, workspaces: {} });
  workspaceHintEl.textContent = t("workspaces.applying", { name });
  try {
    await invoke("use_workspace_command", { name });
    workspaceHintEl.textContent = t("workspaces.updated", { name });
    await loadWorkspaces();
  } catch (error) {
    workspaceHintEl.textContent = String(error);
  } finally {
    appState.workspaceBusy = false;
    if (appState.lastWorkspaces) {
      renderWorkspaceManageList(appState.lastWorkspaces);
    }
  }
}

async function registerWorkspace() {
  if (appState.workspaceBusy) {
    return;
  }

  let selected: string | string[] | null;
  try {
    selected = await open({
      directory: true,
      multiple: false,
      title: t("workspaces.registerPick"),
    });
  } catch (error) {
    workspaceHintEl.textContent = String(error);
    return;
  }

  const path = Array.isArray(selected) ? selected[0] : selected;
  if (!path) {
    return;
  }

  appState.workspaceBusy = true;
  workspaceRegisterEl.disabled = true;
  workspaceHintEl.textContent = t("workspaces.registering");
  try {
    const report = await invoke<{ name: string }>("init_workspace_command", {
      path,
      name: null,
      gitRoot: true,
    });
    workspaceHintEl.textContent = t("workspaces.registered", { name: report.name });
    await loadWorkspaces();
  } catch (error) {
    workspaceHintEl.textContent = String(error);
  } finally {
    appState.workspaceBusy = false;
    workspaceRegisterEl.disabled = false;
    if (appState.lastWorkspaces) {
      renderWorkspaceManageList(appState.lastWorkspaces);
    }
  }
}

async function doctorWorkspace() {
  if (appState.workspaceBusy) {
    return;
  }
  appState.workspaceBusy = true;
  if (appState.lastWorkspaces) {
    renderWorkspaceManageList(appState.lastWorkspaces);
  }
  workspaceHintEl.textContent = t("workspaces.doctorRunning");
  try {
    const report = await invoke<WorkspaceDoctorReport>("workspace_doctor_command");
    renderWorkspaceChecks(report);
  } catch (error) {
    workspaceChecksEl.hidden = true;
    workspaceChecksEl.innerHTML = "";
    workspaceHintEl.textContent = String(error);
  } finally {
    appState.workspaceBusy = false;
    if (appState.lastWorkspaces) {
      renderWorkspaceManageList(appState.lastWorkspaces);
    }
  }
}

async function fixWorkspace() {
  if (appState.workspaceBusy) {
    return;
  }
  appState.workspaceBusy = true;
  if (appState.lastWorkspaces) {
    renderWorkspaceManageList(appState.lastWorkspaces);
  }
  workspaceHintEl.textContent = t("workspaces.fixRunning");
  try {
    const report = await invoke<WorkspaceFixReport>("workspace_fix_command", {
      migrateClaudeMcp: false,
    });
    const applied = report.actions.filter((action) => action.applied).length;
    workspaceHintEl.textContent = t("workspaces.fixSummary", { count: String(applied) });
    appState.workspaceBusy = false;
    await doctorWorkspace();
  } catch (error) {
    workspaceHintEl.textContent = String(error);
    appState.workspaceBusy = false;
    if (appState.lastWorkspaces) {
      renderWorkspaceManageList(appState.lastWorkspaces);
    }
  }
}

function workspaceStatusLabel(status: WorkspaceCheck["status"]): string {
  switch (status) {
    case "pass":
      return t("repair.pass");
    case "warn":
      return t("repair.warn");
    case "fail":
      return t("repair.fail");
  }
}

function workspaceStatusClass(status: WorkspaceCheck["status"]): string {
  if (status === "pass") {
    return "pass";
  }
  if (status === "warn") {
    return "warn";
  }
  return "fail";
}

function renderWorkspaceChecks(report: WorkspaceDoctorReport) {
  if (!report.checks.length) {
    workspaceChecksEl.hidden = true;
    workspaceChecksEl.innerHTML = "";
    workspaceHintEl.textContent = t("workspaces.noActive");
    return;
  }

  workspaceChecksEl.hidden = false;
  workspaceChecksEl.innerHTML = report.checks
    .map(
      (check) => `
        <li class="repair-check">
          <span class="repair-check-status ${workspaceStatusClass(check.status)}">${escapeHtml(workspaceStatusLabel(check.status))}</span>
          <span class="repair-check-body">
            <strong>${escapeHtml(check.title)}</strong>
            <span>${escapeHtml(check.detail)}</span>
          </span>
        </li>
      `,
    )
    .join("");

  let pass = 0;
  let warn = 0;
  let fail = 0;
  for (const check of report.checks) {
    if (check.status === "pass") pass += 1;
    else if (check.status === "warn") warn += 1;
    else fail += 1;
  }
  workspaceHintEl.textContent = t("workspaces.doctorSummary", {
    pass: String(pass),
    warn: String(warn),
    fail: String(fail),
  });
}

let lastRemoteProjects: RemoteProjectRow[] = [];

function remoteHostLabel(id: string, host: RemoteHostsDocument["hosts"][string]): string {
  if (host.hostname) {
    const user = host.user || "root";
    const port = host.port ?? 22;
    return port === 22 ? `${user}@${host.hostname}` : `${user}@${host.hostname}:${port}`;
  }
  return host.ssh_config_host || id;
}

function fillRemoteHostSelect(doc: RemoteHostsDocument): void {
  const ids = Object.keys(doc.hosts).sort();
  const previous = remoteProjectHostEl.value;
  if (ids.length === 0) {
    remoteProjectHostEl.innerHTML = `<option value="">${escapeHtml(t("remote.noHosts"))}</option>`;
    remoteProjectHostEl.disabled = true;
    return;
  }
  remoteProjectHostEl.disabled = false;
  remoteProjectHostEl.innerHTML =
    `<option value="">${escapeHtml(t("remote.selectHost"))}</option>` +
    ids
      .map((id) => {
        const label = remoteHostLabel(id, doc.hosts[id]!);
        return `<option value="${escapeHtml(id)}">${escapeHtml(id)} (${escapeHtml(label)})</option>`;
      })
      .join("");
  if (previous && ids.includes(previous)) {
    remoteProjectHostEl.value = previous;
  }
}

function renderRemoteList(rows: RemoteProjectRow[]): void {
  if (rows.length === 0) {
    remoteListEl.innerHTML = `
      <li class="ws-manage-item">
        <div class="ws-manage-main">
          <strong>${escapeHtml(t("remote.none"))}</strong>
          <span>${escapeHtml(t("remote.noneHint"))}</span>
        </div>
      </li>
    `;
    return;
  }

  remoteListEl.innerHTML = rows
    .map((row) => {
      const target = `${row.host_id}/${row.project_id}`;
      const runtimes =
        row.runtimes.length === 0 ? "all" : row.runtimes.join(", ");
      return `
        <li class="ws-manage-item">
          <div class="ws-manage-main">
            <strong>${escapeHtml(target)}</strong>
            <span>${escapeHtml(row.path)} · ssh ${escapeHtml(row.ssh_config_host)} · ${escapeHtml(runtimes)}</span>
          </div>
          <div class="ws-manage-right">
            <div class="ws-manage-actions">
              <button type="button" class="btn-ghost btn-compact" data-remote-action="doctor" data-remote-target="${escapeHtml(target)}" ${remoteBusy ? "disabled" : ""}>${escapeHtml(t("remote.doctor"))}</button>
              <button type="button" class="btn-ghost btn-compact" data-remote-action="remove" data-remote-host="${escapeHtml(row.host_id)}" data-remote-project="${escapeHtml(row.project_id)}" ${remoteBusy ? "disabled" : ""}>${escapeHtml(t("remote.remove"))}</button>
            </div>
          </div>
        </li>
      `;
    })
    .join("");
}

function remoteCheckClass(status: RemoteProbeCheck["status"]): string {
  if (status === "pass") return "pass";
  if (status === "warn" || status === "not_checked") return "warn";
  if (status === "fail") return "fail";
  return "pass";
}

function remoteCheckLabel(status: RemoteProbeCheck["status"]): string {
  switch (status) {
    case "pass":
      return t("repair.pass");
    case "warn":
    case "not_checked":
      return t("repair.warn");
    case "fail":
      return t("repair.fail");
    default:
      return "—";
  }
}

function renderRemoteChecks(report: RemoteDoctorReport): void {
  const items: Array<{ title: string; message: string; status: RemoteProbeCheck["status"] }> = [];
  for (const check of report.checks) {
    if (check.status === "not_applicable") continue;
    items.push({
      title: check.title,
      message: check.details.length
        ? `${check.message} · ${check.details.join(" · ")}`
        : check.message,
      status: check.status,
    });
  }
  for (const runtime of report.runtimes) {
    for (const check of runtime.checks) {
      if (check.status === "not_applicable") continue;
      items.push({
        title: `${runtime.display_name}: ${check.title}`,
        message: check.details.length
          ? `${check.message} · ${check.details.join(" · ")}`
          : check.message,
        status: check.status,
      });
    }
  }

  if (items.length === 0) {
    remoteChecksEl.hidden = true;
    remoteChecksEl.innerHTML = "";
    return;
  }

  remoteChecksEl.hidden = false;
  remoteChecksEl.innerHTML = items
    .map(
      (check) => `
        <li class="repair-check">
          <span class="repair-check-status ${remoteCheckClass(check.status)}">${escapeHtml(remoteCheckLabel(check.status))}</span>
          <span class="repair-check-body">
            <strong>${escapeHtml(check.title)}</strong>
            <span>${escapeHtml(check.message)}</span>
          </span>
        </li>
      `,
    )
    .join("");

  let pass = 0;
  let warn = 0;
  let fail = 0;
  for (const check of items) {
    if (check.status === "pass") pass += 1;
    else if (check.status === "fail") fail += 1;
    else warn += 1;
  }
  let summary = t("remote.doctorSummary", {
    pass: String(pass),
    warn: String(warn),
    fail: String(fail),
  });
  if (report.report_path) {
    summary += ` · ${t("remote.reportSaved", { path: report.report_path })}`;
  }
  remoteHintEl.textContent = summary;
}

async function loadRemoteProjects(): Promise<void> {
  try {
    const [hosts, projects] = await Promise.all([
      invoke<RemoteHostsDocument>("list_remote_hosts_command"),
      invoke<RemoteProjectRow[]>("list_remote_projects_command"),
    ]);
    lastRemoteProjects = projects;
    fillRemoteHostSelect(hosts);
    renderRemoteList(projects);
    remoteStatusEl.textContent = "";
  } catch (error) {
    remoteStatusEl.textContent = t("remote.doctorFailed", { error: String(error) });
    remoteListEl.innerHTML = "";
  }
}

async function runRemoteDoctorUi(target: string): Promise<void> {
  if (!target || remoteBusy) return;
  remoteBusy = true;
  renderRemoteList(lastRemoteProjects);
  remoteHintEl.textContent = t("remote.doctorRunning");
  try {
    const report = await invoke<RemoteDoctorReport>("run_remote_doctor_command", {
      target,
      runtime: null,
    });
    renderRemoteChecks(report);
  } catch (error) {
    remoteChecksEl.hidden = true;
    remoteChecksEl.innerHTML = "";
    remoteHintEl.textContent = t("remote.doctorFailed", { error: String(error) });
  } finally {
    remoteBusy = false;
    renderRemoteList(lastRemoteProjects);
  }
}

async function removeRemoteProjectUi(host: string, project: string): Promise<void> {
  if (remoteBusy) return;
  remoteBusy = true;
  try {
    await invoke("remove_remote_project_command", { host, name: project });
    await loadRemoteProjects();
    remoteHintEl.textContent = "";
  } catch (error) {
    remoteHintEl.textContent = t("remote.removeFailed", { error: String(error) });
  } finally {
    remoteBusy = false;
    renderRemoteList(lastRemoteProjects);
  }
}


export interface WorkspaceUiApi {
  loadWorkspaces: () => Promise<void>;
  loadRemoteProjects: () => Promise<void>;
  renderWorkspaces: (doc: WorkspacesDocument) => void;
  renderWorkspaceChecks: (report: WorkspaceDoctorReport) => void;
  getLastWorkspaces: () => WorkspacesDocument | null;
  reloadLocale: () => void;
}

export function initWorkspaceUi(d: WorkspaceUiDeps): WorkspaceUiApi {
  deps = d;

  workspaceRegisterEl.addEventListener("click", () => {
    void registerWorkspace();
  });

  workspaceListEl.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-workspace-action]");
    if (!button) {
      return;
    }
    const name = button.dataset.workspace;
    const action = button.dataset.workspaceAction;
    if (!name || !action) {
      return;
    }
    if (action === "use") {
      void applyWorkspace(name);
      return;
    }
    if (action === "doctor") {
      void doctorWorkspace();
      return;
    }
    if (action === "fix") {
      void fixWorkspace();
    }
  });

  remoteRefreshEl.addEventListener("click", () => {
    void loadRemoteProjects();
  });

  remoteListEl.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-remote-action]");
    if (!button) {
      return;
    }
    const action = button.dataset.remoteAction;
    if (action === "doctor") {
      const target = button.dataset.remoteTarget;
      if (target) {
        void runRemoteDoctorUi(target);
      }
      return;
    }
    if (action === "remove") {
      const host = button.dataset.remoteHost;
      const project = button.dataset.remoteProject;
      if (host && project) {
        void removeRemoteProjectUi(host, project);
      }
    }
  });

  remoteBootstrapFormEl.addEventListener("submit", (event) => {
    event.preventDefault();
    const id = remoteBootstrapIdEl.value.trim();
    const hostname = remoteBootstrapHostnameEl.value.trim();
    const user = remoteBootstrapUserEl.value.trim();
    const port = Number(remoteBootstrapPortEl.value) || 22;
    const password = remoteBootstrapPasswordEl.value;
    if (!id || !hostname || !user || !password || remoteBusy) {
      return;
    }
    remoteBusy = true;
    remoteHintEl.textContent = t("remote.bootstrapRunning");
    void (async () => {
      try {
        await invoke("bootstrap_remote_host_command", {
          id,
          hostname,
          user,
          port,
          password,
        });
        remoteBootstrapIdEl.value = "";
        remoteBootstrapHostnameEl.value = "";
        remoteBootstrapUserEl.value = "";
        remoteBootstrapPortEl.value = "22";
        remoteBootstrapPasswordEl.value = "";
        remoteHintEl.textContent = t("remote.bootstrapSaved", { id });
        await loadRemoteProjects();
        const bootstrap = document.querySelector<HTMLDetailsElement>("#remote-bootstrap-host");
        if (bootstrap) {
          bootstrap.open = false;
        }
      } catch (error) {
        remoteHintEl.textContent = t("remote.bootstrapFailed", { error: String(error) });
      } finally {
        remoteBusy = false;
      }
    })();
  });

  remoteHostFormEl.addEventListener("submit", (event) => {
    event.preventDefault();
    const id = remoteHostIdEl.value.trim();
    const ssh = remoteSshHostEl.value.trim();
    if (!id || !ssh || remoteBusy) {
      return;
    }
    remoteBusy = true;
    void (async () => {
      try {
        await invoke("add_remote_host_command", { id, sshConfigHost: ssh });
        remoteHostIdEl.value = "";
        remoteSshHostEl.value = "";
        remoteHintEl.textContent = t("remote.hostSaved", { id });
        await loadRemoteProjects();
        const addHost = document.querySelector<HTMLDetailsElement>("#remote-add-host");
        if (addHost) {
          addHost.open = false;
        }
      } catch (error) {
        remoteHintEl.textContent = t("remote.hostFailed", { error: String(error) });
      } finally {
        remoteBusy = false;
      }
    })();
  });

  remoteProjectFormEl.addEventListener("submit", (event) => {
    event.preventDefault();
    const host = remoteProjectHostEl.value.trim();
    const name = remoteProjectNameEl.value.trim();
    const path = remoteProjectPathEl.value.trim();
    if (!host || !name || !path || remoteBusy) {
      return;
    }
    remoteBusy = true;
    void (async () => {
      try {
        await invoke("add_remote_project_command", {
          host,
          name,
          path,
          runtimes: [],
        });
        const target = `${host}/${name}`;
        remoteProjectNameEl.value = "";
        remoteProjectPathEl.value = "";
        remoteHintEl.textContent = t("remote.projectSaved", { target });
        await loadRemoteProjects();
        const addProject = document.querySelector<HTMLDetailsElement>("#remote-add-project");
        if (addProject) {
          addProject.open = false;
        }
      } catch (error) {
        remoteHintEl.textContent = t("remote.projectFailed", { error: String(error) });
      } finally {
        remoteBusy = false;
      }
    })();
  });

  return {
    loadWorkspaces,
    loadRemoteProjects,
    renderWorkspaces,
    renderWorkspaceChecks,
    getLastWorkspaces: () => appState.lastWorkspaces,
    reloadLocale: () => {
      if (appState.lastWorkspaces) {
        renderWorkspaces(appState.lastWorkspaces);
      }
    },
  };
}
