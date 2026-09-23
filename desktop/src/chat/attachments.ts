import { convertFileSrc } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { withErrorDetail } from "../friendly-error";
import { t } from "../i18n";
import { fileNameFromPath, isImagePath } from "./copy-ui";
import { uid } from "./store";
import { MAX_ATTACHMENTS, type ChatAttachment } from "./types";

export type AttachmentsDeps = {
  attachmentsEl: HTMLElement;
  composerBoxEl: HTMLElement;
  isComposerLocked: () => boolean;
  getPendingAttachments: () => ChatAttachment[];
  setPendingAttachments: (items: ChatAttachment[]) => void;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
};

export type AttachmentsApi = ReturnType<typeof createAttachmentsController>;

export function createAttachmentsController(deps: AttachmentsDeps) {
  function renderPendingAttachments(): void {
    const pending = deps.getPendingAttachments();
    deps.attachmentsEl.replaceChildren();
    deps.attachmentsEl.hidden = pending.length === 0;
    for (const item of pending) {
      const chip = document.createElement("div");
      chip.className = "chat-attach-chip";

      if (item.kind === "image") {
        const img = document.createElement("img");
        img.className = "chat-attach-thumb";
        img.alt = item.name;
        try {
          img.src = convertFileSrc(item.path);
          chip.appendChild(img);
        } catch {
          const icon = document.createElement("div");
          icon.className = "chat-attach-icon";
          icon.textContent = "IMG";
          chip.appendChild(icon);
        }
      } else {
        const icon = document.createElement("div");
        icon.className = "chat-attach-icon";
        icon.textContent = "FILE";
        chip.appendChild(icon);
      }

      const name = document.createElement("span");
      name.className = "chat-attach-name";
      name.textContent = item.name;
      name.title = item.path;

      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "chat-attach-remove";
      remove.setAttribute("aria-label", t("chat.attachRemove"));
      remove.textContent = "×";
      remove.addEventListener("click", () => {
        deps.setPendingAttachments(
          deps.getPendingAttachments().filter((a) => a.id !== item.id),
        );
        renderPendingAttachments();
      });

      chip.append(name, remove);
      deps.attachmentsEl.appendChild(chip);
    }
  }

  function addAttachmentPaths(paths: string[]): void {
    if (deps.isComposerLocked() || paths.length === 0) return;
    let pending = [...deps.getPendingAttachments()];
    let added = 0;
    for (const path of paths) {
      const trimmed = path.trim();
      if (!trimmed) continue;
      if (pending.some((a) => a.path === trimmed)) continue;
      if (pending.length >= MAX_ATTACHMENTS) {
        deps.setStatus(t("chat.attachLimit", { n: String(MAX_ATTACHMENTS) }), "warn");
        break;
      }
      const name = fileNameFromPath(trimmed);
      pending.push({
        id: uid(),
        path: trimmed,
        name,
        kind: isImagePath(trimmed) ? "image" : "file",
      });
      added += 1;
    }
    if (added > 0) {
      deps.setPendingAttachments(pending);
      renderPendingAttachments();
    }
  }

  async function pickAttachments(): Promise<void> {
    if (deps.isComposerLocked()) return;
    try {
      const selected = await open({
        multiple: true,
        title: t("chat.attachPick"),
      });
      if (!selected) return;
      const paths = (Array.isArray(selected) ? selected : [selected]).filter(Boolean);
      addAttachmentPaths(paths);
    } catch (error) {
      deps.setStatus(withErrorDetail(t("chat.attachFailed"), error), "error");
    }
  }

  function setComposerDropTarget(active: boolean): void {
    deps.composerBoxEl.classList.toggle(
      "is-drop-target",
      active && !deps.isComposerLocked(),
    );
  }

  async function setupFileDrop(): Promise<void> {
    try {
      await getCurrentWebview().onDragDropEvent((event) => {
        const type = event.payload.type;
        if (type === "enter" || type === "over") {
          setComposerDropTarget(true);
          return;
        }
        if (type === "leave") {
          setComposerDropTarget(false);
          return;
        }
        if (type === "drop") {
          setComposerDropTarget(false);
          addAttachmentPaths(event.payload.paths ?? []);
        }
      });
    } catch {
      /* browser preview without Tauri drag-drop */
    }
  }

  return {
    renderPendingAttachments,
    addAttachmentPaths,
    pickAttachments,
    setComposerDropTarget,
    setupFileDrop,
  };
}
