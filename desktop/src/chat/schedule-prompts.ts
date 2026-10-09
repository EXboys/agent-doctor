import { t, type MessageKey } from "../i18n";

/** Scene cards for the schedule page. The prompt is filled into the composer, not shown on the card. */
export const SCHEDULE_SCENES = ["morning", "wrap", "weekly", "watch", "once", "repeat"] as const;

export type ScheduleScene = (typeof SCHEDULE_SCENES)[number];

const SCENE_KEYS: Record<
  ScheduleScene,
  { title: MessageKey; intro: MessageKey; when: MessageKey; what: MessageKey }
> = {
  morning: {
    title: "chat.scheduleMorning",
    intro: "chat.scheduleMorningIntro",
    when: "chat.scheduleMorningWhen",
    what: "chat.scheduleMorningWhat",
  },
  wrap: {
    title: "chat.scheduleWrap",
    intro: "chat.scheduleWrapIntro",
    when: "chat.scheduleWrapWhen",
    what: "chat.scheduleWrapWhat",
  },
  weekly: {
    title: "chat.scheduleWeekly",
    intro: "chat.scheduleWeeklyIntro",
    when: "chat.scheduleWeeklyWhen",
    what: "chat.scheduleWeeklyWhat",
  },
  watch: {
    title: "chat.scheduleWatch",
    intro: "chat.scheduleWatchIntro",
    when: "chat.scheduleWatchWhen",
    what: "chat.scheduleWatchWhat",
  },
  once: {
    title: "chat.scheduleOnce",
    intro: "chat.scheduleOnceIntro",
    when: "chat.scheduleOnceWhen",
    what: "chat.scheduleOnceWhat",
  },
  repeat: {
    title: "chat.scheduleRepeat",
    intro: "chat.scheduleRepeatIntro",
    when: "chat.scheduleRepeatWhen",
    what: "chat.scheduleRepeatWhat",
  },
};

export function scheduleSceneTitle(scene: ScheduleScene): string {
  return t(SCENE_KEYS[scene].title);
}

export function scheduleSceneIntro(scene: ScheduleScene): string {
  return t(SCENE_KEYS[scene].intro);
}

/** How this agent should create the job. Slash names are hints for the agent, not a command the app runs. */
export function scheduleHow(runtime: string): string {
  if (runtime === "hermes") return t("chat.scheduleHowHermes");
  if (runtime === "openclaw") return t("chat.scheduleHowOpenclaw");
  if (runtime === "claude-code") return t("chat.scheduleHowClaude");
  return t("chat.scheduleHowOther");
}

export function schedulePrompt(scene: ScheduleScene, runtime: string): string {
  const keys = SCENE_KEYS[scene];
  return t("chat.schedulePrompt", {
    when: t(keys.when),
    what: t(keys.what),
    how: scheduleHow(runtime),
  });
}
