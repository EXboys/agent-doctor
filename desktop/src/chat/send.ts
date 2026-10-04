
import {
  AskMentionMenuController,
  AskResourcesController,
  buildMentionConstraint,
  ensureBrowserMention,
  mergeMentionsForSend,
  stripMentionTokens,
  type AskRuntime,
} from "../ask-resources";
import { getLocale, t } from "../i18n";
import { withErrorDetail } from "../friendly-error";
import type {
  ChatAttachment,
  ChatMessage,
  ChatSession,
  SessionStore,
} from "./types";
import { noteIslandUserText } from "../island/publish";
import { cancelPromptSession, readImageTexts, startPromptSession } from "../ipc";
import {
  consumeDrainIntent,
  enqueueFollowUp,
  holdFollowUps,
  mergedFollowUp,
  takeFollowUps,
  type FollowDraft,
} from "./follow-queue";

export type SendDeps = {
  promptEl: HTMLTextAreaElement;
  elevatedEl: HTMLInputElement;
  askResources: AskResourcesController;
  mentionMenu: AskMentionMenuController;
  getStore: () => SessionStore;
  getBusy: () => boolean;
  getBusyGen: () => number;
  getRunningChatSessionId: () => string | null;
  getPendingAttachments: () => ChatAttachment[];
  setPendingAttachments: (items: ChatAttachment[]) => void;
  getWorkspaceCwd: () => string | null;
  getVerifyMcpTurn: () => boolean;
  setVerifyMcpTurn: (v: boolean) => void;
  getVerifySawBrowserNavigate: () => boolean;
  setVerifySawBrowserNavigate: (v: boolean) => void;
  getVerifyMcpReported: () => boolean;
  setVerifyMcpReported: (v: boolean) => void;
  getVerifyTurnText: () => string;
  setVerifyTurnText: (v: string) => void;
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
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  selectedRuntime: () => AskRuntime;
  activeSession: () => ChatSession;
  ensureListener: () => Promise<void>;
  clearQuickReplies: () => void;
  allowQuickRepliesAgain: () => void;
  setBusy: (next: boolean, chatSessionId?: string | null) => void;
  pushActivity: (phase: string, message: string) => void;
  persistMessage: (
    role: "user" | "assistant" | "meta" | "permission",
    content: string,
    opts?: { id?: string; attachments?: ChatAttachment[]; permission?: import("./types").PermissionMeta },
  ) => ChatMessage;
  appendBubble: (
    kind: "assistant" | "user" | "meta" | "permission",
    text: string,
    opts?: { id?: string; persist?: boolean; attachments?: ChatAttachment[] },
  ) => HTMLElement;
  autoResizePrompt: () => void;
  renderPendingAttachments: () => void;
  buildPromptWithHistory: (
    text: string,
    attachments: ChatAttachment[],
    chatSessionId: string,
    readings?: import("./context").ImageReading[],
  ) => string;
  setDisplayedCwd: (cwd: string) => void;
  sessionById: (id: string | null | undefined) => ChatSession | undefined;
  runTargetSession: () => ChatSession;
  touchSession: (session: ChatSession) => void;
  saveStore: () => void;
  applyVerifyMcpFooter: () => void;
  applyVerifyEvidenceFromAssistant: () => void;
  reportVerifyMcpIfNeeded: () => void;
  expireLivePermissionCards: () => void;
  settleRunRouting: () => void;
  renderSessionList: () => void;
  readImageTextEnabled: () => boolean;
  refreshComposer: () => void;
};

export type SendApi = ReturnType<typeof createSendController>;

export function createSendController(deps: SendDeps) {
  async function cancelAsk(): Promise<void> {
    holdFollowUps();
    const gen = deps.getBusyGen();
    deps.clearQuickReplies();
    try {
      const stopped = await cancelPromptSession();
      deps.setStatus(t("chat.cancelling"), "warn");
      deps.pushActivity("think", t("chat.cancelling"));
      if (!stopped && deps.getBusy() && deps.getBusyGen() === gen) {
        deps.setBusy(false);
        deps.expireLivePermissionCards();
        deps.settleRunRouting();
        deps.setStatus(t("chat.forceStopped"), "warn");
        return;
      }
      window.setTimeout(() => {
        if (deps.getBusy() && deps.getBusyGen() === gen) {
          deps.setBusy(false);
          deps.expireLivePermissionCards();
          deps.settleRunRouting();
          deps.setStatus(t("chat.forceStopped"), "warn");
        }
      }, 2500);
    } catch (error) {
      deps.setBusy(false);
      deps.expireLivePermissionCards();
      deps.settleRunRouting();
      deps.setStatus(withErrorDetail(t("chat.cancelFailed"), error), "error");
    }
  }
  function draftFromComposer(opts?: { verifyMcp?: boolean; fromVoice?: boolean }): FollowDraft {
    return {
      id: crypto.randomUUID(),
      sessionId: deps.getStore().activeId,
      text: deps.promptEl.value.trim(),
      attachments: [...deps.getPendingAttachments()],
      mentions: [...deps.askResources.selectedMentions],
      fromVoice: opts?.fromVoice,
      verifyMcp: opts?.verifyMcp,
    };
  }

  function clearComposer(): void {
    deps.promptEl.value = "";
    deps.mentionMenu.hideMentionMenu();
    deps.askResources.clearMentions();
    deps.autoResizePrompt();
    deps.setPendingAttachments([]);
    deps.renderPendingAttachments();
  }

  async function sendAsk(opts?: {
    verifyMcp?: boolean;
    fromVoice?: boolean;
    draft?: FollowDraft;
  }): Promise<void> {
    const sameRunningChat =
      !deps.getRunningChatSessionId() || deps.getRunningChatSessionId() === deps.getStore().activeId;
    if (deps.getBusy() && !opts?.draft) {
      if (!sameRunningChat) {
        deps.setStatus(t("chat.otherSessionRunning"), "warn");
        return;
      }
      const queued = draftFromComposer(opts);
      if (!queued.text && queued.attachments.length === 0 && queued.mentions.length === 0) {
        deps.setStatus(t("chat.emptyPrompt"), "warn");
        return;
      }
      enqueueFollowUp(deps.getStore().activeId, queued);
      clearComposer();
      deps.setStatus(t("chat.queuedStatus"), "muted");
      deps.refreshComposer();
      return;
    }
    if (deps.getBusy()) {
      if (opts?.draft) enqueueFollowUp(opts.draft.sessionId, opts.draft);
      return;
    }

    const draft = opts?.draft ?? draftFromComposer(opts);
    const text = draft.text;
    const attachments = draft.attachments;
    if (!text && attachments.length === 0 && draft.mentions.length === 0) {
      deps.setStatus(t("chat.emptyPrompt"), "warn");
      return;
    }

    const runtime = deps.selectedRuntime();
    const elevated = deps.selectedRuntime() !== "deepseek-harness" && deps.elevatedEl.checked;
    if (elevated && !draft.fromVoice && !opts?.draft && !window.confirm(t("chat.elevatedConfirm"))) return;

    const chatSessionId = draft.sessionId || deps.getStore().activeId;
    const resumeThreadId =
      (deps.sessionById(chatSessionId) ?? deps.activeSession()).runtimeThreadId?.trim() || null;

    deps.setVerifyMcpTurn(Boolean(draft.verifyMcp));
    deps.setVerifySawBrowserNavigate(false);
    deps.setVerifyMcpReported(false);
    deps.setVerifyTurnText("");

    const mentions = ensureBrowserMention(
      mergeMentionsForSend(
        text,
        draft.mentions,
        deps.askResources.mountedSkills,
        deps.askResources.enabledMcps,
      ),
      text,
      deps.askResources.enabledMcps,
    );
    const cleaned = stripMentionTokens(text);
    const userText = cleaned || text || t("chat.attachOnlyPrompt");
    noteIslandUserText(userText);
    const constraint = buildMentionConstraint(mentions);
    const voiceNote = draft.fromVoice
      ? getLocale() === "zh"
        ? "这是对方用语音说的、再转成文字发过来的，不是键盘打的。请直接回答这句话。不要说你听不到声音，也不要提醒对方只能打字。转写可能少几个字，按能看懂的意思回。\n\n"
        : "This message was spoken and turned into text. It was not typed. Answer it directly. Do not say you cannot hear them, and do not tell them to type. A few words may be missing; reply to the meaning you can follow.\n\n"
      : "";
    const promptBody = `${voiceNote}${userText}`;
    const promptUserText = constraint ? `${constraint}\n\n${promptBody}` : promptBody;
    const selectedMcps = mentions.filter((m) => m.kind === "mcp").map((m) => m.id);

    deps.allowQuickRepliesAgain();
    await deps.ensureListener();
    deps.clearQuickReplies();
    deps.setAssistantBubble(null);
    deps.setAssistantMessageId(null);
    deps.setAssistantRaw("");
    deps.setPendingText("");
    deps.setTurnHadAssistantText(false);
    deps.setBusy(true, chatSessionId);
    if (deps.getVerifyMcpTurn()) {
      deps.pushActivity("info", t("chat.verifyMcpWatching"));
    }
    const userMessage = deps.persistMessage("user", userText, { attachments });
    if (deps.getStore().activeId === chatSessionId) {
      deps.appendBubble("user", userText, { id: userMessage.id, persist: false, attachments });
    }
    if (!opts?.draft) clearComposer();
    deps.setStatus(t("chat.running", { runtime }), "muted");
    deps.pushActivity("think", t("chat.waitingModel"));

    const imagePaths = attachments.filter((item) => item.kind === "image").map((item) => item.path);
    let readings: import("./context").ImageReading[] = [];
    if (imagePaths.length > 0 && deps.readImageTextEnabled()) {
      deps.setStatus(t("chat.readingImages"), "muted");
      deps.pushActivity("think", t("chat.readingImages"));
      try {
        const report = await readImageTexts({ paths: imagePaths });
        readings = (report.readings ?? [])
          .filter((item) => item.ok && item.text.trim())
          .map((item) => ({ name: item.name, text: item.text }));
        if (readings.length === 0) {
          deps.setStatus(t("chat.readingImagesNone"), "warn");
        } else {
          deps.setStatus(t("chat.running", { runtime }), "muted");
        }
      } catch {
        readings = [];
        deps.setStatus(t("chat.readingImagesNone"), "warn");
      }
    }

    const prompt = deps.buildPromptWithHistory(
      promptUserText,
      attachments,
      chatSessionId,
      readings,
    );

    try {
      const report = await startPromptSession({
        runtime,
        prompt,
        cwd: deps.getWorkspaceCwd()?.trim() || null,
        timeoutSec: 600,
        dangerouslySkipPermissions:
          (runtime === "claude-code" || runtime === "hermes") && elevated,
        fullAuto: (runtime === "codex" || runtime === "openclaw") && elevated,
        resumeThreadId,
        selectedMcps,
      });
      deps.setDisplayedCwd(report.cwd);
      if (report.runtime_thread_id?.trim()) {
        const session = deps.sessionById(chatSessionId) ?? deps.runTargetSession();
        session.runtimeThreadId = report.runtime_thread_id.trim();
        deps.touchSession(session);
        deps.saveStore();
      }
      const tone =
        report.status === "succeeded" ? "ok" : report.status === "cancelled" ? "warn" : "error";
      if (deps.getVerifyMcpTurn()) {
        deps.applyVerifyMcpFooter();
      } else {
        deps.setStatus(t("chat.done", { status: report.status, ms: String(report.duration_ms) }), tone);
      }
    } catch (error) {
      const message = String(error);
      if (/busy in another window/i.test(message)) {
        // Diagnose owns the running turn — never cancel it from here.
        deps.setStatus(t("chat.otherWindowRunning"), "warn");
        if (deps.getStore().activeId === chatSessionId) {
          deps.appendBubble("meta", t("chat.otherWindowRunning"), { persist: false });
        }
        if (!deps.promptEl.value.trim()) {
          deps.promptEl.value = text;
          deps.autoResizePrompt();
        }
      } else if (/already running/i.test(message)) {
        try {
          await cancelPromptSession();
        } catch {
          /* ignore */
        }
        deps.setStatus(t("chat.forceStopped"), "warn");
        if (deps.getStore().activeId === chatSessionId) {
          deps.appendBubble("meta", t("chat.forceStopped"), { persist: false });
        }
      } else {
        const failed =
          /session cwd does not exist/i.test(message)
            ? t("chat.failedCwd")
            : withErrorDetail(t("chat.failed"), error);
        deps.setStatus(failed, "error");
        if (deps.getStore().activeId === chatSessionId) {
          deps.appendBubble("meta", failed, { persist: false });
        }
      }
    } finally {
      deps.applyVerifyEvidenceFromAssistant();
      if (deps.getVerifySawBrowserNavigate()) {
        deps.reportVerifyMcpIfNeeded();
        deps.applyVerifyMcpFooter();
      }
      // Yield so any trailing `completed` / delta events from this invoke are
      // handled before we tear down deps.getBusy() UI (avoids dropping the rest of the turn).
      await Promise.resolve();
      deps.setBusy(false);
      deps.expireLivePermissionCards();
      deps.settleRunRouting();
      deps.renderSessionList();
      const wasVerify = deps.getVerifyMcpTurn();
      if (wasVerify && !deps.getVerifyMcpReported()) {
        window.setTimeout(() => {
          deps.applyVerifyEvidenceFromAssistant();
          deps.reportVerifyMcpIfNeeded();
          deps.applyVerifyMcpFooter();
          deps.setVerifyMcpTurn(false);
        }, 80);
      } else {
        deps.setVerifyMcpTurn(false);
      }
      const intent = consumeDrainIntent();
      if (intent !== "hold") {
        const merged = mergedFollowUp(takeFollowUps(chatSessionId));
        if (merged) {
          await sendAsk({ draft: merged, fromVoice: merged.fromVoice, verifyMcp: merged.verifyMcp });
        }
      }
      deps.refreshComposer();
    }
  }

  return { cancelAsk, sendAsk };
}
