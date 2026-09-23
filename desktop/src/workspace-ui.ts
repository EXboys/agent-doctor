import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { t } from "./i18n";
import { withErrorDetail } from "./friendly-error";
import { escapeHtml } from "./format";
import { appState } from "./app-state";
import type {
  RemoteDoctorReport,
  RemoteHostProbeReport,
  RemoteHostProbeStatus,
  RemoteHostRow,
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
const workspaceChecksPanelEl = document.querySelector<HTMLElement>("#workspace-checks-panel")!;
const workspaceChecksEl = document.querySelector<HTMLUListElement>("#workspace-checks")!;
const workspaceChecksSummaryEl = document.querySelector<HTMLElement>("#workspace-checks-summary")!;
const workspaceChecksToggleEl = document.querySelector<HTMLButtonElement>("#workspace-checks-toggle")!;
const workspaceHintEl = document.querySelector<HTMLElement>("#workspace-hint")!;
const workspaceRegisterEl = document.querySelector<HTMLButtonElement>("#workspace-register")!;
const remoteStatusEl = document.querySelector<HTMLElement>("#remote-status")!;
const remoteHostListEl = document.querySelector<HTMLUListElement>("#remote-host-list")!;
const remoteListEl = document.querySelector<HTMLUListElement>("#remote-list")!;
const remoteChecksEl = document.querySelector<HTMLUListElement>("#remote-checks")!;
const remoteHintEl = document.querySelector<HTMLElement>("#remote-hint")!;
const remoteRefreshEl = document.querySelector<HTMLButtonElement>("#remote-refresh")!;
const remoteBootstrapFormEl = document.querySelector<HTMLFormElement>("#remote-bootstrap-form")!;
const remoteProjectFormEl = document.querySelector<HTMLFormElement>("#remote-project-form")!;
const remoteBootstrapIdEl = document.querySelector<HTMLInputElement>("#remote-bootstrap-id")!;
const remoteBootstrapHostnameEl = document.querySelector<HTMLInputElement>("#remote-bootstrap-hostname")!;
const remoteBootstrapUserEl = document.querySelector<HTMLInputElement>("#remote-bootstrap-user")!;
const remoteBootstrapPortEl = document.querySelector<HTMLInputElement>("#remote-bootstrap-port")!;
const remoteBootstrapPasswordEl = document.querySelector<HTMLInputElement>("#remote-bootstrap-password")!;
const remoteBootstrapPathEl = document.querySelector<HTMLInputElement>("#remote-bootstrap-path");
const remoteProjectHostEl = document.querySelector<HTMLSelectElement>("#remote-project-host")!;
const remoteProjectNameEl = document.querySelector<HTMLInputElement>("#remote-project-name")!;
const remoteProjectPathEl = document.querySelector<HTMLInputElement>("#remote-project-path")!;
let remoteBusy = false;

function toRemoteId(raw: string): string {
  const cleaned = raw
    .trim()
    .replace(/[^a-zA-Z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 32);
  return cleaned || "vps";
}

function projectNameFromPath(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const base = trimmed.split("/").filter(Boolean).pop() ?? "app";
  return toRemoteId(base);
}

function openRemoteAddPath(hostId?: string): void {
  const addProject = document.querySelector<HTMLDetailsElement>("#remote-add-project");
  if (!addProject) return;
  addProject.hidden = false;
  addProject.open = true;
  if (hostId) {
    remoteProjectHostEl.value = hostId;
  }
  remoteProjectPathEl.focus();
  addProject.scrollIntoView({ behavior: "smooth", block: "nearest" });
}

function renderWorkspaceManageList(doc: WorkspacesDocument): void {
  const names = Object.keys(doc.workspaces).sort();
  appState.selectedWorkspaceName = doc.active ?? appState.selectedWorkspaceName;

  if (names.length === 0) {
    workspaceListEl.innerHTML = `
      <li class="ws-manage-item">
        <div class="ws-manage-main">
          <strong>${escapeHtml(t("workspaces.none"))}</strong>
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
            ${path ? `<span>${escapeHtml(path)}</span>` : ""}
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
    workspaceHintEl.textContent = withErrorDetail(t("workspaces.actionFailed"), error);
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
    workspaceHintEl.textContent = withErrorDetail(t("workspaces.actionFailed"), error);
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
    workspaceHintEl.textContent = withErrorDetail(t("workspaces.actionFailed"), error);
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
    workspaceHintEl.textContent = withErrorDetail(t("workspaces.actionFailed"), error);
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
    clearWorkspaceChecks();
    workspaceHintEl.textContent = withErrorDetail(t("workspaces.actionFailed"), error);
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
    workspaceHintEl.textContent = withErrorDetail(t("workspaces.actionFailed"), error);
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

function clearWorkspaceChecks() {
  lastWorkspaceDoctorReport = null;
  workspaceChecksCollapsed = false;
  workspaceChecksPanelEl.hidden = true;
  workspaceChecksPanelEl.classList.remove("is-collapsed");
  workspaceChecksEl.innerHTML = "";
  workspaceChecksSummaryEl.textContent = "";
  workspaceChecksToggleEl.textContent = "";
}

function syncWorkspaceChecksToggle() {
  workspaceChecksPanelEl.classList.toggle("is-collapsed", workspaceChecksCollapsed);
  workspaceChecksToggleEl.textContent = workspaceChecksCollapsed
    ? t("workspaces.expandChecks")
    : t("workspaces.collapseChecks");
  workspaceChecksToggleEl.setAttribute(
    "aria-expanded",
    workspaceChecksCollapsed ? "false" : "true",
  );
}

function toggleWorkspaceChecks() {
  if (!lastWorkspaceDoctorReport) {
    return;
  }
  workspaceChecksCollapsed = !workspaceChecksCollapsed;
  syncWorkspaceChecksToggle();
}

function renderWorkspaceChecks(report: WorkspaceDoctorReport) {
  if (!report.checks.length) {
    clearWorkspaceChecks();
    workspaceHintEl.textContent = t("workspaces.noActive");
    return;
  }

  lastWorkspaceDoctorReport = report;
  workspaceChecksCollapsed = false;

  let pass = 0;
  let warn = 0;
  let fail = 0;
  for (const check of report.checks) {
    if (check.status === "pass") pass += 1;
    else if (check.status === "warn") warn += 1;
    else fail += 1;
  }
  const summary = t("workspaces.doctorSummary", {
    pass: String(pass),
    warn: String(warn),
    fail: String(fail),
  });

  workspaceChecksPanelEl.hidden = false;
  workspaceChecksSummaryEl.textContent = summary;
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
  syncWorkspaceChecksToggle();
  workspaceHintEl.textContent = "";
}

let lastWorkspaceDoctorReport: WorkspaceDoctorReport | null = null;
let workspaceChecksCollapsed = false;

let lastRemoteProjects: RemoteProjectRow[] = [];
let lastRemoteHosts: RemoteHostRow[] = [];
const remoteProbeStatus = new Map<string, RemoteHostProbeStatus>();
const remoteProbeMessage = new Map<string, string>();

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
  } else if (ids.length === 1) {
    remoteProjectHostEl.value = ids[0]!;
  }
}

function probeBadge(status: RemoteHostProbeStatus): { className: string; label: string } {
  switch (status) {
    case "ok":
      return { className: "ok", label: t("remote.statusOk") };
    case "fail":
      return { className: "bad", label: t("remote.statusFail") };
    case "probing":
      return { className: "warn", label: t("remote.statusProbing") };
    default:
      return { className: "muted", label: t("remote.statusUnknown") };
  }
}

function renderRemoteHostList(rows: RemoteHostRow[]): void {
  const bootstrap = document.querySelector<HTMLDetailsElement>("#remote-bootstrap-host");
  if (bootstrap) {
    bootstrap.open = rows.length === 0;
  }

  if (rows.length === 0) {
    remoteHostListEl.innerHTML = "";
    return;
  }

  remoteHostListEl.innerHTML = rows
    .map((row) => {
      const status = remoteProbeStatus.get(row.host_id) ?? "unknown";
      const badge = probeBadge(status);
      const detail = remoteProbeMessage.get(row.host_id);
      const projects =
        row.project_count > 0
          ? t("remote.projectsCount", { count: String(row.project_count) })
          : "";
      const sub = [row.target, projects, detail].filter(Boolean).join(" · ");
      const firstProject = lastRemoteProjects.find((p) => p.host_id === row.host_id);
      const doctorBtn = firstProject
        ? `<button type="button" class="btn-ghost btn-compact" data-remote-host-action="doctor" data-remote-host="${escapeHtml(row.host_id)}" data-remote-target="${escapeHtml(`${firstProject.host_id}/${firstProject.project_id}`)}" ${remoteBusy ? "disabled" : ""}>${escapeHtml(t("remote.doctor"))}</button>`
        : `<button type="button" class="btn-ghost btn-compact" data-remote-host-action="add-path" data-remote-host="${escapeHtml(row.host_id)}" ${remoteBusy ? "disabled" : ""}>${escapeHtml(t("remote.addPath"))}</button>`;
      return `
        <li class="ws-manage-item">
          <div class="ws-manage-main">
            <strong>${escapeHtml(row.host_id)}</strong>
            <span>${escapeHtml(sub)}</span>
          </div>
          <div class="ws-manage-right">
            <span class="badge ${badge.className}">${escapeHtml(badge.label)}</span>
            <div class="ws-manage-actions">
              <button type="button" class="btn-ghost btn-compact" data-remote-host-action="probe" data-remote-host="${escapeHtml(row.host_id)}" ${remoteBusy ? "disabled" : ""}>${escapeHtml(t("remote.probe"))}</button>
              ${doctorBtn}
              <button type="button" class="btn-ghost btn-compact" data-remote-host-action="remove" data-remote-host="${escapeHtml(row.host_id)}" ${remoteBusy ? "disabled" : ""}>${escapeHtml(t("remote.removeHost"))}</button>
            </div>
          </div>
        </li>
      `;
    })
    .join("");
}

function renderRemoteList(rows: RemoteProjectRow[]): void {
  if (rows.length === 0) {
    remoteListEl.hidden = true;
    remoteListEl.innerHTML = "";
    return;
  }

  remoteListEl.hidden = false;
  remoteListEl.innerHTML = rows
    .map((row) => {
      const target = `${row.host_id}/${row.project_id}`;
      return `
        <li class="ws-manage-item">
          <div class="ws-manage-main">
            <strong>${escapeHtml(target)}</strong>
            <span>${escapeHtml(row.path)}</span>
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
    const [hostsDoc, hostRows, projects] = await Promise.all([
      invoke<RemoteHostsDocument>("list_remote_hosts_command"),
      invoke<RemoteHostRow[]>("list_remote_host_rows_command"),
      invoke<RemoteProjectRow[]>("list_remote_projects_command"),
    ]);
    const known = new Set(hostRows.map((row) => row.host_id));
    for (const id of [...remoteProbeStatus.keys()]) {
      if (!known.has(id)) {
        remoteProbeStatus.delete(id);
        remoteProbeMessage.delete(id);
      }
    }
    lastRemoteHosts = hostRows;
    lastRemoteProjects = projects;
    fillRemoteHostSelect(hostsDoc);
    renderRemoteHostList(hostRows);
    renderRemoteList(projects);
    remoteStatusEl.textContent = "";
  } catch (error) {
    remoteStatusEl.textContent = withErrorDetail(t("remote.doctorFailed"), error);
    remoteHostListEl.innerHTML = "";
    remoteListEl.innerHTML = "";
  }
}

async function probeRemoteHostUi(id: string): Promise<void> {
  if (!id || remoteBusy) return;
  remoteBusy = true;
  remoteProbeStatus.set(id, "probing");
  remoteProbeMessage.delete(id);
  renderRemoteHostList(lastRemoteHosts);
  renderRemoteList(lastRemoteProjects);
  remoteHintEl.textContent = t("remote.probeRunning", { id });
  // Let the "检测中…" paint before the IPC call.
  await new Promise<void>((resolve) => {
    window.setTimeout(() => resolve(), 0);
  });
  try {
    const report = await invoke<RemoteHostProbeReport>("probe_remote_host_command", { id });
    remoteProbeStatus.set(id, report.ok ? "ok" : "fail");
    remoteProbeMessage.set(id, report.message);
    remoteHintEl.textContent = report.ok
      ? t("remote.probeOk", { id: report.host_id, target: report.target })
      : t("remote.probeFail", { id: report.host_id, error: report.message });
  } catch (error) {
    remoteProbeStatus.set(id, "fail");
    remoteProbeMessage.set(id, String(error));
    remoteHintEl.textContent = withErrorDetail(t("remote.probeFail", { id }), error);
  } finally {
    remoteBusy = false;
    renderRemoteHostList(lastRemoteHosts);
    renderRemoteList(lastRemoteProjects);
  }
}

async function removeRemoteHostUi(id: string): Promise<void> {
  if (!id || remoteBusy) return;
  remoteBusy = true;
  try {
    await invoke("remove_remote_host_command", { id });
    remoteProbeStatus.delete(id);
    remoteProbeMessage.delete(id);
    await loadRemoteProjects();
    remoteHintEl.textContent = "";
  } catch (error) {
    remoteHintEl.textContent = withErrorDetail(t("remote.removeHostFailed"), error);
  } finally {
    remoteBusy = false;
    renderRemoteHostList(lastRemoteHosts);
    renderRemoteList(lastRemoteProjects);
  }
}

async function runRemoteDoctorUi(target: string): Promise<void> {
  if (!target || remoteBusy) return;
  remoteBusy = true;
  renderRemoteHostList(lastRemoteHosts);
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
    remoteHintEl.textContent = withErrorDetail(t("remote.doctorFailed"), error);
  } finally {
    remoteBusy = false;
    renderRemoteHostList(lastRemoteHosts);
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
    remoteHintEl.textContent = withErrorDetail(t("remote.removeFailed"), error);
  } finally {
    remoteBusy = false;
    renderRemoteHostList(lastRemoteHosts);
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

  workspaceChecksToggleEl.addEventListener("click", () => {
    toggleWorkspaceChecks();
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

  remoteHostListEl.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>(
      "[data-remote-host-action]",
    );
    if (!button) {
      return;
    }
    const action = button.dataset.remoteHostAction;
    const host = button.dataset.remoteHost;
    if (!action || !host) {
      return;
    }
    if (action === "probe") {
      void probeRemoteHostUi(host);
      return;
    }
    if (action === "add-path") {
      openRemoteAddPath(host);
      return;
    }
    if (action === "doctor") {
      const target = button.dataset.remoteTarget;
      if (target) {
        void runRemoteDoctorUi(target);
      } else {
        remoteHintEl.textContent = t("remote.needPath");
        openRemoteAddPath(host);
      }
      return;
    }
    if (action === "remove") {
      void removeRemoteHostUi(host);
    }
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
    const displayName = remoteBootstrapIdEl.value.trim();
    const id = toRemoteId(displayName);
    const hostname = remoteBootstrapHostnameEl.value.trim();
    const user = remoteBootstrapUserEl.value.trim();
    const port = Number(remoteBootstrapPortEl.value) || 22;
    const password = remoteBootstrapPasswordEl.value;
    const projectPath = remoteBootstrapPathEl?.value.trim() ?? "";
    if (!displayName || !hostname || !user || !password || remoteBusy) {
      return;
    }
    remoteBusy = true;
    remoteHintEl.textContent = t("remote.bootstrapRunning");
    void (async () => {
      try {
        const doc = await invoke<RemoteHostsDocument>("bootstrap_remote_host_command", {
          id,
          hostname,
          user,
          port,
          password,
        });
        if (projectPath.startsWith("/")) {
          try {
            await invoke("add_remote_project_command", {
              host: id,
              name: projectNameFromPath(projectPath),
              path: projectPath,
              runtimes: [],
            });
          } catch (pathError) {
            remoteHintEl.textContent = t("remote.projectFailed", { error: String(pathError) });
          }
        }
        remoteBootstrapIdEl.value = "";
        remoteBootstrapHostnameEl.value = "";
        remoteBootstrapUserEl.value = "root";
        remoteBootstrapPortEl.value = "22";
        remoteBootstrapPasswordEl.value = "";
        if (remoteBootstrapPathEl) {
          remoteBootstrapPathEl.value = "";
        }
        const target = doc.hosts[id] ? remoteHostLabel(id, doc.hosts[id]!) : hostname;
        remoteProbeStatus.set(id, "ok");
        remoteProbeMessage.set(id, t("remote.statusOk"));
        remoteHintEl.textContent = t("remote.bootstrapSaved", { id, target });
        await loadRemoteProjects();
        const bootstrap = document.querySelector<HTMLDetailsElement>("#remote-bootstrap-host");
        if (bootstrap) {
          bootstrap.open = false;
        }
      } catch (error) {
        remoteHintEl.textContent = withErrorDetail(t("remote.bootstrapFailed"), error);
      } finally {
        remoteBusy = false;
        renderRemoteHostList(lastRemoteHosts);
        renderRemoteList(lastRemoteProjects);
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
          addProject.hidden = true;
        }
      } catch (error) {
        remoteHintEl.textContent = withErrorDetail(t("remote.projectFailed"), error);
      } finally {
        remoteBusy = false;
        renderRemoteHostList(lastRemoteHosts);
        renderRemoteList(lastRemoteProjects);
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
      renderRemoteHostList(lastRemoteHosts);
      renderRemoteList(lastRemoteProjects);
      if (lastWorkspaceDoctorReport) {
        const collapsed = workspaceChecksCollapsed;
        renderWorkspaceChecks(lastWorkspaceDoctorReport);
        workspaceChecksCollapsed = collapsed;
        syncWorkspaceChecksToggle();
      }
    },
  };
}
