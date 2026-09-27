//! Deep diagnose = Agent chat on top of rule-based checks (not another checklist).

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  createAttachmentsController,
  type AttachmentsApi,
} from "../chat/attachments";
import {
  attachmentSummary,
  imageReadingBlock,
  type ImageReading,
} from "../chat/context";
import { enhanceCodeBlocks } from "../chat/copy-ui";
import { readImageTextEnabled } from "../chat/image-text";
import type { ChatAttachment, PromptSessionEvent, PromptSessionReport } from "../chat/types";
import {
  createVoiceInputController,
  type VoiceInputApi,
} from "../chat/voice";
import { withErrorDetail } from "../friendly-error";
import { t } from "../i18n";
import { renderMarkdown } from "../markdown";
import * as dom from "./dom";
import type { DiagnoseSession } from "./session";
import type { RepairPreviewResponse } from "../types";

export type DeepPresetId = "explain" | "fix" | "browser" | "health";

type DeepBubble = {
  id: string;
  role: "user" | "assistant" | "meta";
  text: string;
  attachments?: ChatAttachment[];
};

function attachmentStripHtml(attachments: ChatAttachment[] | undefined): string {
  if (!attachments?.length) {
    return "";
  }
  const items = attachments
    .map((item) => {
      if (item.kind === "image") {
        try {
          const src = convertFileSrc(item.path);
          return `<img class="diagnose-deep-attach-image" src="${escapeHtml(src)}" alt="${escapeHtml(item.name)}" title="${escapeHtml(item.name)}" />`;
        } catch {
          return `<span class="diagnose-deep-attach-file">${escapeHtml(item.name)}</span>`;
        }
      }
      return `<span class="diagnose-deep-attach-file">${escapeHtml(item.name)}</span>`;
    })
    .join("");
  return `<div class="diagnose-deep-attach-strip">${items}</div>`;
}

export type DeepChatApi = {
  paint: () => void;
  send: (text?: string) => Promise<void>;
  usePreset: (id: DeepPresetId) => Promise<void>;
  stop: () => Promise<void>;
  dispose: () => void;
};

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function issueBrief(preview: RepairPreviewResponse | null): string {
  if (!preview) {
    return t("diagnose.flow.deepCtxNone");
  }
  const issues = preview.checks.filter(
    (check) => check.status === "fail" || check.status === "warn",
  );
  if (issues.length === 0) {
    return t("diagnose.flow.deepCtxClean", {
      pass: String(preview.summary.pass),
      total: String(preview.checks.length),
    });
  }
  return issues
    .slice(0, 8)
    .map((check) => {
      const label = check.status === "fail" ? t("repair.fail") : t("repair.warn");
      return `- [${label}] ${check.title}: ${check.message}`;
    })
    .join("\n");
}

function buildAgentPrompt(
  userText: string,
  preview: RepairPreviewResponse | null,
  attachments: ChatAttachment[],
  readings: ImageReading[],
): string {
  const parts = [
    t("diagnose.flow.deepAgentLead"),
    issueBrief(preview),
    "",
  ];
  const pictureText = imageReadingBlock(readings);
  if (pictureText) {
    parts.push(pictureText, "");
  } else if (attachments.length > 0) {
    parts.push(
      `Attached local files for this turn (read them with your tools if needed):\n${attachmentSummary(attachments)}`,
      "",
    );
  }
  parts.push(t("diagnose.flow.deepAgentUser", { text: userText.trim() }));
  return parts.join("\n");
}

export function createDeepChat(session: DiagnoseSession): DeepChatApi {
  const bubbles: DeepBubble[] = [];
  let busy = false;
  let assistantId: string | null = null;
  let statusText = "";
  let unlisten: UnlistenFn | null = null;
  let shellReady = false;
  let pendingAttachments: ChatAttachment[] = [];
  let attachmentsApi: AttachmentsApi | null = null;
  let voiceApi: VoiceInputApi | null = null;

  function inputEl(): HTMLTextAreaElement | null {
    return dom.deepBodyEl.querySelector<HTMLTextAreaElement>("#diagnose-deep-input");
  }

  function activityHtml(): string {
    if (!busy || assistantId) {
      return "";
    }
    return `<div class="diagnose-deep-activity" id="diagnose-deep-activity" aria-live="polite">
      <span class="diagnose-deep-activity-dots" aria-hidden="true"><i></i><i></i><i></i></span>
      <span class="diagnose-deep-activity-text">${escapeHtml(
        statusText || t("diagnose.flow.deepThinking"),
      )}</span>
    </div>`;
  }

  function setStatus(next: string): void {
    statusText = next.trim();
    if (assistantId) {
      dom.deepBodyEl.querySelector("#diagnose-deep-activity")?.remove();
      return;
    }
    const textNode = dom.deepBodyEl.querySelector<HTMLElement>(".diagnose-deep-activity-text");
    if (textNode) {
      textNode.textContent = statusText || t("diagnose.flow.deepThinking");
      return;
    }
    if (busy) {
      const thread = dom.deepBodyEl.querySelector<HTMLElement>("#diagnose-deep-thread");
      if (thread && !thread.querySelector("#diagnose-deep-activity")) {
        thread.insertAdjacentHTML("beforeend", activityHtml());
        thread.scrollTop = thread.scrollHeight;
      }
    }
  }

  function syncComposerControls(): void {
    const presets = dom.deepBodyEl.querySelectorAll<HTMLButtonElement>("[data-deep-preset]");
    for (const btn of presets) {
      btn.disabled = busy;
    }
    const textarea = inputEl();
    if (textarea) {
      textarea.disabled = busy;
      textarea.placeholder = t("diagnose.flow.deepPlaceholder");
    }
    const attachBtn = dom.deepBodyEl.querySelector<HTMLButtonElement>("#diagnose-deep-attach");
    if (attachBtn) {
      attachBtn.disabled = busy;
      attachBtn.title = t("chat.attach");
      attachBtn.setAttribute("aria-label", t("chat.attach"));
    }
    voiceApi?.syncEnabled();
    voiceApi?.applyI18n();

    const actions = dom.deepBodyEl.querySelector<HTMLElement>("#diagnose-deep-actions");
    if (!actions) {
      return;
    }
    if (busy) {
      actions.innerHTML = `<button type="button" class="btn-ghost diagnose-deep-send" id="diagnose-deep-stop">
        ${escapeHtml(t("diagnose.flow.deepStop"))}
      </button>`;
    } else {
      actions.innerHTML = `<button type="submit" class="btn-primary diagnose-deep-send" id="diagnose-deep-send">
        ${escapeHtml(t("diagnose.flow.deepSend"))}
      </button>`;
    }
  }

  function starterCardsHtml(): string {
    const items: { id: DeepPresetId; title: string; hint: string }[] = [
      {
        id: "explain",
        title: t("diagnose.flow.deepPresetExplain"),
        hint: t("diagnose.flow.deepPresetExplainHint"),
      },
      {
        id: "fix",
        title: t("diagnose.flow.deepPresetFix"),
        hint: t("diagnose.flow.deepPresetFixHint"),
      },
      {
        id: "browser",
        title: t("diagnose.flow.deepPresetBrowser"),
        hint: t("diagnose.flow.deepPresetBrowserHint"),
      },
      {
        id: "health",
        title: t("diagnose.flow.deepPresetHealth"),
        hint: t("diagnose.flow.deepPresetHealthHint"),
      },
    ];
    const cards = items
      .map(
        (item) => `<button
          type="button"
          class="diagnose-deep-starter"
          data-deep-preset="${item.id}"
          ${busy ? "disabled" : ""}
        >
          <strong>${escapeHtml(item.title)}</strong>
          <span>${escapeHtml(item.hint)}</span>
        </button>`,
      )
      .join("");
    return `<div class="diagnose-deep-starters">
      <p class="diagnose-deep-starters-kicker">${escapeHtml(t("diagnose.flow.deepStartersKicker"))}</p>
      <div class="diagnose-deep-starter-grid" role="list">${cards}</div>
    </div>`;
  }

  function paintThread(): void {
    const thread = dom.deepBodyEl.querySelector<HTMLElement>("#diagnose-deep-thread");
    if (!thread) {
      return;
    }
    const empty = bubbles.length === 0 && !busy ? starterCardsHtml() : "";
    const list = bubbles
      .map((bubble) => {
        if (bubble.role === "assistant") {
          const html = bubble.text.trim() ? renderMarkdown(bubble.text) : "";
          const streaming =
            busy && assistantId === bubble.id ? " is-streaming" : "";
          return `<div class="diagnose-deep-bubble is-assistant${streaming}" data-bubble-id="${escapeHtml(bubble.id)}">
          <div class="diagnose-deep-bubble-text chat-md">${html}</div>
        </div>`;
        }
        const textHtml = bubble.text.trim()
          ? `<div class="diagnose-deep-bubble-text">${escapeHtml(bubble.text)}</div>`
          : "";
        const attachHtml = attachmentStripHtml(bubble.attachments);
        return `<div class="diagnose-deep-bubble is-${bubble.role}" data-bubble-id="${escapeHtml(bubble.id)}">
          ${textHtml}${attachHtml}
        </div>`;
      })
      .join("");
    thread.innerHTML = `${empty}${list}${activityHtml()}`;
    for (const node of thread.querySelectorAll<HTMLElement>(
      ".diagnose-deep-bubble.is-assistant .diagnose-deep-bubble-text",
    )) {
      if (node.textContent?.trim()) {
        enhanceCodeBlocks(node);
      }
    }
    thread.scrollTop = thread.scrollHeight;
  }

  function wireComposer(): void {
    const attachmentsEl = dom.deepBodyEl.querySelector<HTMLElement>("#diagnose-deep-attachments");
    const composerBoxEl = dom.deepBodyEl.querySelector<HTMLElement>("#diagnose-deep-composer-box");
    const attachBtn = dom.deepBodyEl.querySelector<HTMLButtonElement>("#diagnose-deep-attach");
    const voiceBtn = dom.deepBodyEl.querySelector<HTMLButtonElement>("#diagnose-deep-voice");
    const textarea = inputEl();
    if (!attachmentsEl || !composerBoxEl || !attachBtn || !voiceBtn || !textarea) {
      return;
    }

    attachmentsApi = createAttachmentsController({
      attachmentsEl,
      composerBoxEl,
      isComposerLocked: () => busy,
      getPendingAttachments: () => pendingAttachments,
      setPendingAttachments: (items) => {
        pendingAttachments = items;
      },
      setStatus: (text) => setStatus(text),
    });
    void attachmentsApi.setupFileDrop();
    attachmentsApi.renderPendingAttachments();

    attachBtn.addEventListener("click", () => {
      void attachmentsApi?.pickAttachments();
    });

    voiceApi = createVoiceInputController({
      voiceBtnEl: voiceBtn,
      promptEl: () => inputEl(),
      isComposerLocked: () => busy,
      setStatus: (text) => setStatus(text),
      autoResizePrompt: () => {
        /* fixed rows */
      },
    });

    // Same Enter rules as Ask: Enter send, Shift+Enter newline, IME Enter confirms composition.
    textarea.addEventListener("keydown", (event) => {
      if (event.key !== "Enter") {
        return;
      }
      if (event.shiftKey) {
        return;
      }
      if (
        event.isComposing ||
        event.keyCode === 229 ||
        textarea.dataset.composing === "1"
      ) {
        return;
      }
      event.preventDefault();
      if (busy) {
        return;
      }
      void send();
    });
    textarea.addEventListener("compositionstart", () => {
      textarea.dataset.composing = "1";
    });
    textarea.addEventListener("compositionend", () => {
      window.setTimeout(() => {
        textarea.dataset.composing = "0";
      }, 0);
    });
  }

  function ensureShell(): void {
    // Rebuild if missing bar or still on an older shell layout.
    if (
      shellReady &&
      dom.deepBodyEl.querySelector(".diagnose-deep-composer-bar") &&
      dom.deepBodyEl.querySelector("#diagnose-deep-attach .diagnose-deep-tool-icon") &&
      !dom.deepBodyEl.querySelector(".diagnose-deep-composer-wrap .diagnose-deep-presets") &&
      !dom.deepBodyEl.querySelector("#diagnose-deep-screenshot") &&
      !dom.deepBodyEl.querySelector("#diagnose-deep-status")
    ) {
      return;
    }
    shellReady = false;
    attachmentsApi = null;
    voiceApi = null;
    dom.deepBodyEl.innerHTML = `
      <div class="diagnose-deep-chat">
        <div class="diagnose-deep-thread" id="diagnose-deep-thread"></div>
        <div class="diagnose-deep-composer-wrap">
          <form class="diagnose-deep-composer" id="diagnose-deep-form">
            <div class="diagnose-deep-composer-box" id="diagnose-deep-composer-box">
              <div id="diagnose-deep-attachments" class="diagnose-deep-attachments" hidden></div>
              <textarea
                id="diagnose-deep-input"
                class="diagnose-deep-prompt"
                rows="2"
                placeholder="${escapeHtml(t("diagnose.flow.deepPlaceholder"))}"
              ></textarea>
              <div class="diagnose-deep-composer-bar">
                <div class="diagnose-deep-composer-tools">
                  <button
                    type="button"
                    class="diagnose-deep-tool"
                    id="diagnose-deep-attach"
                    title="${escapeHtml(t("chat.attach"))}"
                    aria-label="${escapeHtml(t("chat.attach"))}"
                  >
                    <svg class="diagnose-deep-tool-icon" viewBox="0 0 24 24" aria-hidden="true">
                      <path
                        fill="currentColor"
                        d="M11 5a1 1 0 1 1 2 0v6h6a1 1 0 1 1 0 2h-6v6a1 1 0 1 1-2 0v-6H5a1 1 0 1 1 0-2h6V5Z"
                      />
                    </svg>
                  </button>
                  <button
                    type="button"
                    class="diagnose-deep-tool diagnose-deep-tool-voice"
                    id="diagnose-deep-voice"
                    title="${escapeHtml(t("chat.voiceStart"))}"
                    aria-label="${escapeHtml(t("chat.voiceStart"))}"
                    aria-pressed="false"
                    hidden
                  >
                    <svg class="diagnose-deep-voice-icon" viewBox="0 0 24 24" aria-hidden="true">
                      <path
                        fill="currentColor"
                        d="M12 14a3 3 0 0 0 3-3V7a3 3 0 1 0-6 0v4a3 3 0 0 0 3 3Zm5-3a5 5 0 0 1-10 0H5a7 7 0 0 0 6 6.92V21h2v-3.08A7 7 0 0 0 19 11h-2Z"
                      />
                    </svg>
                  </button>
                </div>
                <div class="diagnose-deep-composer-actions" id="diagnose-deep-actions"></div>
              </div>
            </div>
          </form>
        </div>
      </div>`;
    wireComposer();
    shellReady = true;
  }

  function paint(): void {
    if (!session.deepOpen) {
      return;
    }
    // While dictating, keep the composer DOM stable so partial text stays visible
    // (same live-fill behavior as Ask).
    if (voiceApi?.isListening()) {
      return;
    }
    ensureShell();
    paintThread();
    syncComposerControls();
  }

  function pushBubble(
    role: DeepBubble["role"],
    text: string,
    attachments?: ChatAttachment[],
  ): string {
    const id = `deep-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    bubbles.push({
      id,
      role,
      text,
      attachments: attachments?.length ? [...attachments] : undefined,
    });
    paint();
    return id;
  }

  function appendAssistant(text: string): void {
    if (!text) {
      return;
    }
    if (!assistantId) {
      assistantId = pushBubble("assistant", text);
      return;
    }
    const bubble = bubbles.find((item) => item.id === assistantId);
    if (!bubble) {
      assistantId = pushBubble("assistant", text);
      return;
    }
    bubble.text += text;
    dom.deepBodyEl.querySelector("#diagnose-deep-activity")?.remove();
    const bubbleEl = dom.deepBodyEl.querySelector<HTMLElement>(
      `[data-bubble-id="${assistantId}"]`,
    );
    const node = bubbleEl?.querySelector<HTMLElement>(".diagnose-deep-bubble-text");
    if (bubbleEl && node) {
      bubbleEl.classList.add("is-streaming");
      node.innerHTML = bubble.text.trim() ? renderMarkdown(bubble.text) : "";
      if (bubble.text.trim()) {
        enhanceCodeBlocks(node);
      }
      const thread = dom.deepBodyEl.querySelector<HTMLElement>("#diagnose-deep-thread");
      if (thread) {
        thread.scrollTop = thread.scrollHeight;
      }
    } else {
      paint();
    }
  }

  async function ensureListen(): Promise<void> {
    if (unlisten) {
      return;
    }
    try {
      unlisten = await listen<PromptSessionEvent>("prompt-session-event", (event) => {
        const payload = event.payload;
        if (!payload || typeof payload !== "object" || !("type" in payload)) {
          return;
        }
        switch (payload.type) {
          case "started":
            setStatus(t("diagnose.flow.deepStarted"));
            break;
          case "status":
            if (payload.message) {
              setStatus(payload.message);
            }
            break;
          case "delta":
            if (payload.text) {
              setStatus(t("diagnose.flow.deepWriting"));
              appendAssistant(payload.text);
            }
            break;
          case "stderr_line":
            if (payload.line?.trim()) {
              setStatus(payload.line.trim().slice(0, 120));
            }
            break;
          case "completed":
            setStatus(
              payload.status === "succeeded"
                ? t("diagnose.flow.deepDone")
                : t("diagnose.flow.deepFailed", { status: String(payload.status) }),
            );
            if (!assistantId && payload.summary?.trim()) {
              appendAssistant(payload.summary.trim());
            }
            break;
          default:
            break;
        }
      });
    } catch {
      unlisten = null;
    }
  }

  async function stop(): Promise<void> {
    setStatus(t("diagnose.flow.deepStopping"));
    try {
      await invoke<boolean>("cancel_prompt_session_command");
    } catch {
      /* ignore */
    }
  }

  async function send(raw?: string, opts?: { displayText?: string }): Promise<void> {
    const textarea = inputEl();
    const typed = (raw ?? textarea?.value ?? "").trim();
    const attachments = [...pendingAttachments];
    const text = typed || (attachments.length > 0 ? t("chat.attachOnlyPrompt") : "");
    if (!text) {
      textarea?.focus();
      return;
    }
    if (busy) {
      pushBubble("meta", t("diagnose.flow.deepBusy"));
      return;
    }

    busy = true;
    assistantId = null;
    statusText = t("diagnose.flow.deepConnecting");
    if (textarea) {
      textarea.value = "";
    }
    pendingAttachments = [];
    attachmentsApi?.renderPendingAttachments();
    const bubbleText = (opts?.displayText ?? typed).trim() || text;
    pushBubble("user", bubbleText, attachments);
    await ensureListen();

    let readings: ImageReading[] = [];
    const imagePaths = attachments.filter((item) => item.kind === "image").map((item) => item.path);
    if (imagePaths.length > 0 && readImageTextEnabled()) {
      setStatus(t("chat.readingImages"));
      try {
        const report = await invoke<{
          readings: { name: string; text: string; ok: boolean }[];
        }>("read_image_texts_command", { paths: imagePaths });
        readings = (report.readings ?? [])
          .filter((item) => item.ok && item.text.trim())
          .map((item) => ({ name: item.name, text: item.text }));
        if (readings.length === 0) {
          setStatus(t("chat.readingImagesNone"));
        }
      } catch {
        readings = [];
        setStatus(t("chat.readingImagesNone"));
      }
    }

    try {
      setStatus(t("diagnose.flow.deepConnecting"));
      const report = await invoke<PromptSessionReport>("start_prompt_session_command", {
        runtime: session.runtimeId,
        prompt: buildAgentPrompt(text, session.preview, attachments, readings),
        cwd: null,
        timeoutSec: 180,
        dangerouslySkipPermissions: true,
        fullAuto: true,
        resumeThreadId: null,
        selectedMcps: null,
      });
      if (!assistantId && report.summary?.trim()) {
        pushBubble("assistant", report.summary.trim());
      } else if (!assistantId) {
        pushBubble(
          "meta",
          report.status === "succeeded"
            ? t("diagnose.flow.deepDone")
            : t("diagnose.flow.deepFailed", { status: report.status }),
        );
      }
    } catch (error) {
      const message = String(error);
      if (/already running/i.test(message)) {
        pushBubble("meta", t("diagnose.flow.deepBusy"));
      } else {
        pushBubble("meta", withErrorDetail(t("diagnose.flow.deepFailedGeneric"), error));
      }
    } finally {
      busy = false;
      assistantId = null;
      statusText = "";
      paint();
    }
  }

  async function usePreset(id: DeepPresetId): Promise<void> {
    const queries: Record<DeepPresetId, string> = {
      explain: t("diagnose.flow.deepQueryExplain"),
      fix: t("diagnose.flow.deepQueryFix"),
      browser: t("diagnose.flow.deepQueryBrowser"),
      health: t("diagnose.flow.deepQueryHealth"),
    };
    const labels: Record<DeepPresetId, string> = {
      explain: t("diagnose.flow.deepPresetExplain"),
      fix: t("diagnose.flow.deepPresetFix"),
      browser: t("diagnose.flow.deepPresetBrowser"),
      health: t("diagnose.flow.deepPresetHealth"),
    };
    await send(queries[id], { displayText: labels[id] });
  }

  function dispose(): void {
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
    void voiceApi?.stopListening();
    shellReady = false;
    attachmentsApi = null;
    voiceApi = null;
  }

  return { paint, send, usePreset, stop, dispose };
}
