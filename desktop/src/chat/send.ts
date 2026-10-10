
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
import { appendKnowledgeToPrompt } from "./knowledge-page";
import {
  explainChatFailure,
  formatChatFailureLine,
  type ChatFailureExplain,
  withErrorDetail,
} from "../friendly-error";
import { pushChatTurnError } from "./turn-errors";
import type {
  ChatAttachment,
  ChatMessage,
  ChatSession,
  PromptSessionReport,
  SessionStore,
} from "./types";
import { noteIslandUserText } from "../island/publish";
import { askImageSupport, cancelPromptSession, readImageTexts, startPromptSession } from "../ipc";
import { MAX_PARALLEL_RUNS } from "./live-runs";
import { planPictures, type PictureInput, type PictureTurn } from "./picture-route";
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
  /** When set, a chat can run while other chats are still working. */
  isChatRunning?: (id: string) => boolean;
  runningCount?: () => number;
  getRunningChatSessionId: () => string | null;
  getPendingAttachments: () => ChatAttachment[];
  setPendingAttachments: (items: ChatAttachment[]) => void;
  resolveSendWorkspace: (session: ChatSession) => {
    cwd: string | null;
    workspaceName: string | null;
  };
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
  appendChatFailure?: (explain: ChatFailureExplain) => void;
  autoResizePrompt: () => void;
  renderPendingAttachments: () => void;
  buildPromptWithHistory: (
    text: string,
    attachments: ChatAttachment[],
    chatSessionId: string,
    pictures?: PictureTurn,
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
  settleRunRouting: (chatSessionId?: string | null) => void;
  renderSessionList: () => void;
  readImageTextEnabled: () => boolean;
  refreshComposer: () => void;
  /** Which provider answers now; empty when the app does not choose one. */
  providerTag?: () => string;
};

export type SendApi = ReturnType<typeof createSendController>;

function appendUserOnSession(
  session: ChatSession,
  text: string,
  attachments: ChatAttachment[],
): ChatMessage | null {
  if (!session.messages) return null;
  const message: ChatMessage = {
    id: crypto.randomUUID(),
    role: "user",
    content: text,
    at: Date.now(),
    attachments: attachments.length ? attachments : undefined,
  };
  session.messages.push(message);
  if (!session.title?.trim()) {
    const seed = text.trim() || attachments[0]?.name || "";
    session.title = seed.split(/\n/)[0].slice(0, 48);
  }
  return message;
}

const PICTURE_REJECTED =
  /(not|n't|no)\s+(support|accept|allow)\w*\s+(image|picture|vision)|image\w*\s+(is|are)?\s*not\s+(supported|allowed)|unsupported\s+(image|content type)|multimodal|不支持(图片|图像|多模态)/i;

function rejectedPictures(report: PromptSessionReport): boolean {
  if (report.status !== "failed") return false;
  return PICTURE_REJECTED.test(`${report.summary ?? ""}\n${report.log_excerpt ?? ""}`);
}

export function createSendController(deps: SendDeps) {
  /** Ask whether the model sees pictures, and read their words when that is on. */
  async function readPictures(
    items: ChatAttachment[],
    runtime: string,
  ): Promise<{ length: number; inputs: PictureInput[]; support: { seesImages: boolean; formats: string[] } }> {
    let support = { seesImages: false, formats: [] as string[] };
    if (items.length === 0) return { length: 0, inputs: [], support };
    try {
      const found = await askImageSupport({ runtime });
      if (found) support = { seesImages: Boolean(found.sees_images), formats: found.formats ?? [] };
    } catch {
      /* treat as text-only */
    }
    const texts = items.map(() => "");
    // Always read locally first: wordy shots go as text, charts go as pictures.
    // The toggle still lets people skip reading (then every sendable shot is a picture).
    if (deps.readImageTextEnabled()) {
      deps.setStatus(t("chat.readingImages"), "muted");
      deps.pushActivity("think", t("chat.readingImages"));
      try {
        const report = await readImageTexts({ paths: items.map((item) => item.path) });
        (report.readings ?? []).forEach((item, i) => {
          if (i < texts.length && item.ok) texts[i] = item.text ?? "";
        });
      } catch {
        /* send without words */
      }
    }
    return {
      length: items.length,
      inputs: items.map((item, i) => ({ path: item.path, name: item.name, text: texts[i] })),
      support,
    };
  }

  function notePictures(turn: PictureTurn, runtime: string): void {
    if (turn.unseenNames.length > 0) {
      deps.setStatus(t("chat.picturesUnseen"), "warn");
      deps.pushActivity("info", t("chat.picturesUnseen"));
      return;
    }
    if (turn.sendPaths.length > 0) {
      deps.pushActivity("info", t("chat.picturesSent", { n: String(turn.sendPaths.length) }));
    } else if (turn.readings.length > 0) {
      deps.pushActivity("info", t("chat.picturesAsText"));
    }
    deps.setStatus(t("chat.running", { runtime }), "muted");
  }

  async function cancelAsk(): Promise<void> {
    holdFollowUps();
    const gen = deps.getBusyGen();
    deps.clearQuickReplies();
    try {
      const stopped = await cancelPromptSession(deps.getStore().activeId);
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

  function targetIsRunning(targetId: string): boolean {
    if (deps.isChatRunning) return deps.isChatRunning(targetId);
    if (!deps.getBusy()) return false;
    const runningId = deps.getRunningChatSessionId();
    return !runningId || runningId === targetId;
  }

  async function sendAsk(opts?: {
    verifyMcp?: boolean;
    fromVoice?: boolean;
    draft?: FollowDraft;
  }): Promise<void> {
    const targetId = opts?.draft?.sessionId || deps.getStore().activeId;
    if (targetIsRunning(targetId)) {
      if (!opts?.draft) {
        const queued = draftFromComposer(opts);
        if (!queued.text && queued.attachments.length === 0 && queued.mentions.length === 0) {
          deps.setStatus(t("chat.emptyPrompt"), "warn");
          return;
        }
        enqueueFollowUp(targetId, queued);
        clearComposer();
        deps.setStatus(t("chat.queuedStatus"), "muted");
        deps.refreshComposer();
        return;
      }
      enqueueFollowUp(opts.draft.sessionId, opts.draft);
      return;
    }
    if ((deps.runningCount?.() ?? 0) >= MAX_PARALLEL_RUNS) {
      deps.setStatus(t("chat.tooManyRunning", { n: String(MAX_PARALLEL_RUNS) }), "warn");
      return;
    }

    const draft = opts?.draft ?? draftFromComposer(opts);
    const text = draft.text;
    const attachments = draft.attachments;
    if (!text && attachments.length === 0 && draft.mentions.length === 0) {
      deps.setStatus(t("chat.emptyPrompt"), "warn");
      return;
    }

    const elevated = deps.elevatedEl.checked;
    if (elevated && !draft.fromVoice && !opts?.draft && !window.confirm(t("chat.elevatedConfirm"))) return;

    const chatSessionId = draft.sessionId || deps.getStore().activeId;
    const foreground = chatSessionId === deps.getStore().activeId;
    const here = () => deps.getStore().activeId === chatSessionId;
    const sendSessionEarly = deps.sessionById(chatSessionId) ?? deps.activeSession();
    const runtimeForTurn = foreground ? deps.selectedRuntime() : sendSessionEarly.runtime;
    const providerTag = deps.providerTag?.() ?? "";
    if (
      providerTag &&
      sendSessionEarly.runtimeThreadId?.trim() &&
      sendSessionEarly.providerTag !== providerTag
    ) {
      // The saved thread holds the old provider's replies. Resuming it on another
      // provider can be refused outright, so carry the words over as text instead.
      sendSessionEarly.runtimeThreadId = null;
    }
    const resumeThreadId = sendSessionEarly.runtimeThreadId?.trim() || null;

    if (foreground) {
      deps.setVerifyMcpTurn(Boolean(draft.verifyMcp));
      deps.setVerifySawBrowserNavigate(false);
      deps.setVerifyMcpReported(false);
      deps.setVerifyTurnText("");
    }

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

    if (foreground) deps.allowQuickRepliesAgain();
    // Claim this chat before the first await, so a second send sees it and queues.
    deps.setBusy(true, chatSessionId);
    try {
    if (foreground) {
      deps.clearQuickReplies();
      deps.setAssistantBubble(null);
      deps.setAssistantMessageId(null);
      deps.setAssistantRaw("");
      deps.setPendingText("");
      deps.setTurnHadAssistantText(false);
    }
    const userMessage = foreground
      ? deps.persistMessage("user", userText, { attachments })
      : appendUserOnSession(sendSessionEarly, userText, attachments);
    if (foreground && userMessage) {
      deps.appendBubble("user", userText, { id: userMessage.id, persist: false, attachments });
    } else if (userMessage) {
      deps.touchSession(sendSessionEarly);
      deps.saveStore();
    }
    if (!opts?.draft && foreground) clearComposer();
    if (here()) deps.setStatus(t("chat.running", { runtime: runtimeForTurn }), "muted");
    if (foreground) deps.pushActivity("think", t("chat.waitingModel"));

    await deps.ensureListener();
    if (foreground && deps.getVerifyMcpTurn()) {
      deps.pushActivity("info", t("chat.verifyMcpWatching"));
    }

    const pictureInputs = await readPictures(
      attachments.filter((item) => item.kind === "image"),
      runtimeForTurn,
    );
    let pictures = pictureInputs.length
      ? planPictures(pictureInputs.inputs, pictureInputs.support)
      : undefined;
    if (pictures && foreground) notePictures(pictures, runtimeForTurn);

    const sendSession = deps.sessionById(chatSessionId) ?? deps.activeSession();
    const { cwd: sessionCwd, workspaceName: sessionWorkspace } =
      deps.resolveSendWorkspace(sendSession);

    const promptWithKnowledge = await appendKnowledgeToPrompt(promptUserText, sessionWorkspace?.trim() || null);
    const startRound = (turn: PictureTurn | undefined) =>
      startPromptSession({
        runtime: runtimeForTurn,
        prompt: deps.buildPromptWithHistory(
          promptWithKnowledge,
          attachments,
          chatSessionId,
          turn,
        ),
        cwd: sessionCwd?.trim() || null,
        workspaceName: sessionWorkspace?.trim() || null,
        timeoutSec: 86_400,
        dangerouslySkipPermissions:
          (runtimeForTurn === "claude-code" ||
            runtimeForTurn === "hermes" ||
            runtimeForTurn === "deepseek-harness") &&
          elevated,
        fullAuto: (runtimeForTurn === "codex" || runtimeForTurn === "openclaw") && elevated,
        resumeThreadId,
        selectedMcps,
        imagePaths: turn?.sendPaths ?? [],
        clientRunId: chatSessionId,
        providerId: sendSession.providerId?.trim() || null,
        model: sendSession.model?.trim() || null,
      });

    const stillGoing = () => (deps.isChatRunning ? deps.isChatRunning(chatSessionId) : deps.getBusy());

      // Stop during picture reading clears this chat's run before it exists.
      // Starting anyway would ignore that click.
      if (!stillGoing()) return;
      let report = await startRound(pictures);
      if (pictures?.sendPaths.length && rejectedPictures(report)) {
        // The provider said no to pictures. Send the words once instead.
        pictures = planPictures(pictureInputs.inputs, { seesImages: false, formats: [] });
        deps.setStatus(t("chat.picturesRetryText"), "warn");
        deps.pushActivity("info", t("chat.picturesRetryText"));
        deps.setAssistantBubble(null);
        deps.setAssistantMessageId(null);
        deps.setAssistantRaw("");
        deps.setPendingText("");
        deps.setTurnHadAssistantText(false);
        if (stillGoing()) report = await startRound(pictures);
      }
      if (here()) deps.setDisplayedCwd(report.cwd);
      const session = deps.sessionById(chatSessionId) ?? deps.runTargetSession();
      session.interrupted =
        report.status === "succeeded" ? null : { status: report.status, at: Date.now() };
      deps.touchSession(session);
      deps.saveStore();
      if (report.runtime_thread_id?.trim()) {
        session.runtimeThreadId = report.runtime_thread_id.trim();
        session.providerTag = providerTag || null;
        deps.touchSession(session);
        deps.saveStore();
      } else if (resumeThreadId && report.status === "failed") {
        // The saved session could not be restored. Drop it so the next send
        // starts clean instead of sitting on “正在恢复 Codex 会话…”.
        session.runtimeThreadId = null;
        deps.touchSession(session);
        deps.saveStore();
      }
      if (foreground && deps.getVerifyMcpTurn() && here()) {
        deps.applyVerifyMcpFooter();
      } else if (report.status === "succeeded" && here()) {
        deps.setStatus("");
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
          await cancelPromptSession(chatSessionId);
        } catch {
          /* ignore */
        }
        deps.setStatus(t("chat.forceStopped"), "warn");
        if (deps.getStore().activeId === chatSessionId) {
          deps.appendBubble("meta", t("chat.forceStopped"), { persist: false });
        }
      } else {
        pushChatTurnError(message);
        const explained = explainChatFailure(message);
        if (explained && deps.appendChatFailure && deps.getStore().activeId === chatSessionId) {
          deps.appendChatFailure(explained);
        } else {
          const failed =
            /session cwd does not exist/i.test(message)
              ? t("chat.failedCwd")
              : explained
                ? formatChatFailureLine(explained)
                : withErrorDetail(t("chat.failed"), error);
          deps.setStatus(failed, "error");
          if (deps.getStore().activeId === chatSessionId) {
            deps.appendBubble("meta", failed, { persist: false });
          }
        }
      }
    } finally {
      if (foreground && here()) {
        deps.applyVerifyEvidenceFromAssistant();
        if (deps.getVerifySawBrowserNavigate()) {
          deps.reportVerifyMcpIfNeeded();
          deps.applyVerifyMcpFooter();
        }
      }
      // Yield so any trailing `completed` / delta events from this invoke are
      // handled before we tear down this chat's run (avoids dropping the rest of the turn).
      await Promise.resolve();
      deps.setBusy(false, chatSessionId);
      if (here()) deps.expireLivePermissionCards();
      deps.settleRunRouting(chatSessionId);
      deps.renderSessionList();
      if (foreground) {
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
