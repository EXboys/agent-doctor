import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { PromptSessionEvent } from "../chat/types";
import { t } from "../i18n";
import { normalizePlanItems, type PlanStep } from "../plan";
import { publishIslandSnapshot } from "../ipc";
import {
  buildIslandRows,
  emptyIslandTrack,
  islandSnapshot,
  rememberIslandSent,
  reduceIslandTrack,
  type IslandLabels,
  type IslandSessionSource,
  type IslandTrack,
} from "./track";

const TYPING_MS = 1200;
const LINGER_MS = 6000;

function labels(): IslandLabels {
  return {
    idle: t("island.idle"),
    browser: t("island.browser"),
    working: t("island.working"),
    needsConfirm: t("island.needsConfirm"),
    needsReply: t("island.needsReply"),
    needsAnswer: t("island.needsAnswer"),
  };
}

let rememberSent: (text: string) => void = () => {};

/** The sentence the user just sent, so the island can show it before a reply arrives. */
export function noteIslandUserText(text: string): void {
  rememberSent(text);
}

/** Tell the top island what this conversation is doing. Ask window only. */
export function startIslandPublisher(
  prompt: HTMLTextAreaElement,
  readFeed?: () => { activeId: string; attentionId: string; sessions: IslandSessionSource[] },
): void {
  let track: IslandTrack = emptyIslandTrack();
  let typingUntil = 0;
  let publishTimer = 0;
  let typingTimer = 0;
  let lingerTimer = 0;
  let lingerUntil = 0;
  let sawWork = false;
  let livePlan: PlanStep[] | null = null;

  rememberSent = (text) => {
    track = rememberIslandSent(track, text);
    schedule();
  };

  const publishNow = () => {
    const composing = Date.now() < typingUntil;
    const working = track.active || Boolean(track.pending);
    if (working) {
      sawWork = true;
      lingerUntil = 0;
    } else if (sawWork) {
      sawWork = false;
      lingerUntil = Date.now() + LINGER_MS;
    }
    const lingering = Date.now() < lingerUntil;
    const feed = readFeed?.() ?? { activeId: "", attentionId: "", sessions: [] };
    const runningId = feed.attentionId || feed.activeId;
    const rows = buildIslandRows(
      feed.sessions,
      feed.activeId,
      { sent: track.sent, spoken: track.spoken },
      {
        // pending.sessionId is the agent's own id, not the conversation id the rows use.
        needsYouId: track.pending ? runningId || track.pending.sessionId : "",
        workingId: track.active && !track.pending ? runningId : "",
      },
    );
    if (livePlan && runningId) {
      const row = rows.find((item) => item.id === runningId);
      if (row) row.plan = livePlan.length ? livePlan : undefined;
    }
    const snap = islandSnapshot(track, labels(), composing, rows);
    if (lingering && !working) snap.title = t("island.done");
    snap.lingering = lingering;
    void publishIslandSnapshot(snap).catch(() => {});
    window.clearTimeout(typingTimer);
    window.clearTimeout(lingerTimer);
    if (composing) {
      typingTimer = window.setTimeout(publishNow, Math.max(0, typingUntil - Date.now()) + 30);
    }
    if (lingering) {
      lingerTimer = window.setTimeout(publishNow, Math.max(0, lingerUntil - Date.now()) + 30);
    }
  };

  const schedule = () => {
    window.clearTimeout(publishTimer);
    publishTimer = window.setTimeout(publishNow, 40);
  };

  const markTyping = () => {
    typingUntil = Date.now() + TYPING_MS;
    schedule();
  };

  prompt.addEventListener("input", markTyping);
  prompt.addEventListener("keydown", markTyping);
  prompt.addEventListener("compositionstart", markTyping);

  void getCurrentWebviewWindow()
    .listen<PromptSessionEvent>("prompt-session-event", (event) => {
      if (event.payload.type === "plan") {
        livePlan = normalizePlanItems(event.payload.items);
      } else if (event.payload.type === "started") {
        livePlan = null;
      }
      track = reduceIslandTrack(track, event.payload);
      schedule();
    })
    .catch(() => {});
  schedule();
}
