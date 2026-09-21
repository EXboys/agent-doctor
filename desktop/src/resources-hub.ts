import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { formatCount, formatRate } from "./format";
import { appState } from "./app-state";
import type {
  McpModuleStatus,
  SkillMountReport,
  SkillsInventoryReport,
} from "./types";

const skillsInventoryEl = document.querySelector<HTMLElement>("#skills-inventory")!;
const skillsRefreshEl = document.querySelector<HTMLButtonElement>("#skills-refresh")!;
const skillsMountAllEl = document.querySelector<HTMLButtonElement>("#skills-mount-all")!;
const skillsDirEl = document.querySelector<HTMLElement>("#skills-dir")!;
const skillsListEl = document.querySelector<HTMLUListElement>("#skills-list")!;
const skillsEmptyEl = document.querySelector<HTMLElement>("#skills-empty")!;
const skillsFootnoteEl = document.querySelector<HTMLElement>("#skills-footnote")!;
const skillsCountEl = document.querySelector<HTMLElement>("#skills-count")!;
const mcpCountEl = document.querySelector<HTMLElement>("#mcp-count")!;

const hubMcpBadgeEl = document.querySelector<HTMLElement>("#hub-mcp-badge")!;
const hubBrowserStatusEl = document.querySelector<HTMLElement>("#hub-browser-status")!;
const hubSkillsCountEl = document.querySelector<HTMLElement>("#hub-skills-count")!;
const hubMcpCountEl = document.querySelector<HTMLElement>("#hub-mcp-count")!;
const hubConfiguredEl = document.querySelector<HTMLElement>("#hub-configured")!;
const openResourcesWindowEl = document.querySelector<HTMLButtonElement>("#open-resources-window")!;
const openResourcesBrowserEl = document.querySelector<HTMLButtonElement>("#open-resources-browser")!;

const RUNTIME_LABELS: Record<string, string> = {
  hermes: "Hermes",
  openclaw: "OpenClaw",
  "claude-code": "Claude",
  codex: "Codex",
  "deepseek-harness": "DeepSeek Harness",
};

function renderSkillsInventory(report: SkillsInventoryReport) {
  skillsInventoryEl.hidden = false;
  skillsDirEl.textContent = t("skills.dir", { dir: report.skills_dir });
  skillsFootnoteEl.textContent = t("skills.footnote");
  skillsListEl.replaceChildren();
  const needRemote =
    report.remote_stats_error === "evotown_not_configured" ||
    report.remote_stats_error === "remote_source_not_configured";
  skillsEmptyEl.textContent = needRemote ? t("skills.emptyNeedEvotown") : t("skills.empty");

  const empty = report.skills.length === 0;
  skillsEmptyEl.hidden = !empty;
  if (empty) return;

  for (const skill of report.skills) {
    const li = document.createElement("li");
    li.className = "skills-item";

    const top = document.createElement("div");
    top.className = "skills-item-top";
    const name = document.createElement("div");
    name.className = "skills-name";
    name.textContent = skill.name || skill.skill_id;
    const ver = document.createElement("div");
    ver.className = "skills-ver";
    ver.textContent =
      skill.download_count != null
        ? `v${skill.version} · ↓${skill.download_count}`
        : `v${skill.version}`;
    top.append(name, ver);

    if (skill.description) {
      const desc = document.createElement("p");
      desc.className = "skills-desc";
      desc.textContent = skill.description;
      li.append(top, desc);
    } else {
      li.append(top);
    }

    const agents = document.createElement("div");
    agents.className = "skills-agents";
    if (skill.agents.length === 0) {
      const none = document.createElement("span");
      none.className = "skills-runtime";
      none.textContent = t("skills.na");
      agents.appendChild(none);
    } else {
      for (const agent of skill.agents) {
        const chip = document.createElement("button");
        chip.type = "button";
        chip.className = agent.mounted ? "skills-runtime is-on" : "skills-runtime";
        const runtimeLabel = RUNTIME_LABELS[agent.runtime] ?? agent.runtime;
        chip.title = agent.mounted
          ? `${t("skills.unmountRuntime", { runtime: runtimeLabel })}\n${agent.path}`
          : `${t("skills.mountRuntime", { runtime: runtimeLabel })}\n${agent.path}`;
        const dot = document.createElement("i");
        dot.className = "skills-runtime-dot";
        const label = document.createElement("span");
        label.textContent = runtimeLabel;
        chip.append(dot, label);
        chip.addEventListener("click", () => {
          void toggleSkillRuntimeMount(chip, skill.skill_id, agent.runtime, agent.mounted);
        });
        agents.appendChild(chip);
      }
    }

    const metrics = document.createElement("div");
    metrics.className = "skills-metrics";
    const metricDefs: Array<[string, string]> = [
      [t("skills.colCalls"), formatCount(skill.call_count)],
      [t("skills.colFirstOk"), formatRate(skill.first_success_rate)],
      [t("skills.colSuccess"), formatRate(skill.success_rate)],
    ];
    for (const [label, value] of metricDefs) {
      const cell = document.createElement("div");
      cell.className = "skills-metric";
      const span = document.createElement("span");
      span.textContent = label;
      const strong = document.createElement("strong");
      strong.textContent = value;
      cell.append(span, strong);
      metrics.appendChild(cell);
    }

    const actions = document.createElement("div");
    actions.className = "skills-item-actions";
    const needsMount = skill.agents.some((a) => !a.mounted);
    if (needsMount) {
      const mountBtn = document.createElement("button");
      mountBtn.type = "button";
      mountBtn.className = "btn-secondary btn-compact";
      mountBtn.textContent = t("skills.mountOne");
      mountBtn.title = t("skills.mountAll");
      mountBtn.addEventListener("click", () => {
        void mountSyncedSkills([skill.skill_id]);
      });
      actions.appendChild(mountBtn);
    }

    li.append(agents, metrics, actions);
    skillsListEl.appendChild(li);
  }
}

async function loadSkillsInventory(opts?: { remoteStats?: boolean }) {
  try {
    const report = await invoke<SkillsInventoryReport>("list_skills_inventory_command", {
      remoteStats: opts?.remoteStats ?? true,
    });
    appState.lastSkillsInventory = report;
    skillsCountEl.textContent = String(report.skills.length);
    renderSkillsInventory(report);
    updateResourcesHubSummary();
  } catch (error) {
    appState.lastSkillsInventory = null;
    skillsCountEl.textContent = "—";
    skillsInventoryEl.hidden = false;
    skillsListEl.replaceChildren();
    skillsEmptyEl.hidden = false;
    skillsEmptyEl.textContent = t("skills.loadFailed", { error: String(error) });
    skillsDirEl.textContent = "";
    skillsFootnoteEl.textContent = "";
    updateResourcesHubSummary();
  }
}

function updateResourcesHubSummary(): void {
  const uniqueMcpNames = new Set(
    (appState.lastMcpStatus?.inventory.servers ?? []).map((server) => server.name.trim().toLowerCase()),
  );
  const skillCount = appState.lastSkillsInventory?.skills.length ?? 0;
  const mcpCount = uniqueMcpNames.size;
  mcpCountEl.textContent = appState.lastMcpStatus ? String(mcpCount) : "—";
  hubSkillsCountEl.textContent = appState.lastSkillsInventory
    ? t("resources.hubSkillsCount", { count: String(skillCount) })
    : "—";
  hubMcpCountEl.textContent = appState.lastMcpStatus ? String(mcpCount) : "—";

  const chrome = appState.lastMcpStatus?.browser;
  const configured = appState.lastMcpStatus?.configured_runtimes ?? [];
  const agentLabels = configured.map((id) => RUNTIME_LABELS[id] ?? id);
  hubConfiguredEl.textContent =
    agentLabels.length > 0
      ? t("resources.hubAgentsOk", { list: agentLabels.join("、") })
      : t("resources.hubAgentsNone");

  hubMcpBadgeEl.classList.remove("ok", "warn", "muted", "bad");
  if (!appState.lastMcpStatus || !chrome) {
    hubMcpBadgeEl.textContent = "—";
    hubMcpBadgeEl.classList.add("muted");
    hubBrowserStatusEl.textContent = "—";
    return;
  }
  if (!chrome.chrome_found) {
    hubMcpBadgeEl.textContent = t("mcp.badgeMissing");
    hubMcpBadgeEl.classList.add("bad");
    hubBrowserStatusEl.textContent = t("resources.hubBrowserMissing");
  } else if (configured.length > 0) {
    hubMcpBadgeEl.textContent = t("mcp.badgeReady");
    hubMcpBadgeEl.classList.add("ok");
    hubBrowserStatusEl.textContent = t("resources.hubBrowserReady");
  } else {
    hubMcpBadgeEl.textContent = t("mcp.badgePartial");
    hubMcpBadgeEl.classList.add("warn");
    hubBrowserStatusEl.textContent = t("resources.hubBrowserReady");
  }
}

async function loadMcpStatus() {
  try {
    const status = await invoke<McpModuleStatus>("mcp_status_command", {
      port: null,
      probeChrome: false,
    });
    appState.lastMcpStatus = status;
    updateResourcesHubSummary();
  } catch {
    appState.lastMcpStatus = null;
    mcpCountEl.textContent = "—";
    updateResourcesHubSummary();
  }
}

async function loadResourcesHub() {
  await Promise.all([loadMcpStatus(), loadSkillsInventory({ remoteStats: false })]);
}

async function openResourcesWindow(section?: "catalog" | "browser"): Promise<void> {
  await invoke("open_resources_window_command", { section: section ?? null });
}

function setSkillsBusy(busy: boolean) {
  skillsInventoryEl.classList.toggle("is-busy", busy);
  skillsMountAllEl.disabled = busy;
  skillsRefreshEl.disabled = busy;
}

async function toggleSkillRuntimeMount(
  chip: HTMLButtonElement,
  skillId: string,
  runtime: string,
  wasMounted: boolean,
) {
  if (chip.classList.contains("is-busy")) return;
  chip.classList.add("is-busy");
  chip.classList.toggle("is-on", !wasMounted);
  skillsFootnoteEl.textContent = wasMounted ? t("skills.unmounting") : t("skills.mounting");
  try {
    const report = wasMounted
      ? await invoke<SkillMountReport>("unmount_synced_skills_command", {
          skillIds: [skillId],
          runtimes: [runtime],
        })
      : await invoke<SkillMountReport>("mount_synced_skills_command", {
          skillIds: [skillId],
          runtimes: [runtime],
        });
    skillsFootnoteEl.textContent = wasMounted
      ? t("skills.unmountOk", {
          unmounted: String(report.unmounted),
          skipped: String(report.skipped),
          failed: String(report.failed),
        })
      : t("skills.mountOk", {
          mounted: String(report.mounted),
          skipped: String(report.skipped),
          failed: String(report.failed),
        });
    // Fast local refresh (skip remote stats) so the click never feels stuck.
    await loadSkillsInventory({ remoteStats: false });
  } catch (error) {
    chip.classList.toggle("is-on", wasMounted);
    skillsFootnoteEl.textContent = t("skills.mountFailed", { error: String(error) });
  } finally {
    chip.classList.remove("is-busy");
  }
}

async function mountSyncedSkills(skillIds?: string[], runtimes?: string[]) {
  setSkillsBusy(true);
  skillsFootnoteEl.textContent = t("skills.mounting");
  try {
    const report = await invoke<SkillMountReport>("mount_synced_skills_command", {
      skillIds: skillIds ?? null,
      runtimes: runtimes ?? null,
    });
    skillsFootnoteEl.textContent = t("skills.mountOk", {
      mounted: String(report.mounted),
      skipped: String(report.skipped),
      failed: String(report.failed),
    });
    await loadSkillsInventory({ remoteStats: false });
  } catch (error) {
    skillsFootnoteEl.textContent = t("skills.mountFailed", { error: String(error) });
  } finally {
    setSkillsBusy(false);
  }
}


export interface ResourcesHubApi {
  renderSkillsInventory: (report: SkillsInventoryReport) => void;
  loadSkillsInventory: (opts?: { remoteStats?: boolean }) => Promise<void>;
  updateResourcesHubSummary: () => void;
  loadMcpStatus: () => Promise<void>;
  loadResourcesHub: () => Promise<void>;
  openResourcesWindow: (section?: "catalog" | "browser") => Promise<void>;
  toggleSkillRuntimeMount: (
    chip: HTMLButtonElement,
    skillId: string,
    runtime: string,
    wasMounted: boolean,
  ) => Promise<void>;
  mountSyncedSkills: (skillIds?: string[], runtimes?: string[]) => Promise<void>;
  setSkillsBusy: (busy: boolean) => void;
  hideSkillsInventory: () => void;
  reloadLocale: () => void;
}

export function initResourcesHub(_deps?: Record<string, never>): ResourcesHubApi {
  openResourcesWindowEl.addEventListener("click", () => {
    void openResourcesWindow("catalog");
  });
  openResourcesBrowserEl.addEventListener("click", () => {
    void openResourcesWindow("browser");
  });
  skillsRefreshEl.addEventListener("click", () => {
    void loadSkillsInventory();
  });
  skillsMountAllEl.addEventListener("click", () => {
    void mountSyncedSkills();
  });

  return {
    renderSkillsInventory,
    loadSkillsInventory,
    updateResourcesHubSummary,
    loadMcpStatus,
    loadResourcesHub,
    openResourcesWindow,
    toggleSkillRuntimeMount,
    mountSyncedSkills,
    setSkillsBusy,
    hideSkillsInventory: () => {
      skillsInventoryEl.hidden = true;
    },
    reloadLocale: () => {
      if (appState.lastSkillsInventory) {
        renderSkillsInventory(appState.lastSkillsInventory);
      }
      updateResourcesHubSummary();
    },
  };
}
