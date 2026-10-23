import { open } from "@tauri-apps/plugin-dialog";
import type {
  AskResourcesController,
  AskRuntime,
  ResourcesScopeTab,
  SkillListCategory,
} from "../ask-resources";
import { paintResourceEmpty } from "../ask-resources";
import {
  loadPinnedSkillIds,
  loadSkillOrder,
  pinSkillId,
  reorderSkillIds,
  savePinnedSkillIds,
  saveSkillOrder,
  togglePinnedSkillId,
} from "./skill-order";
import { rememberSubagentNames } from "./format";
import { t, tNamed } from "../i18n";
import {
  deleteProjectSubagent,
  listProjectSubagents,
  saveProjectSubagent,
  skillInstall,
  skillRemove,
  type ProjectSubagent,
} from "../ipc";
import { isAskRuntime, runtimeDisplayName } from "./runtime";
import { closeKnowledgePage, closeResourcesMainPage, closeSchedulePage } from "./overlay-pages";
import { SKILL_CATEGORY_ORDER } from "../skill-categories";
import { saveSkillCategoryOverride } from "./skill-category-order";

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
let subagentQuery = "";
let resourceKind: "skills" | "mcp" | "subagents" = "skills";
let subagents: ProjectSubagent[] = [];
let editingSubagentId = "";
let pendingSubagentDelete = "";
let subagentEditor = false;
let skillCategory: SkillListCategory = "all";
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
  const skillCategories = document.querySelector<HTMLElement>("#chat-resources-skill-categories");
  if (!scope || !skillsList || !mcpList || !skillsEmpty || !mcpEmpty) return;

  const projects = deps.projects();
  if (tab === "project" && projects.length === 0) {
    scope.textContent = t("chat.resourcesScopeEmptyProjects");
    skillsList.replaceChildren();
    mcpList.replaceChildren();
    skillsEmpty.hidden = false;
    mcpEmpty.hidden = false;
    paintResourceEmpty(skillsEmpty, t("chat.skillsEmpty"), "skill");
    paintResourceEmpty(mcpEmpty, t("chat.mcpEmpty"), "tool");
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
      onPinSkill: (skillId) => {
        const nextPins = togglePinnedSkillId(loadPinnedSkillIds(), skillId);
        savePinnedSkillIds(nextPins);
        if (nextPins.includes(skillId)) {
          saveSkillOrder(pinSkillId(loadSkillOrder(), skillId));
        }
        paintContent();
      },
      onReorderSkill: (fromId, toId, position) => {
        const sourceRow = skillsList.querySelector<HTMLElement>(
          `.chat-res-row[data-kind="skill"][data-id="${CSS.escape(fromId)}"]`,
        );
        const targetRow = skillsList.querySelector<HTMLElement>(
          `.chat-res-row[data-kind="skill"][data-id="${CSS.escape(toId)}"]`,
        );
        const targetCategory = targetRow?.dataset.category as
          | Exclude<SkillListCategory, "all">
          | undefined;
        const pins = loadPinnedSkillIds();
        if (targetCategory === "pinned") {
          if (!pins.includes(fromId)) savePinnedSkillIds([fromId, ...pins]);
        } else if (targetCategory && SKILL_CATEGORY_ORDER.includes(targetCategory)) {
          if (pins.includes(fromId)) {
            savePinnedSkillIds(pins.filter((id) => id !== fromId));
          }
          saveSkillCategoryOverride(fromId, targetCategory);
        }
        const group = targetRow?.closest<HTMLElement>(".chat-res-group-list")
          ?? sourceRow?.closest<HTMLElement>(".chat-res-group-list")
          ?? skillsList;
        const visible = [...group.querySelectorAll<HTMLElement>('.chat-res-row[data-kind="skill"]')]
          .map((row) => row.dataset.id ?? "")
          .filter(Boolean);
        if (!visible.includes(fromId)) visible.push(fromId);
        saveSkillOrder(reorderSkillIds(loadSkillOrder(), visible, fromId, toId, position));
        paintContent();
      },
      skillCategory,
      skillCategoriesEl: skillCategories,
      onSkillCategory: (next) => {
        skillCategory = next;
        pendingDelete = "";
        paintContent();
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

function subagentNote(code: string): string {
  switch (code) {
    case "hermes-unnamed":
      return t("chat.subagentHermesUnnamed");
    case "hermes-model-map":
      return t("chat.subagentHermesModelMap");
    case "hermes-config":
      return t("chat.subagentHermesConfig");
    case "openclaw-unready":
      return t("chat.subagentOpenclawUnready");
    case "occupied":
      return t("chat.subagentOccupied");
    case "write-failed":
      return t("chat.subagentWriteFailed");
    default:
      return "";
  }
}

function subagentProject(): string {
  return deps.currentProject()?.trim() || projectPick.trim();
}

function clearSubagentForm(): void {
  editingSubagentId = "";
  pendingSubagentDelete = "";
  subagentEditor = false;
  const nameInput = document.querySelector<HTMLInputElement>("#chat-subagent-name");
  const dutyInput = document.querySelector<HTMLTextAreaElement>("#chat-subagent-duty");
  if (nameInput) nameInput.value = "";
  if (dutyInput) dutyInput.value = "";
  syncSubagentView();
}

function openSubagentEditor(item?: ProjectSubagent): void {
  if (!subagentProject()) {
    setInstallNote(t("chat.subagentNeedProject"));
    return;
  }
  editingSubagentId = item?.id ?? "";
  pendingSubagentDelete = "";
  const nameInput = document.querySelector<HTMLInputElement>("#chat-subagent-name");
  const dutyInput = document.querySelector<HTMLTextAreaElement>("#chat-subagent-duty");
  if (nameInput) nameInput.value = item?.name ?? "";
  if (dutyInput) dutyInput.value = item?.responsibility ?? "";
  subagentEditor = true;
  syncSubagentView();
  nameInput?.focus();
}

function syncSubagentView(): void {
  const listView = document.querySelector<HTMLElement>("#chat-subagent-list-view");
  const form = document.querySelector<HTMLElement>("#chat-resources-subagent-form");
  if (listView) listView.hidden = subagentEditor;
  if (form) {
    form.hidden = !subagentEditor;
    form.classList.toggle("is-open", subagentEditor);
  }
  const title = document.querySelector<HTMLElement>("#chat-subagent-form-title");
  if (title) title.textContent = editingSubagentId ? t("chat.subagentEditTitle") : t("chat.subagentCreateTitle");
  const remove = document.querySelector<HTMLButtonElement>("#chat-subagent-delete");
  if (remove) {
    remove.hidden = !editingSubagentId;
    remove.textContent =
      pendingSubagentDelete === editingSubagentId
        ? t("chat.subagentConfirmDelete")
        : t("chat.subagentDelete");
  }
}

/** Remember the names a chat should say, for the project that is open now. */
export async function syncSubagentNames(projectName: string): Promise<void> {
  const project = projectName.trim();
  if (!project) {
    rememberSubagentNames([]);
    return;
  }
  try {
    rememberSubagentNames(await listProjectSubagents({ projectName: project }));
  } catch {
    /* the row keeps the name the assistant used */
  }
}

async function paintSubagents(): Promise<void> {
  const list = document.querySelector<HTMLElement>("#chat-resources-subagents-list");
  const empty = document.querySelector<HTMLElement>("#chat-resources-subagents-empty");
  const title = document.querySelector<HTMLElement>("#chat-subagent-empty-title");
  const hint = document.querySelector<HTMLElement>("#chat-subagent-empty-hint");
  const projectLabel = document.querySelector<HTMLElement>("#chat-subagent-project");
  const count = document.querySelector<HTMLElement>("#chat-subagent-count");
  const create = document.querySelector<HTMLButtonElement>("#chat-subagent-create");
  const createEmpty = document.querySelector<HTMLButtonElement>("#chat-subagent-create-empty");
  const search = document.querySelector<HTMLElement>(".chat-subagent-search");
  if (!list || !empty || !title || !hint) return;
  const project = subagentProject();
  const toolbar = document.querySelector<HTMLElement>(".chat-subagent-toolbar");
  if (projectLabel) projectLabel.textContent = project ? t("chat.subagentProject", { name: project }) : "";
  if (toolbar) toolbar.hidden = !project;
  if (!project) {
    subagents = [];
    list.replaceChildren();
    empty.hidden = false;
    title.textContent = t("chat.subagentNeedProject");
    hint.hidden = true;
    if (count) count.hidden = true;
    if (create) create.hidden = true;
    if (createEmpty) createEmpty.hidden = true;
    if (search) search.hidden = true;
    syncSubagentView();
    return;
  }
  try {
    subagents = await listProjectSubagents({ projectName: project });
  } catch {
    subagents = [];
  }
  rememberSubagentNames(subagents);
  const shown = subagents.filter((item) => {
    if (!subagentQuery) return true;
    const hay = `${item.name}\n${item.responsibility}`.toLowerCase();
    return hay.includes(subagentQuery.toLowerCase());
  });
  list.replaceChildren();
  const searching = Boolean(subagentQuery) && shown.length === 0;
  empty.hidden = shown.length > 0;
  title.textContent = searching ? t("chat.subagentNoMatch") : t("chat.subagentsEmpty");
  hint.hidden = searching;
  hint.textContent = t("chat.subagentEmptyHint");
  if (count) {
    count.hidden = false;
    count.textContent = t("chat.subagentCount", { count: String(subagents.length) });
  }
  if (search) search.hidden = false;
  if (create) create.hidden = false;
  if (createEmpty) createEmpty.hidden = searching;
  for (const item of shown) {
    const card = document.createElement("button");
    card.type = "button";
    card.className = "chat-subagent-card";
    const name = document.createElement("span");
    name.className = "chat-subagent-card-name";
    name.textContent = item.name;
    const duty = document.createElement("span");
    duty.className = "chat-subagent-card-duty";
    duty.textContent = item.responsibility.replace(/\s+/g, " ").trim();
    card.append(name, duty);
    card.addEventListener("click", () => openSubagentEditor(item));
    list.append(card);
  }
  syncSubagentView();
}

async function saveSubagentFromForm(): Promise<void> {
  const project = subagentProject();
  const name = document.querySelector<HTMLInputElement>("#chat-subagent-name")?.value.trim() ?? "";
  const duty = document.querySelector<HTMLTextAreaElement>("#chat-subagent-duty")?.value.trim() ?? "";
  if (!project) {
    setInstallNote(t("chat.subagentNeedProject"));
    return;
  }
  if (!name) {
    setInstallNote(t("chat.subagentEmptyName"));
    return;
  }
  if (!duty) {
    setInstallNote(t("chat.subagentEmptyDuty"));
    return;
  }
  const save = document.querySelector<HTMLButtonElement>("#chat-subagent-save");
  if (save) save.disabled = true;
  try {
    const report = await saveProjectSubagent({
      projectName: project,
      id: editingSubagentId || null,
      name,
      responsibility: duty,
    });
    clearSubagentForm();
    const extra = report.notes.map(subagentNote).filter(Boolean);
    setInstallNote([t("chat.subagentSaved"), ...extra].join(" "));
    await paintSubagents();
  } catch (error) {
    setInstallNote(String(error).replace(/^Error:\s*/, "") || t("chat.subagentWriteFailed"));
  } finally {
    if (save) save.disabled = false;
  }
}

async function removeSubagent(id: string): Promise<void> {
  const project = subagentProject();
  pendingSubagentDelete = "";
  if (!project) return;
  try {
    await deleteProjectSubagent({ projectName: project, id });
    if (editingSubagentId === id) clearSubagentForm();
    setInstallNote(t("chat.subagentDeleted"));
    await paintSubagents();
  } catch (error) {
    setInstallNote(String(error).replace(/^Error:\s*/, "") || t("chat.subagentWriteFailed"));
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
  const foot = document.querySelector<HTMLElement>(".chat-resources-page-foot");
  if (foot) foot.hidden = resourceKind === "subagents";
  if (resourceKind === "subagents") void paintSubagents();
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
  paintSubagentChrome();
}

function paintSubagentChrome(): void {
  const on = resourceKind === "subagents";
  pageEl()?.classList.toggle("is-subagents", on);
  const heading = document.querySelector<HTMLElement>("#chat-resources-page h2");
  if (heading) heading.textContent = on ? t("chat.subagentsTitle") : t("chat.resourcesPanelTitle");
  const scopeTabs = document.querySelector<HTMLElement>(".chat-resources-scope-tabs");
  const scope = document.querySelector<HTMLElement>("#chat-resources-scope");
  const picks = document.querySelector<HTMLElement>("#chat-resources-picks");
  if (scopeTabs) scopeTabs.hidden = on;
  if (scope) scope.hidden = on;
  if (on && picks) picks.hidden = true;
  syncSubagentView();
}

export function syncResourcesPageLabels(): void {
  if (isResourcesPageOpen()) paint();
}

/** Open the in-chat skills page, where a skill can be installed. */
export function openSkillsManagePage(): void {
  resourceKind = "skills";
  if (!isResourcesPageOpen()) openResourcesPage();
  else paint();
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
  document.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement | null)?.closest?.<HTMLButtonElement>(
      "[data-empty-action]",
    );
    if (!button) return;
    if (button.dataset.emptyAction === "tool") {
      deps.openManage();
      return;
    }
    if (button.dataset.emptyAction !== "skill") return;
    if (!isResourcesPageOpen()) openResourcesPage();
    requestAnimationFrame(() => {
      const menu = document.querySelector<HTMLElement>("#chat-resources-add-menu");
      if (menu && !menu.hidden) return;
      document.querySelector<HTMLButtonElement>("#chat-resources-add-skill")?.click();
    });
  });
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
    if (nextKind !== "skills" && nextKind !== "mcp" && nextKind !== "subagents") return;
    resourceKind = nextKind;
    pendingDelete = "";
    paint();
  });
  kindTabs?.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const order = ["skills", "mcp", "subagents"] as const;
    const index = order.indexOf(resourceKind);
    const step = event.key === "ArrowRight" ? 1 : -1;
    resourceKind = order[(index + step + order.length) % order.length] ?? "skills";
    paint();
    kindTabs.querySelector<HTMLButtonElement>(`[data-resource-kind="${resourceKind}"]`)?.focus();
  });
  document.querySelector("#chat-resources-subagent-form")?.addEventListener("submit", (event) => {
    event.preventDefault();
    void saveSubagentFromForm();
  });
  document.querySelector("#chat-subagent-create")?.addEventListener("click", () => openSubagentEditor());
  document.querySelector("#chat-subagent-create-empty")?.addEventListener("click", () => openSubagentEditor());
  document.querySelector("#chat-subagent-cancel")?.addEventListener("click", () => {
    subagentEditor = false;
    pendingSubagentDelete = "";
    syncSubagentView();
  });
  document.querySelector("#chat-subagent-refresh")?.addEventListener("click", () => {
    void paintSubagents();
  });
  document.querySelector<HTMLInputElement>("#chat-subagent-search")?.addEventListener("input", (event) => {
    subagentQuery = (event.target as HTMLInputElement).value.trim();
    void paintSubagents();
  });
  document.querySelector("#chat-subagent-delete")?.addEventListener("click", () => {
    if (!editingSubagentId) return;
    if (pendingSubagentDelete !== editingSubagentId) {
      pendingSubagentDelete = editingSubagentId;
      syncSubagentView();
      return;
    }
    void removeSubagent(editingSubagentId);
  });
  document.querySelector("#chat-resources-content")?.addEventListener("click", (event) => {
    const row = (event.target as HTMLElement).closest(".chat-res-row");
    if (!row || row.classList.contains("is-hand")) return;
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
