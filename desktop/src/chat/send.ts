import { invoke } from "@tauri-apps/api/core";
import {
  AskMentionMenuController,
  AskResourcesController,
  buildMentionConstraint,
  ensureBrowserMention,
  mergeMentionsForSend,
  stripMentionTokens,
  type AskRuntime,
} from "../ask-resources";
import { t } from "../i18n";
import { withErrorDetail } from "../friendly-error";
import type {
  ChatAttachment,
  ChatMessage,
  ChatSession,
  PromptSessionReport,
  SessionStore,
} from "./types";

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
};

export type SendApi = ReturnType<typeof createSendController>;

export function createSendController(deps: SendDeps) {
  async function cancelAsk(): Promise<void> {
    const gen = deps.getBusyGen();
    deps.clearQuickReplies();
    try {
      const stopped = await invoke<boolean>("cancel_prompt_session_command");
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
  async function sendAsk(opts?: { verifyMcp?: boolean; fromVoice?: boolean }): Promise<void> {
    if (deps.getBusy()) {
      if (deps.getRunningChatSessionId() && deps.getRunningChatSessionId() !== deps.getStore().activeId) {
        deps.setStatus(t("chat.otherSessionRunning"), "warn");
      }
      return;
    }
    const text = deps.promptEl.value.trim();
    const attachments = [...deps.getPendingAttachments()];
    if (!text && attachments.length === 0 && deps.askResources.selectedMentions.length === 0) {
      deps.setStatus(t("chat.emptyPrompt"), "warn");
      return;
    }

    const runtime = deps.selectedRuntime();
    const elevated = deps.selectedRuntime() !== "deepseek-harness" && deps.elevatedEl.checked;
    if (elevated && !opts?.fromVoice && !window.confirm(t("chat.elevatedConfirm"))) return;

    const chatSessionId = deps.getStore().activeId;
    const resumeThreadId = deps.activeSession().runtimeThreadId?.trim() || null;

    deps.setVerifyMcpTurn(Boolean(opts?.verifyMcp));
    deps.setVerifySawBrowserNavigate(false);
    deps.setVerifyMcpReported(false);
    deps.setVerifyTurnText("");

    const mentions = ensureBrowserMention(
      mergeMentionsForSend(
        text,
        deps.askResources.selectedMentions,
        deps.askResources.mountedSkills,
        deps.askResources.enabledMcps,
      ),
      text,
      deps.askResources.enabledMcps,
    );
    const cleaned = stripMentionTokens(text);
    const userText = cleaned || text || t("chat.attachOnlyPrompt");
    const constraint = buildMentionConstraint(mentions);
    const promptUserText = constraint ? `${constraint}\n\n${userText}` : userText;
    const selectedMcps = mentions.filter((m) => m.kind === "mcp").map((m) => m.id);

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
    deps.promptEl.value = "";
    deps.mentionMenu.hideMentionMenu();
    deps.askResources.clearMentions();
    deps.autoResizePrompt();
    deps.setPendingAttachments([]);
    deps.renderPendingAttachments();
    deps.setStatus(t("chat.running", { runtime }), "muted");
    deps.pushActivity("think", t("chat.waitingModel"));

    const imagePaths = attachments.filter((item) => item.kind === "image").map((item) => item.path);
    let readings: import("./context").ImageReading[] = [];
    if (imagePaths.length > 0 && deps.readImageTextEnabled()) {
      deps.setStatus(t("chat.readingImages"), "muted");
      deps.pushActivity("think", t("chat.readingImages"));
      try {
        const report = await invoke<{
          readings: { name: string; text: string; ok: boolean }[];
        }>("read_image_texts_command", { paths: imagePaths });
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
      const report = await invoke<PromptSessionReport>("start_prompt_session_command", {
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
      if (/already running/i.test(message)) {
        try {
          await invoke<boolean>("cancel_prompt_session_command");
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
    }
  }

  return { cancelAsk, sendAsk };
}
