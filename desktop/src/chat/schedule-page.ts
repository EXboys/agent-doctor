import { listen } from "@tauri-apps/api/event";
import type { WorkspaceDoc } from "../ask-resources";
import { teamupsLoginFailure } from "../friendly-error";
import { getLocale, t, type MessageKey } from "../i18n";
import {
  openBrowserUrl,
  polishSchedulePrompt,
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
  defaultModelPick?: () => {
    runtime: string;
    providerId: string | null;
    model: string | null;
  };
};

const SELF_PLACEHOLDER = "__self__";
const TIME_PRESETS = ["08:00", "09:00", "12:00", "18:00", "21:00"] as const;

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
const emptyTextEl = () => document.querySelector<HTMLElement>("#chat-schedules-empty-text");
const emptyNewEl = () => document.querySelector<HTMLButtonElement>("#chat-schedules-empty-new");
const newEl = () => document.querySelector<HTMLButtonElement>("#chat-schedules-new");
const formEl = () => document.querySelector<HTMLFormElement>("#chat-schedules-form");
const formTitleEl = () => document.querySelector<HTMLElement>("#chat-schedules-form-title");
const polishBtnEl = () => document.querySelector<HTMLButtonElement>("#chat-schedules-polish");
const polishLabelEl = () => document.querySelector<HTMLElement>("#chat-schedules-polish-label");
const polishUndoEl = () => document.querySelector<HTMLButtonElement>("#chat-schedules-polish-undo");
const promptEl = () => document.querySelector<HTMLTextAreaElement>("#chat-schedules-prompt");
const freqEl = () => document.querySelector<HTMLSelectElement>("#chat-schedules-freq");
const timePickerWrapEl = () => document.querySelector<HTMLElement>("#chat-schedules-timepicker");
const timeEl = () => document.querySelector<HTMLInputElement>("#chat-schedules-time");
const timeBtnEl = () => document.querySelector<HTMLButtonElement>("#chat-schedules-time-btn");
const timeLabelEl = () => document.querySelector<HTMLElement>("#chat-schedules-time-label");
const timePopoverEl = () => document.querySelector<HTMLElement>("#chat-schedules-time-popover");
const timePresetsEl = () => document.querySelector<HTMLElement>("#chat-schedules-time-presets");
const timeHoursEl = () => document.querySelector<HTMLElement>("#chat-schedules-time-hours");
const timeMinutesEl = () => document.querySelector<HTMLElement>("#chat-schedules-time-minutes");
const timeCustomEl = () => document.querySelector<HTMLInputElement>("#chat-schedules-time-custom");
const timeDoneEl = () => document.querySelector<HTMLButtonElement>("#chat-schedules-time-done");
const daysEl = () => document.querySelector<HTMLElement>("#chat-schedules-days");
const clientEl = () => document.querySelector<HTMLSelectElement>("#chat-schedules-client");
const saveEl = () => document.querySelector<HTMLButtonElement>("#chat-schedules-save");
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
let ownComputerName = "";
let schedulesLoaded = false;
let schedulesLoading = false;
let formSaving = false;
let polishing = false;
let prePolishDraft: string | null = null;
let editingSchedule: TeamupsSchedule | null = null;
/** `null` 表示跟随当前表单默认（新建=本机，编辑=原绑定），非 `null` 表示用户已明确选过。 */
let pickedClientId: string | null = null;
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

function friendlyScheduleError(error: unknown, fallbackKey: MessageKey): string {
  const raw = error instanceof Error ? error.message.trim() : String(error ?? "").trim();
  if (!raw) return t(fallbackKey);
  if (raw.includes("登录已过期") || raw.includes("还没登录")) {
    channelSignedIn = false;
    return t("chat.schedulesGuardSignIn");
  }
  if (
    /https?:\/\//i.test(raw) ||
    /<(!doctype|html|body)/i.test(raw) ||
    /\b(404|500|502|503|504)\b/.test(raw) ||
    raw.length > 90
  ) {
    return t(fallbackKey);
  }
  return raw;
}

function effectiveSelectedClientId(): string {
  if (pickedClientId !== null) {
    return pickedClientId === SELF_PLACEHOLDER ? ownClientId : pickedClientId;
  }
  if (editingSchedule) return editingSchedule.client_id;
  return ownClientId;
}

function needsLocalChannelWarning(): boolean {
  if (channelSignedIn !== true || channelEnabled) return false;
  const formOpen = Boolean(formEl() && !formEl()!.hidden);
  if (formOpen) {
    const selected = clientEl()?.value ?? effectiveSelectedClientId();
    if (
      !selected ||
      selected === SELF_PLACEHOLDER ||
      (ownClientId && selected === ownClientId)
    ) {
      return true;
    }
  }
  return schedulesCache.some(
    (item) => item.enabled && (!item.client_id || (ownClientId && item.client_id === ownClientId)),
  );
}

function syncRefreshButton(): void {
  const btn = refreshEl();
  if (!btn) return;
  btn.disabled = schedulesLoading;
  btn.classList.toggle("is-loading", schedulesLoading);
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
  const emptyText = emptyTextEl();
  const emptyCta = emptyNewEl();
  const createBtn = newEl();
  const form = formEl();
  if (!list || !empty) return;
  list.replaceChildren();

  if (channelSignedIn === false) {
    list.hidden = true;
    empty.hidden = true;
    if (createBtn) createBtn.hidden = true;
    if (form) form.hidden = true;
    const count = countEl();
    if (count) count.hidden = true;
    return;
  }

  const formOpen = Boolean(form && !form.hidden);
  const count = countEl();
  if (count) {
    count.textContent = String(schedulesCache.length);
    count.hidden = schedulesCache.length === 0;
  }

  if (schedulesLoading && !schedulesLoaded) {
    list.hidden = true;
    empty.hidden = formOpen;
    if (emptyText) emptyText.textContent = t("chat.schedulesLoading");
    if (emptyCta) emptyCta.hidden = true;
    if (createBtn) createBtn.hidden = true;
    return;
  }

  if (schedulesCache.length === 0) {
    list.hidden = true;
    empty.hidden = formOpen;
    if (emptyText) emptyText.textContent = t("chat.schedulesEmpty");
    if (emptyCta) emptyCta.hidden = formOpen;
    if (createBtn) createBtn.hidden = true;
    return;
  }

  empty.hidden = true;
  if (createBtn) createBtn.hidden = formOpen;
  list.hidden = false;
  for (const schedule of schedulesCache) list.append(scheduleRow(schedule));
}

function scheduleRow(schedule: TeamupsSchedule): HTMLElement {
  const row = document.createElement("div");
  row.className = "chat-schedules-row";
  if (!schedule.enabled) row.classList.add("is-off");
  if (editingSchedule?.id === schedule.id) row.classList.add("is-editing");

  const matchedSession = findScheduleSession(schedule);
  const labelText = schedule.title || schedule.prompt.slice(0, 32);

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
    return trimmed || ownComputerName || t("chat.schedulesThisComputer");
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

async function ensureOwnClientInfo(): Promise<string> {
  if (ownClientId && ownComputerName) return ownClientId;
  const pref = await teamupsChannelPref().catch(() => null);
  if (pref) {
    if (pref.client_id?.trim()) ownClientId = pref.client_id.trim();
    if (pref.computer_name?.trim()) ownComputerName = pref.computer_name.trim();
    channelSignedIn = pref.signed_in;
    channelEnabled = pref.enabled;
  }
  if (!ownClientId) {
    const id = (await teamupsChannelClientId().catch(() => "")).trim();
    if (id) ownClientId = id;
  }
  return ownClientId;
}

async function loadSchedules(opts: { silent?: boolean } = {}): Promise<void> {
  if (schedulesLoading) return;
  schedulesLoading = true;
  syncRefreshButton();
  if (!opts.silent) paintSchedules();
  try {
    const pref = await teamupsChannelPref().catch(() => null);
    if (pref) {
      if (pref.client_id?.trim()) ownClientId = pref.client_id.trim();
      if (pref.computer_name?.trim()) ownComputerName = pref.computer_name.trim();
      channelSignedIn = pref.signed_in;
      channelEnabled = pref.enabled;
      if (pref.signed_in) loginWaiting = false;
    } else {
      await ensureOwnClientInfo();
    }
    if (channelSignedIn === false) {
      schedulesCache = [];
      computersCache = [];
      schedulesLoaded = false;
      return;
    }
    const [schedResult, compResult] = await Promise.allSettled([
      teamupsSchedules(),
      teamupsComputers(),
    ]);
    if (compResult.status === "fulfilled") {
      computersCache = compResult.value;
    }
    if (schedResult.status === "fulfilled") {
      schedulesCache = schedResult.value;
      schedulesLoaded = true;
    } else if (!opts.silent) {
      deps.setStatus(
        friendlyScheduleError(schedResult.reason, "chat.schedulesLoadFailed"),
        "warn",
      );
    }
  } catch (error) {
    if (!opts.silent) {
      deps.setStatus(friendlyScheduleError(error, "chat.schedulesLoadFailed"), "warn");
    }
  } finally {
    schedulesLoading = false;
    syncRefreshButton();
    paintClientSelect();
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
      deps.setStatus(friendlyScheduleError(error, "chat.schedulesLoadFailed"), "warn");
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
    deps.setStatus(friendlyScheduleError(error, "chat.schedulesLoadFailed"), "error");
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
    deps.setStatus(friendlyScheduleError(error, "chat.schedulesLoadFailed"), "warn");
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
    deps.setStatus(friendlyScheduleError(error, "chat.schedulesLoadFailed"), "warn");
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

export type ScheduleClientOptionsResult = {
  options: { value: string; label: string }[];
  selected: string;
};

export function buildScheduleClientOptions(params: {
  ownClientId: string;
  ownComputerName: string;
  computers: TeamupsComputer[];
  editingSchedule: Pick<TeamupsSchedule, "client_id" | "client_name"> | null;
  pickedClientId: string | null;
}): ScheduleClientOptionsResult {
  const { ownClientId: ownId, ownComputerName: ownName, computers, editingSchedule: editing, pickedClientId: picked } = params;
  const options: { value: string; label: string }[] = [];

  const ownOnline = ownId ? computers.find((item) => item.id === ownId) : undefined;
  const rawOwnName = (ownOnline?.name || ownName).trim();
  const genericOwn = t("chat.schedulesThisComputer");
  const selfLabel =
    rawOwnName && rawOwnName !== genericOwn && rawOwnName !== "这台电脑" && rawOwnName !== "This computer"
      ? t("chat.schedulesNamedThisComputer", { name: rawOwnName })
      : genericOwn;
  const selfValue = ownId || SELF_PLACEHOLDER;
  options.push({ value: selfValue, label: selfLabel });

  for (const pc of computers) {
    if (ownId && pc.id === ownId) continue;
    options.push({ value: pc.id, label: pc.name || pc.id });
  }

  if (
    editing?.client_id &&
    editing.client_id !== ownId &&
    !computers.some((pc) => pc.id === editing.client_id)
  ) {
    const trimmed = (editing.client_name || "").trim();
    const offlineLabel = trimmed
      ? t("chat.schedulesNamedOffline", { name: trimmed })
      : t("chat.schedulesGoneComputer");
    options.push({ value: editing.client_id, label: offlineLabel });
  }

  const defaultPc = computers.find((item) => item.default);
  const defaultName = defaultPc?.name?.trim() || "";
  const defaultLabel =
    defaultName && (!ownId || defaultPc!.id !== ownId)
      ? t("chat.schedulesFollowNamedDefault", { name: defaultName })
      : t("chat.schedulesFollowDefault");
  options.push({ value: "", label: defaultLabel });

  let desired: string;
  if (picked !== null) {
    desired = picked === SELF_PLACEHOLDER ? selfValue : picked;
  } else if (editing) {
    desired = editing.client_id;
  } else {
    desired = selfValue;
  }

  const selected = options.some((opt) => opt.value === desired) ? desired : selfValue;
  return { options, selected };
}

function paintClientSelect(): void {
  const select = clientEl();
  if (!select) return;
  select.replaceChildren();

  const { options, selected } = buildScheduleClientOptions({
    ownClientId,
    ownComputerName,
    computers: computersCache,
    editingSchedule,
    pickedClientId,
  });

  for (const item of options) {
    const opt = document.createElement("option");
    opt.value = item.value;
    opt.textContent = item.label;
    select.append(opt);
  }
  select.value = selected;
}

function resizePromptTextarea(): void {
  const prompt = promptEl();
  if (!prompt) return;
  prompt.style.height = "auto";
  const next = Math.min(Math.max(prompt.scrollHeight, 76), 200);
  prompt.style.height = `${next}px`;
}

export function normalizeScheduleHhmm(raw: string): string | null {
  const cleaned = raw.trim().replace(/：/g, ":");
  const colonMatch = /^(\d{1,2}):(\d{1,2})$/.exec(cleaned);
  if (colonMatch) {
    const h = Number(colonMatch[1]);
    const m = Number(colonMatch[2]);
    if (h >= 0 && h <= 23 && m >= 0 && m <= 59) {
      return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
    }
    return null;
  }
  const compactMatch = /^(\d{2})(\d{2})$/.exec(cleaned);
  if (compactMatch) {
    const h = Number(compactMatch[1]);
    const m = Number(compactMatch[2]);
    if (h >= 0 && h <= 23 && m >= 0 && m <= 59) {
      return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
    }
  }
  return null;
}

function currentHhmm(): string {
  return normalizeScheduleHhmm(timeEl()?.value || "") || "08:00";
}

function setScheduleTime(raw: string): void {
  const hhmm = normalizeScheduleHhmm(raw) || "08:00";
  const input = timeEl();
  if (input) input.value = hhmm;
  const label = timeLabelEl();
  if (label) label.textContent = hhmm;
  const custom = timeCustomEl();
  if (custom && document.activeElement !== custom) custom.value = hhmm;
  syncTimePopoverSelection(hhmm);
}

function syncTimePopoverSelection(hhmm: string): void {
  const [hourStr, minStr] = hhmm.split(":");
  timePresetsEl()
    ?.querySelectorAll<HTMLButtonElement>("[data-preset-time]")
    .forEach((btn) => {
      btn.classList.toggle("is-on", btn.dataset.presetTime === hhmm);
    });
  timeHoursEl()
    ?.querySelectorAll<HTMLButtonElement>("[data-hour]")
    .forEach((btn) => {
      btn.classList.toggle("is-on", btn.dataset.hour === hourStr);
    });
  timeMinutesEl()
    ?.querySelectorAll<HTMLButtonElement>("[data-minute]")
    .forEach((btn) => {
      btn.classList.toggle("is-on", btn.dataset.minute === minStr);
    });
}

function ensureTimePopoverBuilt(): void {
  const presets = timePresetsEl();
  const hours = timeHoursEl();
  const minutes = timeMinutesEl();
  if (!presets || !hours || !minutes) return;
  if (hours.childElementCount > 0) return;

  for (const preset of TIME_PRESETS) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "chat-timepicker-preset";
    btn.dataset.presetTime = preset;
    btn.textContent = preset;
    btn.addEventListener("click", () => {
      setScheduleTime(preset);
      closeTimePicker();
    });
    presets.append(btn);
  }

  for (let h = 0; h < 24; h += 1) {
    const hh = String(h).padStart(2, "0");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "chat-timepicker-cell";
    btn.dataset.hour = hh;
    btn.textContent = hh;
    btn.addEventListener("click", () => {
      const [, mm] = currentHhmm().split(":");
      setScheduleTime(`${hh}:${mm}`);
    });
    hours.append(btn);
  }

  for (let m = 0; m < 60; m += 1) {
    const mm = String(m).padStart(2, "0");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = m % 5 === 0 ? "chat-timepicker-cell is-step" : "chat-timepicker-cell";
    btn.dataset.minute = mm;
    btn.textContent = mm;
    btn.addEventListener("click", () => {
      const [hh] = currentHhmm().split(":");
      setScheduleTime(`${hh}:${mm}`);
    });
    minutes.append(btn);
  }
}

function openTimePicker(): void {
  const popover = timePopoverEl();
  const btn = timeBtnEl();
  if (!popover || !btn) return;
  ensureTimePopoverBuilt();
  const hhmm = currentHhmm();
  const custom = timeCustomEl();
  if (custom) custom.value = hhmm;
  syncTimePopoverSelection(hhmm);

  const rect = btn.getBoundingClientRect();
  const spaceBelow = window.innerHeight - rect.bottom;
  popover.classList.toggle("is-up", spaceBelow < 290 && rect.top > 290);

  popover.hidden = false;
  btn.setAttribute("aria-expanded", "true");
  btn.classList.add("is-open");

  window.requestAnimationFrame(() => {
    timeHoursEl()
      ?.querySelector<HTMLElement>(".chat-timepicker-cell.is-on")
      ?.scrollIntoView({ block: "center" });
    timeMinutesEl()
      ?.querySelector<HTMLElement>(".chat-timepicker-cell.is-on")
      ?.scrollIntoView({ block: "center" });
  });
}

function closeTimePicker(): void {
  const popover = timePopoverEl();
  const btn = timeBtnEl();
  const custom = timeCustomEl();
  if (custom && custom.value.trim()) {
    const parsed = normalizeScheduleHhmm(custom.value);
    if (parsed) setScheduleTime(parsed);
  }
  if (popover) popover.hidden = true;
  if (btn) {
    btn.setAttribute("aria-expanded", "false");
    btn.classList.remove("is-open");
  }
}

function toggleTimePicker(): void {
  const popover = timePopoverEl();
  if (!popover) return;
  if (popover.hidden) openTimePicker();
  else closeTimePicker();
}

function syncPolishControls(): void {
  const btn = polishBtnEl();
  const label = polishLabelEl();
  const undo = polishUndoEl();
  const prompt = promptEl();
  if (btn) {
    btn.disabled = polishing || formSaving;
    btn.classList.toggle("is-loading", polishing);
  }
  if (label) {
    label.textContent = polishing ? t("chat.schedulesPolishing") : t("chat.schedulesPolish");
  }
  if (undo) {
    undo.hidden = polishing || prePolishDraft === null;
    undo.disabled = polishing || formSaving;
    undo.textContent = t("chat.schedulesPolishUndo");
  }
  if (prompt) {
    prompt.readOnly = polishing;
    prompt.classList.toggle("is-polishing", polishing);
  }
}

async function runPolishDraft(): Promise<void> {
  if (polishing || formSaving) return;
  const prompt = promptEl();
  const draft = (prompt?.value || "").trim();
  if (!draft) {
    deps.setStatus(t("chat.schedulesPolishNeedText"), "warn");
    prompt?.focus();
    return;
  }

  polishing = true;
  syncPolishControls();
  try {
    const pick = deps.defaultModelPick?.();
    const polished = await polishSchedulePrompt({
      prompt: draft,
      runtime: pick?.runtime ?? null,
      providerId: pick?.providerId ?? null,
      model: pick?.model ?? null,
      locale: getLocale(),
    });
    const next = polished.trim();
    if (next && prompt) {
      prePolishDraft = draft;
      prompt.value = next;
      resizePromptTextarea();
      prompt.focus();
      const end = prompt.value.length;
      prompt.setSelectionRange(end, end);
      deps.setStatus(t("chat.schedulesPolished"), "ok");
    }
  } catch (error) {
    const raw = error instanceof Error ? error.message : String(error ?? "");
    if (raw.includes("no_model_ready") || raw.includes("official_provider_missing")) {
      deps.setStatus(t("chat.schedulesPolishNoModel"), "warn");
    } else {
      deps.setStatus(friendlyScheduleError(error, "chat.schedulesPolishFailed"), "warn");
    }
  } finally {
    polishing = false;
    syncPolishControls();
  }
}

function undoPolishDraft(): void {
  if (prePolishDraft === null) return;
  const prompt = promptEl();
  if (prompt) {
    prompt.value = prePolishDraft;
    resizePromptTextarea();
    prompt.focus();
  }
  prePolishDraft = null;
  syncPolishControls();
  deps.setStatus(t("chat.schedulesPolishRestored"), "muted");
}

function syncFormHeaderAndSave(): void {
  const title = formTitleEl();
  if (title) {
    title.textContent = editingSchedule
      ? t("chat.schedulesFormEditTitle")
      : t("chat.schedulesFormNewTitle");
  }
  const save = saveEl();
  if (save) {
    save.disabled = formSaving || polishing;
    save.textContent = formSaving ? t("chat.schedulesSaving") : t("chat.schedulesSave");
  }
  syncPolishControls();
}

function openForm(schedule?: TeamupsSchedule): void {
  editingSchedule = schedule ?? null;
  pickedClientId = schedule ? schedule.client_id : null;
  prePolishDraft = null;
  closeTimePicker();
  const prompt = promptEl();
  const freq = freqEl();
  const days = daysEl();

  if (schedule) {
    if (prompt) prompt.value = schedule.prompt;
    if (freq) freq.value = schedule.freq || "daily";
    setScheduleTime(schedule.hhmm || "08:00");
    pickedDays.clear();
    const parsed = String(schedule.weekdays || "")
      .split(",")
      .map((part) => Number(part.trim()))
      .filter((day) => day >= 1 && day <= 7);
    for (const day of parsed.length > 0 ? parsed : [1, 2, 3, 4, 5]) pickedDays.add(day);
  } else {
    if (prompt) prompt.value = "";
    if (freq) freq.value = "daily";
    setScheduleTime("08:00");
    pickedDays.clear();
    for (const day of [1, 2, 3, 4, 5]) pickedDays.add(day);
  }

  if (days) {
    days.hidden = (freq?.value || "daily") !== "weekly";
  }
  syncFormHeaderAndSave();
  paintDays();
  paintClientSelect();

  formEl()?.removeAttribute("hidden");
  paintSchedules();
  resizePromptTextarea();
  formEl()?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  prompt?.focus();

  if (!ownClientId || !ownComputerName) {
    void ensureOwnClientInfo().then(() => {
      paintClientSelect();
      paintGuard();
    });
  }
}

function closeForm(): void {
  editingSchedule = null;
  pickedClientId = null;
  prePolishDraft = null;
  closeTimePicker();
  formEl()?.setAttribute("hidden", "true");
  const prompt = promptEl();
  if (prompt) {
    prompt.value = "";
    prompt.style.height = "";
  }
  syncPolishControls();
  paintSchedules();
}

async function submitForm(event: Event): Promise<void> {
  event.preventDefault();
  if (formSaving) return;
  const prompt = (promptEl()?.value || "").trim();
  if (!prompt) {
    deps.setStatus(t("chat.schedulesNeedPrompt"), "warn");
    promptEl()?.focus();
    return;
  }
  const freq = freqEl()?.value || "daily";
  if (freq === "weekly" && pickedDays.size === 0) {
    deps.setStatus(t("chat.schedulesNeedDay"), "warn");
    return;
  }
  const hhmm = currentHhmm();
  const weekdays = freq === "weekly" ? [...pickedDays].sort((a, b) => a - b).join(",") : "";
  const tz = Intl.DateTimeFormat().resolvedOptions().timeZone || "Asia/Shanghai";

  formSaving = true;
  syncFormHeaderAndSave();
  try {
    await ensureOwnClientInfo();
    const select = clientEl();
    const rawValue = select ? select.value : effectiveSelectedClientId();
    const clientId = rawValue === SELF_PLACEHOLDER ? ownClientId : rawValue;
    const pickedPc = clientId ? computersCache.find((pc) => pc.id === clientId) : undefined;
    const clientName =
      pickedPc?.name ||
      (clientId && clientId === ownClientId ? ownComputerName : "") ||
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
    deps.setStatus(friendlyScheduleError(error, "chat.schedulesSaveFailed"), "error");
    paintSchedules();
  } finally {
    formSaving = false;
    syncFormHeaderAndSave();
  }
}

function paintManageChrome(): void {
  const bind = document.querySelector<HTMLElement>("#chat-schedules-bind");
  if (bind) bind.textContent = t("chat.schedulesRunOn");
  const promptBox = promptEl();
  if (promptBox) promptBox.placeholder = t("chat.schedulesPromptPlaceholder");
  syncFormHeaderAndSave();
  paintClientSelect();
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
  paintManageChrome();
  paintSchedules();
  void loadSchedules({ silent: schedulesLoaded });
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
    paintManageChrome();
    paintSchedules();
    if (daysEl() && !daysEl()!.hidden) paintDays();
  }
}

export function bindSchedule(next: Deps): void {
  deps = next;
  void ensureOwnClientInfo().then(() => {
    paintClientSelect();
  });
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
  newEl()?.addEventListener("click", () => openForm());
  emptyNewEl()?.addEventListener("click", () => openForm());
  document
    .querySelector("#chat-schedules-cancel")
    ?.addEventListener("click", () => closeForm());
  polishBtnEl()?.addEventListener("click", () => void runPolishDraft());
  polishUndoEl()?.addEventListener("click", () => undoPolishDraft());
  timeBtnEl()?.addEventListener("click", (event) => {
    event.stopPropagation();
    toggleTimePicker();
  });
  timeDoneEl()?.addEventListener("click", () => closeTimePicker());
  timeCustomEl()?.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      const parsed = normalizeScheduleHhmm(timeCustomEl()?.value || "");
      if (parsed) {
        setScheduleTime(parsed);
        closeTimePicker();
      }
    } else if (event.key === "Escape") {
      event.preventDefault();
      closeTimePicker();
    }
  });
  timeCustomEl()?.addEventListener("blur", () => {
    const parsed = normalizeScheduleHhmm(timeCustomEl()?.value || "");
    if (parsed) setScheduleTime(parsed);
    else if (timeCustomEl()) timeCustomEl()!.value = currentHhmm();
  });
  document.addEventListener("pointerdown", (event) => {
    const popover = timePopoverEl();
    if (!popover || popover.hidden) return;
    const wrap = timePickerWrapEl();
    if (wrap && !wrap.contains(event.target as Node)) {
      closeTimePicker();
    }
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && timePopoverEl() && !timePopoverEl()!.hidden) {
      event.stopPropagation();
      closeTimePicker();
    }
  });
  promptEl()?.addEventListener("input", () => resizePromptTextarea());
  promptEl()?.addEventListener("keydown", (event) => {
    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
      event.preventDefault();
      formEl()?.requestSubmit();
    } else if (event.key === "Escape" && !(promptEl()?.value || "").trim()) {
      event.preventDefault();
      closeForm();
    }
  });
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
  clientEl()?.addEventListener("change", () => {
    pickedClientId = clientEl()?.value ?? null;
    paintGuard();
  });
  formEl()?.addEventListener("submit", (event) => void submitForm(event));
  void listen("teamups-account-changed", () => {
    loginWaiting = false;
    schedulesLoaded = false;
    if (isSchedulePageOpen()) void loadSchedules();
  });
}
