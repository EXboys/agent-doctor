import type { AskResourcesController, AskRuntime, ResourcesScopeTab } from "../ask-resources";
import { t, tNamed } from "../i18n";
import { isAskRuntime, runtimeDisplayName } from "./runtime";
import { closeKnowledgePage, closeResourcesMainPage } from "./overlay-pages";

const AGENTS = ["codex", "claude-code", "hermes", "openclaw", "deepseek-harness"] as const;

type Deps = {
  projects: () => string[];
  currentProject: () => string | null;
  currentAgent: () => string;
  askResources: AskResourcesController;
  reload: () => Promise<void>;
  openManage: () => void;
  closeSidePanel: () => void;
};

const shellEl = () => document.querySelector<HTMLElement>("#chat-shell");
const mainEl = () => document.querySelector<HTMLElement>("#chat-main");
const pageEl = () => document.querySelector<HTMLElement>("#chat-resources-page");
const openEl = () => document.querySelector<HTMLButtonElement>("#chat-resources-toggle");

let deps: Deps = {
  projects: () => [],
  currentProject: () => null,
  currentAgent: () => "codex",
  askResources: null as unknown as AskResourcesController,
  reload: async () => {},
  openManage: () => {},
  closeSidePanel: () => {},
};

let tab: ResourcesScopeTab = "global";
let projectPick = "";
let agentPick: string = AGENTS[0];
let query = "";
let resourceKind: "skills" | "mcp" = "skills";

export function isResourcesPageOpen(): boolean {
  const page = pageEl();
  if (page && !page.hidden) return true;
  return mainEl()?.classList.contains("is-resources-page") === true;
}

export function closeResourcesPage(): void {
  closeResourcesMainPage();
}

function agentLabel(runtime: string): string {
  return isAskRuntime(runtime) ? runtimeDisplayName(runtime) : runtime;
}

function paintPicks(): void {
  const host = document.querySelector<HTMLElement>("#chat-resources-picks");
  if (!host) return;
  host.replaceChildren();
  if (tab === "global") {
    host.hidden = true;
    return;
  }
  const items = tab === "project" ? deps.projects() : [...AGENTS];
  if (tab === "project" && items.length === 0) {
    host.hidden = true;
    return;
  }
  host.hidden = false;
  for (const item of items) {
    const id = item;
    const label = tab === "agent" ? agentLabel(id) : id;
    const button = document.createElement("button");
    button.type = "button";
    button.className = "chat-knowledge-pick";
    button.textContent = label;
    const on = id === (tab === "project" ? projectPick : agentPick);
    button.setAttribute("aria-pressed", on ? "true" : "false");
    button.addEventListener("click", () => {
      if (tab === "project") projectPick = id;
      else agentPick = id;
      paint();
    });
    host.append(button);
  }
}

function paintContent(): void {
  const scope = document.querySelector<HTMLElement>("#chat-resources-scope");
  const skillsList = document.querySelector<HTMLElement>("#chat-resources-skills-list");
  const mcpList = document.querySelector<HTMLElement>("#chat-resources-mcp-list");
  const skillsEmpty = document.querySelector<HTMLElement>("#chat-resources-skills-empty");
  const mcpEmpty = document.querySelector<HTMLElement>("#chat-resources-mcp-empty");
  if (!scope || !skillsList || !mcpList || !skillsEmpty || !mcpEmpty) return;

  const projects = deps.projects();
  if (tab === "project" && projects.length === 0) {
    scope.textContent = t("chat.resourcesScopeEmptyProjects");
    skillsList.replaceChildren();
    mcpList.replaceChildren();
    skillsEmpty.hidden = false;
    mcpEmpty.hidden = false;
    skillsEmpty.textContent = t("chat.skillsEmpty");
    mcpEmpty.textContent = t("chat.mcpEmpty");
    return;
  }

  if (tab === "project" && (!projectPick || !projects.includes(projectPick))) {
    projectPick = deps.currentProject() && projects.includes(deps.currentProject()!)
      ? deps.currentProject()!
      : projects[0] ?? "";
  }
  if (!AGENTS.includes(agentPick as (typeof AGENTS)[number])) agentPick = deps.currentAgent();
  if (!AGENTS.includes(agentPick as (typeof AGENTS)[number])) agentPick = AGENTS[0];

  const name = tab === "agent" ? agentLabel(agentPick) : tab === "project" ? projectPick : "";
  scope.textContent =
    tab === "global"
      ? t("chat.resourcesScopeGlobal")
      : tab === "project"
        ? tNamed("chat.resourcesScopeProject", name)
        : tNamed("chat.resourcesScopeAgent", name);

  const runtime = (tab === "agent" ? agentPick : deps.currentAgent()) as AskRuntime;
  deps.askResources.renderScopedResourceLists(
    tab,
    tab === "project" ? projectPick : deps.currentProject(),
    runtime,
    {
      skillsListEl: skillsList,
      mcpListEl: mcpList,
      skillsEmptyEl: skillsEmpty,
      mcpEmptyEl: mcpEmpty,
      query,
    },
  );
}

function paintResourceKind(): void {
  document.querySelectorAll<HTMLButtonElement>("[data-resource-kind]").forEach((button) => {
    const on = button.dataset.resourceKind === resourceKind;
    button.classList.toggle("is-on", on);
    button.setAttribute("aria-selected", on ? "true" : "false");
    button.tabIndex = on ? 0 : -1;
  });
  document.querySelectorAll<HTMLElement>("[data-resource-section]").forEach((section) => {
    section.hidden = section.dataset.resourceSection !== resourceKind;
  });
}

function paint(): void {
  document.querySelectorAll<HTMLButtonElement>("[data-rt-tab]").forEach((button) => {
    const on = button.dataset.rtTab === tab;
    button.setAttribute("aria-selected", on ? "true" : "false");
    button.classList.toggle("is-on", on);
  });
  paintResourceKind();
  paintPicks();
  paintContent();
}

export function syncResourcesPageLabels(): void {
  if (isResourcesPageOpen()) paint();
}

function openResourcesPage(): void {
  const current = deps.currentAgent();
  if (AGENTS.includes(current as (typeof AGENTS)[number])) agentPick = current;
  const project = deps.currentProject();
  if (project) projectPick = project;
  closeKnowledgePage();
  deps.closeSidePanel();
  shellEl()?.classList.add("is-resources-page");
  mainEl()?.classList.add("is-resources-page");
  const page = pageEl();
  if (page) {
    page.hidden = false;
    page.setAttribute("aria-hidden", "false");
  }
  openEl()?.classList.add("is-on");
  openEl()?.setAttribute("aria-pressed", "true");
  void deps.reload().then(() => paint());
}

export function bindResourcesPage(next: Deps): void {
  deps = next;
  openEl()?.addEventListener("click", () => {
    if (isResourcesPageOpen()) closeResourcesPage();
    else openResourcesPage();
  });
  document.querySelector(".chat-resources-scope-tabs")?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-rt-tab]");
    const nextTab = button?.dataset.rtTab;
    if (nextTab !== "global" && nextTab !== "project" && nextTab !== "agent") return;
    tab = nextTab;
    paint();
  });
  const kindTabs = document.querySelector<HTMLElement>(".chat-resources-kind-tabs");
  kindTabs?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-resource-kind]");
    const nextKind = button?.dataset.resourceKind;
    if (nextKind !== "skills" && nextKind !== "mcp") return;
    resourceKind = nextKind;
    paintResourceKind();
  });
  kindTabs?.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    resourceKind = resourceKind === "skills" ? "mcp" : "skills";
    paintResourceKind();
    kindTabs.querySelector<HTMLButtonElement>(`[data-resource-kind="${resourceKind}"]`)?.focus();
  });
  document.querySelector("#chat-resources-content")?.addEventListener("click", (event) => {
    if (!(event.target as HTMLElement).closest(".chat-res-row")) return;
    paintContent();
  });
  document.querySelector<HTMLInputElement>("#chat-resources-page-search")?.addEventListener("input", (event) => {
    query = (event.target as HTMLInputElement).value.trim();
    paintContent();
  });
  document.querySelector<HTMLButtonElement>("#chat-resources-page-refresh")?.addEventListener("click", () => {
    void deps.reload().then(() => paint());
  });
  document.querySelector<HTMLButtonElement>("#chat-resources-page-manage")?.addEventListener("click", () => {
    deps.openManage();
  });
}
