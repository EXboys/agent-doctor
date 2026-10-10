import { t } from "../i18n";
import { resolvePermissionSession } from "../ipc";
import {
  activityKind,
  isQuietPhase,
  isQuietStderr,
  preferPlainSummary,
  splitToolActivity,
  toolSignature,
} from "./format";
import { liveRun, type LiveRun } from "./live-runs";
import { storeToolStep } from "./tool-records";
import { uid } from "./store";
import type { ChatMessage, ChatSession, PendingPermission, PromptSessionEvent } from "./types";

const MAX_THINKING_CHARS = 12_000;

export type BackgroundRunCtx = {
  sessionById: (id: string) => ChatSession | undefined;
  touchSession: (session: ChatSession) => void;
  scheduleStorePersist: () => void;
  scheduleSessionListRender: () => void;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  getActiveId: () => string;
  endRun: (id: string) => void;
  noteUnseen: (id: string) => void;
  onPlan: (chatId: string, items: PromptSessionEvent & { type: "plan" }) => void;
  autoApprove: () => boolean;
};

export function applyBackgroundEvent(ctx: BackgroundRunCtx, chatId: string, payload: PromptSessionEvent): void {
  const session = ctx.sessionById(chatId);
  const run = liveRun(chatId);
  if (!session || !run) return;

  switch (payload.type) {
    case "started":
      ctx.scheduleSessionListRender();
      return;
    case "status":
      if (isQuietPhase(payload.phase) || payload.phase === "writing" || payload.phase === "permission") return;
      if (activityKind(payload.phase) === "tool") {
        rememberTool(session, run, payload.message);
        ctx.touchSession(session);
        ctx.scheduleStorePersist();
      }
      return;
    case "delta":
      upsertAssistant(session, run, payload.text);
      ctx.touchSession(session);
      ctx.scheduleStorePersist();
      return;
    case "stdout_line":
      upsertAssistant(session, run, `${payload.line}\n`);
      ctx.touchSession(session);
      ctx.scheduleStorePersist();
      return;
    case "thinking":
      upsertThinking(session, run, payload.text);
      ctx.touchSession(session);
      ctx.scheduleStorePersist();
      return;
    case "stderr_line": {
      const text = payload.line.trim();
      if (!text || isQuietStderr(text)) return;
      session.messages.push({ id: uid(), role: "meta", content: text, at: Date.now() });
      ctx.touchSession(session);
      ctx.scheduleStorePersist();
      return;
    }
    case "permission_request":
      notePermission(ctx, session, run, payload);
      return;
    case "permission_resolved":
      resolveStoredPermission(session, run, payload.request_id, payload.allowed);
      ctx.touchSession(session);
      ctx.scheduleStorePersist();
      ctx.scheduleSessionListRender();
      return;
    case "tool":
      if (storeToolStep(session.messages, payload.step, run.assistantMessageId, uid)) {
        ctx.touchSession(session);
        ctx.scheduleStorePersist();
      }
      return;
    case "plan":
      ctx.onPlan(chatId, payload);
      return;
    case "completed": {
      if (!run.turnHadAssistantText && !run.assistantRaw.trim() && payload.summary?.trim()) {
        const fallback = preferPlainSummary(payload.summary);
        if (fallback) upsertAssistant(session, run, fallback);
      }
      ctx.touchSession(session);
      ctx.scheduleStorePersist();
      if (ctx.getActiveId() !== chatId) ctx.noteUnseen(chatId);
      ctx.endRun(chatId);
      ctx.setStatus(payload.status === "succeeded" ? t("chat.doneElsewhere") : t("chat.doneElsewhere"), "ok");
      ctx.scheduleSessionListRender();
      return;
    }
    default:
      return;
  }
}

function rememberTool(session: ChatSession, run: LiveRun, text: string): void {
  const parts = splitToolActivity(text);
  const signature = toolSignature(text);
  const messages = session.messages;
  const tail = messages[messages.length - 1];
  const anchor = tail?.role === "assistant" ? messages[messages.length - 2] : tail;
  if (anchor?.role === "tool") {
    const previous = splitToolActivity(anchor.content);
    if (toolSignature(anchor.content) === signature) return;
    if (previous.summary === parts.summary && !previous.detail && parts.detail) {
      anchor.content = text;
      return;
    }
  }
  const toolMessage: ChatMessage = { id: uid(), role: "tool", content: text, at: Date.now() };
  if (tail?.role === "assistant" && tail.id === run.assistantMessageId) {
    messages.splice(messages.length - 1, 0, toolMessage);
    return;
  }
  messages.push(toolMessage);
}

function upsertAssistant(session: ChatSession, run: LiveRun, extra: string): void {
  if (!extra) return;
  run.assistantRaw += extra;
  if (extra.trim()) run.turnHadAssistantText = true;
  if (!run.assistantMessageId) {
    const message: ChatMessage = {
      id: uid(),
      role: "assistant",
      content: run.assistantRaw,
      at: Date.now(),
    };
    run.assistantMessageId = message.id;
    session.messages.push(message);
    return;
  }
  const message = session.messages.find((item) => item.id === run.assistantMessageId);
  if (message) message.content = run.assistantRaw;
}

function upsertThinking(session: ChatSession, run: LiveRun, chunk: string): void {
  if (!chunk) return;
  run.thinkingText = (run.thinkingText + chunk).slice(0, MAX_THINKING_CHARS);
  if (!run.thinkingMessageId) {
    const message: ChatMessage = {
      id: uid(),
      role: "thinking",
      content: run.thinkingText,
      at: Date.now(),
    };
    run.thinkingMessageId = message.id;
    session.messages.push(message);
    return;
  }
  const message = session.messages.find((item) => item.id === run.thinkingMessageId);
  if (message) message.content = run.thinkingText;
}

function notePermission(
  ctx: BackgroundRunCtx,
  session: ChatSession,
  run: LiveRun,
  payload: Extract<PromptSessionEvent, { type: "permission_request" }>,
): void {
  if (run.pendingPermissions.some((item) => item.requestId === payload.request_id)) return;
  const inputMode =
    payload.input_mode === "secret" || payload.input_mode === "line" || payload.input_mode === "options"
      ? payload.input_mode
      : "choice";
  if (inputMode === "choice" && ctx.autoApprove()) {
    void resolvePermissionSession({
      sessionId: payload.session_id,
      requestId: payload.request_id,
      allow: true,
    }).catch(() => {});
    return;
  }
  const message: ChatMessage = {
    id: uid(),
    role: "permission",
    content: payload.detail.trim() || payload.tool_name,
    at: Date.now(),
    permission: {
      requestId: payload.request_id,
      toolName: payload.tool_name,
      detail: payload.detail.trim() || payload.tool_name,
      backendSessionId: payload.session_id,
      allowed: null,
      inputMode,
      inputJson: payload.input_json,
    },
  };
  session.messages.push(message);
  const pending: PendingPermission = {
    sessionId: payload.session_id,
    requestId: payload.request_id,
    toolName: payload.tool_name,
    detail: payload.detail.trim() || payload.tool_name,
    messageId: message.id,
  };
  run.pendingPermissions.push(pending);
  ctx.touchSession(session);
  ctx.scheduleStorePersist();
  ctx.setStatus(t("chat.waitingPermissionElsewhere"), "warn");
  ctx.scheduleSessionListRender();
}

function resolveStoredPermission(session: ChatSession, run: LiveRun, requestId: string, allowed: boolean): void {
  const message = session.messages.find(
    (item) => item.role === "permission" && item.permission?.requestId === requestId,
  );
  if (message?.permission) message.permission.allowed = allowed;
  run.pendingPermissions = run.pendingPermissions.filter((item) => item.requestId !== requestId);
}
