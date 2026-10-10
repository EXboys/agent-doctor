import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { formatChatFailureLine, type ChatFailureExplain } from "../friendly-error";
import { getLocale, t } from "../i18n";
import type { PlanStep } from "../plan";
import { markTurnStarted } from "./activity";
import { preferPlainSummary } from "./format";
import {
  explainCurrentChatTurnFailure,
  markChatFailureBubbleShown,
  resetChatTurnErrors,
} from "./turn-errors";
import type { PromptSessionEvent, SessionStore, ToolStep } from "./types";

export type StreamDeps = {
  getStore: () => SessionStore;
  getRunningChatSessionId: () => string | null;
  setRunningBackendSessionId: (id: string | null) => void;
  getAssistantBubble: () => HTMLElement | null;
  setAssistantBubble: (el: HTMLElement | null) => void;
  getAssistantMessageId: () => string | null;
  setAssistantMessageId: (id: string | null) => void;
  getAssistantRaw: () => string;
  setAssistantRaw: (raw: string) => void;
  getPendingText: () => string;
  setPendingText: (text: string) => void;
  getTurnHadAssistantText: () => boolean;
  setTurnHadAssistantText: (v: boolean) => void;
  getUnseenCompletedSessionIds: () => Set<string>;
  isEventForCurrentRun: (sessionId: string | undefined) => boolean;
  resolveEventChatId: (payload: PromptSessionEvent) => string | null;
  isForegroundChat: (chatId: string) => boolean;
  applyBackgroundEvent: (chatId: string, payload: PromptSessionEvent) => void;
  setDisplayedCwd: (cwd: string) => void;
  pushActivity: (phase: string, message: string) => void;
  pushToolStep: (step: ToolStep) => void;
  flushSessionListRender: () => void;
  noteVerifyBrowserSignal: (text: string, source: "status" | "assistant" | "tool") => void;
  queueAssistantText: (text: string) => void;
  thinking: { append: (text: string) => void; seal: () => void; isLive: () => boolean };
  appendStderrLine: (line: string) => void;
  pushPermissionCard: (payload: {
    session_id: string;
    request_id: string;
    tool_name: string;
    detail: string;
    input_mode?: string;
    input_json?: string;
  }) => void;
  markPermissionResolved: (requestId: string, allowed: boolean) => void;
  isViewingRunningSession: () => boolean;
  flushPendingTextSync: () => void;
  appendAssistantChunk: (chunk: string) => void;
  clearEphemeralActivity: (dropStderr?: boolean) => void;
  sealAssistantBubble: () => void;
  expireLivePermissionCards: () => void;
  hideDecisionDock: () => void;
  appendBubble: (
    kind: "assistant" | "user" | "meta" | "permission",
    text: string,
    opts?: { id?: string; persist?: boolean },
  ) => HTMLElement;
  appendChatFailure: (explain: ChatFailureExplain) => void;
  reportVerifyMcpIfNeeded: () => void;
  applyVerifyMcpFooter: () => void;
  flushStorePersist: () => void;
  setBusy: (next: boolean, chatSessionId?: string | null) => void;
  settleRunRouting: (chatSessionId?: string | null) => void;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  showQuickReplies: (sourceText: string) => void;
  renderSessionList: () => void;
  onPlan: (items: PlanStep[]) => void;
  onTurnCompleted: (text: string, status: string) => void;
  onPermissionNeeded: (payload: {
    session_id: string;
    request_id: string;
    tool_name: string;
    detail: string;
    input_mode?: string;
    input_json?: string;
  }) => void;
};

export type StreamApi = ReturnType<typeof createStreamController>;

type ListenerHolder = { __adPromptSessionUnlisten?: UnlistenFn };

export function createStreamController(deps: StreamDeps) {
  let listening: Promise<void> | null = null;

  function ensureListener(): Promise<void> {
    listening ??= attachListener().catch((error) => {
      listening = null;
      throw error;
    });
    return listening;
  }

  async function attachListener(): Promise<void> {
    // One listener per window: a second one would paint every reply chunk twice.
    // A hot-reloaded copy of this module must drop the old copy's listener.
    const holder = window as unknown as ListenerHolder;
    holder.__adPromptSessionUnlisten?.();
    holder.__adPromptSessionUnlisten = undefined;
    const unlisten = await getCurrentWebviewWindow().listen<PromptSessionEvent>("prompt-session-event", (event) => {
      const payload = event.payload;
      const chatId = deps.resolveEventChatId(payload);
      if (!chatId) return;
      if (!deps.isForegroundChat(chatId)) {
        deps.applyBackgroundEvent(chatId, payload);
        return;
      }
      const thinkingStatus = payload.type === "status" && payload.phase === "thinking";
      if (payload.type !== "thinking" && payload.type !== "stderr_line" && !thinkingStatus) {
        deps.thinking.seal();
      }
      switch (payload.type) {
        case "started":
          resetChatTurnErrors();
          markTurnStarted();
          deps.setRunningBackendSessionId(payload.session_id);
          deps.setDisplayedCwd(payload.cwd);
          deps.setAssistantBubble(null);
          deps.setAssistantMessageId(null);
          deps.setAssistantRaw("");
          deps.setPendingText("");
          deps.setTurnHadAssistantText(false);
          deps.pushActivity("think", t("chat.waitingModel"));
          deps.flushSessionListRender();
          break;
        case "status":
          if (!(thinkingStatus && deps.thinking.isLive())) {
            deps.pushActivity(payload.phase, payload.message);
          }
          deps.noteVerifyBrowserSignal(payload.message, "status");
          break;
        case "thinking":
          deps.thinking.append(payload.text);
          break;
        case "delta":
          deps.queueAssistantText(payload.text);
          deps.noteVerifyBrowserSignal(payload.text, "assistant");
          break;
        case "stdout_line":
          deps.queueAssistantText(`${payload.line}\n`);
          deps.noteVerifyBrowserSignal(payload.line, "assistant");
          break;
        case "stderr_line":
          deps.appendStderrLine(payload.line);
          break;
        case "permission_request":
          deps.pushPermissionCard(payload);
          deps.noteVerifyBrowserSignal(payload.tool_name, "tool");
          deps.onPermissionNeeded(payload);
          break;
        case "permission_resolved":
          deps.markPermissionResolved(payload.request_id, payload.allowed);
          break;
        case "tool":
          deps.pushToolStep(payload.step);
          break;
        case "plan":
          deps.onPlan(payload.items);
          break;
        case "completed": {
          const completedSessionId = chatId;
          const viewing = deps.getStore().activeId === completedSessionId;
          deps.flushPendingTextSync();
          // Fallback only when this turn never streamed assistant text.
          if (!deps.getTurnHadAssistantText() && !deps.getAssistantRaw().trim() && payload.summary?.trim()) {
            const fallback = preferPlainSummary(payload.summary);
            if (fallback) deps.appendAssistantChunk(fallback);
          }
          deps.noteVerifyBrowserSignal(deps.getAssistantRaw(), "assistant");
          const hadAssistantText = deps.getTurnHadAssistantText();
          const finalAssistantText = deps.getAssistantRaw();
          if (viewing) {
            deps.clearEphemeralActivity(payload.status === "succeeded");
          }
          deps.sealAssistantBubble();
          deps.expireLivePermissionCards();
          if (viewing) {
            deps.hideDecisionDock();
            if (!hadAssistantText && payload.status === "succeeded") {
              if (!showChatTurnFailure(deps)) {
                deps.appendBubble("meta", t("chat.emptyReply"), { persist: false });
              }
            } else if (
              payload.status !== "succeeded" &&
              payload.status !== "cancelled" &&
              payload.status !== "timed_out"
            ) {
              showChatTurnFailure(deps);
            }
          }
          deps.reportVerifyMcpIfNeeded();
          deps.applyVerifyMcpFooter();
          deps.flushStorePersist();
          deps.setBusy(false, completedSessionId);
          deps.settleRunRouting(completedSessionId);
          if (!viewing && completedSessionId) {
            deps.getUnseenCompletedSessionIds().add(completedSessionId);
          }
          if (payload.status === "cancelled") {
            deps.setStatus(t("chat.forceStopped"), "warn");
          } else if (payload.status === "timed_out") {
            const notice = timedOutNotice(payload.timeout);
            if (viewing) {
              deps.appendBubble("meta", notice, { persist: false });
            }
            deps.setStatus(notice, "warn");
          } else if (payload.status !== "succeeded") {
            if (viewing) {
              deps.appendBubble(
                "meta",
                t("chat.completed", {
                  status: payload.status,
                  code: payload.exit_code == null ? "—" : String(payload.exit_code),
                }),
                { persist: false },
              );
            }
            deps.setStatus(
              t("chat.completed", {
                status: payload.status,
                code: payload.exit_code == null ? "—" : String(payload.exit_code),
              }),
              "error",
            );
          } else if (finalAssistantText.trim() && viewing) {
            deps.showQuickReplies(finalAssistantText);
          } else if (!viewing) {
            deps.setStatus(t("chat.doneElsewhere"), "ok");
          }
          deps.onTurnCompleted(finalAssistantText, payload.status);
          deps.renderSessionList();
          break;
        }
      }
    });
    holder.__adPromptSessionUnlisten = unlisten;
  }

  return { ensureListener };
}

function spanLabel(sec: number): string {
  const zh = getLocale() === "zh";
  const whole = Math.max(0, Math.round(sec));
  if (whole < 90) {
    const n = Math.max(1, whole);
    return zh ? `${n} 秒` : n === 1 ? "1 second" : `${n} seconds`;
  }
  const minutes = Math.round(whole / 60);
  if (minutes < 90) {
    return zh ? `${minutes} 分钟` : minutes === 1 ? "1 minute" : `${minutes} minutes`;
  }
  const hours = Math.max(1, Math.round(minutes / 60));
  return zh ? `${hours} 小时` : hours === 1 ? "1 hour" : `${hours} hours`;
}

function showChatTurnFailure(
  deps: StreamDeps,
  explain?: ChatFailureExplain | null,
): boolean {
  const resolved = explain ?? explainCurrentChatTurnFailure();
  if (!resolved || !markChatFailureBubbleShown()) {
    return false;
  }
  deps.appendChatFailure(resolved);
  deps.setStatus(formatChatFailureLine(resolved), "error");
  return true;
}

function timedOutNotice(
  note: { kind: string; quiet_sec: number; elapsed_sec: number; last_tool: string } | null | undefined,
): string {
  const tool = note?.last_tool.trim() ?? "";
  if (!note || !Number.isFinite(note.quiet_sec)) return t("chat.timedOut");
  if (note.kind === "absolute") {
    const elapsed = spanLabel(note.elapsed_sec);
    return tool ? t("chat.timedOutLongTool", { elapsed, tool }) : t("chat.timedOutLong", { elapsed });
  }
  const quiet = spanLabel(note.quiet_sec);
  if (note.kind === "tool" && tool) return t("chat.timedOutTool", { quiet, tool });
  if (tool) return t("chat.timedOutAfterTool", { quiet, tool });
  return t("chat.timedOutQuiet", { quiet });
}
