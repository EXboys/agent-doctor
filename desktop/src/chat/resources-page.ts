import { open } from "@tauri-apps/plugin-dialog";
import type { AskResourcesController, AskRuntime, ResourcesScopeTab } from "../ask-resources";
import { t, tNamed } from "../i18n";
import { skillInstall, skillRemove } from "../ipc";
import { isAskRuntime, runtimeDisplayName } from "./runtime";
import { closeKnowledgePage, closeResourcesMainPage, closeSchedulePage } from "./overlay-pages";

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
let installing = false;
let pendingDelete = "";

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
      pendingDeleteId: pendingDelete,
      onDeleteSkill: (skillId) => {
        if (pendingDelete !== skillId) {
          pendingDelete = skillId;
          paintContent();
          return;
        }
        void removeSkill(skillId);
      },
    },
  );
}

async function removeSkill(skillId: string): Promise<void> {
  pendingDelete = "";
  setInstallNote(t("chat.skillDeleting"));
  try {
    await skillRemove({
      scope: tab,
      projectName: tab === "project" ? projectPick : null,
      runtime: tab === "agent" ? agentPick : null,
      skillId,
    });
    await deps.reload();
    paint();
    setInstallNote(t("chat.skillDeleted", { name: skillId }));
  } catch (error) {
    const code = String(error);
    setInstallNote(
      code.includes("not-found")
        ? t("chat.skillDeleteMissing")
        : code.includes("no-project")
          ? t("chat.skillInstallNoProject")
          : code.includes("no-agent")
            ? t("chat.skillInstallNoAgent")
            : t("chat.skillDeleteFailed"),
    );
  }
}

function setInstallNote(text: string): void {
  const note = document.querySelector<HTMLElement>("#chat-resources-install-note");
  if (!note) return;
  note.hidden = text.length === 0;
  note.textContent = text;
}

function installError(error: unknown): string {
  const code = String(error);
  if (code.includes("not-a-skill")) return t("chat.skillInstallNone");
  if (code.includes("no-agent")) return t("chat.skillInstallNoAgent");
  if (code.includes("no-project")) return t("chat.skillInstallNoProject");
  if (code.includes("too-big")) return t("chat.skillInstallTooBig");
  return t("chat.skillInstallFailed");
}

function setAddMenu(open: boolean): void {
  const menu = document.querySelector<HTMLElement>("#chat-resources-add-menu");
  const button = document.querySelector<HTMLButtonElement>("#chat-resources-add-skill");
  if (!menu || !button) return;
  menu.hidden = !open;
  button.setAttribute("aria-expanded", open ? "true" : "false");
}

async function installSkills(directory: boolean): Promise<void> {
  if (installing) return;
  setAddMenu(false);
  if (tab === "project" && deps.projects().length === 0) {
    setInstallNote(t("chat.skillInstallNoProject"));
    return;
  }
  let picked: string | string[] | null;
  try {
    picked = await open({
      multiple: true,
      directory,
      filters: directory ? undefined : [{ name: "zip", extensions: ["zip"] }],
    });
  } catch {
    return;
  }
  const paths = (Array.isArray(picked) ? picked : picked ? [picked] : []).filter(Boolean);
  if (!paths.length) return;
  installing = true;
  const addButton = document.querySelector<HTMLButtonElement>("#chat-resources-add-skill");
  if (addButton) addButton.disabled = true;
  setInstallNote(t("chat.skillInstalling"));
  try {
    const report = await skillInstall({
      scope: tab,
      projectName: tab === "project" ? projectPick : null,
      runtime: tab === "agent" ? agentPick : null,
      paths,
    });
    await deps.reload();
    paint();
    const parts: string[] = [];
    if (report.installed.length) {
      parts.push(t("chat.skillInstalled", { names: report.installed.join("、") }));
    }
    if (report.replaced.length) {
      parts.push(t("chat.skillUpdated", { names: report.replaced.join("、") }));
    }
    if (report.skipped > 0) parts.push(t("chat.skillInstallSkipped"));
    setInstallNote(parts.join(" "));
  } catch (error) {
    setInstallNote(installError(error));
  } finally {
    installing = false;
    const addButton = document.querySelector<HTMLButtonElement>("#chat-resources-add-skill");
    if (addButton) addButton.disabled = false;
  }
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
  const add = document.querySelector<HTMLElement>("#chat-resources-add");
  if (add) add.hidden = resourceKind !== "skills";
  if (resourceKind !== "skills") setAddMenu(false);
}

function paint(): void {
  setInstallNote("");
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
  closeSchedulePage();
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
    pendingDelete = "";
    paint();
  });
  const kindTabs = document.querySelector<HTMLElement>(".chat-resources-kind-tabs");
  kindTabs?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-resource-kind]");
    const nextKind = button?.dataset.resourceKind;
    if (nextKind !== "skills" && nextKind !== "mcp") return;
    resourceKind = nextKind;
    pendingDelete = "";
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
  document.querySelector<HTMLButtonElement>("#chat-resources-add-skill")?.addEventListener("click", () => {
    const menu = document.querySelector<HTMLElement>("#chat-resources-add-menu");
    setAddMenu(menu?.hidden !== false);
  });
  document.querySelector<HTMLButtonElement>("#chat-resources-add-folder")?.addEventListener("click", () => {
    void installSkills(true);
  });
  document.querySelector<HTMLButtonElement>("#chat-resources-add-zip")?.addEventListener("click", () => {
    void installSkills(false);
  });
  document.addEventListener("click", (event) => {
    if ((event.target as HTMLElement).closest("#chat-resources-add")) return;
    setAddMenu(false);
  });
}
