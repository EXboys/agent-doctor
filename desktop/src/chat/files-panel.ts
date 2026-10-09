import { escapeHtml } from "../markdown";
import { t } from "../i18n";
import { highlightCode } from "./code-highlight";
import {
  listWorkspaceDir,
  readWorkspaceFile,
  writeWorkspaceFile,
  type WorkspaceDirEntry,
} from "../ipc";

export type FilesPanelDeps = {
  mainEl: HTMLElement;
  toggleEl: HTMLButtonElement;
  panelEl: HTMLElement;
  listEl: HTMLElement;
  viewEl: HTMLElement;
  backEl: HTMLButtonElement;
  closeEl: HTMLButtonElement;
  breadcrumbEl: HTMLElement;
  fileNameEl: HTMLElement;
  languageEl: HTMLElement;
  placeholderEl: HTMLElement;
  previewEl: HTMLPreElement;
  editorEl: HTMLTextAreaElement;
  editEl: HTMLButtonElement;
  saveEl: HTMLButtonElement;
  emptyEl: HTMLElement;
  getRoot: () => string;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
};

type PanelMode = "closed" | "list" | "file";

export function createFilesPanel(deps: FilesPanelDeps) {
  let mode: PanelMode = "closed";
  let listRelative = "";
  let openFileRelative = "";
  let openFileLanguage = "text";
  let dirty = false;

  function rootOrWarn(): string | null {
    const root = deps.getRoot().trim();
    if (!root || root === "—") {
      deps.setStatus(t("chat.filesNoFolder"), "warn");
      return null;
    }
    return root;
  }

  function setOpen(open: boolean): void {
    mode = open ? "list" : "closed";
    deps.mainEl.classList.toggle("is-files-open", open);
    deps.toggleEl.classList.toggle("is-on", open);
    deps.toggleEl.setAttribute("aria-pressed", open ? "true" : "false");
    deps.panelEl.hidden = !open;
    deps.panelEl.setAttribute("aria-hidden", open ? "false" : "true");
  }

  function showList(): void {
    deps.listEl.hidden = false;
    deps.backEl.hidden = listRelative === "";
    void renderList();
  }

  function breadcrumbLabel(relative: string): string {
    if (!relative) return t("chat.filesHere");
    return relative.split("/").filter(Boolean).join(" / ");
  }

  async function renderList(): Promise<void> {
    const root = rootOrWarn();
    if (!root) {
      setOpen(false);
      return;
    }
    deps.breadcrumbEl.textContent = breadcrumbLabel(listRelative);
    for (const item of deps.listEl.querySelectorAll(".chat-files-item")) item.remove();
    deps.emptyEl.hidden = true;

    try {
      const entries = await listWorkspaceDir({ root, relative: listRelative || null });
      if (listRelative) {
        const up = document.createElement("button");
        up.type = "button";
        up.className = "chat-files-item is-dir";
        up.innerHTML = `<span class="chat-files-item-icon" aria-hidden="true">↩</span><span class="chat-files-item-name">${escapeHtml(t("chat.filesUp"))}</span>`;
        up.addEventListener("click", () => {
          const parts = listRelative.split("/").filter(Boolean);
          parts.pop();
          listRelative = parts.join("/");
          showList();
        });
        deps.listEl.append(up);
      }
      if (entries.length === 0) {
        deps.emptyEl.hidden = false;
        deps.emptyEl.textContent = t("chat.filesEmpty");
        return;
      }
      for (const entry of entries) {
        deps.listEl.append(renderEntry(entry));
      }
    } catch (error) {
      deps.setStatus(String(error ?? t("chat.filesLoadFailed")), "error");
    }
  }

  function renderEntry(entry: WorkspaceDirEntry): HTMLElement {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `chat-files-item${entry.isDir ? " is-dir" : " is-file"}`;
    btn.dataset.path = entry.relativePath;
    if (!entry.isDir && entry.relativePath === openFileRelative) {
      btn.classList.add("is-active");
    }
    const icon = entry.isDir
      ? `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M3.75 6.75A1.75 1.75 0 0 1 5.5 5h4.1l2 2.25h6.9A1.75 1.75 0 0 1 20.25 9v8A1.75 1.75 0 0 1 18.5 18.75h-13A1.75 1.75 0 0 1 3.75 17V6.75Z" stroke="currentColor" stroke-width="1.55" stroke-linejoin="round"/></svg>`
      : `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M6 3.75h7l5 5V20.25H6V3.75Z" stroke="currentColor" stroke-width="1.45" stroke-linejoin="round"/><path d="M13 3.75v5h5" stroke="currentColor" stroke-width="1.45" stroke-linejoin="round"/></svg>`;
    btn.innerHTML = `<span class="chat-files-item-icon" aria-hidden="true">${icon}</span><span class="chat-files-item-name">${escapeHtml(entry.name)}</span>${entry.isDir ? `<span class="chat-files-item-chevron" aria-hidden="true">›</span>` : ""}`;
    btn.addEventListener("click", () => {
      if (entry.isDir) {
        listRelative = entry.relativePath;
        showList();
        return;
      }
      void openFile(entry.relativePath, btn);
    });
    return btn;
  }

  async function openFile(relative: string, row?: HTMLElement): Promise<void> {
    const root = rootOrWarn();
    if (!root) return;
    try {
      const file = await readWorkspaceFile({ root, relative });
      openFileRelative = file.relativePath;
      openFileLanguage = file.language;
      dirty = false;
      deps.viewEl.hidden = false;
      deps.placeholderEl.hidden = true;
      deps.fileNameEl.textContent = file.name;
      deps.languageEl.textContent = file.language.toUpperCase();
      deps.breadcrumbEl.textContent = file.relativePath;
      deps.listEl.querySelectorAll(".chat-files-item.is-active").forEach((item) => {
        item.classList.remove("is-active");
      });
      row?.classList.add("is-active");

      const code = deps.previewEl.querySelector("code");
      if (code) {
        code.className = `language-${file.language}`;
        code.innerHTML = highlightCode(file.content, file.language);
      }
      deps.previewEl.hidden = false;
      deps.editorEl.hidden = true;
      deps.editorEl.value = file.content;
      deps.editorEl.className = `chat-files-editor language-${file.language}`;

      if (!file.editable) {
        deps.editEl.hidden = true;
        deps.saveEl.hidden = true;
        deps.previewEl.hidden = false;
        if (code) code.textContent = t("chat.filesBinary");
        return;
      }

      deps.editEl.hidden = false;
      deps.saveEl.hidden = true;
      deps.editEl.textContent = t("chat.filesEdit");
    } catch (error) {
      deps.setStatus(String(error ?? t("chat.filesLoadFailed")), "error");
    }
  }

  function enterEdit(): void {
    deps.previewEl.hidden = true;
    deps.editorEl.hidden = false;
    deps.editEl.hidden = true;
    deps.saveEl.hidden = false;
    deps.editorEl.focus();
  }

  async function saveFile(): Promise<void> {
    const root = rootOrWarn();
    if (!root || !openFileRelative) return;
    try {
      await writeWorkspaceFile({
        root,
        relative: openFileRelative,
        content: deps.editorEl.value,
      });
      dirty = false;
      const code = deps.previewEl.querySelector("code");
      if (code) {
        code.className = `language-${openFileLanguage}`;
        code.innerHTML = highlightCode(deps.editorEl.value, openFileLanguage);
      }
      deps.previewEl.hidden = false;
      deps.editorEl.hidden = true;
      deps.editEl.hidden = false;
      deps.saveEl.hidden = true;
      deps.setStatus(t("chat.filesSaved"), "ok");
    } catch (error) {
      deps.setStatus(String(error ?? t("chat.filesSaveFailed")), "error");
    }
  }

  deps.toggleEl.addEventListener("click", () => {
    if (mode !== "closed") {
      setOpen(false);
      return;
    }
    setOpen(true);
    showList();
  });

  deps.closeEl.addEventListener("click", () => setOpen(false));

  deps.backEl.addEventListener("click", () => {
    if (listRelative) {
      const parts = listRelative.split("/").filter(Boolean);
      parts.pop();
      listRelative = parts.join("/");
      showList();
    }
  });

  deps.editEl.addEventListener("click", () => enterEdit());
  deps.saveEl.addEventListener("click", () => void saveFile());
  deps.editorEl.addEventListener("input", () => {
    dirty = true;
  });
  deps.editorEl.addEventListener("keydown", (event) => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
      event.preventDefault();
      void saveFile();
      return;
    }
    if (event.key === "Tab") {
      event.preventDefault();
      const start = deps.editorEl.selectionStart;
      const end = deps.editorEl.selectionEnd;
      deps.editorEl.setRangeText("  ", start, end, "end");
      dirty = true;
    }
  });

  return {
    applyI18n() {
      deps.toggleEl.title = t("chat.filesToggleHint");
      deps.toggleEl.setAttribute("aria-label", t("chat.filesToggle"));
      deps.closeEl.textContent = t("chat.filesClose");
      deps.editEl.textContent = t("chat.filesEdit");
      deps.saveEl.textContent = t("chat.filesSave");
    },
    isOpen: () => mode !== "closed",
    close: () => setOpen(false),
    isDirty: () => dirty,
  };
}
