import { t } from "../i18n";
import { renderMarkdown } from "../markdown";
import {
  cleanToolLabel,
  formatPermissionDetail,
  looksLikeToolPayloadJson,
} from "./format";
import {
  assistantMsgWrap,
  bubblePlainText,
  createCopyActionButton,
  enhanceCodeBlocks,
  renderAttachmentStrip,
  syncMessageCopyActions,
} from "./copy-ui";
import { uid } from "./store";
import type {
  ChatAttachment,
  ChatMessage,
  ChatRole,
  ChatSession,
  PendingPermission,
  PermissionMeta,
  SessionStore,
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
};

export type BubblesApi = ReturnType<typeof createBubblesController>;

export function createBubblesController(deps: BubblesDeps) {
  let flushRaf = 0;

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
    if (!deps.getAssistantMessageId() && !deps.getAssistantBubble()) {
      deps.setAssistantRaw("");
      return;
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
        syncAssistantCopyButton(deps.getAssistantBubble()!);
      }
    }
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
        deps.pushActivity("tool", formatted.summary || cleanToolLabel(chunk));
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
    deps.finishToolGroup(true);
    deps.dismissLifecycleActivity();
    const bubble = ensureAssistantBubble();
    if (!bubble.isConnected) return;
    deps.collapseResolvedPermissionsBeforeAssistant(bubble);
    setAssistantMarkdown(bubble, deps.getAssistantRaw());
    deps.logEl.scrollTop = deps.logEl.scrollHeight;
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
    if (role === "user" && !session.title.trim()) {
      const seed = content.trim() || opts?.attachments?.[0]?.name || "";
      session.title = seed.split(/\n/)[0].slice(0, 48);
    }
    if (!deps.getBusy() || session.id === deps.getStore().activeId) {
      session.runtime = deps.selectedRuntime();
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
  function updateAssistantMessage(id: string, content: string, opts?: { persist?: boolean }): void {
    const session = deps.getBusy() ? deps.runTargetSession() : deps.activeSession();
    const message = session.messages.find((m) => m.id === id);
    if (!message) return;
    message.content = content;
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
  function syncAssistantCopyButton(bubble: HTMLElement): void {
    const source = assistantMarkdownSource(bubble);
    syncMessageCopyActions(
      assistantMsgWrap(bubble),
      bubble,
      bubble.classList.contains("is-streaming"),
      source,
    );
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
    // Default copy = markdown source (better for paste/edit).
    actions.appendChild(createCopyActionButton("text", () => assistantMarkdownSource(bubble)));
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
  function setAssistantMarkdown(bubble: HTMLElement, markdown: string): void {
    bubble.innerHTML = markdown.trim() ? renderMarkdown(markdown) : "";
    if (markdown.trim()) enhanceCodeBlocks(bubble);
    syncAssistantCopyButton(bubble);
  }
  function appendBubble(
    kind: ChatRole,
    text: string,
    opts?: { id?: string; persist?: boolean; attachments?: ChatAttachment[] },
  ): HTMLElement {
    if (kind === "assistant") {
      const { wrap, bubble } = createAssistantBubbleEl({ id: opts?.id });
      setAssistantMarkdown(bubble, text);
      deps.logEl.appendChild(wrap);
      deps.logEl.scrollTop = deps.logEl.scrollHeight;
      return bubble;
    }
    if (kind === "user") {
      const { wrap, bubble } = createUserBubbleEl({
        id: opts?.id,
        text,
        attachments: opts?.attachments,
      });
      deps.logEl.appendChild(wrap);
      deps.logEl.scrollTop = deps.logEl.scrollHeight;
      return bubble;
    }
    const bubble = document.createElement("div");
    bubble.className = `chat-bubble ${kind}`;
    if (opts?.id) bubble.dataset.messageId = opts.id;
    bubble.textContent = text;
    const strip = renderAttachmentStrip(opts?.attachments);
    if (strip) bubble.appendChild(strip);
    deps.logEl.appendChild(bubble);
    deps.logEl.scrollTop = deps.logEl.scrollHeight;
    return bubble;
  }
  function renderActiveMessages(): void {
    try {
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
      if (session.messages.length === 0) {
        appendBubble("meta", t("chat.welcome"), { persist: false });
        return;
      }
      for (let i = 0; i < session.messages.length; ) {
        const message = session.messages[i];
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
            deps.logEl.appendChild(deps.renderPermissionCard(message, false));
            i += 1;
            continue;
          }
          const run: ChatMessage[] = [message];
          while (
            i + run.length < session.messages.length &&
            session.messages[i + run.length].role === "permission"
          ) {
            const next = session.messages[i + run.length];
            if (next.permission?.allowed == null) break;
            const nextPending =
              deps.isViewingRunningSession() &&
              deps.getPendingPermissionBatch().some((p) => p.requestId === next.permission?.requestId);
            if (nextPending) break;
            run.push(next);
          }
          if (run.length >= 2) {
            try {
              deps.logEl.appendChild(deps.renderPermissionGroup(run, false));
            } catch {
              for (const item of run) {
                deps.logEl.appendChild(deps.renderPermissionCard(item, false));
              }
            }
          } else {
            deps.logEl.appendChild(deps.renderPermissionCard(message, false));
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
          setAssistantMarkdown(bubble, liveContent);
          if (deps.getBusy() && message.id === deps.getAssistantMessageId()) {
            bubble.classList.add("is-streaming");
            deps.setAssistantBubble(bubble);
          }
          deps.logEl.appendChild(wrap);
        } else if (message.role === "user") {
          const { wrap } = createUserBubbleEl({
            id: message.id,
            text: message.content,
            attachments: message.attachments,
          });
          deps.logEl.appendChild(wrap);
        } else {
          const bubble = document.createElement("div");
          bubble.className = `chat-bubble ${message.role}`;
          bubble.dataset.messageId = message.id;
          bubble.textContent = message.content;
          const strip = renderAttachmentStrip(message.attachments);
          if (strip) bubble.appendChild(strip);
          deps.logEl.appendChild(bubble);
        }
        i += 1;
      }
      deps.logEl.scrollTop = deps.logEl.scrollHeight;
      deps.updateContextMeter();
    } catch (error) {
      console.error("Ask: failed to render messages", error);
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
