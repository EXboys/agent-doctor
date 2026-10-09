import type { WorkspaceDoc } from "../ask-resources";
import { t } from "../i18n";
import {
  closeKnowledgePage,
  closeResourcesMainPage,
  closeSchedulePage,
  isSchedulePageOpen,
} from "./overlay-pages";
import { closeChatSettings } from "./settings-page";
import {
  SCHEDULE_SCENES,
  schedulePrompt,
  scheduleSceneIntro,
  scheduleSceneTitle,
  type ScheduleScene,
} from "./schedule-prompts";
import { sessionWorkspaceName } from "./session-workspace";
import type { ChatSession } from "./types";

type Deps = {
  activeSession: () => ChatSession;
  sessions: () => ChatSession[];
  workspaceDoc: () => WorkspaceDoc | null;
  switchSession: (id: string) => void;
  fillPrompt: (text: string) => void;
  setStatus: (text: string, tone?: "ok" | "warn" | "muted" | "error") => void;
};

const shellEl = () => document.querySelector<HTMLElement>("#chat-shell");
const mainEl = () => document.querySelector<HTMLElement>("#chat-main");
const pageEl = () => document.querySelector<HTMLElement>("#chat-schedule");
const openEl = () => document.querySelector<HTMLButtonElement>("#chat-schedule-open");
const gridEl = () => document.querySelector<HTMLElement>("#chat-schedule-grid");

let deps: Deps = {
  activeSession: () => {
    throw new Error("schedule page is not bound");
  },
  sessions: () => [],
  workspaceDoc: () => null,
  switchSession: () => {},
  fillPrompt: () => {},
  setStatus: () => {},
};

function paint(): void {
  const grid = gridEl();
  if (!grid) return;
  grid.replaceChildren();
  for (const scene of SCHEDULE_SCENES) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "chat-schedule-card";
    button.dataset.scheduleScene = scene;
    const title = document.createElement("strong");
    title.textContent = scheduleSceneTitle(scene);
    const intro = document.createElement("span");
    intro.textContent = scheduleSceneIntro(scene);
    button.append(title, intro);
    grid.append(button);
  }
}

function openSchedule(): void {
  closeKnowledgePage();
  closeResourcesMainPage();
  closeChatSettings();
  shellEl()?.classList.add("is-schedule");
  mainEl()?.classList.add("is-schedule");
  const page = pageEl();
  if (page) {
    page.hidden = false;
    page.setAttribute("aria-hidden", "false");
  }
  openEl()?.classList.add("is-on");
  openEl()?.setAttribute("aria-pressed", "true");
  paint();
}

/** Latest chat in the project that is open in the main window. */
export function latestSessionInSelectedProject(
  active: ChatSession,
  sessions: ChatSession[],
  doc: WorkspaceDoc | null,
): ChatSession {
  const project = sessionWorkspaceName(active, doc);
  const inProject = sessions.filter((session) => sessionWorkspaceName(session, doc) === project);
  inProject.sort((left, right) => right.updatedAt - left.updatedAt);
  return inProject[0] ?? active;
}

function useScene(scene: ScheduleScene): void {
  const target = latestSessionInSelectedProject(
    deps.activeSession(),
    deps.sessions(),
    deps.workspaceDoc(),
  );
  const prompt = schedulePrompt(scene, target.runtime);
  deps.switchSession(target.id);
  deps.fillPrompt(prompt);
  deps.setStatus(t("chat.scheduleFilled"), "ok");
}

export function syncScheduleLabels(): void {
  const button = openEl();
  if (button) {
    const label = t("chat.schedule");
    button.title = label;
    button.setAttribute("aria-label", label);
  }
  if (isSchedulePageOpen()) paint();
}

export function bindSchedule(next: Deps): void {
  deps = next;
  openEl()?.addEventListener("click", () => {
    if (isSchedulePageOpen()) closeSchedulePage();
    else openSchedule();
  });
  gridEl()?.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-schedule-scene]");
    const scene = button?.dataset.scheduleScene;
    if (!scene || !SCHEDULE_SCENES.includes(scene as ScheduleScene)) return;
    useScene(scene as ScheduleScene);
  });
}
