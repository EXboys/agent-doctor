
import { listen } from "@tauri-apps/api/event";
import { setAgentBrandIcon } from "./agent-brand";
import { t, tRuntimeBlurb } from "./i18n";
import { withErrorDetail } from "./friendly-error";
import { runtimeCatalog } from "./runtime-catalog";
import type { InstallProgressEvent, RuntimeDoctorResult } from "./types";
import { resourcesState, agentsListEl, agentsEmptyEl, agentInstallInFlight, agentOpenInFlight, agentInstallHint } from "./resources-state";
import { agentChipLabel } from "./resources-skills";
import { renderResourcesList, syncPanelLead } from "./resources";
import { installRuntime, openSession, runDoctor } from "./ipc";

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
    .sort((a, b) => Number(a.installed) - Number(b.installed));
}

export function countAgentMatches(): number {
  return catalogAgents().length;
}

export function renderAgentCatalog(): void {
  const rows = catalogAgents();
  const missing = rows.some((runtime) => !runtime.installed);
  const hintEl = document.querySelector<HTMLElement>("#resources-agents-hint");
  if (hintEl) {
    hintEl.textContent = missing ? t("resources.agentCatalogHint") : t("resources.agentCatalogHintAllOn");
  }
  if (resourcesState.activeSection === "agents") syncPanelLead();
  agentsListEl.replaceChildren();
  if (rows.length === 0) {
    agentsEmptyEl.hidden = false;
    agentsEmptyEl.textContent = resourcesState.resourceQuery ? t("chat.resourcesNoMatch") : t("resources.emptyAgents");
    return;
  }
  agentsEmptyEl.hidden = true;
  for (const runtime of rows) {
    const li = document.createElement("li");
    li.className = "res-catalog-item";
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
    const desc = document.createElement("div");
    desc.className = "res-catalog-desc";
    desc.textContent = agentCatalogBlurb(runtime.id);
    const hint = document.createElement("p");
    hint.className = "res-agent-install-hint";
    const saved = agentInstallHint.get(runtime.id);
    hint.hidden = !saved;
    hint.textContent = saved ?? "";
    body.append(titleRow, desc, hint);

    const metaWrap = document.createElement("div");
    metaWrap.className = "res-catalog-meta mall-actions";
    if (runtime.installed) {
      const opening = agentOpenInFlight.has(runtime.id);
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "btn-secondary btn-compact";
      btn.textContent = opening ? t("runtime.opening") : t("runtime.open");
      btn.disabled = opening;
      btn.addEventListener("click", () => {
        void openCatalogAgent(runtime.id);
      });
      metaWrap.appendChild(btn);
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
    agentsListEl.appendChild(li);
  }
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
    if (!report.install_needed || report.install_succeeded || report.after_installed) {
      agentInstallHint.delete(runtime);
    } else {
      const detail =
        report.skipped.map((item) => item.reason).find(Boolean) ||
        report.manual_fallback[0] ||
        t("runtime.installFailed");
      setAgentInstallHint(runtime, `${t("runtime.installFailed")} ${detail}`, "error");
    }
    await loadDoctor();
  } catch (error) {
    setAgentInstallHint(runtime, withErrorDetail(t("runtime.installFailed"), error), "error");
  } finally {
    agentInstallInFlight.delete(runtime);
    unlisten();
    renderAgentCatalog();
  }
}

export async function openCatalogAgent(runtime: string): Promise<void> {
  if (agentOpenInFlight.has(runtime) || agentInstallInFlight.has(runtime)) return;
  agentOpenInFlight.add(runtime);
  renderAgentCatalog();
  setAgentInstallHint(runtime, t("runtime.opening"), "busy");
  try {
    await openSession({
      runtime,
      cwd: null,
      prompt: null,
      terminal: null,
    });
    setAgentInstallHint(runtime, t("resources.agentOpened"), "ok");
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
