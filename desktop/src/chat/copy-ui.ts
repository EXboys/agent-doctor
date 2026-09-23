import { convertFileSrc } from "@tauri-apps/api/core";
import { t } from "../i18n";
import type { ChatAttachment, CopyIdleKind } from "./types";
import { IMAGE_EXTS, SHORT_MSG_COPY_CHARS, SHORT_MSG_COPY_LINES } from "./types";

export function fileNameFromPath(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}

export function isImagePath(path: string): boolean {
  const ext = fileNameFromPath(path).split(".").pop()?.toLowerCase() ?? "";
  return IMAGE_EXTS.includes(ext);
}

export function bubblePlainText(bubble: HTMLElement): string {
  return (bubble.innerText ?? "").trim();
}

export function isShortCopyLayout(bubble: HTMLElement, source: string): boolean {
  if (bubble.querySelector("pre, .chat-code-block, table, .chat-bubble-attachments")) return false;
  const text = source.trim();
  if (!text) return false;
  const lines = text.split(/\n/).filter((line) => line.trim().length > 0);
  return text.length <= SHORT_MSG_COPY_CHARS && lines.length <= SHORT_MSG_COPY_LINES;
}

export function copyIconSvg(kind: "copy" | "check"): string {
  if (kind === "check") {
    return `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M20 6L9 17l-5-5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
  }
  return `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true"><rect x="9" y="9" width="13" height="13" rx="2" stroke="currentColor" stroke-width="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
}

export function copyIdleLabel(kind: CopyIdleKind): string {
  return kind === "code" ? t("chat.copyCode") : t("chat.copy");
}

export function setCopyButtonState(
  btn: HTMLButtonElement,
  state: "idle" | "copied" | "failed",
  idleKind: CopyIdleKind = "text",
): void {
  if (state === "copied") {
    btn.innerHTML = copyIconSvg("check");
    btn.classList.add("is-copied");
    btn.title = t("chat.copied");
    btn.setAttribute("aria-label", t("chat.copied"));
    return;
  }
  btn.innerHTML = copyIconSvg("copy");
  btn.classList.remove("is-copied");
  const label = state === "failed" ? t("chat.copyFailed") : copyIdleLabel(idleKind);
  btn.title = label;
  btn.setAttribute("aria-label", label);
}

export async function copyTextToClipboard(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.style.position = "fixed";
  area.style.left = "-9999px";
  document.body.appendChild(area);
  area.select();
  document.execCommand("copy");
  area.remove();
}

export async function runCopyButton(
  btn: HTMLButtonElement,
  getText: () => string,
  idleKind: CopyIdleKind,
): Promise<void> {
  const raw = getText();
  const text = idleKind === "code" ? raw.replace(/\n+$/, "") : raw.trim();
  if (!text) return;
  try {
    await copyTextToClipboard(text);
    setCopyButtonState(btn, "copied", idleKind);
    window.setTimeout(() => {
      if (!btn.isConnected) return;
      setCopyButtonState(btn, "idle", idleKind);
    }, 1600);
  } catch {
    setCopyButtonState(btn, "failed", idleKind);
    window.setTimeout(() => {
      if (!btn.isConnected) return;
      setCopyButtonState(btn, "idle", idleKind);
    }, 1600);
  }
}

export function createCopyActionButton(
  idleKind: CopyIdleKind,
  getText: () => string,
): HTMLButtonElement {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "chat-msg-copy";
  btn.dataset.copyKind = idleKind;
  btn.hidden = true;
  setCopyButtonState(btn, "idle", idleKind);
  btn.addEventListener("click", () => {
    void runCopyButton(btn, getText, idleKind);
  });
  return btn;
}

export function syncMessageCopyActions(
  wrap: HTMLElement,
  bubble: HTMLElement,
  streaming: boolean,
  layoutSource: string,
): void {
  const hasText = layoutSource.trim().length > 0;
  const show = hasText && !streaming;
  const actions = wrap.querySelector<HTMLElement>(":scope > .chat-msg-actions");
  const btn = actions?.querySelector<HTMLButtonElement>(".chat-msg-copy");
  if (btn) {
    const kind: CopyIdleKind = btn.dataset.copyKind === "code" ? "code" : "text";
    btn.hidden = !show;
    if (show) setCopyButtonState(btn, "idle", kind);
  }
  if (actions) actions.hidden = !show;
  wrap.classList.toggle("is-compact", show && isShortCopyLayout(bubble, layoutSource));
}

export function enhanceCodeBlocks(root: HTMLElement): void {
  for (const pre of Array.from(root.querySelectorAll("pre"))) {
    if (pre.parentElement?.classList.contains("chat-code-block")) continue;
    const wrap = document.createElement("div");
    wrap.className = "chat-code-block";
    pre.replaceWith(wrap);

    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "chat-code-copy";
    setCopyButtonState(btn, "idle", "code");
    btn.addEventListener("click", (event) => {
      event.stopPropagation();
      const code = pre.querySelector("code");
      const text = code?.textContent ?? pre.textContent ?? "";
      void runCopyButton(btn, () => text, "code");
    });

    wrap.append(btn, pre);
  }
}

export function renderAttachmentStrip(attachments: ChatAttachment[] | undefined): HTMLElement | null {
  if (!attachments?.length) return null;
  const strip = document.createElement("div");
  strip.className = "chat-bubble-attachments";
  for (const item of attachments) {
    if (item.kind === "image") {
      const img = document.createElement("img");
      img.className = "chat-bubble-attachment-image";
      img.src = convertFileSrc(item.path);
      img.alt = item.name;
      img.title = item.path;
      strip.appendChild(img);
    } else {
      const chip = document.createElement("span");
      chip.className = "chat-bubble-attachment-file";
      chip.textContent = item.name;
      chip.title = item.path;
      strip.appendChild(chip);
    }
  }
  return strip;
}

export function msgWrap(el: HTMLElement, role: "assistant" | "user"): HTMLElement {
  return el.closest(`.chat-msg-${role}`) ?? el;
}

export function assistantMsgWrap(bubble: HTMLElement): HTMLElement {
  return msgWrap(bubble, "assistant");
}

export function formatTime(ts: number): string {
  try {
    return new Intl.DateTimeFormat(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    }).format(new Date(ts));
  } catch {
    return "";
  }
}
