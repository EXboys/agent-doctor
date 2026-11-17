import { listen } from "@tauri-apps/api/event";
import type { WorkspaceDoc } from "../ask-resources";
import { teamupsLoginFailure } from "../friendly-error";
import { getLocale, t } from "../i18n";
import {
  openBrowserUrl,
  startTeamupsLogin,
  teamupsChannelClientId,
  teamupsChannelPref,
  teamupsChannelSetEnabled,
  teamupsComputers,
  teamupsCreateSchedule,
  teamupsDeleteSchedule,
  teamupsRunSchedule,
  teamupsSchedules,
  teamupsUpdateSchedule,
  type TeamupsComputer,
  type TeamupsSchedule,
} from "../ipc";
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
const guardEl = () => document.querySelector<HTMLElement>("#chat-schedules-guard");
const guardTextEl = () => document.querySelector<HTMLElement>("#chat-schedules-guard-text");
const guardActionEl = () => document.querySelector<HTMLButtonElement>("#chat-schedules-guard-action");
const listEl = () => document.querySelector<HTMLElement>("#chat-schedules-list");
const emptyEl = () => document.querySelector<HTMLElement>("#chat-schedules-empty");
const newEl = () => document.querySelector<HTMLButtonElement>("#chat-schedules-new");
const formEl = () => document.querySelector<HTMLFormElement>("#chat-schedules-form");
const promptEl = () => document.querySelector<HTMLTextAreaElement>("#chat-schedules-prompt");
const freqEl = () => document.querySelector<HTMLSelectElement>("#chat-schedules-freq");
const timeEl = () => document.querySelector<HTMLInputElement>("#chat-schedules-time");
const daysEl = () => document.querySelector<HTMLElement>("#chat-schedules-days");
const clientEl = () => document.querySelector<HTMLSelectElement>("#chat-schedules-client");
const refreshEl = () => document.querySelector<HTMLButtonElement>("#chat-schedules-refresh");
const countEl = () => document.querySelector<HTMLElement>("#chat-schedules-count");

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

// ---- 排期管理：站点上那批排期，手机管的是同一份数据 ----

let schedulesCache: TeamupsSchedule[] = [];
let computersCache: TeamupsComputer[] = [];
let ownClientId = "";
let schedulesLoaded = false;
let schedulesLoading = false;
let editingSchedule: TeamupsSchedule | null = null;
let channelSignedIn: boolean | null = null;
let channelEnabled = true;
let loginWaiting = false;
const pickedDays = new Set<number>([1, 2, 3, 4, 5]);

function findScheduleSession(schedule: TeamupsSchedule): ChatSession | undefined {
  const key = `phone:${(schedule.chat || `sched-${schedule.id}`).replace(/^phone:/, "")}`;
  return deps.sessions().find((session) => (session.channelKey?.trim() ?? "") === key);
}

function openScheduleSession(session: ChatSession): void {
  closeSchedulePage();
  deps.switchSession(session.id);
}

function needsLocalChannelWarning(): boolean {
  if (channelSignedIn !== true || channelEnabled) return false;
  const formOpen = Boolean(formEl() && !formEl()!.hidden);
  if (formOpen) {
    const selected = clientEl()?.value ?? ownClientId;
    if (!selected || (ownClientId && selected === ownClientId)) return true;
  }
  return schedulesCache.some(
    (item) => item.enabled && (!item.client_id || (ownClientId && item.client_id === ownClientId)),
  );
}

function paintGuard(): void {
  const box = guardEl();
  const text = guardTextEl();
  const action = guardActionEl();
  if (!box || !text || !action) return;

  if (channelSignedIn === false) {
    box.hidden = false;
    text.textContent = loginWaiting
      ? t("chat.schedulesGuardSigningIn")
      : t("chat.schedulesGuardSignIn");
    action.textContent = t("chat.schedulesGuardSignInAction");
    action.disabled = loginWaiting;
    action.dataset.mode = "signin";
    return;
  }

  if (needsLocalChannelWarning()) {
    box.hidden = false;
    text.textContent = t("chat.schedulesGuardChannelOff");
    action.textContent = t("chat.schedulesGuardChannelAction");
    action.disabled = false;
    action.dataset.mode = "channel";
    return;
  }

  box.hidden = true;
  delete action.dataset.mode;
}

function paintSchedules(): void {
  paintGuard();
  const list = listEl();
  const empty = emptyEl();
  const createBtn = newEl();
  const form = formEl();
  if (!list || !empty) return;
  list.replaceChildren();

  if (channelSignedIn === false) {
    empty.hidden = true;
    if (createBtn) createBtn.hidden = true;
    if (form) form.hidden = true;
    const count = countEl();
    if (count) count.hidden = true;
    return;
  }

  if (createBtn && (!form || form.hidden)) {
    createBtn.hidden = false;
  }
  const count = countEl();
  if (count) {
    count.textContent = String(schedulesCache.length);
    count.hidden = schedulesCache.length === 0;
  }

  if (schedulesLoading && !schedulesLoaded) {
    empty.hidden = false;
    empty.textContent = t("chat.schedulesLoading");
    return;
  }
  if (schedulesCache.length === 0) {
    empty.hidden = false;
    empty.textContent = t("chat.schedulesEmpty");
    return;
  }
  empty.hidden = true;
  for (const schedule of schedulesCache) list.append(scheduleRow(schedule));
}

function scheduleRow(schedule: TeamupsSchedule): HTMLElement {
  const row = document.createElement("div");
  row.className = "chat-schedules-row";
  if (!schedule.enabled) row.classList.add("is-off");
  if (editingSchedule?.id === schedule.id) row.classList.add("is-editing");

  const matchedSession = findScheduleSession(schedule);
  const labelText = schedule.title || schedule.prompt.slice(0, 24);

  const copy = document.createElement("div");
  copy.className = "chat-schedules-copy";
  const topline = document.createElement("div");
  topline.className = "chat-schedules-topline";
  if (matchedSession) {
    const titleBtn = document.createElement("button");
    titleBtn.type = "button";
    titleBtn.className = "chat-schedules-title-btn";
    titleBtn.textContent = labelText;
    titleBtn.title = t("chat.schedulesOpenChat");
    titleBtn.addEventListener("click", () => openScheduleSession(matchedSession));
    topline.append(titleBtn);
  } else {
    const title = document.createElement("strong");
    title.textContent = labelText;
    topline.append(title);
  }
  const status = document.createElement("span");
  status.className = `chat-schedules-status is-${schedule.last_status || "quiet"}`;
  status.textContent = statusLabel(schedule.last_status, schedule.enabled);
  topline.append(status);
  copy.append(topline);
  const summary = document.createElement("span");
  summary.className = "chat-schedules-summary";
  summary.textContent = [
    freqSummary(schedule),
    `${t("chat.schedulesNext")} ${nextRunText(schedule.next_run_at)}`,
    computerName(schedule.client_id, schedule.client_name),
  ].join(" · ");
  copy.append(summary);

  const actions = document.createElement("div");
  actions.className = "chat-schedules-actions";
  if (matchedSession) {
    const view = document.createElement("button");
    view.type = "button";
    view.className = "chat-schedules-mini";
    view.textContent = t("chat.schedulesOpenChat");
    view.addEventListener("click", () => openScheduleSession(matchedSession));
    actions.append(view);
  }
  const run = document.createElement("button");
  run.type = "button";
  run.className = "chat-schedules-mini";
  run.textContent = t("chat.schedulesRunNow");
  run.addEventListener("click", () => void runNow(schedule, run));
  const edit = document.createElement("button");
  edit.type = "button";
  edit.className = "chat-schedules-mini";
  edit.textContent = t("chat.schedulesEdit");
  edit.addEventListener("click", () => openForm(schedule));
  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "chat-schedules-mini";
  toggle.textContent = schedule.enabled
    ? t("chat.schedulesDisable")
    : t("chat.schedulesEnable");
  toggle.addEventListener("click", () => void toggleSchedule(schedule, toggle));
  const del = document.createElement("button");
  del.type = "button";
  del.className = "chat-schedules-mini is-danger";
  del.textContent = t("chat.schedulesDelete");
  del.addEventListener("click", () => void removeSchedule(schedule));
  actions.append(run, edit, toggle, del);

  row.append(copy, actions);
  return row;
}

function freqSummary(schedule: TeamupsSchedule): string {
  const clock = schedule.hhmm;
  if (schedule.freq === "once") return t("chat.schedulesOnceAt", { time: clock });
  if (schedule.freq === "weekly") {
    const names = String(schedule.weekdays || "")
      .split(",")
      .filter(Boolean)
      .map((day) => weekdayLabel(Number(day)))
      .join("·");
    return `${t("chat.schedulesWeeklyAt", { time: clock })}${names ? ` ${names}` : ""}`;
  }
  return t("chat.schedulesDailyAt", { time: clock });
}

function weekdayLabel(isoDay: number): string {
  // weekdays 里 1=周一…7=周日；narrow 名随界面语言走。
  const anchor = new Date(Date.UTC(2024, 0, 1 + ((isoDay - 1) % 7)));
  try {
    return new Intl.DateTimeFormat(getLocale() === "en" ? "en-US" : "zh-CN", {
      weekday: "short",
      timeZone: "UTC",
    }).format(anchor);
  } catch {
    return String(isoDay);
  }
}

function nextRunText(at: number): string {
  if (!at) return "—";
  const date = new Date(at);
  const now = new Date();
  const clock = date.toLocaleTimeString(getLocale() === "en" ? "en-US" : "zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  if (date.toDateString() === now.toDateString()) return clock;
  return `${date.getMonth() + 1}/${date.getDate()} ${clock}`;
}

function computerName(clientId: string, savedName = ""): string {
  if (!clientId) return t("chat.schedulesDefaultComputer");
  const found = computersCache.find((item) => item.id === clientId);
  if (found) return found.name;
  const trimmed = savedName.trim();
  if (ownClientId && clientId === ownClientId) {
    return trimmed || t("chat.schedulesThisComputer");
  }
  if (trimmed) {
    return t("chat.schedulesNamedOffline", { name: trimmed });
  }
  return t("chat.schedulesGoneComputer");
}

function statusLabel(lastStatus: string, enabled: boolean): string {
  if (!enabled) return t("chat.schedulesDisabled");
  switch (lastStatus) {
    case "sent":
      return t("chat.schedulesStSent");
    case "queued":
      return t("chat.schedulesStQueued");
    case "done":
      return t("chat.schedulesStDone");
    case "failed":
      return t("chat.schedulesStFailed");
    case "missed":
      return t("chat.schedulesStMissed");
    default:
      return t("chat.schedulesStWaiting");
  }
}

async function ensureOwnClientId(): Promise<string> {
  if (ownClientId) return ownClientId;
  const id = (await teamupsChannelClientId().catch(() => "")).trim();
  if (id) ownClientId = id;
  return ownClientId;
}

async function loadSchedules(opts: { silent?: boolean } = {}): Promise<void> {
  if (schedulesLoading) return;
  schedulesLoading = true;
  if (!opts.silent) paintSchedules();
  try {
    const pref = await teamupsChannelPref().catch(() => null);
    if (pref) {
      if (pref.client_id?.trim()) ownClientId = pref.client_id.trim();
      channelSignedIn = pref.signed_in;
      channelEnabled = pref.enabled;
      if (pref.signed_in) loginWaiting = false;
    } else {
      await ensureOwnClientId();
    }
    if (channelSignedIn === false) {
      schedulesCache = [];
      computersCache = [];
      schedulesLoaded = false;
      return;
    }
    const [rows, computers] = await Promise.all([
      teamupsSchedules(),
      teamupsComputers().catch(() => [] as TeamupsComputer[]),
    ]);
    schedulesCache = rows;
    computersCache = computers;
    schedulesLoaded = true;
    if (formEl() && !formEl()!.hidden) {
      paintClientSelect(clientEl()?.value ?? editingSchedule?.client_id ?? ownClientId);
    }
  } catch (error) {
    if (!opts.silent) {
      deps.setStatus(error instanceof Error ? error.message : t("chat.schedulesLoadFailed"), "warn");
    }
  } finally {
    schedulesLoading = false;
    paintSchedules();
  }
}

async function handleGuardAction(): Promise<void> {
  const mode = guardActionEl()?.dataset.mode;
  if (mode === "signin") {
    if (loginWaiting) return;
    loginWaiting = true;
    paintGuard();
    try {
      const started = await startTeamupsLogin();
      await openBrowserUrl(started.verification_url);
    } catch (error) {
      loginWaiting = false;
      paintGuard();
      deps.setStatus(teamupsLoginFailure(error), "warn");
    }
    return;
  }
  if (mode === "channel") {
    try {
      await teamupsChannelSetEnabled(true);
      channelEnabled = true;
      const settingsSwitch = document.querySelector<HTMLInputElement>(
        "#chat-settings-channel-switch",
      );
      if (settingsSwitch && !settingsSwitch.checked) {
        settingsSwitch.checked = true;
        settingsSwitch.dispatchEvent(new Event("change"));
      }
      paintGuard();
      deps.setStatus(t("chat.schedulesChannelTurnedOn"), "ok");
    } catch (error) {
      deps.setStatus(error instanceof Error ? error.message : t("chat.schedulesLoadFailed"), "warn");
    }
  }
}

async function runNow(schedule: TeamupsSchedule, button: HTMLButtonElement): Promise<void> {
  button.disabled = true;
  try {
    const outcome = await teamupsRunSchedule(schedule.id);
    deps.setStatus(
      outcome.sent
        ? t("chat.schedulesRunSent")
        : t("chat.schedulesRunQueued"),
      outcome.sent ? "ok" : "muted",
    );
    void loadSchedules({ silent: true });
    if (outcome.sent) {
      window.setTimeout(() => {
        if (isSchedulePageOpen()) paintSchedules();
      }, 700);
    }
  } catch (error) {
    deps.setStatus(error instanceof Error ? error.message : t("chat.schedulesLoadFailed"), "error");
  } finally {
    button.disabled = false;
  }
}

async function toggleSchedule(schedule: TeamupsSchedule, button: HTMLButtonElement): Promise<void> {
  button.disabled = true;
  try {
    const updated = await teamupsUpdateSchedule(schedule.id, {
      enabled: !schedule.enabled,
    });
    schedulesCache = schedulesCache.map((item) => (item.id === updated.id ? updated : item));
    paintSchedules();
  } catch (error) {
    deps.setStatus(error instanceof Error ? error.message : t("chat.schedulesLoadFailed"), "warn");
  } finally {
    button.disabled = false;
  }
}

async function removeSchedule(schedule: TeamupsSchedule): Promise<void> {
  if (!window.confirm(t("chat.schedulesDeleteConfirm"))) return;
  try {
    await teamupsDeleteSchedule(schedule.id);
    schedulesCache = schedulesCache.filter((item) => item.id !== schedule.id);
    if (editingSchedule?.id === schedule.id) closeForm();
    paintSchedules();
    deps.setStatus(t("chat.schedulesDeleted"), "ok");
  } catch (error) {
    deps.setStatus(error instanceof Error ? error.message : t("chat.schedulesLoadFailed"), "warn");
  }
}

function paintDays(): void {
  const box = daysEl();
  if (!box) return;
  box.replaceChildren();
  for (let isoDay = 1; isoDay <= 7; isoDay += 1) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = pickedDays.has(isoDay)
      ? "chat-schedules-day is-on"
      : "chat-schedules-day";
    button.textContent = weekdayLabel(isoDay);
    button.addEventListener("click", () => {
      if (pickedDays.has(isoDay)) pickedDays.delete(isoDay);
      else pickedDays.add(isoDay);
      paintDays();
    });
    box.append(button);
  }
}

function paintClientSelect(selectedId: string): void {
  const select = clientEl();
  if (!select) return;
  select.replaceChildren();

  const addOption = (value: string, label: string) => {
    const opt = document.createElement("option");
    opt.value = value;
    opt.textContent = label;
    select.append(opt);
  };

  if (ownClientId) {
    const ownOnline = computersCache.find((item) => item.id === ownClientId);
    const ownLabel = ownOnline?.name
      ? t("chat.schedulesNamedThisComputer", { name: ownOnline.name })
      : t("chat.schedulesThisComputer");
    addOption(ownClientId, ownLabel);
  }

  for (const pc of computersCache) {
    if (ownClientId && pc.id === ownClientId) continue;
    addOption(pc.id, pc.name || pc.id);
  }

  if (
    editingSchedule?.client_id &&
    editingSchedule.client_id !== ownClientId &&
    !computersCache.some((pc) => pc.id === editingSchedule!.client_id)
  ) {
    addOption(
      editingSchedule.client_id,
      computerName(editingSchedule.client_id, editingSchedule.client_name),
    );
  }

  addOption("", t("chat.schedulesDefaultComputer"));
  select.value = [...select.options].some((opt) => opt.value === selectedId) ? selectedId : "";
}

function openForm(schedule?: TeamupsSchedule): void {
  editingSchedule = schedule ?? null;
  const prompt = promptEl();
  const freq = freqEl();
  const time = timeEl();
  const days = daysEl();

  if (schedule) {
    if (prompt) prompt.value = schedule.prompt;
    if (freq) freq.value = schedule.freq || "daily";
    if (time) time.value = schedule.hhmm || "08:00";
    pickedDays.clear();
    const parsed = String(schedule.weekdays || "")
      .split(",")
      .map((part) => Number(part.trim()))
      .filter((day) => day >= 1 && day <= 7);
    for (const day of parsed.length > 0 ? parsed : [1, 2, 3, 4, 5]) pickedDays.add(day);
  } else {
    if (prompt) prompt.value = "";
    if (freq) freq.value = "daily";
    if (time) time.value = "08:00";
    pickedDays.clear();
    for (const day of [1, 2, 3, 4, 5]) pickedDays.add(day);
  }

  if (days) {
    days.hidden = (freq?.value || "daily") !== "weekly";
  }
  paintDays();
  paintClientSelect(schedule ? schedule.client_id : ownClientId);

  newEl()?.setAttribute("hidden", "true");
  formEl()?.removeAttribute("hidden");
  paintSchedules();
  formEl()?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  prompt?.focus();
}

function closeForm(): void {
  editingSchedule = null;
  formEl()?.setAttribute("hidden", "true");
  if (channelSignedIn !== false) newEl()?.removeAttribute("hidden");
  const prompt = promptEl();
  if (prompt) prompt.value = "";
  paintSchedules();
}

async function submitForm(event: Event): Promise<void> {
  event.preventDefault();
  const prompt = (promptEl()?.value || "").trim();
  if (!prompt) {
    deps.setStatus(t("chat.schedulesNeedPrompt"), "warn");
    return;
  }
  const freq = freqEl()?.value || "daily";
  if (freq === "weekly" && pickedDays.size === 0) {
    deps.setStatus(t("chat.schedulesNeedDay"), "warn");
    return;
  }
  const hhmm = timeEl()?.value || "08:00";
  const weekdays = freq === "weekly" ? [...pickedDays].sort((a, b) => a - b).join(",") : "";
  const tz = Intl.DateTimeFormat().resolvedOptions().timeZone || "Asia/Shanghai";

  try {
    await ensureOwnClientId();
    const select = clientEl();
    const clientId = select ? select.value : ownClientId;
    const pickedPc = clientId ? computersCache.find((pc) => pc.id === clientId) : undefined;
    const clientName =
      pickedPc?.name ||
      (editingSchedule && editingSchedule.client_id === clientId
        ? editingSchedule.client_name
        : "");

    if (editingSchedule) {
      const updated = await teamupsUpdateSchedule(editingSchedule.id, {
        prompt,
        freq,
        hhmm,
        weekdays,
        tz,
        clientId,
        ...(clientId && clientName ? { clientName } : {}),
      });
      schedulesCache = schedulesCache.map((item) => (item.id === updated.id ? updated : item));
      closeForm();
      deps.setStatus(t("chat.schedulesUpdated"), "ok");
    } else {
      await teamupsCreateSchedule({
        prompt,
        freq,
        hhmm,
        weekdays,
        tz,
        client_id: clientId,
        ...(clientId && clientName ? { client_name: clientName } : {}),
      });
      closeForm();
      deps.setStatus(t("chat.schedulesSaved"), "ok");
    }
    void loadSchedules({ silent: true });
  } catch (error) {
    deps.setStatus(error instanceof Error ? error.message : t("chat.schedulesLoadFailed"), "error");
  }
}

function paintManageChrome(): void {
  const bind = document.querySelector<HTMLElement>("#chat-schedules-bind");
  if (bind) bind.textContent = t("chat.schedulesRunOn");
  const promptBox = promptEl();
  if (promptBox) promptBox.placeholder = t("chat.schedulesPromptPlaceholder");
  if (formEl() && !formEl()!.hidden) {
    paintClientSelect(clientEl()?.value ?? editingSchedule?.client_id ?? ownClientId);
  }
}

// ---- 场景模板（原有能力） ----

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
  paintSchedules();
  void loadSchedules({ silent: schedulesLoaded });
  paintManageChrome();
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
  if (isSchedulePageOpen()) {
    paint();
    paintSchedules();
    paintManageChrome();
    if (daysEl() && !daysEl()!.hidden) paintDays();
  }
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
  guardActionEl()?.addEventListener("click", () => void handleGuardAction());
  refreshEl()?.addEventListener("click", () => void loadSchedules());
  newEl()?.addEventListener("click", () => {
    if (formEl()?.hidden) openForm();
    else closeForm();
  });
  document
    .querySelector("#chat-schedules-cancel")
    ?.addEventListener("click", () => closeForm());
  freqEl()?.addEventListener("change", () => {
    const box = daysEl();
    if (!box) return;
    if (freqEl()?.value === "weekly") {
      box.hidden = false;
      paintDays();
    } else {
      box.hidden = true;
    }
  });
  clientEl()?.addEventListener("change", () => paintGuard());
  formEl()?.addEventListener("submit", (event) => void submitForm(event));
  void listen("teamups-account-changed", () => {
    loginWaiting = false;
    schedulesLoaded = false;
    if (isSchedulePageOpen()) void loadSchedules();
  });
}
