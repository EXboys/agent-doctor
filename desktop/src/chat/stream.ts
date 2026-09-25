import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { t } from "../i18n";
import { preferPlainSummary } from "./format";
import type { PromptSessionEvent, SessionStore } from "./types";

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
  setDisplayedCwd: (cwd: string) => void;
  pushActivity: (phase: string, message: string) => void;
  flushSessionListRender: () => void;
  noteVerifyBrowserSignal: (text: string, source: "status" | "assistant" | "tool") => void;
  queueAssistantText: (text: string) => void;
  appendStderrLine: (line: string) => void;
  pushPermissionCard: (payload: {
    session_id: string;
    request_id: string;
    tool_name: string;
    detail: string;
  }) => void;
  markPermissionResolved: (requestId: string, allowed: boolean) => void;
  isViewingRunningSession: () => boolean;
  flushPendingTextSync: () => void;
  appendAssistantChunk: (chunk: string) => void;
  clearEphemeralActivity: () => void;
  sealAssistantBubble: () => void;
  expireLivePermissionCards: () => void;
  hideDecisionDock: () => void;
  appendBubble: (
    kind: "assistant" | "user" | "meta" | "permission",
    text: string,
    opts?: { id?: string; persist?: boolean },
  ) => HTMLElement;
  reportVerifyMcpIfNeeded: () => void;
  applyVerifyMcpFooter: () => void;
  flushStorePersist: () => void;
  setBusy: (next: boolean, chatSessionId?: string | null) => void;
  settleRunRouting: () => void;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  showQuickReplies: (sourceText: string) => void;
  renderSessionList: () => void;
  onTurnCompleted: (text: string, status: string) => void;
  onPermissionNeeded: () => void;
};

export type StreamApi = ReturnType<typeof createStreamController>;

export function createStreamController(deps: StreamDeps) {
  let unlisten: UnlistenFn | null = null;

  async function ensureListener(): Promise<void> {
    if (unlisten) return;
    unlisten = await listen<PromptSessionEvent>("prompt-session-event", (event) => {
      const payload = event.payload;
      const eventSessionId = "session_id" in payload ? payload.session_id : undefined;
      if (payload.type !== "started" && !deps.isEventForCurrentRun(eventSessionId)) {
        return;
      }
      switch (payload.type) {
        case "started":
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
          deps.pushActivity(payload.phase, payload.message);
          deps.noteVerifyBrowserSignal(payload.message, "status");
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
          deps.onPermissionNeeded();
          break;
        case "permission_resolved":
          deps.markPermissionResolved(payload.request_id, payload.allowed);
          break;
        case "completed": {
          const completedSessionId = deps.getRunningChatSessionId();
          const viewing = deps.isViewingRunningSession() || deps.getStore().activeId === completedSessionId;
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
            deps.clearEphemeralActivity();
          }
          deps.sealAssistantBubble();
          deps.expireLivePermissionCards();
          if (viewing) {
            deps.hideDecisionDock();
            if (!hadAssistantText) {
              deps.appendBubble("meta", t("chat.emptyReply"), { persist: false });
            }
          }
          deps.reportVerifyMcpIfNeeded();
          deps.applyVerifyMcpFooter();
          deps.flushStorePersist();
          deps.setBusy(false);
          deps.settleRunRouting();
          if (!viewing && completedSessionId) {
            deps.getUnseenCompletedSessionIds().add(completedSessionId);
          }
          if (payload.status === "cancelled") {
            deps.setStatus(t("chat.forceStopped"), "warn");
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
  }

  return { ensureListener };
}
