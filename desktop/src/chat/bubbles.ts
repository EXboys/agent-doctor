import { t } from "../i18n";
import { clearEmptyChatStart, sessionHasChatContent } from "./empty-start";
import { paintChatPlan, planStepsFromMessages } from "../plan";
import { renderMarkdown } from "../markdown";
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
import { followChatLog, lockChatRowHeights, pinChatLog, renderToolHistoryGroup, withoutChatLogJump } from "./activity";
import { markThinkingLive, renderThinkingBlock } from "./thinking";
import { linkFileMentions } from "./file-links";
import { appendRunLogMessage, omitBareToolStarts, storeToolStep } from "./tool-records";
import { sealWorkTrail, wrapProcessRows } from "./work-trail";
import { syncQuestionRail } from "./question-rail";
import {
  assistantMsgWrap,
  bubblePlainText,
  createCopyActionButton,
  enhanceCodeBlocks,
  releaseDeferredHighlights,
  renderAttachmentStrip,
  syncMessageCopyActions,
} from "./copy-ui";
import { HISTORY_BATCH, historyRenderCut, messageContinuesRun } from "./history-window";
import { assistantTurnCopyOwners, joinAssistantTurnCopy } from "./turn-copy";
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
  /** Buttons for a failed run log whose sentence is already on the row. */
  attachRunLogFailure?: (row: HTMLElement, content: string) => void;
};

export type BubblesApi = ReturnType<typeof createBubblesController>;

const STREAM_PAINT_MIN_MS = 80;
const STREAM_HEAVY_CHARS = 2_500;

export function createBubblesController(deps: BubblesDeps) {
  let flushRaf = 0;
  let streamPaintTimer = 0;
  let streamPaintAt = 0;
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
  deps.logEl.addEventListener("chat-flush-history", () => {
    flushPendingHistory();
  });

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
    if (deps.getActivityEl()?.dataset.kind === "tool") deps.settleActivity();
    deps.finishToolGroup(false);
    deps.dismissLifecycleActivity();
    sealWorkTrail(deps.logEl);
    const bubble = ensureAssistantBubble();
    if (!bubble.isConnected) return;
    deps.collapseResolvedPermissionsBeforeAssistant(bubble);
    paintStreamingMarkdown(bubble);
    syncReplyChangeCard();
    syncVisiblePlan();
    followChatLog(deps.logEl);
  }
  function paintStreamingMarkdown(bubble: HTMLElement): void {
    const markdown = deps.getAssistantRaw();
    const heavy = markdown.length > STREAM_HEAVY_CHARS;
    const now = performance.now();
    if (heavy && now - streamPaintAt < STREAM_PAINT_MIN_MS) {
      if (!streamPaintTimer) {
        streamPaintTimer = window.setTimeout(() => {
          streamPaintTimer = 0;
          const live = deps.getAssistantBubble();
          if (!live?.isConnected) return;
          streamPaintAt = performance.now();
          const raw = deps.getAssistantRaw();
          setAssistantMarkdown(live, raw, { deferHighlight: true, skipCopySync: true });
          syncReplyChangeCard();
          followChatLog(deps.logEl);
        }, STREAM_PAINT_MIN_MS);
      }
      return;
    }
    streamPaintAt = now;
    setAssistantMarkdown(bubble, markdown, { deferHighlight: heavy, skipCopySync: true });
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
    if (session.id === deps.activeSession().id) syncQuestionRail(session.messages, deps.logEl);
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
    const children = Array.from(deps.logEl.children);
    const roles = children.map((child) => {
      if (child.classList.contains("chat-msg-user")) return "user";
      if (child.classList.contains("chat-msg-assistant")) return "assistant";
      return "other";
    });
    const owners = assistantTurnCopyOwners(roles);
    children.forEach((child, index) => {
      if (roles[index] !== "assistant") return;
      const bubble = child.querySelector<HTMLElement>(":scope > .chat-bubble.assistant");
      if (!bubble) return;
      syncMessageCopyActions(
        child as HTMLElement,
        bubble,
        bubble.classList.contains("is-streaming"),
        assistantMarkdownSource(bubble),
        owners[index] === true,
      );
    });
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
    wrap.append(bubble, actions);
    syncMessageCopyActions(wrap, bubble, false, getText());
    return { wrap, bubble };
  }
  function setAssistantMarkdown(
    bubble: HTMLElement,
    markdown: string,
    opts?: { deferHighlight?: boolean; skipCopySync?: boolean },
  ): void {
    bubble.innerHTML = markdown.trim() ? renderMarkdown(markdown) : "";
    if (markdown.trim()) {
      enhanceCodeBlocks(bubble, { defer: opts?.deferHighlight });
      linkFileMentions(bubble);
    }
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
  function placeToolsBeforeReply(source: ChatMessage[]): ChatMessage[] {
    const out: ChatMessage[] = [];
    for (let i = 0; i < source.length; ) {
      const message = source[i];
      if (message.role !== "assistant") {
        out.push(message);
        i += 1;
        continue;
      }
      i += 1;
      const tools: ChatMessage[] = [];
      while (i < source.length && source[i].role === "tool") {
        tools.push(source[i]);
        i += 1;
      }
      out.push(...tools, message);
    }
    return out;
  }

  function renderEmptyChat(): HTMLElement {
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
    let start = Math.max(0, end - HISTORY_BATCH);
    while (start > 0 && messageContinuesRun(job.messages, start)) start -= 1;
    const bucket = document.createElement("div");
    renderMessageRange(job.messages, start, end, bucket, job.covered, job.sessionRuntime);
    prependKeepingScroll(Array.from(bucket.children) as HTMLElement[]);
    job.cursor = start;
    return start > 0;
  }
  function flushPendingHistory(): void {
    if (!historyJob || historyJob.token !== historyToken) return;
    try {
      while (mountHistoryBatch()) {
        // Keep going until the older rows exist, so a question jump can find its bubble.
      }
    } catch (error) {
      console.error("Ask: failed to render earlier messages", error);
    }
    if (historyJob?.token === historyToken) {
      historyJob = null;
      syncTurnCopyButtons();
    }
  }
  function pumpHistory(): void {
    historyFrame = 0;
    if (!historyJob || historyJob.token !== historyToken) return;
    const started = performance.now();
    try {
      let more = true;
      do {
        more = mountHistoryBatch();
      } while (more && performance.now() - started < 12);
      if (more && historyJob?.token === historyToken) {
        historyFrame = window.requestAnimationFrame(pumpHistory);
        return;
      }
    } catch (error) {
      console.error("Ask: failed to render earlier messages", error);
      historyJob = null;
      return;
    }
    if (historyJob?.token === historyToken) {
      historyJob = null;
      syncTurnCopyButtons();
    }
  }
  function renderMessageRange(
    messages: ChatMessage[],
    start: number,
    end: number,
    target: HTMLElement,
    covered: { hidden: Set<number>; failedLogs: Set<number> },
    runtime: ChatSession["runtime"],
  ): void {
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
        const live = deps.getBusy() && deps.isViewingRunningSession() && i >= messages.length;
        if (live) {
          const lastTools = [...rows].reverse().find((row) => row.classList.contains("chat-tool-group"));
          if (lastTools instanceof HTMLDetailsElement) lastTools.open = true;
          const lastThink = [...rows].reverse().find((row) => row.classList.contains("chat-thinking"));
          if (lastThink instanceof HTMLDetailsElement) markThinkingLive(lastThink);
        }
        if (rows.length === 1) target.appendChild(rows[0]);
        else if (rows.length > 1) target.appendChild(wrapProcessRows(rows, live));
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
      if (!sessionHasChatContent(session)) {
        deps.logEl.appendChild(renderEmptyChat());
        syncVisiblePlan();
        syncQuestionRail(session.messages, deps.logEl);
        return;
      }
      const messages = placeToolsBeforeReply(session.messages);
      const covered = coveredRunLogPointers(messages);
      const cut = historyRenderCut(messages);
      renderMessageRange(messages, cut, messages.length, deps.logEl, covered, session.runtime);
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
