import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { PromptSessionEvent } from "../chat/types";
import { t } from "../i18n";
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

  rememberSent = (text) => {
    track = rememberIslandSent(track, text);
    schedule();
  };

  const publishNow = () => {
    const composing = Date.now() < typingUntil;
    const feed = readFeed?.() ?? { activeId: "", attentionId: "", sessions: [] };
    const rows = buildIslandRows(
      feed.sessions,
      feed.activeId,
      { sent: track.sent, spoken: track.spoken },
      { needsYouId: track.pending ? feed.attentionId || feed.activeId : "" },
    );
    void publishIslandSnapshot(islandSnapshot(track, labels(), composing, rows)).catch(() => {});
    window.clearTimeout(typingTimer);
    if (!composing) return;
    typingTimer = window.setTimeout(publishNow, Math.max(0, typingUntil - Date.now()) + 30);
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
      track = reduceIslandTrack(track, event.payload);
      schedule();
    })
    .catch(() => {});
  schedule();
}
