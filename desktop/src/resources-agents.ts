
import { listen } from "@tauri-apps/api/event";
import { setAgentBrandIcon } from "./agent-brand";
import { t, tRuntimeBlurb } from "./i18n";
import {
  AGENT_CATALOG_GROUP_ORDER,
  agentCatalogRank,
  groupAgentCatalog,
  installsDesktopAndCli,
  isExperimentalRuntime,
  type AgentCatalogGroupId,
  type AgentCatalogTabId,
} from "./agent-catalog-groups";
import { experimentalInstallFailure, installDoneSentence, installFailureSentenceFor, withErrorDetail } from "./friendly-error";
import { isAskRuntimeId, runtimeCatalog, DOCTOR_RUNTIME_ID } from "./runtime-catalog";
import type { InstallProgressEvent, RuntimeDoctorResult } from "./types";
import {
  resourcesState,
  agentsTabsEl,
  agentsListEl,
  agentsEmptyEl,
  agentInstallInFlight,
  agentOpenInFlight,
  agentJustOpened,
  agentInstallHint,
} from "./resources-state";
import { agentChipLabel } from "./resources-skills";
import { renderResourcesList, syncPanelLead } from "./resources";
import { installRuntime, openAskWindow, openSession, runDoctor } from "./ipc";
import { mountInstallReply } from "./install-reply";

const JUST_OPENED_MS = 2000;

export function agentCatalogBlurb(runtime: string): string {
  return tRuntimeBlurb(runtime);
}

export function agentMatchesQuery(runtime: RuntimeDoctorResult): boolean {
  if (!resourcesState.resourceQuery) return true;
  const blob = `${agentChipLabel(runtime.id)} ${runtime.display_name} ${agentCatalogBlurb(runtime.id)}`.toLowerCase();
  return blob.includes(resourcesState.resourceQuery);
}

export function catalogFallbackRuntime(id: string, displayName: string): RuntimeDoctorResult {
  return {
    id,
    display_name: displayName,
    installed: false,
    version: null,
    binary_path: null,
    config_paths: [],
    profile: { gateway_url: null, key_source: null },
  };
}

export function cursorAlreadyOnThisComputer(runtime: RuntimeDoctorResult): boolean {
  if (runtime.id !== "cursor") return runtime.installed;
  return (
    runtime.installed ||
    resourcesState.cursorOnThisComputer ||
    Boolean(runtime.profile?.key_source) ||
    (runtime.binary_path ?? "").includes("Cursor.app")
  );
}

export function withCatalogInstallState(runtime: RuntimeDoctorResult): RuntimeDoctorResult {
  if (runtime.id !== "cursor" || runtime.installed || !cursorAlreadyOnThisComputer(runtime)) {
    return runtime;
  }
  return { ...runtime, installed: true };
}

export async function refreshCursorOnThisComputer(): Promise<void> {
  const known = resourcesState.lastDoctorReport?.runtimes.find((runtime) => runtime.id === "cursor");
  if (
    known?.installed ||
    known?.profile?.key_source ||
    (known?.binary_path ?? "").includes("Cursor.app")
  ) {
    resourcesState.cursorOnThisComputer = true;
  }
}

export function catalogAgents(): RuntimeDoctorResult[] {
  const fromDoctor = resourcesState.lastDoctorReport?.runtimes ?? [];
  const seen = new Set(fromDoctor.map((runtime) => runtime.id));
  const extras = runtimeCatalog()
    .filter((entry) => !seen.has(entry.id))
    .map((entry) => catalogFallbackRuntime(entry.id, entry.label));
  return [...fromDoctor, ...extras]
    .map(withCatalogInstallState)
    .filter((runtime) => agentMatchesQuery(runtime))
    .sort((a, b) => {
      const rankDiff = agentCatalogRank(a.id) - agentCatalogRank(b.id);
      if (rankDiff !== 0) return rankDiff;
      return Number(a.installed) - Number(b.installed);
    });
}

export function countAgentMatches(): number {
  return catalogAgents().length;
}

/**
 * The top line says "点安装" when a normal agent is still missing.
 * Experimental agents have their own note, unless they are all the search shows.
 */
export function stableCatalogNeedsInstall(): boolean {
  const rows = catalogAgents().filter((runtime) => runtime.id !== DOCTOR_RUNTIME_ID);
  const stable = rows.filter((runtime) => !isExperimentalRuntime(runtime.id));
  const needs = (runtime: RuntimeDoctorResult) => !runtime.installed || Boolean(runtime.incomplete_install);
  if (stable.some(needs)) return true;
  return stable.length === 0 && rows.some(needs);
}

function renderAgentRow(runtime: RuntimeDoctorResult): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "res-catalog-item is-agent-row";
  li.dataset.runtime = runtime.id;

  const icon = document.createElement("span");
  icon.className = "res-catalog-icon is-agent";
  setAgentBrandIcon(icon, runtime.id);

  const body = document.createElement("div");
  body.className = "res-catalog-body";
  const titleRow = document.createElement("div");
  titleRow.className = "res-catalog-title-row";
  const strong = document.createElement("strong");
  strong.textContent = agentChipLabel(runtime.id);
  titleRow.appendChild(strong);
  if (installsDesktopAndCli(runtime.id)) {
    const scopeBadge = document.createElement("span");
    scopeBadge.className = "res-catalog-badge is-dual";
    scopeBadge.textContent = t("resources.agentInstallDual");
    titleRow.appendChild(scopeBadge);
  }
  if (isExperimentalRuntime(runtime.id)) {
    const badge = document.createElement("span");
    badge.className = "res-catalog-badge is-experimental";
    badge.textContent =
      runtime.id === "hermes"
        ? t("resources.agentExperimentalBadgeNetwork")
        : t("resources.agentExperimentalBadge");
    titleRow.appendChild(badge);
  }
  const desc = document.createElement("div");
  desc.className = "res-catalog-desc";
  desc.textContent = agentCatalogBlurb(runtime.id);
  // The built-in doctor is a virtual agent: nothing to install, and the
  // backend has no adapter for it. Dual-install runtimes keep the Install
  // button until both CLI and desktop app are on this computer.
  const openable =
    runtime.id === DOCTOR_RUNTIME_ID || (runtime.installed && !runtime.incomplete_install);
  if (runtime.id === DOCTOR_RUNTIME_ID) {
    agentInstallHint.delete(runtime.id);
  }

  const hint = document.createElement("p");
  hint.className = "res-agent-install-hint";
  const saved = agentInstallHint.get(runtime.id);
  hint.hidden = !saved;
  hint.textContent = saved ?? "";
  body.append(titleRow, desc, hint);

  const metaWrap = document.createElement("div");
  metaWrap.className = "res-catalog-meta mall-actions";
  if (openable) {
    const opening = agentOpenInFlight.has(runtime.id);
    const opened = agentJustOpened.has(runtime.id);
    const dual = installsDesktopAndCli(runtime.id) && isAskRuntimeId(runtime.id);
    if (dual) {
      const askBtn = document.createElement("button");
      askBtn.type = "button";
      askBtn.className = "btn-secondary btn-compact";
      askBtn.textContent = opening
        ? t("runtime.opening")
        : opened
          ? t("resources.agentOpened")
          : t("runtime.ask");
      askBtn.disabled = opening;
      askBtn.title = t("runtime.askHint");
      askBtn.addEventListener("click", () => {
        void openCatalogAgent(runtime.id, "ask");
      });
      const appBtn = document.createElement("button");
      appBtn.type = "button";
      appBtn.className = "btn-secondary btn-compact";
      appBtn.textContent = opening ? t("runtime.opening") : t("runtime.openDesktop");
      appBtn.disabled = opening;
      appBtn.title = t("runtime.openDesktopHint");
      appBtn.addEventListener("click", () => {
        void openCatalogAgent(runtime.id, "app");
      });
      metaWrap.append(askBtn, appBtn);
    } else {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "btn-secondary btn-compact";
      btn.textContent = opening
        ? t("runtime.opening")
        : opened
          ? t("resources.agentOpened")
          : t("runtime.open");
      btn.disabled = opening;
      btn.addEventListener("click", () => {
        void openCatalogAgent(runtime.id);
      });
      metaWrap.appendChild(btn);
    }
  } else {
    const busy = agentInstallInFlight.has(runtime.id);
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "btn-primary btn-compact";
    btn.textContent = busy ? t("runtime.installing") : t("runtime.install");
    btn.disabled = busy;
    btn.addEventListener("click", () => {
      void installCatalogAgent(runtime.id);
    });
    metaWrap.appendChild(btn);
  }

  li.append(icon, body, metaWrap);
  return li;
}

function groupCopy(group: AgentCatalogGroupId): { title: string; hint: string } {
  switch (group) {
    case "builtin":
      return {
        title: t("resources.agentGroupBuiltinTitle"),
        hint: t("resources.agentGroupBuiltinHint"),
      };
    case "npm":
      return {
        title: t("resources.agentGroupNpmTitle"),
        hint: t("resources.agentGroupNpmHint"),
      };
    case "domestic":
      return {
        title: t("resources.agentGroupDomesticTitle"),
        hint: t("resources.agentGroupDomesticHint"),
      };
    case "experimental":
      return {
        title: t("resources.agentExperimentalTitle"),
        hint: t("resources.agentExperimentalHint"),
      };
  }
}

function groupTabLabel(group: AgentCatalogGroupId): string {
  switch (group) {
    case "builtin":
      return t("resources.agentGroupBuiltinTitle");
    case "npm":
      return t("resources.agentGroupNpmTitle");
    case "domestic":
      return t("resources.agentGroupDomesticTab");
    case "experimental":
      return t("resources.agentExperimentalTitle");
  }
}

function renderAgentTabs(
  allCount: number,
  grouped: Record<AgentCatalogGroupId, RuntimeDoctorResult[]>,
): void {
  agentsTabsEl.replaceChildren();
  const visibleSection = resourcesState.activeSection === "agents";
  if (allCount === 0) {
    agentsTabsEl.hidden = true;
    return;
  }
  agentsTabsEl.hidden = !visibleSection;
  const tabs: Array<{ id: AgentCatalogTabId; label: string; count: number }> = [
    { id: "all", label: t("resources.agentTabAll"), count: allCount },
    ...AGENT_CATALOG_GROUP_ORDER.map((id) => ({
      id,
      label: groupTabLabel(id),
      count: grouped[id].length,
    })).filter((tab) => tab.count > 0),
  ];

  if (!tabs.some((tab) => tab.id === resourcesState.agentCatalogTab)) {
    resourcesState.agentCatalogTab = "all";
  }

  for (const tab of tabs) {
    const btn = document.createElement("button");
    btn.type = "button";
    const active = resourcesState.agentCatalogTab === tab.id;
    btn.className = `filter-chip${active ? " is-active" : ""}`;
    btn.dataset.agentTab = tab.id;
    btn.setAttribute("role", "tab");
    btn.setAttribute("aria-selected", active ? "true" : "false");
    const labelSpan = document.createElement("span");
    labelSpan.textContent = tab.label;
    const countSpan = document.createElement("span");
    countSpan.className = "chip-count";
    countSpan.textContent = String(tab.count);
    btn.append(labelSpan, countSpan);
    btn.addEventListener("click", () => {
      if (resourcesState.agentCatalogTab === tab.id) return;
      resourcesState.agentCatalogTab = tab.id;
      renderAgentCatalog();
      document.querySelector<HTMLElement>("#panel-agents")?.scrollTo({ top: 0, behavior: "smooth" });
    });
    agentsTabsEl.appendChild(btn);
  }
}

function appendAgentSection(group: AgentCatalogGroupId, rows: RuntimeDoctorResult[]): void {
  if (rows.length === 0) return;
  const { title, hint } = groupCopy(group);
  const heading = document.createElement("div");
  heading.className = "res-catalog-group res-catalog-group-row";
  heading.dataset.agentGroup = group;

  const titleEl = document.createElement("span");
  titleEl.className = "res-catalog-group-title";
  titleEl.textContent = `${title} · ${rows.length}`;
  heading.appendChild(titleEl);

  if (hint) {
    const subEl = document.createElement("span");
    subEl.className = "res-catalog-group-sub";
    subEl.textContent = hint;
    heading.appendChild(subEl);
  }
  agentsListEl.appendChild(heading);

  const ul = document.createElement("ul");
  ul.className = "res-catalog-list";
  for (const runtime of rows) ul.appendChild(renderAgentRow(runtime));
  agentsListEl.appendChild(ul);
}

export function renderAgentCatalog(): void {
  const rows = catalogAgents();
  const grouped = groupAgentCatalog(rows);
  renderAgentTabs(rows.length, grouped);
  const hintEl = document.querySelector<HTMLElement>("#resources-agents-hint");
  if (hintEl) {
    hintEl.textContent = stableCatalogNeedsInstall()
      ? t("resources.agentCatalogHint")
      : t("resources.agentCatalogHintAllOn");
  }
  if (resourcesState.activeSection === "agents") syncPanelLead();
  agentsListEl.replaceChildren();
  const visibleGroups =
    resourcesState.agentCatalogTab === "all"
      ? AGENT_CATALOG_GROUP_ORDER
      : [resourcesState.agentCatalogTab];
  const visibleCount = visibleGroups.reduce((sum, id) => sum + grouped[id].length, 0);
  if (visibleCount === 0) {
    agentsEmptyEl.hidden = false;
    agentsEmptyEl.textContent = resourcesState.resourceQuery ? t("chat.resourcesNoMatch") : t("resources.emptyAgents");
    return;
  }
  agentsEmptyEl.hidden = true;
  if (resourcesState.agentCatalogTab === "all") {
    for (const group of visibleGroups) {
      appendAgentSection(group, grouped[group]);
    }
    return;
  }
  const ul = document.createElement("ul");
  ul.className = "res-catalog-list";
  for (const group of visibleGroups) {
    for (const runtime of grouped[group]) {
      ul.appendChild(renderAgentRow(runtime));
    }
  }
  agentsListEl.appendChild(ul);
}

export function setAgentInstallHint(runtime: string, message: string, tone: "ok" | "warn" | "error" | "busy"): void {
  const row = agentsListEl.querySelector<HTMLElement>(`[data-runtime="${CSS.escape(runtime)}"]`);
  const hint = row?.querySelector<HTMLElement>(".res-agent-install-hint");
  if (!hint) return;
  hint.hidden = !message;
  hint.textContent = message;
  hint.classList.toggle("is-ok", tone === "ok");
  hint.classList.toggle("is-warn", tone === "warn" || tone === "error");
  if (message) agentInstallHint.set(runtime, message);
  else agentInstallHint.delete(runtime);
}

export async function installCatalogAgent(runtime: string): Promise<void> {
  if (agentInstallInFlight.has(runtime)) return;
  agentInstallInFlight.add(runtime);
  renderAgentCatalog();
  setAgentInstallHint(runtime, t("runtime.installing"), "busy");
  const unlisten = await listen<InstallProgressEvent>("install-progress", (event) => {
    if (event.payload.runtime_id !== runtime) return;
    const row = agentsListEl.querySelector<HTMLElement>(`[data-runtime="${CSS.escape(runtime)}"]`);
    if (event.payload.phase === "needs_input" && row) {
      mountInstallReply(
        row,
        event.payload.input_kind === "secret" || event.payload.input_kind === "session"
          ? event.payload.input_kind
          : "line",
      );
      return;
    }
    if (event.payload.phase === "verifying" || event.payload.phase === "done") {
      if (row) mountInstallReply(row, "");
    }
    const text = event.payload.message.trim();
    const status =
      event.payload.phase === "verifying"
        ? t("runtime.installVerifying")
        : text && !text.startsWith("$ ")
          ? text
          : t("runtime.installing");
    setAgentInstallHint(runtime, status, "busy");
  });
  try {
    const report = await installRuntime({
      runtime,
      force: false,
    });
    if (!report.install_needed) {
      agentInstallHint.delete(runtime);
    } else if (report.install_succeeded || report.after_installed) {
      agentInstallHint.set(runtime, installDoneSentence(runtime, false));
    } else {
      const detail =
        report.skipped.map((item) => item.reason).find(Boolean) ||
        report.manual_fallback[0] ||
        "";
      setAgentInstallHint(runtime, installFailureSentenceFor(runtime, detail), "error");
    }
    await loadDoctor();
  } catch (error) {
    const message = isExperimentalRuntime(runtime)
      ? experimentalInstallFailure(runtime)
      : withErrorDetail(t("runtime.installFailed"), error);
    setAgentInstallHint(runtime, message, "error");
  } finally {
    agentInstallInFlight.delete(runtime);
    unlisten();
    renderAgentCatalog();
  }
}

function markAgentJustOpened(runtime: string): void {
  const prior = agentJustOpened.get(runtime);
  if (prior !== undefined) window.clearTimeout(prior);
  agentJustOpened.set(
    runtime,
    window.setTimeout(() => {
      agentJustOpened.delete(runtime);
      renderAgentCatalog();
    }, JUST_OPENED_MS),
  );
}

export async function openCatalogAgent(
  runtime: string,
  entry: "ask" | "app" | "default" = "default",
): Promise<void> {
  if (agentOpenInFlight.has(runtime) || agentInstallInFlight.has(runtime)) return;
  agentOpenInFlight.add(runtime);
  agentInstallHint.delete(runtime);
  renderAgentCatalog();
  try {
    if (entry === "app") {
      await openSession({
        runtime,
        cwd: null,
        prompt: null,
        terminal: null,
        app: true,
      });
    } else if (isAskRuntimeId(runtime)) {
      await openAskWindow({ runtime });
    } else {
      await openSession({
        runtime,
        cwd: null,
        prompt: null,
        terminal: null,
      });
    }
    markAgentJustOpened(runtime);
  } catch (error) {
    setAgentInstallHint(runtime, withErrorDetail(t("runtime.openFailed"), error), "error");
  } finally {
    agentOpenInFlight.delete(runtime);
    renderAgentCatalog();
  }
}

export async function loadDoctor(): Promise<void> {
  try {
    resourcesState.lastDoctorReport = await runDoctor();
  } catch {
    resourcesState.lastDoctorReport = {
      profile_env_path: null,
      profile_env_exists: false,
      active_preset: null,
      runtimes: [],
    };
  }
  await refreshCursorOnThisComputer();
  renderAgentCatalog();
  if (resourcesState.activeSection === "skills") renderResourcesList();
}
