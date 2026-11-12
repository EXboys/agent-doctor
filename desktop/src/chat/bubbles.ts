import { t } from "../i18n";
import { reportFeedback } from "../ipc";
import { clearEmptyChatStart, sessionHasChatContent } from "./empty-start";
import { doctorStartCards } from "./doctor-start";
import { paintChatPlan, planStepsFromMessages } from "../plan";
import { markdownCutInfo, renderMarkdown, safeMarkdownCut } from "../markdown";
import { perfMark } from "./perf-probe";
import {
  cleanToolLabel,
  formatPermissionDetail,
  looksLikeToolPayloadJson,
  appendRunLog,
  agentFailedLine,
  coveredRunLogPointers,
  isLogMessage,
  isRunLogPointer,
  labelCurrentRunLog,
  markTurnLogFailed,
  runLogFailureLabel,
  mountFailedCommand,
  splitToolActivity,
  toolSignature,
} from "./format";
import { followChatLog, lockChatRowHeights, noteTurnUserAt, pinChatLog, renderToolHistoryGroup, withoutChatLogJump } from "./activity";
import { completedTrail } from "./work-trail";
import { markThinkingLive, renderThinkingBlock } from "./thinking";
import { linkFileMentions } from "./file-links";
import { appendRunLogMessage, omitBareToolStarts, storeToolStep } from "./tool-records";
import { wrapProcessRows } from "./work-trail";
import { scheduleQuestionRail, syncQuestionRail } from "./question-rail";
import {
  assistantMsgWrap,
  bubblePlainText,
  createCopyActionButton,
  enhanceCodeBlocks,
  releaseDeferredHighlights,
  renderAttachmentStrip,
  syncMessageCopyActions,
} from "./copy-ui";
import { HISTORY_BATCH, HISTORY_RUN_SLICE, historyRenderCut, snapRunBoundary } from "./history-window";

/** Mounted-transcript ceiling: layout cost grows with every mounted row, so
 * the pump stops here and the "show earlier" button loads the rest on
 * demand. Jumps and search flush the whole history and bypass the cap.
 * 80 rows is Paseo's mounted-recent scale (20 stream items); at 400 the
 * first open of a long session rendered for seconds and every invalidation
 * afterwards reflowed all of it. */
const MAX_MOUNTED_ROWS = 80;
import { assistantTurnCopyOwners, joinAssistantTurnCopy } from "./turn-copy";
import {
  createFeedbackControls,
  openFeedbackSupplement,
  showFeedbackNote,
  syncFeedbackControls,
  takeFeedbackSupplement,
  type FeedbackDirection,
} from "./feedback";
import { paintReplyAge, refreshReplyAges } from "./reply-age";
import { mountReplyChangeCard } from "./reply-changes";
import { stripContextCompactSummaries, uid } from "./store";
import type {
  ChatAttachment,
  ChatMessage,
  ChatRole,
  ChatSession,
  PendingPermission,
  PermissionMeta,
  SessionStore,
  ToolStep,
} from "./types";
import type { AskRuntime } from "../ask-resources";

export type BubblesDeps = {
  logEl: HTMLElement;
  titleEl: HTMLElement;
  getBusy: () => boolean;
  getStore: () => SessionStore;
  isViewingRunningSession: () => boolean;
  /** True while the completed repaint runs: the turn is over, not live. */
  isTurnCompleting: () => boolean;
  runTargetSession: () => ChatSession;
  activeSession: () => ChatSession;
  selectedRuntime: () => AskRuntime;
  sessionTitle: (session: ChatSession) => string;
  touchSession: (session: ChatSession) => void;
  saveStore: () => void;
  flushStorePersist: () => void;
  scheduleStorePersist: (delayMs?: number) => void;
  scheduleSessionListRender: (delayMs?: number) => void;
  renderSessionList: () => void;
  updateContextMeter: () => void;
  pushActivity: (phase: string, message: string) => void;
  settleActivity: () => void;
  finishToolGroup: (collapse?: boolean) => void;
  dismissLifecycleActivity: () => void;
  collapseResolvedPermissionsBeforeAssistant: (anchor: HTMLElement) => void;
  renderPermissionCard: (message: ChatMessage, interactive: boolean) => HTMLElement;
  renderPermissionGroup: (messages: ChatMessage[], interactive: boolean) => HTMLDetailsElement;
  getPendingPermissionBatch: () => PendingPermission[];
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
  getActivityEl: () => HTMLElement | null;
  setActivityEl: (el: HTMLElement | null) => void;
  getToolGroupEl: () => HTMLDetailsElement | null;
  setToolGroupEl: (el: HTMLDetailsElement | null) => void;
  getLifecycleActivityEl: () => HTMLElement | null;
  setLifecycleActivityEl: (el: HTMLElement | null) => void;
  addProject: () => void;
  openProvider: () => void;
  /** Drop starter text into the composer for the person to edit and send. */
  draftPrompt: (text: string) => void;
  /** Empty-state card: start a chat with the built-in doctor agent. */
  openDoctorChat?: () => void;
  /** Buttons for a failed run log whose sentence is already on the row. */
  attachRunLogFailure?: (row: HTMLElement, content: string) => void;
};

export type BubblesApi = ReturnType<typeof createBubblesController>;

const STREAM_PAINT_MIN_MS = 80;
const STREAM_HEAVY_CHARS = 2_500;
/** Tool folding and the plan dock redraw at most this often while streaming. */
const STREAM_AUX_MIN_MS = 400;
/** A reply this long renders a bounded slice with a "show full" toggle
 * (Paseo's web render limit); the store keeps the whole text either way. */
const RENDER_MAX_CHARS = 32_000;
/** User messages fold instead of growing a wall of text. */
const USER_FOLD_CHARS = 600;

export function createBubblesController(deps: BubblesDeps) {
  let flushRaf = 0;
  let streamPaintTimer = 0;
  let streamPaintAt = 0;
  let streamAuxAt = 0;
  // Streaming markdown is split into a frozen head (rendered once per
  // completed block) and a live tail, so a delta never re-parses the whole
  // reply — Paseo's head/tail split, minus the framework.
  let streamHeadText = "";
  let streamHeadBox: HTMLDivElement | null = null;
  let streamTailBox: HTMLDivElement | null = null;
  let streamBoxBubble: HTMLElement | null = null;
  let historyToken = 0;
  let historyFrame = 0;
  let historyJob: {
    token: number;
    messages: ChatMessage[];
    covered: { hidden: Set<number>; failedLogs: Set<number> };
    cursor: number;
    sessionRuntime: ChatSession["runtime"];
  } | null = null;
  const refreshAges = () => refreshReplyAges(deps.logEl);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") refreshAges();
  });
  window.setInterval(refreshAges, 30_000);
  deps.logEl.addEventListener("chat-load-earlier", () => {
    loadEarlierPage();
  });
  deps.logEl.addEventListener("chat-seek-history", (event) => {
    const detail = (event as CustomEvent<{ id: string; onMounted?: (found: boolean) => void }>).detail;
    if (!detail?.id) return;
    seekEarlier(detail.id, detail.onMounted);
  });
  let topFillTimer = 0;
  deps.logEl.addEventListener(
    "scroll",
    () => {
      // Paseo's history-start pagination shape: filling older pages while the
      // wheel is still moving raced the reader; wait for a pause near the top.
      if (topFillTimer) window.clearTimeout(topFillTimer);
      topFillTimer = window.setTimeout(() => {
        topFillTimer = 0;
        if (deps.logEl.scrollTop > 240) return;
        loadEarlierPage();
      }, 200);
    },
    { passive: true },
  );

  function flushPendingTextSync(): void {
    if (flushRaf) {
      window.cancelAnimationFrame(flushRaf);
      flushRaf = 0;
    }
    if (!deps.getPendingText()) return;
    const chunk = deps.getPendingText();
    deps.setPendingText("");
    appendAssistantChunk(chunk);
  }
  function sealAssistantBubble(): void {
    if (streamPaintTimer) {
      window.clearTimeout(streamPaintTimer);
      streamPaintTimer = 0;
    }
    // The seal re-renders the full markdown (and re-runs the code-block
    // enhancement), so the streaming head/tail split ends here.
    streamHeadText = "";
    streamHeadBox = null;
    streamTailBox = null;
    streamBoxBubble = null;
    if (!deps.getAssistantMessageId() && !deps.getAssistantBubble()) {
      deps.setAssistantRaw("");
      return;
    }
    if (deps.getAssistantBubble()?.isConnected && deps.getAssistantRaw().trim()) {
      const raw = deps.getAssistantRaw();
      setAssistantMarkdown(deps.getAssistantBubble()!, raw, {
        deferHighlight: raw.length > STREAM_HEAVY_CHARS,
        skipCopySync: true,
      });
    }
    if (deps.getAssistantBubble()?.isConnected) {
      deps.getAssistantBubble()!.classList.remove("is-streaming");
    }
    if (!deps.getAssistantRaw().trim()) {
      // Drop empty placeholder bubbles so tools aren't preceded by a blank card.
      const emptyId = deps.getAssistantMessageId();
      if (deps.getAssistantBubble()?.isConnected) {
        assistantMsgWrap(deps.getAssistantBubble()!).remove();
      }
      if (emptyId) {
        const session = deps.runTargetSession();
        session.messages = session.messages.filter((m) => m.id !== emptyId);
        deps.saveStore();
      }
    } else {
      if (deps.getAssistantMessageId()) {
        updateAssistantMessage(deps.getAssistantMessageId()!, deps.getAssistantRaw());
        deps.flushStorePersist();
      }
      if (deps.getAssistantBubble()?.isConnected) {
        const bubble = deps.getAssistantBubble()!;
        syncAssistantCopyButton(bubble);
        const id = deps.getAssistantMessageId();
        const stamped = id
          ? deps.runTargetSession().messages.find((message) => message.id === id)
          : undefined;
        syncAssistantTime(bubble, stamped?.at, false);
        lockChatRowHeights([assistantMsgWrap(bubble)], true);
      }
    }
    // Every seal is a segment boundary: the reply text that comes after a
    // tool opens a fresh message and bubble, so tools interleave with the
    // reply in chronological order (Paseo's timeline model).
    deps.setAssistantRaw("");
    deps.setAssistantMessageId(null);
    deps.setAssistantBubble(null);
    syncTurnCopyButtons();
    syncVisiblePlan();
    deps.setAssistantBubble(null);
    deps.setAssistantMessageId(null);
    deps.setAssistantRaw("");
  }
  function ensureAssistantMessage(): string {
    if (deps.getAssistantMessageId()) return deps.getAssistantMessageId()!;
    const message = persistMessage("assistant", "");
    deps.setAssistantMessageId(message.id);
    deps.setAssistantRaw("");
    return deps.getAssistantMessageId()!;
  }
  function ensureAssistantBubble(): HTMLElement {
    ensureAssistantMessage();
    if (deps.getAssistantBubble()?.isConnected) {
      deps.getAssistantBubble()!.classList.add("is-streaming");
      return deps.getAssistantBubble()!;
    }
    const existing = deps.getAssistantMessageId()
      ? deps.logEl.querySelector<HTMLElement>(
          `.chat-bubble.assistant[data-message-id="${CSS.escape(deps.getAssistantMessageId()!)}"]`,
        )
      : null;
    if (existing) {
      deps.setAssistantBubble(existing);
      deps.getAssistantBubble()!.classList.add("is-streaming");
      return deps.getAssistantBubble()!;
    }
    const created = appendBubble("assistant", deps.getAssistantRaw(), {
      id: deps.getAssistantMessageId() ?? undefined,
      persist: false,
    });
    deps.setAssistantBubble(created);
    created.classList.add("is-streaming");
    return created;
  }
  function appendAssistantChunk(chunk: string): void {
    if (!chunk) return;
    if (looksLikeToolPayloadJson(chunk)) {
      if (deps.isViewingRunningSession()) {
        const formatted = formatPermissionDetail(chunk.trim());
        const summary = formatted.summary || cleanToolLabel(chunk);
        const detail =
          formatted.full && formatted.full !== summary ? formatted.full : "";
        deps.pushActivity("tool", detail ? `${summary}\n${detail}` : summary);
      }
      return;
    }
    deps.setTurnHadAssistantText(true);
    ensureAssistantMessage();
    deps.setAssistantRaw(deps.getAssistantRaw() + chunk);
    if (deps.getAssistantMessageId()) {
      updateAssistantMessage(deps.getAssistantMessageId()!, deps.getAssistantRaw(), { persist: false });
    }
    deps.scheduleStorePersist();
    if (!deps.isViewingRunningSession()) return;
    if (deps.getActivityEl()) deps.settleActivity();
    deps.dismissLifecycleActivity();
    const bubble = ensureAssistantBubble();
    if (!bubble.isConnected) return;
    // Tool folding and the plan dock walk every row of the turn; they lag a
    // few hundred ms behind the text instead of running on every frame.
    const now = performance.now();
    if (now - streamAuxAt > STREAM_AUX_MIN_MS) {
      streamAuxAt = now;
      deps.collapseResolvedPermissionsBeforeAssistant(bubble);
      syncVisiblePlan();
    }
    paintStreamingMarkdown(bubble);
    syncReplyChangeCard();
    followChatLog(deps.logEl);
  }
  function setStreamingMarkdown(bubble: HTMLElement, markdown: string): void {
    if (streamBoxBubble !== bubble) {
      streamHeadBox = document.createElement("div");
      streamHeadBox.className = "chat-stream-part";
      streamTailBox = document.createElement("div");
      streamTailBox.className = "chat-stream-part";
      bubble.replaceChildren(streamHeadBox, streamTailBox);
      streamHeadText = "";
      streamBoxBubble = bubble;
    }
    const { cut, fenced } = markdownCutInfo(markdown);
    const head = markdown.slice(0, cut);
    if (head !== streamHeadText) {
      streamHeadBox!.innerHTML = head.trim() ? renderMarkdown(head) : "";
      streamHeadText = head;
    }
    const tail = markdown.slice(cut);
    const tailBox = streamTailBox!;
    if (fenced) {
      // A streaming fence has no safe cut inside it, so this tail re-parses
      // whole on every paint - the long-reply jank. It is code: render it as
      // plain text per tick, and markdown takes over exactly once when the
      // fence closes and the cut moves past it.
      const pre = document.createElement("pre");
      const code = document.createElement("code");
      code.textContent = tail;
      pre.append(code);
      tailBox.replaceChildren(pre);
      return;
    }
    tailBox.innerHTML = tail.trim() ? renderMarkdown(tail) : "";
  }
  function paintStreamingMarkdown(bubble: HTMLElement): void {
    const now = performance.now();
    if (now - streamPaintAt < STREAM_PAINT_MIN_MS) {
      if (!streamPaintTimer) {
        streamPaintTimer = window.setTimeout(() => {
          streamPaintTimer = 0;
          const live = deps.getAssistantBubble();
          if (!live?.isConnected) return;
          streamPaintAt = performance.now();
          setStreamingMarkdown(live, deps.getAssistantRaw());
          syncReplyChangeCard();
          followChatLog(deps.logEl);
        }, STREAM_PAINT_MIN_MS);
      }
      return;
    }
    streamPaintAt = now;
    // Code-block wrappers, copy bars and file links are seal-time work:
    // rebuilding them per frame re-highlighted the same code over and over.
    setStreamingMarkdown(bubble, deps.getAssistantRaw());
  }
  function persistMessage(
    role: ChatRole,
    content: string,
    opts?: { id?: string; attachments?: ChatAttachment[]; permission?: PermissionMeta },
  ): ChatMessage {
    const session = deps.getBusy() ? deps.runTargetSession() : deps.activeSession();
    const message: ChatMessage = {
      id: opts?.id ?? uid(),
      role,
      content,
      at: Date.now(),
      attachments: opts?.attachments?.length ? opts.attachments : undefined,
      permission: opts?.permission,
    };
    session.messages.push(message);
    if (role === "user") {
      noteTurnUserAt(message.at);
      if (session.id === deps.getStore().activeId) clearEmptyChatStart(deps.logEl);
    }
    if (role === "user" && !session.title.trim()) {
      const seed = content.trim() || opts?.attachments?.[0]?.name || "";
      session.title = seed.split(/\n/)[0].slice(0, 48);
    }
    if (!deps.getBusy() || session.id === deps.getStore().activeId) {
      const nextRuntime = deps.selectedRuntime();
      if (session.runtime !== nextRuntime) {
        session.runtimeThreadId = null;
      }
      session.runtime = nextRuntime;
    }
    deps.touchSession(session);
    if (deps.getBusy()) {
      deps.scheduleStorePersist();
      deps.scheduleSessionListRender();
    } else {
      deps.saveStore();
      deps.renderSessionList();
    }
    if (session.id === deps.getStore().activeId) {
      deps.titleEl.textContent = deps.sessionTitle(session);
      deps.updateContextMeter();
    }
    return message;
  }
  function rememberTool(text: string): void {
    const session = deps.getBusy() ? deps.runTargetSession() : deps.activeSession();
    // A tool interrupts the streaming reply: seal the text segment first so
    // the tool message lands after it (Paseo's timeline model).
    if (deps.getAssistantRaw().trim()) sealAssistantBubble();
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
        deps.touchSession(session);
        deps.scheduleStorePersist();
        return;
      }
    }
    const toolMessage: ChatMessage = {
      id: uid(),
      role: "tool",
      content: text,
      at: Date.now(),
    };
    const streamingId = deps.getAssistantMessageId();
    if (tail?.role === "assistant" && tail.id === streamingId) {
      messages.splice(messages.length - 1, 0, toolMessage);
      deps.touchSession(session);
      deps.scheduleStorePersist();
      return;
    }
    persistMessage("tool", text);
  }
  function rememberToolStep(step: ToolStep): void {
    const session = deps.getBusy() ? deps.runTargetSession() : deps.activeSession();
    // Same segment cut: the step message must follow the text written before
    // the tool started, and later text opens a fresh segment.
    if (deps.getAssistantRaw().trim()) sealAssistantBubble();
    if (!storeToolStep(session.messages, step, deps.getAssistantMessageId(), uid)) return;
    deps.touchSession(session);
    deps.scheduleStorePersist();
  }
  function rememberRuntimeLog(text: string): void {
    const session = deps.getBusy() ? deps.runTargetSession() : deps.activeSession();
    appendRunLogMessage(session.messages, text, uid);
    deps.touchSession(session);
    deps.scheduleStorePersist();
  }
  /** A failed turn with only a log: one sentence on that row, original stays inside. */
  function noteFailedRunLog(): string | null {
    const session = deps.getBusy() ? deps.runTargetSession() : deps.activeSession();
    if (!markTurnLogFailed(session.messages)) return null;
    deps.touchSession(session);
    deps.scheduleStorePersist();
    const content = latestTurnLogContent(session.messages);
    const label = runLogFailureLabel(content, session.runtime);
    if (deps.isViewingRunningSession()) {
      const row = labelCurrentRunLog(deps.logEl, label);
      if (row) deps.attachRunLogFailure?.(row, content);
    }
    return label;
  }
  function latestTurnLogContent(messages: ChatMessage[]): string {
    for (let i = messages.length - 1; i >= 0; i -= 1) {
      const message = messages[i];
      if (message?.role === "user") break;
      if (message?.role === "meta" && message.log) return message.content;
    }
    return "";
  }
  function rememberTurnNotice(text: string): void {
    const session = deps.getBusy() ? deps.runTargetSession() : deps.activeSession();
    session.messages.push({ id: uid(), role: "meta", content: text, at: Date.now() });
    deps.touchSession(session);
    deps.scheduleStorePersist();
  }
  function updateAssistantMessage(id: string, content: string, opts?: { persist?: boolean }): void {
    const session = deps.getBusy() ? deps.runTargetSession() : deps.activeSession();
    const message = session.messages.find((m) => m.id === id);
    if (!message) return;
    message.content = content;
    // The rail reads the whole transcript to place its current tick, and that
    // read forces a layout of every row above it. On a running answer it waits
    // for a quiet moment instead of running on every chunk.
    if (session.id === deps.activeSession().id) {
      scheduleQuestionRail(() => session.messages, deps.logEl);
    }
    if (opts?.persist === false) return;
    message.at = Date.now();
    deps.touchSession(session);
    if (deps.getBusy()) {
      deps.scheduleStorePersist();
      deps.scheduleSessionListRender();
    } else {
      deps.saveStore();
    }
  }
  function assistantMarkdownSource(bubble: HTMLElement): string {
    const id = bubble.dataset.messageId;
    if (id) {
      const session = deps.getBusy() ? deps.runTargetSession() : deps.activeSession();
      const message = session.messages.find((m) => m.id === id);
      if (message?.content?.trim()) return message.content;
    }
    if (bubble === deps.getAssistantBubble() && deps.getAssistantRaw().trim()) return deps.getAssistantRaw();
    return bubblePlainText(bubble);
  }
  function turnCopySource(bubble: HTMLElement): string {
    const wrap = assistantMsgWrap(bubble);
    const log = wrap.parentElement;
    if (!log) return assistantMarkdownSource(bubble);
    const children = Array.from(log.children);
    const index = children.indexOf(wrap);
    if (index < 0) return assistantMarkdownSource(bubble);
    let start = index;
    while (start > 0 && !children[start - 1].classList.contains("chat-msg-user")) start -= 1;
    let end = index;
    while (end + 1 < children.length && !children[end + 1].classList.contains("chat-msg-user")) end += 1;
    const parts: string[] = [];
    for (let i = start; i <= end; i++) {
      const child = children[i];
      if (!child.classList.contains("chat-msg-assistant")) continue;
      const node = child.querySelector<HTMLElement>(":scope > .chat-bubble.assistant");
      if (!node) continue;
      const text = assistantMarkdownSource(node).trim();
      if (text) parts.push(text);
    }
    return joinAssistantTurnCopy(parts);
  }
  function syncTurnCopyButtons(): void {
    perfMark("turn-copy");
    const children = Array.from(deps.logEl.children);
    const roles = children.map((child) => {
      if (child.classList.contains("chat-msg-user")) return "user";
      if (child.classList.contains("chat-msg-assistant")) return "assistant";
      return "other";
    });
    const owners = assistantTurnCopyOwners(roles);
    children.forEach((child, index) => {
      if (roles[index] !== "assistant") return;
      const wrap = child as HTMLElement;
      const bubble = wrap.querySelector<HTMLElement>(":scope > .chat-bubble.assistant");
      if (!bubble) return;
      // The owner exemption is what lets the action row paint at all: rows
      // render under content-visibility: auto, whose paint containment clips
      // the below-box action row. See the .is-turn-owner rule in chat.css.
      wrap.classList.toggle("is-turn-owner", owners[index] === true);
      syncMessageCopyActions(
        wrap,
        bubble,
        bubble.classList.contains("is-streaming"),
        assistantMarkdownSource(bubble),
        owners[index] === true,
      );
      const rated = feedbackMessage(bubble)?.feedback;
      syncFeedbackControls(wrap, {
        owner: owners[index] === true,
        streaming: bubble.classList.contains("is-streaming"),
        rated: rated === "up" || rated === "down" ? rated : undefined,
      });
    });
  }
  /** The message a rating acts on, found through the bubble's data attribute. */
  function feedbackMessage(bubble: HTMLElement): ChatMessage | undefined {
    const id = bubble.dataset.messageId;
    if (!id) return undefined;
    return deps.activeSession().messages.find((message) => message.id === id);
  }
  /** Persist the rating, then report it anonymously with the diagnosis log. */
  function handleFeedbackClick(wrap: HTMLElement, bubble: HTMLElement, direction: FeedbackDirection): void {
    const session = deps.activeSession();
    const message = feedbackMessage(bubble);
    if (!message) return;
    if (message.feedback === "up" || message.feedback === "down") {
      showFeedbackNote(wrap, t("chat.feedbackAlready"), true);
      return;
    }
    message.feedback = direction;
    deps.touchSession(session);
    if (deps.getBusy()) deps.scheduleStorePersist();
    else deps.saveStore();
    syncTurnCopyButtons();
    reportFeedback({
      reason: direction === "up" ? "reply-up" : "reply-down",
      runtime: session.runtime,
    })
      .then(() => {
        showFeedbackNote(wrap, direction === "up" ? t("chat.feedbackUpSent") : t("chat.feedbackDownSent"));
        if (direction === "down") openFeedbackSupplement(wrap);
      })
      .catch(() => showFeedbackNote(wrap, t("chat.feedbackSendFailed"), true));
  }
  function handleFeedbackSupplement(wrap: HTMLElement): void {
    const note = takeFeedbackSupplement(wrap);
    const session = deps.activeSession();
    if (!note) return;
    reportFeedback({ reason: "reply-note", note, runtime: session.runtime })
      .then(() => showFeedbackNote(wrap, t("chat.feedbackNoteSent")))
      .catch(() => showFeedbackNote(wrap, t("chat.feedbackSendFailed"), true));
  }
  function syncVisiblePlan(): void {
    const onScreen = deps.activeSession();
    const source = deps.getBusy() && deps.isViewingRunningSession() ? deps.runTargetSession() : onScreen;
    if (source.id !== onScreen.id) return;
    const finished = !(deps.getBusy() && deps.isViewingRunningSession());
    const items = source.plan?.items.length
      ? source.plan.items
      : planStepsFromMessages(source.messages, finished);
    paintChatPlan(deps.logEl, items.length ? { items, at: source.plan?.at ?? Date.now() } : null);
  }
  function syncReplyChangeCard(): void {
    const onScreen = deps.activeSession();
    const source = deps.getBusy() && deps.isViewingRunningSession() ? deps.runTargetSession() : onScreen;
    if (source.id !== onScreen.id) return;
    const running = deps.getBusy() && deps.isViewingRunningSession();
    mountReplyChangeCard(deps.logEl, source.messages, { running });
  }
  function syncAssistantCopyButton(bubble: HTMLElement): void {
    // A bubble that is not in the transcript yet is picked up after it is appended.
    if (!assistantMsgWrap(bubble).isConnected) return;
    syncTurnCopyButtons();
  }
  function syncAssistantTime(bubble: HTMLElement, at: number | undefined, streaming: boolean): void {
    const time = assistantMsgWrap(bubble).querySelector<HTMLTimeElement>(":scope > .chat-msg-actions time.chat-msg-time");
    if (time) paintReplyAge(time, at, streaming);
  }
  function createAssistantBubbleEl(opts?: { id?: string }): { wrap: HTMLElement; bubble: HTMLElement } {
    const wrap = document.createElement("div");
    wrap.className = "chat-msg chat-msg-assistant";

    const bubble = document.createElement("div");
    bubble.className = "chat-bubble assistant chat-md";
    if (opts?.id) bubble.dataset.messageId = opts.id;

    const actions = document.createElement("div");
    actions.className = "chat-msg-actions";
    actions.hidden = true;
    // One button copies the whole turn, not just this fragment.
    actions.appendChild(createCopyActionButton("text", () => turnCopySource(bubble)));
    const time = document.createElement("time");
    time.className = "chat-msg-time";
    time.hidden = true;
    actions.appendChild(time);
    const feedback = createFeedbackControls();
    feedback.addEventListener("click", (event) => {
      const target = event.target as HTMLElement;
      const btn = target.closest<HTMLElement>(".chat-feedback-btn");
      if (btn) {
        if (btn.dataset.feedback === "up" || btn.dataset.feedback === "down") {
          handleFeedbackClick(wrap, bubble, btn.dataset.feedback);
        }
        return;
      }
      if (target.closest(".chat-feedback-send")) handleFeedbackSupplement(wrap);
    });
    feedback.addEventListener("keydown", (event) => {
      if (event.key !== "Enter") return;
      const input = event.target as HTMLElement;
      if (input.classList.contains("chat-feedback-input")) {
        event.preventDefault();
        handleFeedbackSupplement(wrap);
      }
    });
    actions.appendChild(feedback);
    wrap.append(bubble, actions);
    return { wrap, bubble };
  }
  function createUserBubbleEl(opts?: {
    id?: string;
    text?: string;
    attachments?: ChatAttachment[];
  }): { wrap: HTMLElement; bubble: HTMLElement } {
    const wrap = document.createElement("div");
    wrap.className = "chat-msg chat-msg-user";

    const bubble = document.createElement("div");
    bubble.className = "chat-bubble user";
    if (opts?.id) bubble.dataset.messageId = opts.id;
    bubble.textContent = opts?.text ?? "";
    const strip = renderAttachmentStrip(opts?.attachments);
    if (strip) bubble.appendChild(strip);

    const foldable = (opts?.text ?? "").length > USER_FOLD_CHARS;
    if (foldable) bubble.classList.add("is-folded");

    const actions = document.createElement("div");
    actions.className = "chat-msg-actions";
    const getText = () => {
      const id = bubble.dataset.messageId;
      if (id) {
        const message = deps.activeSession().messages.find((m) => m.id === id);
        if (message) return message.content;
      }
      return opts?.text ?? bubblePlainText(bubble);
    };
    actions.appendChild(createCopyActionButton("text", getText));
    if (foldable) {
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.className = "chat-bubble-fold";
      toggle.textContent = t("chat.userFoldMore");
      toggle.addEventListener("click", () => {
        const folded = bubble.classList.toggle("is-folded");
        toggle.textContent = folded ? t("chat.userFoldMore") : t("chat.userFoldLess");
      });
      wrap.append(bubble, toggle, actions);
    } else {
      wrap.append(bubble, actions);
    }
    syncMessageCopyActions(wrap, bubble, false, getText());
    return { wrap, bubble };
  }
  /// A bounded slice at a safe block boundary; the full text stays in the
  /// store and can be rendered on demand.
  function renderAssistantHtml(markdown: string): { html: string; truncated: boolean } {
    if (markdown.length <= RENDER_MAX_CHARS) {
      return { html: renderMarkdown(markdown), truncated: false };
    }
    const window = markdown.slice(0, RENDER_MAX_CHARS);
    const safe = safeMarkdownCut(`${window}\n`);
    const cutText = safe > 0 ? window.slice(0, safe) : window.slice(0, window.lastIndexOf("\n") + 1);
    return { html: renderMarkdown(cutText), truncated: true };
  }

  function appendFullReplyToggle(
    bubble: HTMLElement,
    markdown: string,
    opts?: { deferHighlight?: boolean; skipCopySync?: boolean },
  ): void {
    const note = document.createElement("p");
    note.className = "chat-render-note";
    const label = document.createElement("span");
    label.textContent = t("chat.renderTruncated");
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = t("chat.showFullReply");
    button.addEventListener("click", () => {
      bubble.innerHTML = renderMarkdown(markdown);
      enhanceCodeBlocks(bubble, { defer: opts?.deferHighlight });
      linkFileMentions(bubble);
      note.remove();
    });
    note.append(label, button);
    bubble.appendChild(note);
  }

  function setAssistantMarkdown(
    bubble: HTMLElement,
    markdown: string,
    opts?: { deferHighlight?: boolean; skipCopySync?: boolean },
  ): void {
    const source = markdown.trim();
    if (!source) {
      bubble.innerHTML = "";
      return;
    }
    const { html, truncated } = renderAssistantHtml(source);
    bubble.innerHTML = html;
    if (truncated) appendFullReplyToggle(bubble, source, opts);
    enhanceCodeBlocks(bubble, { defer: opts?.deferHighlight });
    linkFileMentions(bubble);
    if (!opts?.skipCopySync) syncAssistantCopyButton(bubble);
  }
  function appendBubble(
    kind: ChatRole,
    text: string,
    opts?: { id?: string; persist?: boolean; attachments?: ChatAttachment[] },
  ): HTMLElement {
    clearEmptyChatStart(deps.logEl);
    if (kind === "assistant") {
      const { wrap, bubble } = createAssistantBubbleEl({ id: opts?.id });
      if (opts?.id && opts.id === deps.getAssistantMessageId() && deps.getBusy()) {
        bubble.classList.add("is-streaming");
      }
      setAssistantMarkdown(bubble, text);
      deps.logEl.appendChild(wrap);
      syncTurnCopyButtons();
      syncReplyChangeCard();
      followChatLog(deps.logEl);
      return bubble;
    }
    if (kind === "user") {
      const { wrap, bubble } = createUserBubbleEl({
        id: opts?.id,
        text,
        attachments: opts?.attachments,
      });
      deps.logEl.appendChild(wrap);
      syncReplyChangeCard();
      pinChatLog(deps.logEl);
      syncQuestionRail(deps.activeSession().messages, deps.logEl);
      return bubble;
    }
    const bubble = document.createElement("div");
    bubble.className = `chat-bubble ${kind}`;
    if (opts?.id) bubble.dataset.messageId = opts.id;
    if (!mountFailedCommand(bubble, text)) bubble.textContent = text;
    const strip = renderAttachmentStrip(opts?.attachments);
    if (strip) bubble.appendChild(strip);
    deps.logEl.appendChild(bubble);
    followChatLog(deps.logEl);
    return bubble;
  }
  /** Tools saved after the reply still belong above that reply. */
  function renderEmptyChat(): HTMLElement {
    if (deps.activeSession().runtime === "doctor") {
      return renderDoctorStartChat();
    }
    const wrap = document.createElement("div");
    wrap.className = "chat-start";
    const project = document.createElement("button");
    project.type = "button";
    project.className = "chat-start-card";
    project.innerHTML = `<strong></strong><span></span>`;
    project.querySelector("strong")!.textContent = t("chat.emptyProjectsAction");
    project.querySelector("span")!.textContent = t("chat.emptyProjectsHint");
    project.addEventListener("click", () => deps.addProject());
    const provider = document.createElement("button");
    provider.type = "button";
    provider.className = "chat-start-card";
    provider.innerHTML = `<strong></strong><span></span>`;
    provider.querySelector("strong")!.textContent = t("chat.startProvider");
    provider.querySelector("span")!.textContent = t("chat.startProviderHint");
    provider.addEventListener("click", () => deps.openProvider());
    wrap.append(project, provider);
    if (deps.openDoctorChat) {
      const doctor = document.createElement("button");
      doctor.type = "button";
      doctor.className = "chat-start-card";
      doctor.innerHTML = `<strong></strong><span></span>`;
      doctor.querySelector("strong")!.textContent = t("chat.emptyDoctorAction");
      doctor.querySelector("span")!.textContent = t("chat.emptyDoctorHint");
      doctor.addEventListener("click", () => deps.openDoctorChat?.());
      wrap.append(doctor);
    }
    return wrap;
  }
  /** A doctor chat greets with its real abilities; a card drafts its question. */
  function renderDoctorStartChat(): HTMLElement {
    const wrap = document.createElement("div");
    wrap.className = "chat-start";
    for (const card of doctorStartCards()) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "chat-start-card";
      button.innerHTML = `<strong></strong><span></span>`;
      button.querySelector("strong")!.textContent = card.title;
      button.querySelector("span")!.textContent = card.hint;
      button.addEventListener("click", () => deps.draftPrompt(card.prompt));
      wrap.append(button);
    }
    return wrap;
  }

  function cancelHistoryPaint(): void {
    historyToken += 1;
    historyJob = null;
    if (historyFrame) {
      window.cancelAnimationFrame(historyFrame);
      historyFrame = 0;
    }
  }
  function prependKeepingScroll(nodes: HTMLElement[]): void {
    if (nodes.length === 0) return;
    const log = deps.logEl;
    for (const node of nodes) node.style.setProperty("content-visibility", "visible");
    const pinned = log.dataset.pin !== "0";
    const before = log.scrollHeight;
    const top = log.scrollTop;
    withoutChatLogJump(() => {
      log.prepend(...nodes);
      const delta = log.scrollHeight - before;
      log.scrollTop = pinned ? log.scrollHeight : top + Math.max(0, delta);
    });
    lockChatRowHeights(nodes);
    // The rows have been measured. A hand-set "visible" would keep them from
    // ever being skipped again, and a transcript that is never skipped is a
    // transcript that is slow to lay out on every line that arrives.
    for (const node of nodes) node.style.removeProperty("content-visibility");
    if (pinned) {
      withoutChatLogJump(() => {
        log.scrollTop = log.scrollHeight;
      });
    }
  }
  function mountHistoryBatch(): boolean {
    const job = historyJob;
    if (!job || job.token !== historyToken || job.cursor <= 0) return false;
    const end = job.cursor;
    const start = snapRunBoundary(
      job.messages,
      Math.max(0, end - HISTORY_BATCH),
      HISTORY_RUN_SLICE,
    );
    const bucket = document.createElement("div");
    renderMessageRange(job.messages, start, end, bucket, job.covered, job.sessionRuntime);
    prependKeepingScroll(Array.from(bucket.children) as HTMLElement[]);
    job.cursor = start;
    return start > 0;
  }
  /** One chunked page of older rows (Paseo's history-start pagination):
   * batches render across frames, the reader compensation runs per prepend -
   * the old "flush everything" path rendered a multi-thousand-row transcript
   * inside one synchronous loop, which froze the window for minutes.
   * 50 rows is Telegram Android's paging size (happy's telegram-chat.md). */
  const HISTORY_PAGE_ROWS = 50;
  let pageTargetCursor: number | null = null;
  let seekTargetId: string | null = null;
  let seekDone: ((found: boolean) => void) | null = null;

  function loadEarlierPage(): void {
    perfMark("load-earlier-page");
    if (!historyJob || historyJob.token !== historyToken || historyJob.cursor <= 0) {
      deps.logEl.querySelector(".chat-history-earlier")?.remove();
      return;
    }
    pageTargetCursor = Math.max(0, historyJob.cursor - HISTORY_PAGE_ROWS);
    seekTargetId = null;
    seekDone = null;
    if (!historyFrame) pumpHistory();
  }

  function bubbleMounted(id: string): boolean {
    return (
      deps.logEl.querySelector(
        `.chat-bubble.user[data-message-id="${CSS.escape(id)}"]`,
      ) !== null
    );
  }

  function seekEarlier(id: string, onMounted?: (found: boolean) => void): void {
    if (bubbleMounted(id)) {
      onMounted?.(true);
      return;
    }
    if (!historyJob || historyJob.token !== historyToken) {
      onMounted?.(false);
      return;
    }
    seekTargetId = id;
    seekDone = onMounted ?? null;
    pageTargetCursor = null;
    if (!historyFrame) pumpHistory();
  }

  function finishSeek(found: boolean): void {
    const done = seekDone;
    seekTargetId = null;
    seekDone = null;
    done?.(found);
  }
  function pumpHistory(): void {
    perfMark("history-pump");
    historyFrame = 0;
    if (!historyJob || historyJob.token !== historyToken) return;
    const started = performance.now();
    try {
      let more = true;
      let capped = false;
      do {
        // Bound the mounted transcript: layout cost grows with every row
        // here, and a long chat must not make every streaming frame slower.
        // An explicit page load (top-fill, the show-earlier button) or a
        // seek bypasses the cap - the person asked for those older rows,
        // and they render across frames instead of in one go.
        const explicit = pageTargetCursor !== null || seekTargetId !== null;
        if (!explicit && deps.logEl.childElementCount >= MAX_MOUNTED_ROWS && historyJob.cursor > 0) {
          more = false;
          capped = true;
          break;
        }
        more = mountHistoryBatch();
        if (seekTargetId && bubbleMounted(seekTargetId)) {
          finishSeek(true);
          more = false;
          break;
        }
        if (pageTargetCursor !== null && historyJob.cursor <= pageTargetCursor) {
          pageTargetCursor = null;
        }
      } while (more && performance.now() - started < 12);
      if (more && historyJob?.token === historyToken) {
        historyFrame = window.requestAnimationFrame(pumpHistory);
        return;
      }
      if (capped && historyJob?.token === historyToken) {
        showEarlierButton();
        syncTurnCopyButtons();
        return;
      }
    } catch (error) {
      console.error("Ask: failed to render earlier messages", error);
      historyJob = null;
      return;
    }
    if (historyJob?.token === historyToken) {
      if (seekTargetId) finishSeek(false);
      if (historyJob.cursor > 0) {
        // The boundary sits higher up: keep the marker there for the next
        // explicit load instead of dropping it.
        showEarlierButton();
        syncTurnCopyButtons();
        return;
      }
      historyJob = null;
      syncTurnCopyButtons();
      deps.logEl.querySelector(".chat-history-earlier")?.remove();
    }
  }
  function showEarlierButton(): void {
    const log = deps.logEl;
    const existing = log.querySelector<HTMLButtonElement>(".chat-history-earlier");
    if (existing) {
      // Prepends slid the marker down; it belongs at the boundary.
      log.prepend(existing);
      existing.disabled = false;
      return;
    }
    const button = document.createElement("button");
    button.type = "button";
    button.className = "chat-history-earlier";
    button.textContent = t("chat.showEarlier");
    button.addEventListener("click", () => {
      log.dispatchEvent(new CustomEvent("chat-load-earlier"));
    });
    log.prepend(button);
  }
  function renderMessageRange(
    messages: ChatMessage[],
    start: number,
    end: number,
    target: HTMLElement,
    covered: { hidden: Set<number>; failedLogs: Set<number> },
    runtime: ChatSession["runtime"],
  ): void {
    perfMark("history-render");
    for (let i = start; i < end; ) {
      const message = messages[i];
      if (message.role === "tool" || message.role === "thinking") {
        const rows: HTMLElement[] = [];
        while (i < end) {
          const current = messages[i];
          if (current.role === "tool") {
            const run: ChatMessage[] = [current];
            while (i + run.length < end && messages[i + run.length].role === "tool") {
              run.push(messages[i + run.length]);
            }
            rows.push(
              renderToolHistoryGroup(
                omitBareToolStarts(
                  run.map((item) =>
                    item.step || item.skippedTools
                      ? { text: item.content, step: item.step, skipped: item.skippedTools }
                      : item.content,
                  ),
                ),
              ),
            );
            i += run.length;
            continue;
          }
          if (current.role === "thinking") {
            if (current.content.trim()) {
              rows.push(renderThinkingBlock(current.content, { id: current.id }));
            }
            i += 1;
            continue;
          }
          break;
        }
        const live =
          deps.getBusy() &&
          deps.isViewingRunningSession() &&
          !deps.isTurnCompleting() &&
          i >= messages.length;
        if (live) {
          const lastTools = [...rows].reverse().find((row) => row.classList.contains("chat-tool-group"));
          if (lastTools instanceof HTMLDetailsElement) lastTools.open = true;
          const lastThink = [...rows].reverse().find((row) => row.classList.contains("chat-thinking"));
          if (lastThink instanceof HTMLDetailsElement) markThinkingLive(lastThink);
        }
        if (rows.length === 1) {
          // A lone tool run still folds: history windows often split between a
          // thinking block and its tools, and a bare open group read as a
          // bullet list that never folded.
          const only = rows[0]!;
          target.appendChild(
            !live && only.classList.contains("chat-tool-group") ? completedTrail([only]) : only,
          );
        } else if (rows.length > 1) target.appendChild(wrapProcessRows(rows, live));
        continue;
      }
      if (message.role === "permission") {
        const pendingLive =
          deps.isViewingRunningSession() &&
          message.permission?.allowed == null &&
          deps.getPendingPermissionBatch().some((p) => p.requestId === message.permission?.requestId);
        if (pendingLive) {
          i += 1;
          continue;
        }
        // Unanswered asks stay as standalone cards — never fold into "$ N tools".
        if (message.permission?.allowed == null) {
          const needsReply =
            message.permission?.inputMode === "line" ||
            message.permission?.inputMode === "secret" ||
            message.permission?.inputMode === "options";
          target.appendChild(deps.renderPermissionCard(message, needsReply));
          i += 1;
          continue;
        }
        const run: ChatMessage[] = [message];
        while (i + run.length < end && messages[i + run.length].role === "permission") {
          const next = messages[i + run.length];
          if (next.permission?.allowed == null) break;
          const nextPending =
            deps.isViewingRunningSession() &&
            deps.getPendingPermissionBatch().some((p) => p.requestId === next.permission?.requestId);
          if (nextPending) break;
          run.push(next);
        }
        if (run.length >= 2) {
          try {
            target.appendChild(deps.renderPermissionGroup(run, false));
          } catch {
            for (const item of run) target.appendChild(deps.renderPermissionCard(item, false));
          }
        } else {
          target.appendChild(deps.renderPermissionCard(message, false));
        }
        i += run.length;
        continue;
      }
      if (message.role === "assistant") {
        const { wrap, bubble } = createAssistantBubbleEl({ id: message.id });
        const liveContent =
          deps.getBusy() && message.id === deps.getAssistantMessageId() && deps.getAssistantRaw()
            ? deps.getAssistantRaw()
            : message.content;
        const streaming = Boolean(deps.getBusy() && message.id === deps.getAssistantMessageId());
        if (streaming) {
          bubble.classList.add("is-streaming");
          deps.setAssistantBubble(bubble);
        }
        setAssistantMarkdown(bubble, liveContent, { deferHighlight: true, skipCopySync: true });
        syncAssistantTime(bubble, message.at, streaming);
        target.appendChild(wrap);
      } else if (message.role === "user") {
        const { wrap } = createUserBubbleEl({
          id: message.id,
          text: message.content,
          attachments: message.attachments,
        });
        target.appendChild(wrap);
      } else if (covered.hidden.has(i)) {
        i += 1;
        continue;
      } else if (isLogMessage(message)) {
        const failed = covered.failedLogs.has(i);
        const row = appendRunLog(
          target,
          message.content,
          failed ? runLogFailureLabel(message.content, runtime) : undefined,
        );
        if (failed && row) deps.attachRunLogFailure?.(row, message.content);
      } else {
        const bubble = document.createElement("div");
        bubble.className = `chat-bubble ${message.role}`;
        bubble.dataset.messageId = message.id;
        const shown = isRunLogPointer(message.content) ? agentFailedLine(runtime) : message.content;
        if (!mountFailedCommand(bubble, shown)) bubble.textContent = shown;
        const strip = renderAttachmentStrip(message.attachments);
        if (strip) bubble.appendChild(strip);
        target.appendChild(bubble);
      }
      i += 1;
    }
  }
  function renderActiveMessages(): void {
    cancelHistoryPaint();
    if (streamPaintTimer) {
      window.clearTimeout(streamPaintTimer);
      streamPaintTimer = 0;
    }
    releaseDeferredHighlights(deps.logEl);
    const previous = Array.from(deps.logEl.childNodes);
    try {
      if (stripContextCompactSummaries(deps.getStore())) deps.saveStore();
      deps.logEl.replaceChildren();
      deps.setAssistantBubble(null);
      deps.setActivityEl(null);
      deps.setToolGroupEl(null);
      deps.setLifecycleActivityEl(null);
      // Keep in-flight run memory so background events and switch-back still work.
      if (!deps.getBusy()) {
        deps.setAssistantMessageId(null);
        deps.setAssistantRaw("");
        deps.setTurnHadAssistantText(false);
      }
      const session = deps.activeSession();
      if (!session.messages.length) {
        deps.logEl.appendChild(renderEmptyChat());
        syncVisiblePlan();
        syncQuestionRail(session.messages, deps.logEl);
        return;
      }
      // Tools are stored in chronological order (segment cuts + tail appends);
      // rendering must not hoist them above the reply segments.
      const messages = session.messages;
      const covered = coveredRunLogPointers(messages);
      const cut = historyRenderCut(messages);
      renderMessageRange(messages, cut, messages.length, deps.logEl, covered, session.runtime);
      if (!sessionHasChatContent(session)) {
        // Only the run's traces survived. The words are in the database and on
        // their way back — say that, instead of leaving a bare tool log that
        // reads like the conversation is gone.
        const note = document.createElement("div");
        note.className = "chat-bubble meta";
        note.textContent = "这段对话的文字记录正在从本地数据库恢复，下面是这次运行留下的痕迹。";
        deps.logEl.prepend(note);
      }
      syncReplyChangeCard();
      syncTurnCopyButtons();
      syncVisiblePlan();
      pinChatLog(deps.logEl);
      syncQuestionRail(session.messages, deps.logEl);
      deps.updateContextMeter();
      if (cut > 0) {
        historyJob = {
          token: historyToken,
          messages,
          covered,
          cursor: cut,
          sessionRuntime: session.runtime,
        };
        historyFrame = window.requestAnimationFrame(pumpHistory);
      }
    } catch (error) {
      console.error("Ask: failed to render messages", error);
      if (previous.length > 0) {
        deps.logEl.replaceChildren(...previous);
        return;
      }
      deps.logEl.replaceChildren();
      appendBubble("meta", t("chat.welcome"), { persist: false });
    }
  }
  function queueAssistantText(text: string): void {
    if (!text) return;
    deps.setPendingText(deps.getPendingText() + text);
    if (flushRaf) return;
    flushRaf = window.requestAnimationFrame(() => {
      flushRaf = 0;
      const chunk = deps.getPendingText();
      deps.setPendingText("");
      if (!chunk) return;
      appendAssistantChunk(chunk);
    });
  }

  return {
    flushPendingTextSync,
    sealAssistantBubble,
    ensureAssistantMessage,
    ensureAssistantBubble,
    appendAssistantChunk,
    persistMessage,
    rememberTool,
    rememberToolStep,
    rememberRuntimeLog,
    noteFailedRunLog,
    rememberTurnNotice,
    updateAssistantMessage,
    assistantMarkdownSource,
    syncAssistantCopyButton,
    createAssistantBubbleEl,
    createUserBubbleEl,
    setAssistantMarkdown,
    appendBubble,
    renderActiveMessages,
    queueAssistantText,
  };
}
