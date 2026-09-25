import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { withErrorDetail } from "./friendly-error";
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
const openResourcesAgentsEl = document.querySelector<HTMLButtonElement>("#open-resources-agents")!;
const hubAgentsCountEl = document.querySelector<HTMLElement>("#hub-agents-count")!;

const RUNTIME_LABELS: Record<string, string> = {
  hermes: "Hermes",
  openclaw: "OpenClaw",
  "claude-code": "Claude",
  codex: "Codex",
  "deepseek-harness": "DeepSeek Harness",
  qoder: "Qoder",
  workbuddy: "WorkBuddy",
  cursor: "Cursor",
};

const SKILL_MOUNT_RUNTIME_ORDER = [
  "hermes",
  "openclaw",
  "claude-code",
  "codex",
  "deepseek-harness",
  "cursor",
] as const;

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
    const fromApi = new Map(skill.agents.map((agent) => [agent.runtime, agent]));
    const agentRows = SKILL_MOUNT_RUNTIME_ORDER.map(
      (runtime) =>
        fromApi.get(runtime) ?? {
          runtime,
          scope: "not mounted",
          path: "",
          mounted: false,
        },
    );
    if (agentRows.length === 0) {
      const none = document.createElement("span");
      none.className = "skills-runtime";
      none.textContent = t("skills.na");
      agents.appendChild(none);
    } else {
      for (const agent of agentRows) {
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

/** Full skill rows only when the inventory list is actually on screen. */
function paintSkillsInventory(report: SkillsInventoryReport): void {
  if (skillsInventoryEl.hidden) {
    return;
  }
  renderSkillsInventory(report);
}

async function loadSkillsInventory(opts?: { remoteStats?: boolean }) {
  try {
    const report = await invoke<SkillsInventoryReport>("list_skills_inventory_command", {
      remoteStats: opts?.remoteStats ?? true,
    });
    appState.lastSkillsInventory = report;
    skillsCountEl.textContent = String(report.skills.length);
    paintSkillsInventory(report);
    updateResourcesHubSummary();
  } catch (error) {
    appState.lastSkillsInventory = null;
    skillsCountEl.textContent = "—";
    skillsInventoryEl.hidden = false;
    skillsListEl.replaceChildren();
    skillsEmptyEl.hidden = false;
    skillsEmptyEl.textContent = withErrorDetail(t("skills.loadFailed"), error);
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
  const knownAgents = appState.lastReport?.runtimes ?? [];
  const missingAgents = knownAgents.filter((runtime) => !runtime.installed).length;
  hubAgentsCountEl.textContent = knownAgents.length
    ? missingAgents > 0
      ? t("resources.hubAgentsAvailable", { count: String(missingAgents) })
      : t("resources.hubAgentsAllOn")
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
  if (appState.lastMcpStatus.browser_deferred) {
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
      discoverChrome: false,
    });
    appState.lastMcpStatus = status;
    updateResourcesHubSummary();
  } catch {
    appState.lastMcpStatus = null;
    mcpCountEl.textContent = "—";
    updateResourcesHubSummary();
  }
}

let hubRefresh: Promise<void> | null = null;
let hubRefreshedAt = 0;

async function loadResourcesHub() {
  // Cards only need counts already in memory. Scanning 100+ skills here freezes the tab.
  updateResourcesHubSummary();
  const now = Date.now();
  const mcpFresh = appState.lastMcpStatus != null && now - hubRefreshedAt < 60_000;
  if (mcpFresh) {
    return;
  }
  if (hubRefresh) {
    return hubRefresh;
  }
  hubRefresh = loadMcpStatus()
    .then(() => {
      hubRefreshedAt = Date.now();
    })
    .finally(() => {
      hubRefresh = null;
    });
  return hubRefresh;
}

async function openResourcesWindow(
  section?: "agents" | "skills" | "mall" | "tools" | "browser" | "catalog" | "store",
): Promise<void> {
  const normalized =
    !section || section === "catalog"
      ? "skills"
      : section === "store"
        ? "mall"
        : section;
  await invoke("open_resources_window_command", { section: normalized });
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
    skillsFootnoteEl.textContent = withErrorDetail(t("skills.mountFailed"), error);
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
    skillsFootnoteEl.textContent = withErrorDetail(t("skills.mountFailed"), error);
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
  openResourcesWindow: (section?: "agents" | "skills" | "mall" | "tools" | "browser" | "catalog" | "store") => Promise<void>;
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
    void openResourcesWindow("skills");
  });
  openResourcesBrowserEl.addEventListener("click", () => {
    void openResourcesWindow("browser");
  });
  openResourcesAgentsEl.addEventListener("click", () => {
    void openResourcesWindow("agents");
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
        paintSkillsInventory(appState.lastSkillsInventory);
      }
      updateResourcesHubSummary();
    },
  };
}
